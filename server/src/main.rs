use chrono::{DateTime, Utc};
use poem::{
    listener::TcpListener,
    middleware::AddData,
    EndpointExt, Route, Server,
};
use poem_openapi::{param::Query, payload::Json, ApiResponse, Object, OpenApi, OpenApiService};
use std::{
    collections::{HashMap, VecDeque},
    sync::Arc,
    time::Duration,
};
use tokio::sync::RwLock;
use uuid::Uuid;

const HEARTBEAT_TIMEOUT_SECS: i64 = 30;
const MAX_INBOX: usize = 100;

// ── state ─────────────────────────────────────────────────────────────────────

struct AgentRecord {
    id: String,
    name: String,
    description: String,
    last_heartbeat: DateTime<Utc>,
    repo_path: Option<String>,
}

struct Message {
    from: String,
    body: String,
    timestamp: DateTime<Utc>,
}

#[derive(Clone, PartialEq)]
enum TaskStatus {
    Queued,
    InProgress,
    Done,
    Failed,
}

impl TaskStatus {
    fn as_str(&self) -> &'static str {
        match self {
            Self::Queued => "queued",
            Self::InProgress => "in_progress",
            Self::Done => "done",
            Self::Failed => "failed",
        }
    }
}

struct TaskRecord {
    id: String,
    agent_id: String,
    title: String,
    body: String,
    status: TaskStatus,
    created_at: DateTime<Utc>,
    started_at: Option<DateTime<Utc>>,
    finished_at: Option<DateTime<Utc>>,
    result: Option<String>,
}

struct AppState {
    agents: RwLock<HashMap<String, AgentRecord>>,
    inboxes: RwLock<HashMap<String, VecDeque<Message>>>,
    tasks: RwLock<HashMap<String, TaskRecord>>,
}

impl Default for AppState {
    fn default() -> Self {
        Self {
            agents: Default::default(),
            inboxes: Default::default(),
            tasks: Default::default(),
        }
    }
}

type SharedState = Arc<AppState>;

// ── response / object types ───────────────────────────────────────────────────

#[derive(Object)]
struct RegisterResponse {
    agent_id: String,
}

#[derive(Object)]
struct AgentSummary {
    id: String,
    name: String,
    description: String,
    last_heartbeat: String,
}

#[derive(Object)]
struct TeamResponse {
    agents: Vec<AgentSummary>,
}

#[derive(Object)]
struct MessageItem {
    from: String,
    body: String,
    timestamp: String,
}

#[derive(Object)]
struct InboxResponse {
    messages: Vec<MessageItem>,
}

#[derive(Object)]
struct BroadcastResponse {
    delivered: usize,
}

#[derive(Object)]
struct ErrorBody {
    error: String,
}

#[derive(Object, Clone)]
struct TaskItem {
    id: String,
    agent_id: String,
    title: String,
    body: String,
    status: String,
    created_at: String,
    started_at: Option<String>,
    finished_at: Option<String>,
    result: Option<String>,
}

impl TaskItem {
    fn from_record(r: &TaskRecord) -> Self {
        Self {
            id: r.id.clone(),
            agent_id: r.agent_id.clone(),
            title: r.title.clone(),
            body: r.body.clone(),
            status: r.status.as_str().to_string(),
            created_at: r.created_at.to_rfc3339(),
            started_at: r.started_at.map(|t| t.to_rfc3339()),
            finished_at: r.finished_at.map(|t| t.to_rfc3339()),
            result: r.result.clone(),
        }
    }
}

#[derive(Object)]
struct TaskListResponse {
    tasks: Vec<TaskItem>,
}

#[derive(ApiResponse)]
enum TaskOrNotFound {
    /// Task found
    #[oai(status = 200)]
    Ok(Json<TaskItem>),
    /// Task or agent not found
    #[oai(status = 404)]
    NotFound(Json<ErrorBody>),
}

#[derive(ApiResponse)]
enum OkOrNotFound {
    /// Success
    #[oai(status = 200)]
    Ok,
    /// Agent not found
    #[oai(status = 404)]
    NotFound(Json<ErrorBody>),
}

// ── API ───────────────────────────────────────────────────────────────────────

struct PunchclockApi;

