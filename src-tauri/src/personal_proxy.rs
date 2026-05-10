use crate::browser::ProxySettings;
use crate::proxy_manager::PROXY_MANAGER;
use crate::settings_manager::SettingsManager;
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct PersonalProxyServer {
  pub id: i64,
  pub name: String,
  pub country: String,
  pub flag: String,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct PersonalProxyConfig {
  pub id: i64,
  pub display_name: String,
  pub server_name: String,
  pub server_ip: String,
  pub port: u16,
  pub http_port: u16,
  pub username: String,
  pub password: String,
  pub created_at: String,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct PersonalSubscriptionInfo {
  pub has_subscription: bool,
  pub telegram_id: i64,
  pub proxy_count: Option<i32>,
  pub proxy_limit: Option<i32>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct PersonalProxySettings {
  pub api_url: Option<String>,
  pub api_key: Option<String>,
  pub telegram_id: Option<i64>,
}

fn get_api_settings() -> Result<(String, String, i64), String> {
  let settings = SettingsManager::instance()
    .load_settings()
    .map_err(|e| e.to_string())?;
  let api_url = settings
    .personal_proxy_api_url
    .ok_or_else(|| "Personal proxy API URL not configured".to_string())?;
  let api_key = settings
    .personal_proxy_api_key
    .ok_or_else(|| "Personal proxy API key not configured".to_string())?;
  let telegram_id = settings
    .personal_proxy_telegram_id
    .ok_or_else(|| "Telegram ID not configured".to_string())?;
  Ok((api_url, api_key, telegram_id))
}

async fn api_get(url: &str, api_key: &str) -> Result<String, String> {
  let client = reqwest::Client::new();
  let resp = client
    .get(url)
    .header("X-API-Key", api_key)
    .timeout(std::time::Duration::from_secs(15))
    .send()
    .await
    .map_err(|e| format!("Request failed: {e}"))?;
  if !resp.status().is_success() {
    return Err(format!("API error: {}", resp.status()));
  }
  resp.text().await.map_err(|e| e.to_string())
}

async fn api_post(url: &str, api_key: &str, body: serde_json::Value) -> Result<String, String> {
  let client = reqwest::Client::new();
  let resp = client
    .post(url)
    .header("X-API-Key", api_key)
    .json(&body)
    .timeout(std::time::Duration::from_secs(15))
    .send()
    .await
    .map_err(|e| format!("Request failed: {e}"))?;
  if !resp.status().is_success() {
    let status = resp.status();
    let body = resp.text().await.unwrap_or_default();
    return Err(format!("API error {status}: {body}"));
  }
  resp.text().await.map_err(|e| e.to_string())
}

async fn api_delete(url: &str, api_key: &str) -> Result<(), String> {
  let client = reqwest::Client::new();
  let resp = client
    .delete(url)
    .header("X-API-Key", api_key)
    .timeout(std::time::Duration::from_secs(15))
    .send()
    .await
    .map_err(|e| format!("Request failed: {e}"))?;
  if !resp.status().is_success() {
    return Err(format!("API error: {}", resp.status()));
  }
  Ok(())
}

#[tauri::command]
pub async fn personal_proxy_get_servers() -> Result<Vec<PersonalProxyServer>, String> {
  let (api_url, api_key, _) = get_api_settings()?;
  let url = format!("{}/donut/servers", api_url.trim_end_matches('/'));
  let body = api_get(&url, &api_key).await?;
  serde_json::from_str(&body).map_err(|e| format!("Parse error: {e}"))
}

#[tauri::command]
pub async fn personal_proxy_get_list() -> Result<Vec<PersonalProxyConfig>, String> {
  let (api_url, api_key, telegram_id) = get_api_settings()?;
  let url = format!(
    "{}/donut/proxies/{}",
    api_url.trim_end_matches('/'),
    telegram_id
  );
  let body = api_get(&url, &api_key).await?;
  serde_json::from_str(&body).map_err(|e| format!("Parse error: {e}"))
}

#[tauri::command]
pub async fn personal_proxy_check_subscription() -> Result<PersonalSubscriptionInfo, String> {
  let (api_url, api_key, telegram_id) = get_api_settings()?;
  let url = format!(
    "{}/donut/subscription/{}",
    api_url.trim_end_matches('/'),
    telegram_id
  );
  let body = api_get(&url, &api_key).await?;
  serde_json::from_str(&body).map_err(|e| format!("Parse error: {e}"))
}

#[tauri::command]
pub async fn personal_proxy_create(
  app_handle: tauri::AppHandle,
  server_id: i64,
  display_name: String,
) -> Result<String, String> {
  let (api_url, api_key, telegram_id) = get_api_settings()?;

  // Проверить подписку
  let sub_url = format!(
    "{}/donut/subscription/{}",
    api_url.trim_end_matches('/'),
    telegram_id
  );
  let sub_body = api_get(&sub_url, &api_key).await?;
  let sub: PersonalSubscriptionInfo =
    serde_json::from_str(&sub_body).map_err(|e| format!("Parse error: {e}"))?;
  if !sub.has_subscription {
    return Err("No active subscription".to_string());
  }

  // Создать прокси через бот API
  let create_url = format!(
    "{}/donut/proxies/{}",
    api_url.trim_end_matches('/'),
    telegram_id
  );
  let body = api_post(
    &create_url,
    &api_key,
    serde_json::json!({ "server_id": server_id, "display_name": display_name }),
  )
  .await?;

  let config: PersonalProxyConfig =
    serde_json::from_str(&body).map_err(|e| format!("Parse error: {e}"))?;

  // Добавить прокси в Cloud Browser
  let proxy_name = format!("Personal: {}", config.display_name);
  let proxy_settings = ProxySettings {
    proxy_type: "socks5".to_string(),
    host: config.server_ip.clone(),
    port: config.port,
    username: Some(config.username.clone()),
    password: Some(config.password.clone()),
  };

  PROXY_MANAGER
    .create_stored_proxy(&app_handle, proxy_name, proxy_settings)
    .map_err(|e| e.to_string())?;

  Ok(format!(
    "Proxy '{}' created successfully",
    config.display_name
  ))
}

#[tauri::command]
pub async fn personal_proxy_delete(config_id: i64) -> Result<(), String> {
  let (api_url, api_key, telegram_id) = get_api_settings()?;
  let url = format!(
    "{}/donut/proxies/{}/{}",
    api_url.trim_end_matches('/'),
    telegram_id,
    config_id
  );
  api_delete(&url, &api_key).await
}

#[tauri::command]
pub async fn personal_proxy_save_settings(
  api_url: Option<String>,
  api_key: Option<String>,
  telegram_id: Option<i64>,
) -> Result<(), String> {
  let mut settings = SettingsManager::instance()
    .load_settings()
    .map_err(|e| e.to_string())?;
  settings.personal_proxy_api_url = api_url;
  settings.personal_proxy_api_key = api_key;
  settings.personal_proxy_telegram_id = telegram_id;
  SettingsManager::instance()
    .save_settings(&settings)
    .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn personal_proxy_get_settings() -> Result<PersonalProxySettings, String> {
  let settings = SettingsManager::instance()
    .load_settings()
    .map_err(|e| e.to_string())?;
  Ok(PersonalProxySettings {
    api_url: settings.personal_proxy_api_url,
    api_key: settings.personal_proxy_api_key,
    telegram_id: settings.personal_proxy_telegram_id,
  })
}
