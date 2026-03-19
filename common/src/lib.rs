use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "poem-openapi", derive(poem_openapi::Object))]
pub struct RegisterResponse {
    pub agent_id: String,
}

#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "poem-openapi", derive(poem_openapi::Object))]
pub struct AgentSummary {
    pub id: String,
    pub name: String,
    pub description: String,
    pub last_heartbeat: String,
}

#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "poem-openapi", derive(poem_openapi::Object))]
pub struct TeamResponse {
    pub agents: Vec<AgentSummary>,
}

#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "poem-openapi", derive(poem_openapi::Object))]
pub struct MessageItem {
    pub id: String,
    pub from: String,
    pub body: String,
    pub timestamp: String,
    pub acked: bool,
}

#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "poem-openapi", derive(poem_openapi::Object))]
pub struct InboxResponse {
    pub messages: Vec<MessageItem>,
}

#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "poem-openapi", derive(poem_openapi::Object))]
pub struct BroadcastResponse {
    pub delivered: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "poem-openapi", derive(poem_openapi::Object))]
pub struct TaskItem {
    pub id: String,
    pub agent_id: String,
    pub title: String,
    pub body: String,
    pub status: String,
    pub created_at: String,
    pub started_at: Option<String>,
    pub finished_at: Option<String>,
    pub result: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "poem-openapi", derive(poem_openapi::Object))]
pub struct TaskListResponse {
    pub tasks: Vec<TaskItem>,
}

#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "poem-openapi", derive(poem_openapi::Object))]
pub struct TaskSyncItem {
    pub id: String,
    pub title: String,
    pub status: String,
    pub modified_at: String,
}

#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "poem-openapi", derive(poem_openapi::Object))]
pub struct TaskSyncRequest {
    pub agent_id: String,
    pub hash: u64,
    pub tasks: Vec<TaskSyncItem>,
}

#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "poem-openapi", derive(poem_openapi::Object))]
pub struct MessageStatusResponse {
    pub id: String,
    pub acked: bool,
}

#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "poem-openapi", derive(poem_openapi::Object))]
pub struct MessageStatusListResponse {
    pub messages: Vec<MessageStatusResponse>,
}

#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "poem-openapi", derive(poem_openapi::Object))]
pub struct ErrorBody {
    pub error: String,
}