#[OpenApi]
impl PunchclockApi {
    /// Register a new agent.
    ///
    /// Returns an `agent_id` the agent must use for heartbeat, inbox, and
    /// sending messages.
    #[oai(path = "/register", method = "get")]
    async fn register(
        &self,
        state: poem::web::Data<&SharedState>,
        /// Display name for this agent
        name: Query<String>,
        /// Human-readable description of what this agent does
        description: Query<String>,
        /// Absolute path to the agent's git repo root (used by task/list)
        repo_path: Query<Option<String>>,
    ) -> Json<RegisterResponse> {
        let id = Uuid::new_v4().to_string();
        let record = AgentRecord {
            id: id.clone(),
            name: name.0,
            description: description.0,
            last_heartbeat: Utc::now(),
            repo_path: repo_path.0,
        };
        state.agents.write().await.insert(id.clone(), record);
        state.inboxes.write().await.insert(id.clone(), VecDeque::new());
        Json(RegisterResponse { agent_id: id })
    }

    /// Send a heartbeat to stay online.
    ///
    /// Agents must call this at least every 30 seconds or they will be
    /// removed from the team roster.
    #[oai(path = "/heartbeat", method = "get")]
    async fn heartbeat(
        &self,
        state: poem::web::Data<&SharedState>,
        /// Agent ID returned by `/register`
        agent_id: Query<String>,
        /// Agent name (used to re-register if the agent was reaped)
        name: Query<Option<String>>,
        /// Agent description (used to re-register if the agent was reaped)
        description: Query<Option<String>>,
        /// Absolute path to the agent's repo root (used by task/list)
        repo_path: Query<Option<String>>,
    ) -> OkOrNotFound {
        let mut agents = state.agents.write().await;
        if let Some(a) = agents.get_mut(&agent_id.0) {
            a.last_heartbeat = Utc::now();
            if let Some(rp) = repo_path.0 {
                a.repo_path = Some(rp);
            }
            return OkOrNotFound::Ok;
        }
        // Agent was reaped — re-register in place if name/description supplied
        if let (Some(name), Some(desc)) = (name.0, description.0) {
            let record = AgentRecord {
                id: agent_id.0.clone(),
                name,
                description: desc,
                last_heartbeat: Utc::now(),
                repo_path: repo_path.0,
            };
            agents.insert(agent_id.0.clone(), record);
            drop(agents);
            state.inboxes.write().await
                .entry(agent_id.0.clone())
                .or_insert_with(VecDeque::new);
            tracing::info!(agent_id = %agent_id.0, "re-registered reaped agent via heartbeat");
            OkOrNotFound::Ok
        } else {
            OkOrNotFound::NotFound(Json(ErrorBody {
                error: format!("agent {} not found", agent_id.0),
            }))
        }
    }

    /// List all currently online agents.
    ///
    /// An agent is considered online if it has sent a heartbeat within the
    /// last 30 seconds.
    #[oai(path = "/team", method = "get")]
    async fn team(&self, state: poem::web::Data<&SharedState>) -> Json<TeamResponse> {
        let cutoff = Utc::now() - chrono::Duration::seconds(HEARTBEAT_TIMEOUT_SECS);
        let agents = state.agents.read().await;
        let online = agents
            .values()
            .filter(|a| a.last_heartbeat > cutoff)
            .map(|a| AgentSummary {
                id: a.id.clone(),
                name: a.name.clone(),
                description: a.description.clone(),
                last_heartbeat: a.last_heartbeat.to_rfc3339(),
            })
            .collect();
        Json(TeamResponse { agents: online })
    }

    /// Send a message to another agent's inbox.
    #[oai(path = "/message/send", method = "get")]
    async fn send_message(
        &self,
        state: poem::web::Data<&SharedState>,
        /// Recipient agent ID
        to: Query<String>,
        /// Sender agent ID (or any label identifying the sender)
        from: Query<String>,
        /// Message text
        body: Query<String>,
    ) -> OkOrNotFound {
        let mut inboxes = state.inboxes.write().await;
        match inboxes.get_mut(&to.0) {
            Some(inbox) => {
                if inbox.len() >= MAX_INBOX {
                    inbox.pop_front();
                }
                inbox.push_back(Message {
                    from: from.0,
                    body: body.0,
                    timestamp: Utc::now(),
                });
                OkOrNotFound::Ok
            }
            None => OkOrNotFound::NotFound(Json(ErrorBody {
                error: format!("agent {} not found", to.0),
            })),
        }
    }

