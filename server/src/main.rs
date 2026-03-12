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
}

struct Message {
    from: String,
    body: String,
    timestamp: DateTime<Utc>,
}

#[derive(Default)]
struct AppState {
    agents: RwLock<HashMap<String, AgentRecord>>,
    inboxes: RwLock<HashMap<String, VecDeque<Message>>>,
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
    ) -> Json<RegisterResponse> {
        let id = Uuid::new_v4().to_string();
        let record = AgentRecord {
            id: id.clone(),
            name: name.0,
            description: description.0,
            last_heartbeat: Utc::now(),
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
    ) -> OkOrNotFound {
        let mut agents = state.agents.write().await;
        match agents.get_mut(&agent_id.0) {
            Some(a) => {
                a.last_heartbeat = Utc::now();
                OkOrNotFound::Ok
            }
            None => OkOrNotFound::NotFound(Json(ErrorBody {
                error: format!("agent {} not found", agent_id.0),
            })),
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
    let _ = dotenvy::dotenv(); // ok if .env is absent

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
