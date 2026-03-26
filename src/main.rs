use std::io::{self, BufRead, Read, Write};
use std::net::TcpListener;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

const SCOPES: &str = "https://graph.microsoft.com/Team.ReadBasic.All \
    https://graph.microsoft.com/Channel.ReadBasic.All \
    https://graph.microsoft.com/ChannelMessage.Send \
    https://graph.microsoft.com/Chat.ReadBasic \
    https://graph.microsoft.com/ChatMessage.Send \
    offline_access";

// Placeholder Teams icon (purple 1×1 PNG)
const TEAMS_ICON: &str =
    "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAAC0lEQVQI12NgAAIABQAABjE+ibYAAAAASUVORK5CYII=";

// --- Protocol types ---

#[derive(Deserialize)]
#[serde(tag = "command", rename_all = "snake_case")]
enum Request {
    GetInfo,
    GetTargets,
    GetConfigSchema,
    SetConfig {
        values: serde_json::Value,
    },
    Send {
        target_id: String,
        content: String,
        format: String,
    },
}

#[derive(Serialize)]
struct InfoResponse {
    name: &'static str,
    version: &'static str,
    description: &'static str,
    author: &'static str,
}

#[derive(Serialize, Clone)]
struct Target {
    id: String,
    provider: String,
    formats: Vec<String>,
    title: String,
    description: String,
    image: String,
}

#[derive(Serialize)]
struct TargetsResponse {
    targets: Vec<Target>,
}

#[derive(Serialize)]
struct SendResponse {
    success: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
}

// --- Config ---

/// Stored at:
///   Windows : %APPDATA%\clipygo-plugin-msteams\config.json
///   macOS   : ~/Library/Application Support/clipygo-plugin-msteams/config.json
///   Linux   : ~/.config/clipygo-plugin-msteams/config.json
///
/// Minimal example (OAuth2):
/// ```json
/// { "tenant_id": "<AAD tenant id>", "client_id": "<app registration id>" }
/// ```
///
/// Password (ROPC) example:
/// ```json
/// {
///   "tenant_id": "...", "client_id": "...",
///   "auth_method": "password",
///   "username": "user@example.com", "password": "secret"
/// }
/// ```
#[derive(Serialize, Deserialize, Default, Clone)]
struct Config {
    #[serde(default)]
    tenant_id: String,
    #[serde(default)]
    client_id: String,
    /// "oauth2" (default) or "password"
    #[serde(default = "default_auth_method")]
    auth_method: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    username: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    password: Option<String>,
    // Cached tokens — written back automatically
    #[serde(skip_serializing_if = "Option::is_none")]
    access_token: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    refresh_token: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    token_expiry: Option<u64>,
}

fn default_auth_method() -> String {
    "oauth2".to_string()
}

fn config_path() -> std::path::PathBuf {
    let dir = dirs::config_dir()
        .unwrap_or_else(|| std::path::PathBuf::from("."))
        .join("clipygo-plugin-msteams");
    let _ = std::fs::create_dir_all(&dir);
    dir.join("config.json")
}

fn load_config() -> Config {
    std::fs::read_to_string(config_path())
        .ok()
        .and_then(|data| serde_json::from_str(&data).ok())
        .unwrap_or_default()
}

fn save_config(config: &Config) {
    if let Ok(data) = serde_json::to_string_pretty(config) {
        let _ = std::fs::write(config_path(), data);
    }
}

// --- Token management ---

#[derive(Deserialize)]
struct TokenResponse {
    access_token: String,
    refresh_token: Option<String>,
    expires_in: u64,
}

fn now_unix() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn get_valid_token(config: &mut Config) -> Result<String, String> {
    // Return cached token if still valid (60 s buffer)
    if let (Some(token), Some(expiry)) = (&config.access_token, config.token_expiry) {
        if now_unix() + 60 < expiry {
            return Ok(token.clone());
        }
    }

    // Try refresh
    if let Some(refresh) = config.refresh_token.clone() {
        match do_refresh(config, &refresh) {
            Ok(tr) => {
                apply_tokens(config, tr);
                save_config(config);
                return Ok(config.access_token.clone().unwrap());
            }
            Err(e) => {
                eprintln!("[msteams] Token refresh failed ({e}), re-authenticating…");
                config.access_token = None;
                config.refresh_token = None;
                config.token_expiry = None;
            }
        }
    }

    // Full authentication
    let tr = match config.auth_method.as_str() {
        "password" => auth_password(config)?,
        _ => auth_oauth2(config)?,
    };
    apply_tokens(config, tr);
    save_config(config);
    Ok(config.access_token.clone().unwrap())
}

