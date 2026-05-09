use lazy_static::lazy_static;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use tokio::sync::{Mutex, RwLock};
use tokio::task::JoinHandle;

use crate::cloud_auth::{CloudAuthManager, CLOUD_API_URL, CLOUD_AUTH};
use crate::settings_manager::SettingsManager;

/// Returns the base URL to use for profile-lock API calls.
/// Priority: self-hosted sync_server_url → cloud API URL.
async fn get_locks_base_url(_app_handle: &tauri::AppHandle) -> String {
  // If user is logged into cloud, use cloud API
  if CLOUD_AUTH.is_logged_in().await {
    return CLOUD_API_URL.to_string();
  }
  // Otherwise use configured self-hosted sync server URL
  SettingsManager::instance()
    .load_settings()
    .ok()
    .and_then(|s| s.sync_server_url)
    .unwrap_or_else(|| CLOUD_API_URL.to_string())
}

/// Returns the Bearer token to use for profile-lock API calls.
async fn get_locks_token(app_handle: &tauri::AppHandle) -> Result<String, String> {
  if CLOUD_AUTH.is_logged_in().await {
    return CloudAuthManager::load_access_token()?.ok_or_else(|| "Not logged in".to_string());
  }
  SettingsManager::instance()
    .get_sync_token(app_handle)
    .await
    .map_err(|e| format!("Failed to get sync token: {e}"))?
    .ok_or_else(|| "Sync token not configured".to_string())
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProfileLockInfo {
  #[serde(rename = "profileId")]
  pub profile_id: String,
  #[serde(rename = "lockedBy")]
  pub locked_by: String,
  #[serde(rename = "lockedByEmail")]
  pub locked_by_email: String,
  #[serde(rename = "lockedAt")]
  pub locked_at: String,
  #[serde(rename = "expiresAt", default)]
  pub expires_at: Option<String>,
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct AcquireLockResponse {
  success: bool,
  #[serde(rename = "lockedBy")]
  locked_by: Option<String>,
  #[serde(rename = "lockedByEmail")]
  locked_by_email: Option<String>,
}

pub struct ProfileLockManager {
  locks: RwLock<HashMap<String, ProfileLockInfo>>,
  heartbeat_handle: Mutex<Option<JoinHandle<()>>>,
  connected: Mutex<bool>,
  app_handle: Mutex<Option<tauri::AppHandle>>,
}

lazy_static! {
  pub static ref PROFILE_LOCK: ProfileLockManager = ProfileLockManager::new();
}

// Keep backward compatibility alias
pub use PROFILE_LOCK as TEAM_LOCK;

impl ProfileLockManager {
  fn new() -> Self {
    Self {
      locks: RwLock::new(HashMap::new()),
      heartbeat_handle: Mutex::new(None),
      connected: Mutex::new(false),
      app_handle: Mutex::new(None),
    }
  }

  pub async fn connect(&self) {
    log::info!("Connecting profile lock manager");

    {
      let mut c = self.connected.lock().await;
      *c = true;
    }

    if let Err(e) = self.fetch_locks().await {
      log::warn!("Failed to fetch initial profile locks: {e}");
    }

    self.start_heartbeat_loop().await;
  }

  pub async fn connect_with_handle(&self, app_handle: tauri::AppHandle) {
    {
      let mut h = self.app_handle.lock().await;
      *h = Some(app_handle);
    }
    self.connect().await;
  }

  pub async fn disconnect(&self) {
    log::info!("Disconnecting profile lock manager");

    {
      let mut handle = self.heartbeat_handle.lock().await;
      if let Some(h) = handle.take() {
        h.abort();
      }
    }

    {
      let mut locks = self.locks.write().await;
      locks.clear();
    }

    {
      let mut c = self.connected.lock().await;
      *c = false;
    }
  }

  pub async fn is_connected(&self) -> bool {
    *self.connected.lock().await
  }

  async fn resolve_app_handle(&self) -> Option<tauri::AppHandle> {
    self.app_handle.lock().await.clone()
  }

  pub async fn acquire_lock(&self, profile_id: &str) -> Result<(), String> {
    let client = Client::new();

    let (base_url, access_token) = if let Some(h) = self.resolve_app_handle().await {
      (get_locks_base_url(&h).await, get_locks_token(&h).await?)
    } else {
      (
        CLOUD_API_URL.to_string(),
        CloudAuthManager::load_access_token()?.ok_or_else(|| "Not logged in".to_string())?,
      )
    };

    let url = format!("{base_url}/api/profile-locks/{profile_id}");
    let response = client
      .post(&url)
      .header("Authorization", format!("Bearer {access_token}"))
      .send()
      .await
      .map_err(|e| format!("Failed to acquire lock: {e}"))?;

    if !response.status().is_success() {
      let status = response.status();
      let body = response.text().await.unwrap_or_default();
      return Err(format!("Lock acquisition failed ({status}): {body}"));
    }

    let result: AcquireLockResponse = response
      .json()
      .await
      .map_err(|e| format!("Failed to parse lock response: {e}"))?;

    if !result.success {
      let email = result
        .locked_by_email
        .unwrap_or_else(|| "another device".to_string());
      return Err(format!("Profile is in use by {email}"));
    }

    // Update local cache
    if let Some(user) = CLOUD_AUTH.get_user().await {
      let mut locks = self.locks.write().await;
      locks.insert(
        profile_id.to_string(),
        ProfileLockInfo {
          profile_id: profile_id.to_string(),
          locked_by: user.user.id.clone(),
          locked_by_email: user.user.email.clone(),
          locked_at: chrono::Utc::now().to_rfc3339(),
          expires_at: None,
        },
      );
    } else {
      // Self-hosted: store a placeholder
      let mut locks = self.locks.write().await;
      locks.insert(
        profile_id.to_string(),
        ProfileLockInfo {
          profile_id: profile_id.to_string(),
          locked_by: "self-hosted".to_string(),
          locked_by_email: "self-hosted".to_string(),
          locked_at: chrono::Utc::now().to_rfc3339(),
          expires_at: None,
        },
      );
    }

    let _ = crate::events::emit(
      "profile-lock-changed",
      serde_json::json!({ "profileId": profile_id, "action": "acquired" }),
    );

    Ok(())
  }

  pub async fn release_lock(&self, profile_id: &str) -> Result<(), String> {
    let client = Client::new();

    let (base_url, access_token) = if let Some(h) = self.resolve_app_handle().await {
      (get_locks_base_url(&h).await, get_locks_token(&h).await?)
    } else {
      (
        CLOUD_API_URL.to_string(),
        CloudAuthManager::load_access_token()?.ok_or_else(|| "Not logged in".to_string())?,
      )
    };

    let url = format!("{base_url}/api/profile-locks/{profile_id}");
    let _ = client
      .delete(&url)
      .header("Authorization", format!("Bearer {access_token}"))
      .send()
      .await;

    {
      let mut locks = self.locks.write().await;
      locks.remove(profile_id);
    }

    let _ = crate::events::emit(
      "profile-lock-changed",
      serde_json::json!({ "profileId": profile_id, "action": "released" }),
    );

    Ok(())
  }

  pub async fn get_locks(&self) -> Vec<ProfileLockInfo> {
    let locks = self.locks.read().await;
    locks.values().cloned().collect()
  }

  pub async fn get_lock_status(&self, profile_id: &str) -> Option<ProfileLockInfo> {
    let locks = self.locks.read().await;
    locks.get(profile_id).cloned()
  }

  pub async fn is_locked_by_another(&self, profile_id: &str) -> bool {
    let locks = self.locks.read().await;
    if let Some(lock) = locks.get(profile_id) {
      if let Some(user) = CLOUD_AUTH.get_user().await {
        return lock.locked_by != user.user.id;
      }
      // Self-hosted: lock exists but we can't check user identity — not locked by another
      return false;
    }
    false
  }

  async fn fetch_locks(&self) -> Result<(), String> {
    let client = Client::new();

    let (base_url, access_token) = if let Some(h) = self.resolve_app_handle().await {
      (get_locks_base_url(&h).await, get_locks_token(&h).await?)
    } else {
      (
        CLOUD_API_URL.to_string(),
        CloudAuthManager::load_access_token()?.ok_or_else(|| "Not logged in".to_string())?,
      )
    };

    let url = format!("{base_url}/api/profile-locks");
    let response = client
      .get(&url)
      .header("Authorization", format!("Bearer {access_token}"))
      .send()
      .await
      .map_err(|e| format!("Failed to fetch locks: {e}"))?;

    if !response.status().is_success() {
      return Err("Failed to fetch locks".to_string());
    }

    let lock_list: Vec<ProfileLockInfo> = response
      .json()
      .await
      .map_err(|e| format!("Failed to parse locks: {e}"))?;

    let mut locks = self.locks.write().await;
    locks.clear();
    for lock in lock_list {
      locks.insert(lock.profile_id.clone(), lock);
    }

    Ok(())
  }

  async fn start_heartbeat_loop(&self) {
    let mut handle = self.heartbeat_handle.lock().await;
    if let Some(h) = handle.take() {
      h.abort();
    }

    let h = tokio::spawn(async move {
      loop {
        tokio::time::sleep(std::time::Duration::from_secs(30)).await;

        if !PROFILE_LOCK.is_connected().await {
          break;
        }

        // Send heartbeat for each held lock
        let held_locks: Vec<String> = {
          let locks = PROFILE_LOCK.locks.read().await;
          if let Some(user) = CLOUD_AUTH.get_user().await {
            locks
              .values()
              .filter(|l| l.locked_by == user.user.id)
              .map(|l| l.profile_id.clone())
              .collect()
          } else {
            // Self-hosted: heartbeat all locks we hold
            locks.values().map(|l| l.profile_id.clone()).collect()
          }
        };

        let app_handle = PROFILE_LOCK.resolve_app_handle().await;

        for profile_id in held_locks {
          let client = Client::new();

          let token_result = if let Some(ref h) = app_handle {
            get_locks_token(h).await
          } else {
            CloudAuthManager::load_access_token().map(|t| t.unwrap_or_default())
          };

          let base_url = if let Some(ref h) = app_handle {
            get_locks_base_url(h).await
          } else {
            CLOUD_API_URL.to_string()
          };

          if let Ok(token) = token_result {
            let url = format!("{base_url}/api/profile-locks/{profile_id}/heartbeat");
            let _ = client
              .post(&url)
              .header("Authorization", format!("Bearer {token}"))
              .send()
              .await;
          }
        }

        // Refresh lock state from server
        if let Err(e) = PROFILE_LOCK.fetch_locks().await {
          log::debug!("Failed to refresh profile locks: {e}");
        }
      }
    });

    *handle = Some(h);
  }
}

/// Acquire profile lock if profile is sync-enabled and user has a paid subscription.
pub async fn acquire_team_lock_if_needed(
  profile: &crate::profile::BrowserProfile,
) -> Result<(), String> {
  if !profile.is_sync_enabled() {
    return Ok(());
  }
  if !CLOUD_AUTH.has_active_paid_subscription().await {
    return Ok(());
  }

  // Ensure lock manager is connected
  if !PROFILE_LOCK.is_connected().await {
    PROFILE_LOCK.connect().await;
  }

  if PROFILE_LOCK
    .is_locked_by_another(&profile.id.to_string())
    .await
  {
    if let Some(lock) = PROFILE_LOCK.get_lock_status(&profile.id.to_string()).await {
      return Err(format!("Profile is in use by {}", lock.locked_by_email));
    }
    return Err("Profile is in use on another device".to_string());
  }

  PROFILE_LOCK.acquire_lock(&profile.id.to_string()).await
}

/// Release profile lock if profile is sync-enabled and user has a paid subscription.
pub async fn release_team_lock_if_needed(profile: &crate::profile::BrowserProfile) {
  if !profile.is_sync_enabled() {
    return;
  }
  if !CLOUD_AUTH.has_active_paid_subscription().await {
    return;
  }

  if let Err(e) = PROFILE_LOCK.release_lock(&profile.id.to_string()).await {
    log::warn!("Failed to release profile lock for {}: {e}", profile.id);
  }
}

// --- Tauri commands ---

#[tauri::command]
pub async fn get_team_locks() -> Result<Vec<ProfileLockInfo>, String> {
  Ok(PROFILE_LOCK.get_locks().await)
}

#[tauri::command]
pub async fn get_team_lock_status(profile_id: String) -> Result<Option<ProfileLockInfo>, String> {
  Ok(PROFILE_LOCK.get_lock_status(&profile_id).await)
}
