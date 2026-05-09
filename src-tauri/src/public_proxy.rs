/// Public proxy manager — reads proxy files from /proxy_folder/ on disk.
/// Replaces cloud_get_countries / cloud_get_regions / cloud_get_cities /
/// cloud_get_isps / create_cloud_location_proxy with local file-based logic.
///
/// Directory layout expected:
///   by_country/<CountryName>/<type>_clean_final_<cc>.txt
///   by_country_anonymity/<CountryName>/<type>-<anon>_<cc>.txt
///
/// Each file: one proxy per line, format  ip:port  (may have blank/broken lines)
use rand::seq::IndexedRandom;
use serde::{Deserialize, Serialize};
use std::fs;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};

/// Mirrors cloud_auth::LocationItem — frontend needs zero changes.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LocationItem {
  pub code: String,
  pub name: String,
}

/// Where proxy files live. Override with env var PROXY_FOLDER_PATH.
fn proxy_folder() -> PathBuf {
  std::env::var("PROXY_FOLDER_PATH")
    .map(PathBuf::from)
    .unwrap_or_else(|_| PathBuf::from("/root/proxy_folder"))
}

// ── file helpers ──────────────────────────────────────────────────────────────

fn read_proxy_lines(path: &Path) -> Vec<String> {
  let file = match fs::File::open(path) {
    Ok(f) => f,
    Err(_) => return vec![],
  };
  BufReader::new(file)
    .lines()
    .map_while(Result::ok)
    .map(|l| l.trim().to_string())
    .filter(|l| is_valid_proxy_line(l))
    .collect()
}

fn is_valid_proxy_line(line: &str) -> bool {
  if line.is_empty() || line.starts_with('#') || line.starts_with("//") {
    return false;
  }
  let parts: Vec<&str> = line.split(':').collect();
  if parts.len() < 2 {
    return false;
  }
  parts.last().unwrap_or(&"").parse::<u16>().is_ok()
}

fn find_first_proxy_file(dir: &Path) -> Option<PathBuf> {
  fs::read_dir(dir)
    .ok()?
    .filter_map(|e| e.ok())
    .find(|e| e.path().extension().map(|x| x == "txt").unwrap_or(false))
    .map(|e| e.path())
}

fn detect_proxy_type_from_path(path: &Path) -> String {
  let name = path
    .file_name()
    .unwrap_or_default()
    .to_string_lossy()
    .to_lowercase();
  if name.contains("socks5") {
    "socks5".to_string()
  } else if name.contains("socks4") {
    "socks4".to_string()
  } else if name.contains("https") {
    "https".to_string()
  } else {
    "http".to_string()
  }
}

fn parse_proxy_line(line: &str, path: &Path) -> Result<(String, u16, String), String> {
  let parts: Vec<&str> = line.split(':').collect();
  if parts.len() < 2 {
    return Err(format!("Malformed proxy line: {}", line));
  }
  let host = parts[0].to_string();
  let port: u16 = parts[1]
    .parse()
    .map_err(|_| format!("Invalid port: {}", line))?;
  let proxy_type = detect_proxy_type_from_path(path);
  Ok((host, port, proxy_type))
}

// ── discovery ─────────────────────────────────────────────────────────────────

pub fn list_countries() -> Vec<LocationItem> {
  let dir = proxy_folder().join("by_country");
  let mut countries: Vec<LocationItem> = fs::read_dir(&dir)
    .into_iter()
    .flatten()
    .filter_map(|e| e.ok())
    .filter(|e| e.path().is_dir())
    .map(|e| {
      let name = e.file_name().to_string_lossy().to_string();
      LocationItem {
        code: name.clone(),
        name: format!("{} {}", country_flag(&name), format_country_display(&name)),
      }
    })
    .collect();
  countries.sort_by(|a, b| a.name.cmp(&b.name));
  countries
}

/// Regions: public proxies have no region data — return empty so UI stays enabled.
pub fn list_regions(_country: &str) -> Vec<LocationItem> {
  vec![]
}

/// Cities: protocol/quality variants available for a country.
pub fn list_cities(country: &str, _region: Option<&str>) -> Vec<LocationItem> {
  let dir = proxy_folder().join("by_country").join(country);
  if !dir.exists() {
    return vec![];
  }
  let mut items: Vec<LocationItem> = fs::read_dir(&dir)
    .into_iter()
    .flatten()
    .filter_map(|e| e.ok())
    .filter(|e| e.path().extension().map(|x| x == "txt").unwrap_or(false))
    .map(|e| {
      let fname = e.file_name().to_string_lossy().to_string();
      let code = fname.trim_end_matches(".txt").to_string();
      let label = protocol_label_from_filename(&fname);
      LocationItem { code, name: label }
    })
    .collect();
  items.sort_by(|a, b| a.name.cmp(&b.name));
  items
}

/// ISPs: anonymity variants for a country.
pub fn list_isps(country: &str, _region: Option<&str>, _city: Option<&str>) -> Vec<LocationItem> {
  let dir = proxy_folder().join("by_country_anonymity").join(country);
  if !dir.exists() {
    return vec![];
  }
  let mut items: Vec<LocationItem> = fs::read_dir(&dir)
    .into_iter()
    .flatten()
    .filter_map(|e| e.ok())
    .filter(|e| e.path().extension().map(|x| x == "txt").unwrap_or(false))
    .map(|e| {
      let fname = e.file_name().to_string_lossy().to_string();
      let code = fname.trim_end_matches(".txt").to_string();
      let label = anonymity_label_from_filename(&fname);
      LocationItem { code, name: label }
    })
    .collect();
  items.sort_by(|a, b| a.name.cmp(&b.name));
  items
}

