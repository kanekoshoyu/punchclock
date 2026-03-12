# Integration tests

Spin up a real server in a test harness and exercise the full flow.

- Use `tokio::test` + bind server on a random port
- Test: register → heartbeat → team → send → recv round-trip
- Test: reaper removes agent after heartbeat timeout
- Test: inbox cap (100 messages) drops oldest on overflow
- Live in `server/tests/` or a top-level `tests/` crate

## Result

All 3 tests pass. Here's what was done:

**Refactor (`server/src/lib.rs` + `server/src/main.rs`)**
- Extracted all server logic into a library crate (`punchclock_server`)
- Added `pub struct ServerConfig` with `heartbeat_timeout_secs`, `reaper_interval_secs`, and `max_inbox` — all configurable so tests can use short timeouts
- `AppState::new(config)` stores the config; `team`, `broadcast`, `send_message` all read from it instead of hard-coded constants
- `pub fn build_app(state, base_url) -> BoxEndpoint<'static>` and `pub async fn reaper(state)` are the two public entry points
- `main.rs` is now a thin wrapper

**Tests (`server/tests/integration_test.rs`)**

| Test | What it covers |
|---|---|
| `test_register_heartbeat_team_send_recv` | Full round-trip: register two agents, heartbeat, confirm both appear in `/team`, send a message, drain inbox, verify empty on second read |
| `test_reaper_removes_stale_agent` | Uses `heartbeat_timeout_secs: 1, reaper_interval_secs: 1`; waits 3 s; confirms agent disappears from `/team` and `/heartbeat` returns 404 |
| `test_inbox_cap_drops_oldest` | Sends 101 messages to a 100-cap inbox; confirms 100 remain and `msg-0` was evicted (unique `from` per sender to avoid the rate limiter's 60 req/min cap) |
