use serde::Deserialize;

use crate::auth::http_client;
use crate::protocol::Target;

// Placeholder Teams icon (purple 1×1 PNG)
const TEAMS_ICON: &str =
    "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAAC0lEQVQI12NgAAIABQAABjE+ibYAAAAASUVORK5CYII=";

#[derive(Deserialize)]
pub struct GraphList<T> {
    pub value: Vec<T>,
}

#[derive(Deserialize)]
pub struct GraphTeam {
    pub id: String,
    #[serde(rename = "displayName")]
    pub display_name: String,
}

#[derive(Deserialize)]
pub struct GraphChannel {
    pub id: String,
    #[serde(rename = "displayName")]
    pub display_name: String,
}

#[derive(Deserialize)]
pub struct GraphChat {
    pub id: String,
    #[serde(rename = "chatType")]
    pub chat_type: String,
    pub topic: Option<String>,
    pub members: Option<Vec<GraphChatMember>>,
}

#[derive(Deserialize)]
pub struct GraphChatMember {
    #[serde(rename = "displayName")]
    pub display_name: Option<String>,
}

pub fn chat_title(chat: &GraphChat) -> String {
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

pub fn fetch_recent_chats(token: &str) -> Result<Vec<Target>, String> {
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

pub fn fetch_channel_targets(token: &str) -> Result<Vec<Target>, String> {
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

pub fn post_message(token: &str, target_id: &str, content: &str) -> Result<(), String> {
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
