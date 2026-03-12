use punchclock_server::{AppState, ServerConfig, SharedState, build_app, reaper};
use reqwest::Client;
use serde_json::Value;
use std::sync::Arc;
use std::time::Duration;

// ── test harness ──────────────────────────────────────────────────────────────

/// Bind port 0 to find a free port, then release it for poem to rebind.
fn free_port() -> u16 {
    let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    l.local_addr().unwrap().port()
}

struct TestServer {
    base: String,
    client: Client,
}

impl TestServer {
    async fn start(config: ServerConfig) -> Self {
        let port = free_port();
        let base = format!("http://127.0.0.1:{port}");
        let state: SharedState = Arc::new(AppState::new(config));

        let app = build_app(state.clone(), &base);
        tokio::spawn(
            poem::Server::new(poem::listener::TcpListener::bind(format!("127.0.0.1:{port}")))
                .run(app),
        );
        tokio::spawn(reaper(state));

        // Give the server a moment to accept connections.
        tokio::time::sleep(Duration::from_millis(150)).await;

        Self { base, client: Client::new() }
    }

    fn url(&self, path: &str) -> String {
        format!("{}{}", self.base, path)
    }

    async fn get(&self, path: &str, params: &[(&str, &str)]) -> Value {
        self.client
            .get(self.url(path))
            .query(params)
            .send()
            .await
            .expect("request failed")
            .json()
            .await
            .expect("invalid JSON")
    }

    async fn get_status(&self, path: &str, params: &[(&str, &str)]) -> u16 {
        self.client
            .get(self.url(path))
            .query(params)
            .send()
            .await
            .expect("request failed")
            .status()
            .as_u16()
    }
}

// ── helpers ───────────────────────────────────────────────────────────────────

async fn register(srv: &TestServer, name: &str, desc: &str) -> String {
    let resp = srv.get("/register", &[("name", name), ("description", desc)]).await;
    resp["agent_id"].as_str().unwrap().to_string()
}

// ── tests ─────────────────────────────────────────────────────────────────────

/// register → heartbeat → team → send → recv round-trip
#[tokio::test]
async fn test_register_heartbeat_team_send_recv() {
    let srv = TestServer::start(ServerConfig::default()).await;

    // Register two agents.
    let alice_id = register(&srv, "alice", "first agent").await;
    let bob_id = register(&srv, "bob", "second agent").await;

    // Heartbeat for alice.
    let status = srv.get_status("/heartbeat", &[("agent_id", &alice_id)]).await;
    assert_eq!(status, 200);

    // Both should appear in /team.
    let team = srv.get("/team", &[]).await;
    let agents = team["agents"].as_array().unwrap();
    assert_eq!(agents.len(), 2);
    let names: Vec<&str> = agents.iter().map(|a| a["name"].as_str().unwrap()).collect();
    assert!(names.contains(&"alice"));
    assert!(names.contains(&"bob"));

    // Alice sends a message to bob.
    let status = srv
        .get_status(
            "/message/send",
            &[("to", &bob_id), ("from", &alice_id), ("body", "hello bob")],
        )
        .await;
    assert_eq!(status, 200);

    // Bob receives it.
    let inbox = srv.get("/message/recv", &[("agent_id", &bob_id)]).await;
    let msgs = inbox["messages"].as_array().unwrap();
    assert_eq!(msgs.len(), 1);
    assert_eq!(msgs[0]["body"].as_str().unwrap(), "hello bob");
    assert_eq!(msgs[0]["from"].as_str().unwrap(), alice_id);

    // Second recv is empty (inbox was drained).
    let inbox2 = srv.get("/message/recv", &[("agent_id", &bob_id)]).await;
    assert_eq!(inbox2["messages"].as_array().unwrap().len(), 0);
}

/// Reaper removes agent whose heartbeat has expired.
#[tokio::test]
async fn test_reaper_removes_stale_agent() {
    let config = ServerConfig {
        heartbeat_timeout_secs: 1,
        reaper_interval_secs: 1,
        max_inbox: 100,
    };
    let srv = TestServer::start(config).await;

    let id = register(&srv, "ephemeral", "will be reaped").await;

    // Should be online immediately after registration.
    let team = srv.get("/team", &[]).await;
    assert_eq!(team["agents"].as_array().unwrap().len(), 1);

    // Stop heartbeating and wait for timeout + one reaper cycle (1s + 1s + buffer).
    tokio::time::sleep(Duration::from_secs(3)).await;

    // Agent should be gone from /team.
    let team = srv.get("/team", &[]).await;
    assert_eq!(team["agents"].as_array().unwrap().len(), 0);

    // Heartbeat without name/description returns 404.
    let status = srv.get_status("/heartbeat", &[("agent_id", &id)]).await;
    assert_eq!(status, 404);
}

/// Inbox capped at max_inbox: the oldest message is dropped on overflow.
#[tokio::test]
async fn test_inbox_cap_drops_oldest() {
    let config = ServerConfig {
        heartbeat_timeout_secs: 30,
        reaper_interval_secs: 10,
        max_inbox: 100,
    };
    let srv = TestServer::start(config).await;

    let id = register(&srv, "capped", "inbox cap test").await;

    // Send 101 messages; the first one should be evicted.
    // Use a unique `from` per message so the per-sender rate limiter (60 req/min)
    // doesn't interfere with the inbox-cap logic under test.
    for i in 0..=100_u32 {
        let body = format!("msg-{i}");
        let from = format!("sender-{i}");
        srv.get_status(
            "/message/send",
            &[("to", &id), ("from", &from), ("body", &body)],
        )
        .await;
    }

    let inbox = srv.get("/message/recv", &[("agent_id", &id)]).await;
    let msgs = inbox["messages"].as_array().unwrap();

    // Exactly 100 messages remain.
    assert_eq!(msgs.len(), 100);

    // msg-0 was evicted; inbox starts at msg-1.
    assert_eq!(msgs[0]["body"].as_str().unwrap(), "msg-1");
    // Last message is msg-100.
    assert_eq!(msgs[99]["body"].as_str().unwrap(), "msg-100");
}