fn apply_tokens(config: &mut Config, tr: TokenResponse) {
    config.token_expiry = Some(now_unix() + tr.expires_in);
    if let Some(rt) = tr.refresh_token {
        config.refresh_token = Some(rt);
    }
    config.access_token = Some(tr.access_token);
}

fn token_url(tenant_id: &str) -> String {
    format!("https://login.microsoftonline.com/{tenant_id}/oauth2/v2.0/token")
}

fn auth_password(config: &Config) -> Result<TokenResponse, String> {
    let username = config
        .username
        .as_deref()
        .ok_or("username not set in config")?;
    let password = config
        .password
        .as_deref()
        .ok_or("password not set in config")?;

    let resp = http_client()?
        .post(token_url(&config.tenant_id))
        .form(&[
            ("grant_type", "password"),
            ("client_id", config.client_id.as_str()),
            ("username", username),
            ("password", password),
            ("scope", SCOPES),
        ])
        .send()
        .map_err(|e| format!("Auth request failed: {e}"))?;

    parse_token_response(resp)
}

fn auth_oauth2(config: &Config) -> Result<TokenResponse, String> {
    let listener = TcpListener::bind("127.0.0.1:0").map_err(|e| format!("Failed to bind: {e}"))?;
    let port = listener.local_addr().map_err(|e| e.to_string())?.port();
    let redirect_uri = format!("http://localhost:{port}/");

    let auth_url = format!(
        "https://login.microsoftonline.com/{}/oauth2/v2.0/authorize\
         ?client_id={}\
         &response_type=code\
         &redirect_uri={}\
         &scope={}\
         &response_mode=query",
        config.tenant_id,
        config.client_id,
        urlencoding::encode(&redirect_uri),
        urlencoding::encode(SCOPES),
    );

    eprintln!("[msteams] Opening browser for authentication…");
    open_url(&auth_url)?;
    eprintln!("[msteams] Waiting for callback on port {port}…");

    let (mut stream, _) = listener
        .accept()
        .map_err(|e| format!("Callback accept failed: {e}"))?;
    let _ = stream.set_read_timeout(Some(Duration::from_secs(120)));

    let code = extract_code(&mut stream)?;

    let _ = stream.write_all(
        b"HTTP/1.1 200 OK\r\nContent-Type: text/html\r\n\r\n\
          <html><body><h2>Authenticated! You can close this tab.</h2></body></html>",
    );

    let resp = http_client()?
        .post(token_url(&config.tenant_id))
        .form(&[
            ("grant_type", "authorization_code"),
            ("client_id", config.client_id.as_str()),
            ("code", code.as_str()),
            ("redirect_uri", redirect_uri.as_str()),
            ("scope", SCOPES),
        ])
        .send()
        .map_err(|e| format!("Token exchange failed: {e}"))?;

    parse_token_response(resp)
}

fn do_refresh(config: &Config, refresh_token: &str) -> Result<TokenResponse, String> {
    let resp = http_client()?
        .post(token_url(&config.tenant_id))
        .form(&[
            ("grant_type", "refresh_token"),
            ("client_id", config.client_id.as_str()),
            ("refresh_token", refresh_token),
            ("scope", SCOPES),
        ])
        .send()
        .map_err(|e| e.to_string())?;

    parse_token_response(resp)
}

fn parse_token_response(resp: reqwest::blocking::Response) -> Result<TokenResponse, String> {
    if resp.status().is_success() {
        resp.json::<TokenResponse>()
            .map_err(|e| format!("Invalid token response: {e}"))
    } else {
        let status = resp.status();
        let body = resp.text().unwrap_or_default();
        Err(format!("Token error {status}: {body}"))
    }
}

