# punchclock — Claude context

## Project

Rust workspace with two crates:
- `server/` — `punchclock-server` binary: poem HTTP API, in-memory state, reaper task
- `client/` — `punchclock` binary: clap CLI that calls the server

## Build & run

```sh
cargo build                          # both crates
cargo run -p punchclock-server       # start server (default :8421)
cargo run -p punchclock -- <cmd>     # run CLI
cargo test                           # run all tests
```

## Key facts

- All state is in-memory — lost on restart. No database yet.
- Heartbeat timeout: 30 s (`HEARTBEAT_TIMEOUT_SECS`). Reaper polls every 10 s.
- Inbox cap: 100 messages per agent (`MAX_INBOX`); oldest dropped on overflow.
- All endpoints currently use GET (including writes — known issue, see `.task/todo`).
- No authentication — any caller can impersonate any `from` field.
- Swagger UI at `/docs`, OpenAPI JSON at `/openapi.json`.

## Conventions

- Keep server and client response types in sync manually (no shared crate yet).
- Use `tracing::info!` / `tracing::warn!` for structured logs, not `println!`.
- Error responses use `{"error": "<message>"}` JSON body.
- Use `git mv` (never plain `mv`) when moving files between `.task/todo/` and `.task/done/`.
