use serde::{Deserialize, Serialize};

#[derive(Deserialize)]
#[serde(tag = "command", rename_all = "snake_case")]
pub enum Request {
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
pub struct InfoResponse {
    pub name: &'static str,
    pub version: &'static str,
    pub description: &'static str,
    pub author: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub link: Option<&'static str>,
}

#[derive(Serialize, Clone)]
pub struct Target {
    pub id: String,
    pub provider: String,
    pub formats: Vec<String>,
    pub title: String,
    pub description: String,
    pub image: String,
}

#[derive(Serialize)]
pub struct TargetsResponse {
    pub targets: Vec<Target>,
}

#[derive(Serialize)]
pub struct SendResponse {
    pub success: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}