fn extract_code(stream: &mut std::net::TcpStream) -> Result<String, String> {
    let mut buf = [0u8; 8192];
    let n = stream.read(&mut buf).map_err(|e| e.to_string())?;
    let text =
        std::str::from_utf8(&buf[..n]).map_err(|_| "Invalid UTF-8 in request".to_string())?;

    // First line: "GET /?code=M.xxx&session_state=yyy HTTP/1.1"
    let first_line = text.lines().next().unwrap_or("");
    let path = first_line.split_whitespace().nth(1).unwrap_or("");
    let query = path.splitn(2, '?').nth(1).unwrap_or("");

    for pair in query.split('&') {
        let Some((key, val)) = pair.split_once('=') else {
            continue;
        };
        match key {
            "error_description" => {
                return Err(format!(
                    "OAuth error: {}",
                    urlencoding::decode(val).unwrap_or_default()
                ));
            }
            "code" => {
                return urlencoding::decode(val)
                    .map(|s| s.into_owned())
                    .map_err(|e| e.to_string());
            }
            _ => {}
        }
    }

    Err("No authorization code in callback".to_string())
}

fn open_url(url: &str) -> Result<(), String> {
    #[cfg(target_os = "windows")]
    std::process::Command::new("cmd")
        .args(["/c", "start", "", url])
        .spawn()
        .map_err(|e| format!("Failed to open browser: {e}"))?;

    #[cfg(target_os = "macos")]
    std::process::Command::new("open")
        .arg(url)
        .spawn()
        .map_err(|e| format!("Failed to open browser: {e}"))?;

    #[cfg(target_os = "linux")]
    std::process::Command::new("xdg-open")
        .arg(url)
        .spawn()
        .map_err(|e| format!("Failed to open browser: {e}"))?;

    Ok(())
}

fn http_client() -> Result<reqwest::blocking::Client, String> {
    reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(30))
        .build()
        .map_err(|e| format!("Failed to build HTTP client: {e}"))
}

// --- Graph API types ---

#[derive(Deserialize)]
struct GraphList<T> {
    value: Vec<T>,
}

#[derive(Deserialize)]
struct GraphTeam {
    id: String,
    #[serde(rename = "displayName")]
    display_name: String,
}

#[derive(Deserialize)]
struct GraphChannel {
    id: String,
    #[serde(rename = "displayName")]
    display_name: String,
}

#[derive(Deserialize)]
struct GraphChat {
    id: String,
    #[serde(rename = "chatType")]
    chat_type: String,
    topic: Option<String>,
    members: Option<Vec<GraphChatMember>>,
}

#[derive(Deserialize)]
struct GraphChatMember {
    #[serde(rename = "displayName")]
    display_name: Option<String>,
}

// --- Graph API calls ---

fn chat_title(chat: &GraphChat) -> String {
    if let Some(topic) = &chat.topic {
        if !topic.is_empty() {
            return topic.clone();
        }
    }
    let names: Vec<String> = chat
        .members
        .as_ref()
        .map(|ms| {
            ms.iter()
                .filter_map(|m| m.display_name.clone())
                .filter(|n| !n.is_empty())
                .take(4)
                .collect()
        })
        .unwrap_or_default();
    if names.is_empty() {
        match chat.chat_type.as_str() {
            "oneOnOne" => "Direct message".to_string(),
            _ => "Group chat".to_string(),
        }
    } else {
        names.join(", ")
    }
}

fn fetch_recent_chats(token: &str) -> Result<Vec<Target>, String> {
    let client = http_client()?;

    let chats: GraphList<GraphChat> = client
        .get("https://graph.microsoft.com/v1.0/me/chats?$top=20&$expand=members($top=6)")
        .bearer_auth(token)
        .send()
        .map_err(|e| e.to_string())?
        .error_for_status()
        .map_err(|e| format!("Failed to list chats: {e}"))?
        .json()
        .map_err(|e| e.to_string())?;

    let targets = chats
        .value
        .iter()
        .map(|chat| {
            let title = chat_title(chat);
            let description = match chat.chat_type.as_str() {
                "oneOnOne" => "Direct message",
                "meeting" => "Meeting chat",
                _ => "Group chat",
            }
            .to_string();
            Target {
                id: format!("chat:{}", chat.id),
                provider: "MS Teams".to_string(),
                formats: vec!["text".to_string()],
                title,
                description,
                image: TEAMS_ICON.to_string(),
            }
        })
        .collect();

    Ok(targets)
}

