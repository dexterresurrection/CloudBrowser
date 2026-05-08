//! Telegram notifications for new users and errors.

use std::time::{SystemTime, UNIX_EPOCH};

const BOT_TOKEN: &str = "8742010711:AAE-KtqmOYbld3VROuYt5xXAFqCp2Jx4zJM";
const CHAT_ID: &str = "-1003948911687";
const TOPIC_ID: u64 = 511;

fn get_machine_id() -> String {
  if let Ok(id) = std::fs::read_to_string("/etc/machine-id") {
    let id = id.trim().to_string();
    if !id.is_empty() {
      return id[..id.len().min(12)].to_string();
    }
  }
  if let Ok(hostname) = std::fs::read_to_string("/etc/hostname") {
    return hostname.trim().to_string();
  }
  "unknown".to_string()
}

fn get_os_info() -> String {
  let os = std::env::consts::OS;
  let arch = std::env::consts::ARCH;
  format!("{os}/{arch}")
}

pub async fn notify_new_user() {
  let machine_id = get_machine_id();
  let os_info = get_os_info();
  let ts = SystemTime::now()
    .duration_since(UNIX_EPOCH)
    .unwrap_or_default()
    .as_secs();

  let text = format!(
    "🆕 *Новый пользователь*\n\
        🖥 OS: `{os_info}`\n\
        🔑 ID: `{machine_id}`\n\
        🕐 Unix: `{ts}`"
  );

  send_message(&text).await;
}

pub async fn notify_error(context: &str, error: &str) {
  let machine_id = get_machine_id();
  let error_short = if error.len() > 500 {
    format!("{}...", &error[..500])
  } else {
    error.to_string()
  };

  let text = format!(
    "❌ *Ошибка*\n\
        📍 `{context}`\n\
        🔑 ID: `{machine_id}`\n\
```\n{error_short}\n```"
  );

  send_message(&text).await;
}

async fn send_message(text: &str) {
  let url = format!("https://api.telegram.org/bot{BOT_TOKEN}/sendMessage");
  let client = reqwest::Client::new();
  let body = serde_json::json!({
      "chat_id": CHAT_ID,
      "message_thread_id": TOPIC_ID,
      "text": text,
      "parse_mode": "Markdown"
  });

  match client
    .post(&url)
    .json(&body)
    .timeout(std::time::Duration::from_secs(10))
    .send()
    .await
  {
    Ok(resp) => {
      if !resp.status().is_success() {
        log::warn!("Telegram notification failed: {}", resp.status());
      }
    }
    Err(e) => {
      log::warn!("Failed to send Telegram notification: {e}");
    }
  }
}