    /// Broadcast a message to all currently-online agents.
    ///
    /// Delivers to every agent whose last heartbeat is within the timeout window.
    /// Returns the count of inboxes that received the message.
    #[oai(path = "/message/broadcast", method = "get")]
    async fn broadcast(
        &self,
        state: poem::web::Data<&SharedState>,
        /// Sender agent ID (or any label identifying the sender)
        from: Query<String>,
        /// Message text
        body: Query<String>,
    ) -> Json<BroadcastResponse> {
        let cutoff = Utc::now() - chrono::Duration::seconds(HEARTBEAT_TIMEOUT_SECS);
        let live_ids: Vec<String> = state
            .agents
            .read()
            .await
            .values()
            .filter(|a| a.last_heartbeat > cutoff)
            .map(|a| a.id.clone())
            .collect();

        let mut inboxes = state.inboxes.write().await;
        let mut delivered = 0usize;
        for id in &live_ids {
            if let Some(inbox) = inboxes.get_mut(id) {
                if inbox.len() >= MAX_INBOX {
                    inbox.pop_front();
                }
                inbox.push_back(Message {
                    from: from.0.clone(),
                    body: body.0.clone(),
                    timestamp: Utc::now(),
                });
                delivered += 1;
            }
        }
        Json(BroadcastResponse { delivered })
    }

    /// Drain and return all pending messages for an agent.
    #[oai(path = "/message/recv", method = "get")]
    async fn recv_messages(
        &self,
        state: poem::web::Data<&SharedState>,
        /// Your agent ID
        agent_id: Query<String>,
    ) -> Result<Json<InboxResponse>, poem::Error> {
        let mut inboxes = state.inboxes.write().await;
        match inboxes.get_mut(&agent_id.0) {
            Some(inbox) => {
                let messages = inbox
                    .drain(..)
                    .map(|m| MessageItem {
                        from: m.from,
                        body: m.body,
                        timestamp: m.timestamp.to_rfc3339(),
                    })
                    .collect();
                Ok(Json(InboxResponse { messages }))
            }
            None => Err(poem::Error::from_status(poem::http::StatusCode::NOT_FOUND)),
        }
    }

    /// Push a task onto an agent's queue.
    #[oai(path = "/task/push", method = "get")]
    async fn task_push(
        &self,
        state: poem::web::Data<&SharedState>,
        /// Target agent ID
        agent_id: Query<String>,
        /// Short title
        title: Query<String>,
        /// Full task description (sent to Claude)
        body: Query<String>,
    ) -> TaskOrNotFound {
        let agents = state.agents.read().await;
        if !agents.contains_key(&agent_id.0) {
            return TaskOrNotFound::NotFound(Json(ErrorBody {
                error: format!("agent {} not found", agent_id.0),
            }));
        }
        drop(agents);
        let id = Uuid::new_v4().to_string();
        let record = TaskRecord {
            id: id.clone(),
            agent_id: agent_id.0,
            title: title.0,
            body: body.0,
            status: TaskStatus::Queued,
            created_at: Utc::now(),
            started_at: None,
            finished_at: None,
            result: None,
        };
        let item = TaskItem::from_record(&record);
        state.tasks.write().await.insert(id, record);
        TaskOrNotFound::Ok(Json(item))
    }

    /// List all tasks for an agent, read from the agent's .task/ directory.
    ///
    /// Returns files from `.task/todo/` as `queued` and `.task/done/` as `done`.
    /// Falls back to the in-memory task list if the agent has no repo_path set.
    #[oai(path = "/task/list", method = "get")]
    async fn task_list(
        &self,
        state: poem::web::Data<&SharedState>,
        /// Agent ID to query
        agent_id: Query<String>,
    ) -> Json<TaskListResponse> {
        let repo_path = state
            .agents
            .read()
            .await
            .get(&agent_id.0)
            .and_then(|a| a.repo_path.clone());

        if let Some(root) = repo_path {
            let mut items = Vec::new();
            for (subdir, status) in &[("todo", "queued"), ("done", "done")] {
                let dir = std::path::Path::new(&root).join(".task").join(subdir);
                let Ok(entries) = std::fs::read_dir(&dir) else { continue };
                for entry in entries.flatten() {
                    let path = entry.path();
                    if path.extension().and_then(|e| e.to_str()) != Some("md") {
                        continue;
                    }
                    let filename = path.file_stem()
                        .and_then(|s| s.to_str())
                        .unwrap_or("")
                        .to_string();
                    let content = std::fs::read_to_string(&path).unwrap_or_default();
                    let title = content
                        .lines()
                        .find(|l| l.starts_with("# "))
                        .map(|l| l.trim_start_matches("# ").to_string())
                        .unwrap_or_else(|| filename.clone());
                    let modified = entry.metadata()
                        .and_then(|m| m.modified())
                        .map(|t| DateTime::<Utc>::from(t).to_rfc3339())
                        .unwrap_or_default();
                    items.push(TaskItem {
                        id: filename,
                        agent_id: agent_id.0.clone(),
                        title,
                        body: String::new(),
                        status: status.to_string(),
                        created_at: modified,
                        started_at: None,
                        finished_at: None,
                        result: None,
                    });
                }
            }
            items.sort_by(|a, b| a.created_at.cmp(&b.created_at));
            return Json(TaskListResponse { tasks: items });
        }

        // fallback: in-memory
        let tasks = state.tasks.read().await;
        let mut items: Vec<TaskItem> = tasks
            .values()
            .filter(|t| t.agent_id == agent_id.0)
            .map(TaskItem::from_record)
            .collect();
        items.sort_by(|a, b| a.created_at.cmp(&b.created_at));
        Json(TaskListResponse { tasks: items })
    }

