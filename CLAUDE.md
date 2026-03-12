# punchclock — Claude context

## Project

Rust workspace with two crates:
- `server/` — `punchclock-server` binary: poem HTTP API, in-memory presence + inbox, reaper task
- `client/` — `punchclock` binary: clap CLI + agent daemon that calls the server

## Build & run

```sh
cargo build                          # both crates
cargo run -p punchclock-server       # start server (default :8421)
cargo run -p punchclock -- <cmd>     # run CLI
cargo install --path client          # install to ~/.cargo/bin
cargo test                           # run all tests
```

## Key facts

- **Agent state is in-memory** — presence and inboxes are lost on restart. Agents re-register automatically on the next heartbeat (daemon passes `name`, `description`, `repo_path` on every heartbeat call).
- **Tasks live on disk, in the agent's repo — not in the punchclock repo.** `task/list` reads `.task/todo/`, `.task/done/`, and `.task/blocked/` from the `repo_path` registered by the agent (e.g. `~/Documents/trading/albatross/.task/`). The server reads that path directly from the filesystem at request time. Nothing in the punchclock server's own directory is involved.
- `repo_path` is the absolute path to the agent's git repo root. It is stored on `AgentRecord` and sent on both `/register` and `/heartbeat` so it survives reaper evictions.
- Heartbeat timeout: 30 s (`HEARTBEAT_TIMEOUT_SECS`). Reaper polls every 10 s.
- Inbox cap: 100 messages per agent (`MAX_INBOX`); oldest dropped on overflow.
- All endpoints use GET (writes via query params — known issue, see `.task/todo/use-post-for-writes.md`).
- No authentication — any caller can use any `from` field.
- Tutorial page at `/`, Swagger UI at `/docs`, OpenAPI JSON at `/openapi.json`.
- Daemon unsets `CLAUDECODE` env var before spawning `claude -p` to allow nested sessions.
- If `claude -p` output starts with `BLOCKED: <reason>`, the daemon moves the file to `.task/blocked/`, appends the reason under `## Blocked`, and calls `/task/block`. Otherwise done/failed go to `.task/done/`.
- Agent conventions for using `.task/` are in `.task/AGENTS.md` — copy this file into any repo you register as an agent so Claude knows the rules.

## Architecture

```
.task/todo/<id>.md  ──(daemon polls every 5s)──▶  task/claim  ──▶  claude -p  ──▶  git mv → .task/done/
```

The daemon (`punchclock agent run`) does three things concurrently:
1. Heartbeat loop (every 15 s) — keeps the agent alive; re-registers if reaped
2. Task loop (every 5 s) — claims one task at a time, runs it through `claude -p`, marks done/failed
3. Message loop (every 5 s) — drains inbox, routes each message to `claude -p`, replies to sender

## Conventions

- Keep server and client response types in sync manually (no shared crate yet).
- Use `tracing::info!` / `tracing::warn!` for structured logs, not `println!`.
- Error responses use `{"error": "<message>"}` JSON body.
- Use `git mv` (never plain `mv`) when moving files between `.task/todo/` and `.task/done/`.
- The landing page HTML lives as `const INDEX_HTML: &str` in `server/src/main.rs` and is served via the OpenAPI `GET /` endpoint (not a separate poem route — avoids root-path routing conflicts).
