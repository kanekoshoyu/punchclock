use chrono::{DateTime, Utc};
use poem::{
    endpoint::BoxEndpoint,
    middleware::{AddData, Cors},
    Endpoint, EndpointExt, IntoResponse, Middleware, Request, Response, Route,
};
use poem_openapi::{param::Query, payload::{Html, Json}, ApiResponse, OpenApi, OpenApiService};
use punchclock_common::{
    AgentSummary, BroadcastResponse, ErrorBody, InboxResponse, MessageItem, MessageStatusListResponse,
    MessageStatusResponse, RegisterResponse, TaskItem, TaskListResponse, TaskSyncRequest, TeamResponse,
};
use std::{
    collections::{HashMap, VecDeque},
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
use tokio::sync::RwLock;
use uuid::Uuid;

// ── config ────────────────────────────────────────────────────────────────────

pub struct ServerConfig {
    pub heartbeat_timeout_secs: i64,
    pub reaper_interval_secs: u64,
    pub max_inbox: usize,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            heartbeat_timeout_secs: 30,
            reaper_interval_secs: 10,
            max_inbox: 100,
        }
    }
}

// ── state ─────────────────────────────────────────────────────────────────────

pub struct AgentRecord {
    pub id: String,
    pub name: String,
    pub description: String,
    pub last_heartbeat: DateTime<Utc>,
    pub task_snapshot: Vec<TaskItem>,
    pub task_snapshot_hash: u64,
}

struct Message {
    id: String,
    from: String,
    body: String,
    timestamp: DateTime<Utc>,
    acked: bool,
}

#[derive(Clone, PartialEq)]
enum TaskStatus {
    Queued,
    InProgress,
    Done,
    Failed,
    Blocked,
}

