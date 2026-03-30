use serde::{Deserialize, Serialize};

/// Stored at:
///   Windows : %APPDATA%\clipygo-plugin-msteams\config.json
///   macOS   : ~/Library/Application Support/clipygo-plugin-msteams/config.json
///   Linux   : ~/.config/clipygo-plugin-msteams/config.json
#[derive(Serialize, Deserialize, Default, Clone)]
pub struct Config {
    #[serde(default)]
    pub tenant_id: String,
    #[serde(default)]
    pub client_id: String,
    /// "oauth2" (default), "password", or "device_code"
    #[serde(default = "default_auth_method")]
    pub auth_method: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub username: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub password: Option<String>,
    // Cached tokens — written back automatically
    #[serde(skip_serializing_if = "Option::is_none")]
    pub access_token: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub refresh_token: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub token_expiry: Option<u64>,
}

fn default_auth_method() -> String {
    "oauth2".to_string()
}

pub fn config_path() -> std::path::PathBuf {
    let dir = dirs::config_dir()
        .unwrap_or_else(|| std::path::PathBuf::from("."))
        .join("clipygo-plugin-msteams");
    let _ = std::fs::create_dir_all(&dir);
    dir.join("config.json")
}

pub fn load_config() -> Config {
    std::fs::read_to_string(config_path())
        .ok()
        .and_then(|data| serde_json::from_str(&data).ok())
        .unwrap_or_default()
}

pub fn save_config(config: &Config) {
    if let Ok(data) = serde_json::to_string_pretty(config) {
        let _ = std::fs::write(config_path(), data);
    }
}