fn fetch_channel_targets(token: &str) -> Result<Vec<Target>, String> {
    let client = http_client()?;

    let teams: GraphList<GraphTeam> = client
        .get("https://graph.microsoft.com/v1.0/me/joinedTeams")
        .bearer_auth(token)
        .send()
        .map_err(|e| e.to_string())?
        .error_for_status()
        .map_err(|e| format!("Failed to list teams: {e}"))?
        .json()
        .map_err(|e| e.to_string())?;

    let mut targets = Vec::new();

    for team in teams.value {
        let channels: GraphList<GraphChannel> = match client
            .get(format!(
                "https://graph.microsoft.com/v1.0/teams/{}/channels",
                team.id
            ))
            .bearer_auth(token)
            .send()
            .and_then(|r| r.error_for_status())
            .and_then(|r| r.json())
        {
            Ok(c) => c,
            Err(e) => {
                eprintln!(
                    "[msteams] Skipping channels for '{}': {e}",
                    team.display_name
                );
                continue;
            }
        };

        for channel in channels.value {
            // target_id: "channel:{team_guid}:{channel_id}"
            // team_guid has no colons; split_once(':') on the suffix recovers both.
            targets.push(Target {
                id: format!("channel:{}:{}", team.id, channel.id),
                provider: "MS Teams".to_string(),
                formats: vec!["text".to_string()],
                title: format!("{} › {}", team.display_name, channel.display_name),
                description: team.display_name.clone(),
                image: TEAMS_ICON.to_string(),
            });
        }
    }

    Ok(targets)
}

fn post_message(token: &str, target_id: &str, content: &str) -> Result<(), String> {
    let url = if let Some(chat_id) = target_id.strip_prefix("chat:") {
        format!("https://graph.microsoft.com/v1.0/chats/{chat_id}/messages")
    } else if let Some(rest) = target_id.strip_prefix("channel:") {
        let Some((team_id, channel_id)) = rest.split_once(':') else {
            return Err(format!("Invalid channel target id: {target_id}"));
        };
        format!("https://graph.microsoft.com/v1.0/teams/{team_id}/channels/{channel_id}/messages")
    } else {
        return Err(format!("Unknown target id format: {target_id}"));
    };

    http_client()?
        .post(url)
        .bearer_auth(token)
        .json(&serde_json::json!({
            "body": { "content": content, "contentType": "text" }
        }))
        .send()
        .map_err(|e| format!("Request failed: {e}"))?
        .error_for_status()
        .map_err(|e| format!("Send failed: {e}"))?;

    Ok(())
}

// --- Handler ---

