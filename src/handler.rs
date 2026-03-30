use crate::auth::get_valid_token;
use crate::config::{config_path, load_config, save_config};
use crate::graph::{fetch_channel_targets, fetch_recent_chats, post_message};
use crate::protocol::{InfoResponse, Request, SendResponse, TargetsResponse};

pub fn handle(request: Request) -> serde_json::Value {
    match request {
        Request::GetInfo => serde_json::to_value(InfoResponse {
            name: "MS Teams",
            version: env!("CARGO_PKG_VERSION"),
            description: "Send clipboard content to Microsoft Teams chats and channels",
            author: "clipygo",
            link: Some("https://github.com/it-atelier-gn/clipygo-plugin-msteams"),
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

            if config.auth_method == "password" {
                let missing_user = config.username.as_deref().map_or(true, |s| s.is_empty());
                let missing_pass = config.password.as_deref().map_or(true, |s| s.is_empty());
                if missing_user || missing_pass {
                    eprintln!(
                        "[msteams] Password auth requires username and password. \
                         Configure them in plugin settings."
                    );
                    return serde_json::to_value(TargetsResponse { targets: vec![] }).unwrap();
                }
            }

            let token = match get_valid_token(&mut config) {
                Ok(t) => t,
                Err(e) => {
                    eprintln!("[msteams] Authentication failed: {e}");
                    return serde_json::to_value(TargetsResponse { targets: vec![] }).unwrap();
                }
            };

            let mut targets = Vec::new();

            match fetch_recent_chats(&token) {
                Ok(mut chats) => targets.append(&mut chats),
                Err(e) => eprintln!("[msteams] Failed to fetch chats: {e}"),
            }

            match fetch_channel_targets(&token) {
                Ok(mut channels) => targets.append(&mut channels),
                Err(e) => eprintln!("[msteams] Failed to fetch channels: {e}"),
            }

            serde_json::to_value(TargetsResponse { targets }).unwrap()
        }

        Request::GetConfigSchema => {
            let config = load_config();
            serde_json::json!({
                "instructions": "Requires an Azure AD app registration:\n\
                    1. Go to Azure Portal → App registrations → New registration\n\
                    2. Set redirect URI to http://localhost (Web) for OAuth2\n\
                    3. Under API permissions, add Microsoft Graph delegated permissions:\n\
                       Team.ReadBasic.All, Channel.ReadBasic.All, ChannelMessage.Send,\n\
                       Chat.ReadBasic, ChatMessage.Send, offline_access\n\
                    4. Copy the Tenant ID and Client ID below",
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
                            "enum": ["oauth2", "device_code", "password"],
                            "enumTitles": ["OAuth2 (browser login)", "Device Code (no browser needed on this machine)", "Username / Password (ROPC)"],
                            "default": "oauth2"
                        },
                        "username": {
                            "type": "string",
                            "title": "Username",
                            "description": "UPN (user@company.com)",
                            "visibleIf": { "auth_method": "password" }
                        },
                        "password": {
                            "type": "string",
                            "title": "Password",
                            "format": "password",
                            "visibleIf": { "auth_method": "password" }
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
            let auth_method_changed = values
                .get("auth_method")
                .and_then(|v| v.as_str())
                .is_some_and(|v| v != config.auth_method);

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

            // Clear cached tokens if identity or auth method changed
            if tenant_changed || client_changed || auth_method_changed {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;
    use crate::graph::{GraphChat, GraphChatMember};
    use crate::protocol::Request;

    #[test]
    fn get_info_fields() {
        let resp = handle(Request::GetInfo);
        assert_eq!(resp["name"], "MS Teams");
        assert!(resp["version"].is_string());
        assert!(resp["description"].is_string());
        assert!(resp["author"].is_string());
    }

    #[test]
    fn get_info_includes_link() {
        let resp = handle(Request::GetInfo);
        assert!(resp["link"].is_string());
        assert!(resp["link"].as_str().unwrap().starts_with("https://"));
    }

    #[test]
    fn get_config_schema_includes_instructions() {
        let resp = handle(Request::GetConfigSchema);
        assert!(resp.get("instructions").is_some());
        let instructions = resp["instructions"].as_str().unwrap();
        assert!(instructions.contains("Azure"));
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
    fn get_config_schema_includes_device_code() {
        let resp = handle(Request::GetConfigSchema);
        let auth_enum = &resp["schema"]["properties"]["auth_method"]["enum"];
        let methods: Vec<&str> = auth_enum
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_str().unwrap())
            .collect();
        assert!(methods.contains(&"device_code"));
    }

    #[test]
    fn set_config_returns_success() {
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
        assert_eq!(crate::graph::chat_title(&chat), "Project Alpha");
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
        let title = crate::graph::chat_title(&chat);
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
        assert_eq!(crate::graph::chat_title(&chat), "Direct message");
    }
}