impl TaskStatus {
    fn as_str(&self) -> &'static str {
        match self {
            Self::Queued => "queued",
            Self::InProgress => "in_progress",
            Self::Done => "done",
            Self::Failed => "failed",
            Self::Blocked => "blocked",
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

pub struct AppState {
    pub config: ServerConfig,
    pub(crate) agents: RwLock<HashMap<String, AgentRecord>>,
    pub(crate) inboxes: RwLock<HashMap<String, VecDeque<Message>>>,
    tasks: RwLock<HashMap<String, TaskRecord>>,
    message_acks: RwLock<HashMap<String, HashMap<String, bool>>>,
}

impl AppState {
    pub fn new(config: ServerConfig) -> Self {
        Self {
            config,
            agents: Default::default(),
            inboxes: Default::default(),
            tasks: Default::default(),
            message_acks: Default::default(),
        }
    }
}

impl Default for AppState {
    fn default() -> Self {
        Self::new(ServerConfig::default())
    }
}

pub type SharedState = Arc<AppState>;

// ── rate limiter ──────────────────────────────────────────────────────────────

const MSG_RATE_MAX: usize = 60;
const MSG_RATE_WINDOW: u64 = 60;
const IP_RATE_MAX: usize = 300;
const IP_RATE_WINDOW: u64 = 60;

struct Bucket {
    timestamps: VecDeque<Instant>,
}

impl Bucket {
    fn new() -> Self {
        Self { timestamps: VecDeque::new() }
    }

    fn check(&mut self, max: usize, window_secs: u64) -> bool {
        let now = Instant::now();
        let cutoff = now - Duration::from_secs(window_secs);
        while self.timestamps.front().map_or(false, |&t| t < cutoff) {
            self.timestamps.pop_front();
        }
        if self.timestamps.len() >= max {
            return false;
        }
        self.timestamps.push_back(now);
        true
    }
}

struct RateLimiterInner {
    msg_buckets: HashMap<String, Bucket>,
    ip_buckets: HashMap<String, Bucket>,
}

#[derive(Clone)]
struct RateLimiter(Arc<Mutex<RateLimiterInner>>);

impl RateLimiter {
    fn new() -> Self {
        Self(Arc::new(Mutex::new(RateLimiterInner {
            msg_buckets: HashMap::new(),
            ip_buckets: HashMap::new(),
        })))
    }
}

impl<E: Endpoint> Middleware<E> for RateLimiter {
    type Output = RateLimitedEndpoint<E>;
    fn transform(&self, ep: E) -> Self::Output {
        RateLimitedEndpoint { inner: ep, limiter: Arc::clone(&self.0) }
    }
}

struct RateLimitedEndpoint<E> {
    inner: E,
    limiter: Arc<Mutex<RateLimiterInner>>,
}

impl<E: Endpoint> Endpoint for RateLimitedEndpoint<E> {
    type Output = Response;

    async fn call(&self, req: Request) -> poem::Result<Self::Output> {
        let path = req.uri().path().to_string();
        let query = req.uri().query().unwrap_or("").to_string();
        let remote_str = req.remote_addr().to_string();
        let ip = remote_str
            .rsplit_once(':')
            .map(|(h, _)| h.to_string())
            .unwrap_or(remote_str);

        let is_msg = path == "/message/send" || path == "/message/broadcast";

        let allowed = {
            let mut state = self.limiter.lock().unwrap();
            if is_msg {
                let from = query_param(&query, "from")
                    .unwrap_or(&ip)
                    .to_string();
                state
                    .msg_buckets
                    .entry(from)
                    .or_insert_with(Bucket::new)
                    .check(MSG_RATE_MAX, MSG_RATE_WINDOW)
            } else {
                state
                    .ip_buckets
                    .entry(ip)
                    .or_insert_with(Bucket::new)
                    .check(IP_RATE_MAX, IP_RATE_WINDOW)
            }
        };

        if !allowed {
            let retry = if is_msg { MSG_RATE_WINDOW } else { IP_RATE_WINDOW };
            return Ok(Response::builder()
                .status(poem::http::StatusCode::TOO_MANY_REQUESTS)
                .header("Retry-After", retry.to_string())
                .header("Content-Type", "application/json")
                .body(r#"{"error":"rate limit exceeded"}"#));
        }

        self.inner.call(req).await.map(IntoResponse::into_response)
    }
}

fn query_param<'a>(query: &'a str, name: &str) -> Option<&'a str> {
    query.split('&').find_map(|pair| {
        let (k, v) = pair.split_once('=')?;
        if k == name { Some(v) } else { None }
    })
}

// ── response helpers ──────────────────────────────────────────────────────────

fn task_item_from_record(r: &TaskRecord) -> TaskItem {
    TaskItem {
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

#[derive(ApiResponse)]
enum TaskOrNotFound {
    #[oai(status = 200)]
    Ok(Json<TaskItem>),
    #[oai(status = 404)]
    NotFound(Json<ErrorBody>),
}

#[derive(ApiResponse)]
enum OkOrNotFound {
    #[oai(status = 200)]
    Ok,
    #[oai(status = 404)]
    NotFound(Json<ErrorBody>),
}

// ── API ───────────────────────────────────────────────────────────────────────

struct PunchclockApi;

#[OpenApi]
impl PunchclockApi {
    /// Tutorial and quick-start guide.
    #[oai(path = "/", method = "get")]
    async fn index(&self) -> Html<&'static str> {
        Html(crate::INDEX_HTML)
    }

    /// Register a new agent.
    #[oai(path = "/register", method = "get")]
    async fn register(
        &self,
        state: poem::web::Data<&SharedState>,
        name: Query<String>,
        description: Query<String>,
    ) -> Json<RegisterResponse> {
        let id = Uuid::new_v4().to_string();
        let record = AgentRecord {
            id: id.clone(),
            name: name.0,
            description: description.0,
            last_heartbeat: Utc::now(),
            task_snapshot: Vec::new(),
            task_snapshot_hash: 0,
        };
        state.agents.write().await.insert(id.clone(), record);
        state.inboxes.write().await.insert(id.clone(), VecDeque::new());
        Json(RegisterResponse { agent_id: id })
    }

    /// Send a heartbeat to stay online.
    #[oai(path = "/heartbeat", method = "get")]
    async fn heartbeat(
        &self,
        state: poem::web::Data<&SharedState>,
        agent_id: Query<String>,
        name: Query<Option<String>>,
        description: Query<Option<String>>,
    ) -> OkOrNotFound {
        let mut agents = state.agents.write().await;
        if let Some(a) = agents.get_mut(&agent_id.0) {
            a.last_heartbeat = Utc::now();
            return OkOrNotFound::Ok;
        }
        if let (Some(name), Some(desc)) = (name.0, description.0) {
            let record = AgentRecord {
                id: agent_id.0.clone(),
                name,
                description: desc,
                last_heartbeat: Utc::now(),
                task_snapshot: Vec::new(),
                task_snapshot_hash: 0,
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
    #[oai(path = "/team", method = "get")]
    async fn team(&self, state: poem::web::Data<&SharedState>) -> Json<TeamResponse> {
        let timeout = state.config.heartbeat_timeout_secs;
        let cutoff = Utc::now() - chrono::Duration::seconds(timeout);
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
        to: Query<String>,
        from: Query<String>,
        body: Query<String>,
    ) -> OkOrNotFound {
        let max = state.config.max_inbox;
        let msg_id = Uuid::new_v4().to_string();
        let mut inboxes = state.inboxes.write().await;
        match inboxes.get_mut(&to.0) {
            Some(inbox) => {
                if inbox.len() >= max {
                    if let Some(old_msg) = inbox.pop_front() {
                        let mut acks = state.message_acks.write().await;
                        if let Some(agent_acks) = acks.get_mut(&to.0) {
                            agent_acks.remove(&old_msg.id);
                        }
                    }
                }
                inbox.push_back(Message {
                    id: msg_id.clone(),
                    from: from.0,
                    body: body.0,
                    timestamp: Utc::now(),
                    acked: false,
                });
                let mut acks = state.message_acks.write().await;
                acks.entry(to.0.clone()).or_default().insert(msg_id, false);
                OkOrNotFound::Ok
            }
            None => OkOrNotFound::NotFound(Json(ErrorBody {
                error: format!("agent {} not found", to.0),
            })),
        }
    }

    /// Broadcast a message to all currently-online agents.
    #[oai(path = "/message/broadcast", method = "get")]
    async fn broadcast(
        &self,
        state: poem::web::Data<&SharedState>,
        from: Query<String>,
        body: Query<String>,
    ) -> Json<BroadcastResponse> {
        let timeout = state.config.heartbeat_timeout_secs;
        let max = state.config.max_inbox;
        let cutoff = Utc::now() - chrono::Duration::seconds(timeout);
        let live_ids: Vec<String> = state
            .agents
            .read()
            .await
            .values()
            .filter(|a| a.last_heartbeat > cutoff)
            .map(|a| a.id.clone())
            .collect();

        let mut inboxes = state.inboxes.write().await;
        let mut acks = state.message_acks.write().await;
        let mut delivered = 0usize;
        for id in &live_ids {
            if let Some(inbox) = inboxes.get_mut(id) {
                if inbox.len() >= max {
                    if let Some(old_msg) = inbox.pop_front() {
                        if let Some(agent_acks) = acks.get_mut(id) {
                            agent_acks.remove(&old_msg.id);
                        }
                    }
                }
                let msg_id = Uuid::new_v4().to_string();
                inbox.push_back(Message {
                    id: msg_id.clone(),
                    from: from.0.clone(),
                    body: body.0.clone(),
                    timestamp: Utc::now(),
                    acked: false,
                });
                acks.entry(id.clone()).or_default().insert(msg_id, false);
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
        agent_id: Query<String>,
    ) -> Result<Json<InboxResponse>, poem::Error> {
        let mut inboxes = state.inboxes.write().await;
        let acks = state.message_acks.read().await;
        let agent_acks = acks.get(&agent_id.0).cloned().unwrap_or_default();
        let messages = match inboxes.get_mut(&agent_id.0) {
            Some(inbox) => inbox
                .drain(..)
                .map(|m| {
                    let acked = agent_acks.get(&m.id).copied().unwrap_or(false);
                    MessageItem {
                        id: m.id,
                        from: m.from,
                        body: m.body,
                        timestamp: m.timestamp.to_rfc3339(),
                        acked,
                    }
                })
                .collect(),
            None => vec![],
        };
        Ok(Json(InboxResponse { messages }))
    }

    /// Acknowledge a message (mark as received).
    #[oai(path = "/message/ack", method = "post")]
    async fn ack_message(
        &self,
        state: poem::web::Data<&SharedState>,
        agent_id: Query<String>,
        message_id: Query<String>,
    ) -> OkOrNotFound {
        let mut acks = state.message_acks.write().await;
        match acks.get_mut(&agent_id.0) {
            Some(agent_acks) => {
                if agent_acks.contains_key(&message_id.0) {
                    agent_acks.insert(message_id.0, true);
                    OkOrNotFound::Ok
                } else {
                    OkOrNotFound::NotFound(Json(ErrorBody {
                        error: format!("message {} not found", message_id.0),
                    }))
                }
            }
            None => OkOrNotFound::NotFound(Json(ErrorBody {
                error: format!("agent {} not found", agent_id.0),
            })),
        }
    }

    /// Get ack status of all messages for an agent.
    #[oai(path = "/message/status", method = "get")]
    async fn message_status(
        &self,
        state: poem::web::Data<&SharedState>,
        agent_id: Query<String>,
    ) -> Result<Json<MessageStatusListResponse>, poem::Error> {
        let acks = state.message_acks.read().await;
        let messages = acks
            .get(&agent_id.0)
            .map(|agent_acks| {
                agent_acks
                    .iter()
                    .map(|(id, acked)| MessageStatusResponse {
                        id: id.clone(),
                        acked: *acked,
                    })
                    .collect()
            })
            .unwrap_or_default();
        Ok(Json(MessageStatusListResponse { messages }))
    }

    /// Push a task onto an agent's queue.
    #[oai(path = "/task/push", method = "get")]
    async fn task_push(
        &self,
        state: poem::web::Data<&SharedState>,
        agent_id: Query<String>,
        title: Query<String>,
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
        let item = task_item_from_record(&record);
        state.tasks.write().await.insert(id, record);
        TaskOrNotFound::Ok(Json(item))
    }

    /// Sync task list from daemon.
    #[oai(path = "/task/sync", method = "post")]
    async fn task_sync(
        &self,
        state: poem::web::Data<&SharedState>,
        body: Json<TaskSyncRequest>,
    ) -> OkOrNotFound {
        let mut agents = state.agents.write().await;
        match agents.get_mut(&body.agent_id) {
            Some(agent) => {
                if agent.task_snapshot_hash == body.hash {
                    return OkOrNotFound::Ok;
                }
                agent.task_snapshot_hash = body.hash;
                agent.task_snapshot = body.tasks.iter().map(|t| TaskItem {
                    id: t.id.clone(),
                    agent_id: body.agent_id.clone(),
                    title: t.title.clone(),
                    body: String::new(),
                    status: t.status.clone(),
                    created_at: t.modified_at.clone(),
                    started_at: None,
                    finished_at: None,
                    result: None,
                }).collect();
                OkOrNotFound::Ok
            }
            None => OkOrNotFound::NotFound(Json(ErrorBody {
                error: format!("agent {} not found", body.agent_id),
            })),
        }
    }

    /// List all tasks for an agent.
    #[oai(path = "/task/list", method = "get")]
    async fn task_list(
        &self,
        state: poem::web::Data<&SharedState>,
        agent_id: Query<String>,
    ) -> Json<TaskListResponse> {
        let snapshot = {
            let agents = state.agents.read().await;
            agents.get(&agent_id.0)
                .map(|a| a.task_snapshot.clone())
                .unwrap_or_default()
        };

        if !snapshot.is_empty() {
            let mut items = snapshot;
            items.sort_by(|a, b| a.created_at.cmp(&b.created_at));
            return Json(TaskListResponse { tasks: items });
        }

        let tasks = state.tasks.read().await;
        let mut items: Vec<TaskItem> = tasks
            .values()
            .filter(|t| t.agent_id == agent_id.0)
            .map(task_item_from_record)
            .collect();
        items.sort_by(|a, b| a.created_at.cmp(&b.created_at));
        Json(TaskListResponse { tasks: items })
    }

    /// Claim the next queued task for an agent.
    #[oai(path = "/task/claim", method = "get")]
    async fn task_claim(
        &self,
        state: poem::web::Data<&SharedState>,
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
                TaskOrNotFound::Ok(Json(task_item_from_record(t)))
            }
            None => TaskOrNotFound::NotFound(Json(ErrorBody {
                error: "no queued tasks".to_string(),
            })),
        }
    }

    /// Mark a task as done or failed.
    #[oai(path = "/task/finish", method = "get")]
    async fn task_finish(
        &self,
        state: poem::web::Data<&SharedState>,
        task_id: Query<String>,
        status: Query<String>,
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
                TaskOrNotFound::Ok(Json(task_item_from_record(t)))
            }
            None => TaskOrNotFound::NotFound(Json(ErrorBody {
                error: format!("task {} not found", task_id.0),
            })),
        }
    }

    /// Mark a task as blocked.
    #[oai(path = "/task/block", method = "get")]
    async fn task_block(
        &self,
        state: poem::web::Data<&SharedState>,
        task_id: Query<String>,
        reason: Query<String>,
    ) -> TaskOrNotFound {
        let mut tasks = state.tasks.write().await;
        match tasks.get_mut(&task_id.0) {
            Some(t) => {
                t.status = TaskStatus::Blocked;
                t.result = Some(reason.0);
                TaskOrNotFound::Ok(Json(task_item_from_record(t)))
            }
            None => TaskOrNotFound::NotFound(Json(ErrorBody {
                error: format!("task {} not found", task_id.0),
            })),
        }
    }
}

// ── reaper ────────────────────────────────────────────────────────────────────

pub async fn reaper(state: SharedState) {
    let interval = state.config.reaper_interval_secs;
    let timeout = state.config.heartbeat_timeout_secs;
    let mut tick = tokio::time::interval(Duration::from_secs(interval));
    loop {
        tick.tick().await;
        let cutoff = Utc::now() - chrono::Duration::seconds(timeout);
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

// ── app builder ───────────────────────────────────────────────────────────────

pub fn build_app(state: SharedState, base_url: &str) -> BoxEndpoint<'static> {
    let api_service =
        OpenApiService::new(PunchclockApi, "Punchclock", "0.1").server(base_url);
    let swagger_ui = api_service.swagger_ui();
    let openapi_json = api_service.spec_endpoint();

    Route::new()
        .nest("/", api_service)
        .nest("/docs", swagger_ui)
        .at("/openapi.json", openapi_json)
        .with(AddData::new(state))
        .with(Cors::new())
        .with(RateLimiter::new())
        .boxed()
}

// ── landing page ──────────────────────────────────────────────────────────────

pub const INDEX_HTML: &str = r#"<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>punchclock</title>
<style>
  * { box-sizing: border-box; margin: 0; padding: 0; }
  body { font-family: ui-monospace, monospace; font-size: 14px; line-height: 1.6;
         background: #0d0d0d; color: #d4d4d4; padding: 2rem; }
  a { color: #7ec8e3; }
  h1 { font-size: 1.4rem; color: #fff; margin-bottom: 0.25rem; }
  h2 { font-size: 1rem; color: #aaa; margin: 2rem 0 0.5rem; text-transform: uppercase;
       letter-spacing: 0.08em; }
  p  { margin-bottom: 0.75rem; max-width: 70ch; }
  pre { background: #1a1a1a; border: 1px solid #333; border-radius: 4px;
        padding: 0.75rem 1rem; margin: 0.5rem 0 1rem; overflow-x: auto; }
  code { color: #ce9178; }
  pre code { color: #d4d4d4; }
  .tag  { display: inline-block; background: #1e3a2f; color: #4ec994;
          border-radius: 3px; padding: 0 0.4em; font-size: 0.85em; margin-right: 0.3em; }
  .warn { display: inline-block; background: #3a2a1e; color: #e09050;
          border-radius: 3px; padding: 0 0.4em; font-size: 0.85em; margin-right: 0.3em; }
  table { border-collapse: collapse; width: 100%; max-width: 80ch; margin-bottom: 1rem; }
  th { text-align: left; color: #888; border-bottom: 1px solid #333; padding: 0.25rem 1rem 0.25rem 0; }
  td { padding: 0.2rem 1rem 0.2rem 0; vertical-align: top; }
  td:first-child { color: #7ec8e3; white-space: nowrap; }
  .divider { border: none; border-top: 1px solid #222; margin: 2rem 0; }
</style>
</head>
<body>

<h1>punchclock</h1>
<p style="color:#888">Lightweight presence + messaging + task-routing bus for Claude agents.</p>
<p><a href="/docs">Swagger UI →</a> &nbsp; <a href="/openapi.json">OpenAPI JSON →</a></p>

<hr class="divider">

<h2>What it is</h2>
<p>
  Punchclock is a thin HTTP rendezvous server. It lets Claude Code agents running
  in different git repos find each other, exchange messages, and pick up work items.
  It has no database — all state is in-memory and intentionally ephemeral.
</p>
<p>
  The companion CLI (<code>punchclock</code>) wraps every endpoint so you can script
  agent workflows from the shell or wire them into CI.
</p>

<h2>What it is not</h2>
<p>
  <span class="warn">not</span> a task database — tasks live in <code>.task/todo/</code>
  and <code>.task/done/</code> inside each repo; the daemon pushes a snapshot via
  <code>task/sync</code> so <code>task/list</code> works on remote servers too.<br>
  <span class="warn">not</span> a message queue with durability — inbox is capped at
  100 messages and lost on restart.<br>
  <span class="warn">not</span> an auth system — any caller can use any
  <code>from</code> field; there is no token or signature validation.<br>
  <span class="warn">not</span> a scheduler — it does not retry tasks or enforce deadlines.
</p>

<hr class="divider">

<h2>Core concepts</h2>

<table>
  <tr><th>Concept</th><th>Description</th></tr>
  <tr><td>Agent</td><td>Any process that registers and sends heartbeats. Usually one per git repo running <code>punchclock agent run</code>.</td></tr>
  <tr><td>Heartbeat</td><td>Must arrive every 30 s or the agent is reaped. The daemon sends one every 15 s and re-registers automatically if reaped.</td></tr>
  <tr><td>Inbox</td><td>Per-agent FIFO message queue, capped at 100. Drained on read.</td></tr>
  <tr><td>Task</td><td>A markdown file in <code>.task/todo/</code>. The daemon claims one at a time, runs it through <code>claude -p</code>, then <code>git mv</code>s it to <code>.task/done/</code>.</td></tr>
</table>

<hr class="divider">

<h2>Quick start</h2>

<p><strong>1. Start the server</strong></p>
<pre><code>cargo run -p punchclock-server</code></pre>

<p><strong>2. Register a repo as an agent</strong></p>
<pre><code>cd ~/your-repo
punchclock agent init      # interactive: name, description, server URL
punchclock agent run       # heartbeat + poll + forward tasks to claude</code></pre>

<p><strong>3. Send a task</strong></p>
<p>Drop a markdown file into the repo's <code>.task/todo/</code> directory.
The daemon picks it up within 5 s, runs <code>claude -p &lt;body&gt;</code>, and
moves the file to <code>.task/done/</code> with the result appended.</p>

<p><strong>4. Message another agent</strong></p>
<pre><code>punchclock send --from alice --to &lt;agent_id&gt; "please review src/lib.rs"</code></pre>

<hr class="divider">

<h2>Endpoints</h2>

<table>
  <tr><th>Path</th><th>Description</th></tr>
  <tr><td>GET /register</td><td>Create an agent. Returns <code>agent_id</code>.</td></tr>
  <tr><td>GET /heartbeat</td><td>Keep alive. Re-registers automatically if reaped (pass <code>name</code> + <code>description</code>).</td></tr>
  <tr><td>GET /team</td><td>List agents with a heartbeat in the last 30 s.</td></tr>
  <tr><td>GET /message/send</td><td>Push a message to an agent's inbox. Returns message with <code>id</code> and <code>acked</code> status.</td></tr>
  <tr><td>GET /message/recv</td><td>Drain your inbox (destructive read). Messages include <code>id</code> and <code>acked</code> status.</td></tr>
  <tr><td>GET /message/broadcast</td><td>Send to all online agents at once.</td></tr>
  <tr><td>POST /message/ack</td><td>Acknowledge a message as received. Pass <code>agent_id</code> and <code>message_id</code>.</td></tr>
  <tr><td>GET /message/status</td><td>Get ack status of all messages for an agent. Pass <code>agent_id</code>.</td></tr>
  <tr><td>GET /task/list</td><td>Return the daemon-pushed task snapshot (or in-memory tasks created via <code>task/push</code>).</td></tr>
  <tr><td>GET /task/claim</td><td>Atomically grab the next queued task (sets it in-progress).</td></tr>
  <tr><td>GET /task/block</td><td>Mark a task blocked and record why. Daemon moves file to <code>.task/blocked/</code>.</td></tr>
  <tr><td>GET /task/push</td><td>Enqueue a task in-memory (for agents without a local repo path).</td></tr>
  <tr><td>GET /task/finish</td><td>Mark a claimed task done or failed.</td></tr>
</table>

<p style="color:#555; margin-top:2rem">All endpoints use GET — writes via query params.</p>

</body>
</html>
"#;