fn handle(request: Request) -> serde_json::Value {
    match request {
        Request::GetInfo => serde_json::to_value(InfoResponse {
            name: "MS Teams",
            version: env!("CARGO_PKG_VERSION"),
            description: "Send clipboard content to Microsoft Teams chats and channels",
            author: "clipygo",
        })
        .unwrap(),

        Request::GetTargets => {
            let mut config = load_config();

            if config.tenant_id.is_empty() || config.client_id.is_empty() {
                eprintln!(
                    "[msteams] Not configured. Edit {:?} with tenant_id and client_id.",
                    config_path()
                );
                return serde_json::to_value(TargetsResponse { targets: vec![] }).unwrap();
            }

            let token = match get_valid_token(&mut config) {
                Ok(t) => t,
                Err(e) => {
                    eprintln!("[msteams] Authentication failed: {e}");
                    return serde_json::to_value(TargetsResponse { targets: vec![] }).unwrap();
                }
            };

            let mut targets = Vec::new();

            // Recent chats first (most relevant for quick sharing)
            match fetch_recent_chats(&token) {
                Ok(mut chats) => targets.append(&mut chats),
                Err(e) => eprintln!("[msteams] Failed to fetch chats: {e}"),
            }

            // Then team channels
            match fetch_channel_targets(&token) {
                Ok(mut channels) => targets.append(&mut channels),
                Err(e) => eprintln!("[msteams] Failed to fetch channels: {e}"),
            }

            serde_json::to_value(TargetsResponse { targets }).unwrap()
        }

        Request::GetConfigSchema => {
            let config = load_config();
            serde_json::json!({
                "schema": {
                    "type": "object",
                    "title": "MS Teams",
                    "properties": {
                        "tenant_id": {
                            "type": "string",
                            "title": "Tenant ID",
                            "description": "Azure Active Directory tenant ID (Azure Portal → Azure AD → Overview)"
                        },
                        "client_id": {
                            "type": "string",
                            "title": "Client ID",
                            "description": "App registration client ID (Azure Portal → App registrations)"
                        },
                        "auth_method": {
                            "type": "string",
                            "title": "Authentication Method",
                            "enum": ["oauth2", "password"],
                            "enumTitles": ["OAuth2 (browser login)", "Username / Password (ROPC)"],
                            "default": "oauth2"
                        },
                        "username": {
                            "type": "string",
                            "title": "Username",
                            "description": "UPN (user@company.com) — only for password auth"
                        },
                        "password": {
                            "type": "string",
                            "title": "Password",
                            "format": "password",
                            "description": "Only for password auth"
                        }
                    },
                    "required": ["tenant_id", "client_id"]
                },
                "values": {
                    "tenant_id": config.tenant_id,
                    "client_id": config.client_id,
                    "auth_method": config.auth_method,
                    "username": config.username.unwrap_or_default(),
                    "password": config.password.unwrap_or_default()
                }
            })
        }

        Request::SetConfig { values } => {
            let mut config = load_config();

            let tenant_changed = values
                .get("tenant_id")
                .and_then(|v| v.as_str())
                .is_some_and(|v| v != config.tenant_id);
            let client_changed = values
                .get("client_id")
                .and_then(|v| v.as_str())
                .is_some_and(|v| v != config.client_id);

            if let Some(v) = values.get("tenant_id").and_then(|v| v.as_str()) {
                config.tenant_id = v.to_string();
            }
            if let Some(v) = values.get("client_id").and_then(|v| v.as_str()) {
                config.client_id = v.to_string();
            }
            if let Some(v) = values.get("auth_method").and_then(|v| v.as_str()) {
                config.auth_method = v.to_string();
            }
            if let Some(v) = values.get("username").and_then(|v| v.as_str()) {
                config.username = if v.is_empty() {
                    None
                } else {
                    Some(v.to_string())
                };
            }
            if let Some(v) = values.get("password").and_then(|v| v.as_str()) {
                config.password = if v.is_empty() {
                    None
                } else {
                    Some(v.to_string())
                };
            }

            // Clear cached tokens if identity changed
            if tenant_changed || client_changed {
                config.access_token = None;
                config.refresh_token = None;
                config.token_expiry = None;
                eprintln!("[msteams] Credentials changed — cleared cached tokens");
            }

            save_config(&config);

            serde_json::to_value(SendResponse {
                success: true,
                error: None,
            })
            .unwrap()
        }

        Request::Send {
            target_id,
            content,
            format,
        } => {
            if format != "text" {
                return serde_json::to_value(SendResponse {
                    success: false,
                    error: Some(format!("Unsupported format: {format}")),
                })
                .unwrap();
            }

            let mut config = load_config();

            let token = match get_valid_token(&mut config) {
                Ok(t) => t,
                Err(e) => {
                    return serde_json::to_value(SendResponse {
                        success: false,
                        error: Some(e),
                    })
                    .unwrap()
                }
            };

            match post_message(&token, &target_id, &content) {
                Ok(()) => serde_json::to_value(SendResponse {
                    success: true,
                    error: None,
                })
                .unwrap(),
                Err(e) => serde_json::to_value(SendResponse {
                    success: false,
                    error: Some(e),
                })
                .unwrap(),
            }
        }
    }
}