// ── proxy picking ─────────────────────────────────────────────────────────────

pub fn pick_proxy(
  country: &str,
  city: Option<&str>,
  isp: Option<&str>,
) -> Result<(String, u16, String), String> {
  let base = proxy_folder();

  let path: PathBuf = if let Some(isp_code) = isp.filter(|s| !s.is_empty()) {
    base
      .join("by_country_anonymity")
      .join(country)
      .join(format!("{}.txt", isp_code))
  } else if let Some(city_code) = city.filter(|s| !s.is_empty()) {
    base
      .join("by_country")
      .join(country)
      .join(format!("{}.txt", city_code))
  } else {
    find_first_proxy_file(&base.join("by_country").join(country))
      .ok_or_else(|| format!("No proxy files found for country: {}", country))?
  };

  if !path.exists() {
    return Err(format!("Proxy file not found: {}", path.display()));
  }

  let lines = read_proxy_lines(&path);
  if lines.is_empty() {
    return Err(format!("No valid proxies in: {}", path.display()));
  }

  let line = lines.choose(&mut rand::rng()).unwrap();
  parse_proxy_line(line, &path)
}

// ── display helpers ───────────────────────────────────────────────────────────

fn format_country_display(folder_name: &str) -> String {
  folder_name
    .chars()
    .enumerate()
    .map(|(i, c)| {
      if i > 0 && c.is_uppercase() {
        format!(" {}", c)
      } else {
        c.to_string()
      }
    })
    .collect()
}

fn protocol_label_from_filename(fname: &str) -> String {
  let f = fname.to_lowercase();
  let proto = if f.starts_with("socks5") {
    "SOCKS5"
  } else if f.starts_with("socks4") {
    "SOCKS4"
  } else if f.starts_with("https") {
    "HTTPS"
  } else {
    "HTTP"
  };
  let quality = if f.contains("premium") {
    " Premium"
  } else if f.contains("good") {
    " Good"
  } else if f.contains("browser") {
    " Browser"
  } else if f.contains("clean") {
    " Clean"
  } else {
    ""
  };
  format!("{}{}", proto, quality)
}

fn anonymity_label_from_filename(fname: &str) -> String {
  let f = fname.to_lowercase();
  let proto = if f.contains("socks5") {
    "SOCKS5"
  } else if f.contains("socks4") {
    "SOCKS4"
  } else if f.contains("https") {
    "HTTPS"
  } else {
    "HTTP"
  };
  let anon = if f.contains("elite") {
    "Elite"
  } else if f.contains("anonymous") {
    "Anonymous"
  } else if f.contains("transparent") {
    "Transparent"
  } else {
    ""
  };
  if anon.is_empty() {
    proto.to_string()
  } else {
    format!("{} — {}", proto, anon)
  }
}

fn country_flag(name: &str) -> &'static str {
  match name {
    "Austria" => "🇦🇹",
    "China" => "🇨🇳",
    "Colombia" => "🇨🇴",
    "Estonia" => "🇪🇪",
    "Finland" => "🇫🇮",
    "France" => "🇫🇷",
    "Germany" => "🇩🇪",
    "HongKong" => "🇭🇰",
    "India" => "🇮🇳",
    "Japan" => "🇯🇵",
    "Kazakhstan" => "🇰🇿",
    "Moldova" => "🇲🇩",
    "Netherlands" => "🇳🇱",
    "Russia" => "🇷🇺",
    "SouthKorea" => "🇰🇷",
    "UAE" => "🇦🇪",
    "UK" => "🇬🇧",
    "USA" => "🇺🇸",
    "Vietnam" => "🇻🇳",
    _ => "🌐",
  }
}

// ── Tauri commands ─────────────────────────────────────────────────────────────
// These replace the cloud_auth:: versions with identical signatures
// so lib.rs just swaps the module prefix.

#[tauri::command]
pub fn cloud_get_countries() -> Result<Vec<LocationItem>, String> {
  Ok(list_countries())
}

#[tauri::command]
pub fn cloud_get_regions(country: String) -> Result<Vec<LocationItem>, String> {
  Ok(list_regions(&country))
}

#[tauri::command]
#[allow(unused_variables)]
pub fn cloud_get_cities(
  country: String,
  region: Option<String>,
) -> Result<Vec<LocationItem>, String> {
  Ok(list_cities(&country, region.as_deref()))
}

#[tauri::command]
#[allow(unused_variables)]
pub fn cloud_get_isps(
  country: String,
  region: Option<String>,
  city: Option<String>,
) -> Result<Vec<LocationItem>, String> {
  Ok(list_isps(&country, region.as_deref(), city.as_deref()))
}

#[tauri::command]
pub fn create_cloud_location_proxy(
  app_handle: tauri::AppHandle,
  name: String,
  country: String,
  _region: Option<String>,
  city: Option<String>,
  isp: Option<String>,
) -> Result<crate::proxy_manager::StoredProxy, String> {
  use crate::browser::ProxySettings;

  let (host, port, proxy_type) = pick_proxy(&country, city.as_deref(), isp.as_deref())?;

  let proxy_settings = ProxySettings {
    proxy_type,
    host,
    port,
    username: None,
    password: None,
  };

  crate::proxy_manager::PROXY_MANAGER.create_stored_proxy(&app_handle, name, proxy_settings)
}