    /// Claim the next queued task for an agent (sets it to in_progress).
    ///
    /// Returns 404 if there is no queued task.
    #[oai(path = "/task/claim", method = "get")]
    async fn task_claim(
        &self,
        state: poem::web::Data<&SharedState>,
        /// Agent ID claiming a task
        agent_id: Query<String>,
    ) -> TaskOrNotFound {
        let mut tasks = state.tasks.write().await;
        let next = tasks
            .values_mut()
            .filter(|t| t.agent_id == agent_id.0 && t.status == TaskStatus::Queued)
            .min_by_key(|t| t.created_at);
        match next {
            Some(t) => {
                t.status = TaskStatus::InProgress;
                t.started_at = Some(Utc::now());
                TaskOrNotFound::Ok(Json(TaskItem::from_record(t)))
            }
            None => TaskOrNotFound::NotFound(Json(ErrorBody {
                error: "no queued tasks".to_string(),
            })),
        }
    }

    /// Mark a task as done or failed and store the result.
    #[oai(path = "/task/finish", method = "get")]
    async fn task_finish(
        &self,
        state: poem::web::Data<&SharedState>,
        /// Task ID to finish
        task_id: Query<String>,
        /// New status: "done" or "failed"
        status: Query<String>,
        /// Result text (Claude's reply or error message)
        result: Query<Option<String>>,
    ) -> TaskOrNotFound {
        let mut tasks = state.tasks.write().await;
        match tasks.get_mut(&task_id.0) {
            Some(t) => {
                t.status = match status.0.as_str() {
                    "failed" => TaskStatus::Failed,
                    _ => TaskStatus::Done,
                };
                t.finished_at = Some(Utc::now());
                t.result = result.0;
                TaskOrNotFound::Ok(Json(TaskItem::from_record(t)))
            }
            None => TaskOrNotFound::NotFound(Json(ErrorBody {
                error: format!("task {} not found", task_id.0),
            })),
        }
    }
}

// ── reaper ────────────────────────────────────────────────────────────────────

async fn reaper(state: SharedState) {
    let mut tick = tokio::time::interval(Duration::from_secs(10));
    loop {
        tick.tick().await;
        let cutoff = Utc::now() - chrono::Duration::seconds(HEARTBEAT_TIMEOUT_SECS);
        let mut agents = state.agents.write().await;
        let mut inboxes = state.inboxes.write().await;
        let expired: Vec<String> = agents
            .values()
            .filter(|a| a.last_heartbeat <= cutoff)
            .map(|a| a.id.clone())
            .collect();
        for id in expired {
            tracing::info!(agent_id = %id, "heartbeat expired — removing agent");
            agents.remove(&id);
            inboxes.remove(&id);
        }
    }
}

// ── main ──────────────────────────────────────────────────────────────────────

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let _ = dotenvy::from_filename(concat!(env!("CARGO_MANIFEST_DIR"), "/.env")); // ok if absent

    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "punchclock_server=info,poem=info".into()),
        )
        .init();

    let api_base_url = std::env::var("API_BASE_URL")
        .unwrap_or_else(|_| "http://localhost:8421".to_string());

    // Derive bind port from API_BASE_URL, default 8421
    let port: u16 = api_base_url
        .trim_end_matches('/')
        .rsplit(':')
        .next()
        .and_then(|s| s.parse().ok())
        .unwrap_or(8421);

    let state: SharedState = Arc::new(AppState::default());
    tokio::spawn(reaper(state.clone()));

    let api_service =
        OpenApiService::new(PunchclockApi, "Punchclock", "0.1").server(&api_base_url);
    let swagger_ui = api_service.swagger_ui();
    let openapi_json = api_service.spec_endpoint();

    let app = Route::new()
        .nest("/", api_service)
        .nest("/docs", swagger_ui)
        .at("/openapi.json", openapi_json)
        .with(AddData::new(state));

    let addr = format!("0.0.0.0:{port}");
    tracing::info!("punchclock listening on {addr}  docs → {api_base_url}/docs");
    Server::new(TcpListener::bind(addr)).run(app).await?;
    Ok(())
}