// --- Tests ---

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn get_info_fields() {
        let resp = handle(Request::GetInfo);
        assert_eq!(resp["name"], "MS Teams");
        assert!(resp["version"].is_string());
        assert!(resp["description"].is_string());
        assert!(resp["author"].is_string());
    }

    #[test]
    fn send_unsupported_format_returns_error() {
        let resp = handle(Request::Send {
            target_id: "chat:some-id".to_string(),
            content: "data".to_string(),
            format: "image".to_string(),
        });
        assert_eq!(resp["success"], false);
        assert!(resp["error"].as_str().unwrap().contains("image"));
    }

    #[test]
    fn get_config_schema_returns_schema_and_values() {
        let resp = handle(Request::GetConfigSchema);
        assert!(resp.get("schema").is_some());
        assert!(resp.get("values").is_some());
        let props = &resp["schema"]["properties"];
        assert!(props.get("tenant_id").is_some());
        assert!(props.get("client_id").is_some());
        assert!(props.get("auth_method").is_some());
    }

    #[test]
    fn set_config_returns_success() {
        // Only tests that the handler runs without panicking on valid input.
        // Actual disk write is skipped if config dir is unavailable in test env.
        let resp = handle(Request::SetConfig {
            values: serde_json::json!({
                "tenant_id": "test-tenant",
                "client_id": "test-client",
                "auth_method": "oauth2"
            }),
        });
        assert_eq!(resp["success"], true);
    }

    #[test]
    fn invalid_json_rejected() {
        assert!(serde_json::from_str::<Request>("not json").is_err());
    }

    #[test]
    fn unknown_command_rejected() {
        assert!(serde_json::from_str::<Request>(r#"{"command":"unknown"}"#).is_err());
    }

    #[test]
    fn config_roundtrip() {
        let config = Config {
            tenant_id: "test-tenant".to_string(),
            client_id: "test-client".to_string(),
            auth_method: "oauth2".to_string(),
            username: None,
            password: None,
            access_token: Some("tok".to_string()),
            refresh_token: None,
            token_expiry: Some(9_999_999_999),
        };
        let json = serde_json::to_string(&config).unwrap();
        let back: Config = serde_json::from_str(&json).unwrap();
        assert_eq!(back.tenant_id, "test-tenant");
        assert_eq!(back.auth_method, "oauth2");
        assert_eq!(back.access_token.as_deref(), Some("tok"));
    }

    #[test]
    fn default_auth_method_is_oauth2() {
        let config: Config = serde_json::from_str(r#"{"tenant_id":"t","client_id":"c"}"#).unwrap();
        assert_eq!(config.auth_method, "oauth2");
    }

    #[test]
    fn chat_title_uses_topic_when_set() {
        let chat = GraphChat {
            id: "123".to_string(),
            chat_type: "group".to_string(),
            topic: Some("Project Alpha".to_string()),
            members: None,
        };
        assert_eq!(chat_title(&chat), "Project Alpha");
    }

    #[test]
    fn chat_title_uses_member_names_for_1on1() {
        let chat = GraphChat {
            id: "123".to_string(),
            chat_type: "oneOnOne".to_string(),
            topic: None,
            members: Some(vec![
                GraphChatMember {
                    display_name: Some("Alice".to_string()),
                },
                GraphChatMember {
                    display_name: Some("Bob".to_string()),
                },
            ]),
        };
        let title = chat_title(&chat);
        assert!(title.contains("Alice"));
        assert!(title.contains("Bob"));
    }

    #[test]
    fn chat_title_fallback_when_no_members() {
        let chat = GraphChat {
            id: "123".to_string(),
            chat_type: "oneOnOne".to_string(),
            topic: None,
            members: None,
        };
        assert_eq!(chat_title(&chat), "Direct message");
    }
}

// --- Main ---

fn main() {
    let stdin = io::stdin();
    let stdout = io::stdout();
    let mut out = stdout.lock();

    for line in stdin.lock().lines() {
        let line = match line {
            Ok(l) => l,
            Err(_) => break,
        };

        if line.trim().is_empty() {
            continue;
        }

        let response = match serde_json::from_str::<Request>(&line) {
            Ok(request) => handle(request),
            Err(e) => serde_json::json!({ "error": format!("Bad request: {e}") }),
        };

        let _ = writeln!(out, "{response}");
        let _ = out.flush();
    }
}
