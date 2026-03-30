use std::io::{Read, Write};
use std::net::TcpListener;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::Deserialize;

use crate::config::{save_config, Config};

pub const SCOPES: &str = "https://graph.microsoft.com/Team.ReadBasic.All \
    https://graph.microsoft.com/Channel.ReadBasic.All \
    https://graph.microsoft.com/ChannelMessage.Send \
    https://graph.microsoft.com/Chat.ReadBasic \
    https://graph.microsoft.com/ChatMessage.Send \
    offline_access";

#[derive(Deserialize)]
pub struct TokenResponse {
    pub access_token: String,
    pub refresh_token: Option<String>,
    pub expires_in: u64,
}

#[derive(Deserialize)]
struct DeviceCodeResponse {
    device_code: String,
    user_code: String,
    verification_uri: String,
    expires_in: u64,
    interval: u64,
    message: String,
}

#[derive(Deserialize)]
struct DeviceCodePollResponse {
    access_token: Option<String>,
    refresh_token: Option<String>,
    expires_in: Option<u64>,
    error: Option<String>,
}

fn now_unix() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

pub fn get_valid_token(config: &mut Config) -> Result<String, String> {
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
        "device_code" => auth_device_code(config)?,
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
        .filter(|s| !s.is_empty())
        .ok_or("username not set — required for password auth")?;
    let password = config
        .password
        .as_deref()
        .filter(|s| !s.is_empty())
        .ok_or("password not set — required for password auth")?;

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

fn auth_device_code(config: &Config) -> Result<TokenResponse, String> {
    let device_code_url = format!(
        "https://login.microsoftonline.com/{}/oauth2/v2.0/devicecode",
        config.tenant_id
    );

    let resp = http_client()?
        .post(&device_code_url)
        .form(&[("client_id", config.client_id.as_str()), ("scope", SCOPES)])
        .send()
        .map_err(|e| format!("Device code request failed: {e}"))?;

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().unwrap_or_default();
        return Err(format!("Device code request failed {status}: {body}"));
    }

    let dc: DeviceCodeResponse = resp
        .json()
        .map_err(|e| format!("Invalid device code response: {e}"))?;

    eprintln!("[msteams] {}", dc.message);
    eprintln!(
        "[msteams] Go to {} and enter code: {}",
        dc.verification_uri, dc.user_code
    );

    let interval = Duration::from_secs(dc.interval.max(5));
    let deadline = now_unix() + dc.expires_in;
    let client = http_client()?;

    loop {
        if now_unix() >= deadline {
            return Err("Device code flow timed out — user did not complete authentication".into());
        }

        std::thread::sleep(interval);

        let poll_resp = client
            .post(token_url(&config.tenant_id))
            .form(&[
                ("grant_type", "urn:ietf:params:oauth:grant-type:device_code"),
                ("client_id", config.client_id.as_str()),
                ("device_code", &dc.device_code),
            ])
            .send()
            .map_err(|e| format!("Poll request failed: {e}"))?;

        let body: DeviceCodePollResponse = poll_resp
            .json()
            .map_err(|e| format!("Invalid poll response: {e}"))?;

        if let Some(access_token) = body.access_token {
            return Ok(TokenResponse {
                access_token,
                refresh_token: body.refresh_token,
                expires_in: body.expires_in.unwrap_or(3600),
            });
        }

        match body.error.as_deref() {
            Some("authorization_pending") => continue,
            Some("slow_down") => {
                std::thread::sleep(Duration::from_secs(5));
                continue;
            }
            Some("authorization_declined") => {
                return Err("User declined the authentication request".into());
            }
            Some("expired_token") => {
                return Err(
                    "Device code expired — user did not complete authentication in time".into(),
                );
            }
            Some(other) => {
                return Err(format!("Device code auth error: {other}"));
            }
            None => continue,
        }
    }
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
    let query = path.split_once('?').map(|x| x.1).unwrap_or("");

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

pub fn http_client() -> Result<reqwest::blocking::Client, String> {
    reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(30))
        .build()
        .map_err(|e| format!("Failed to build HTTP client: {e}"))
}
