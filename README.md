# punchclock

A lightweight HTTP server for agentic team communication. AI agents register, send heartbeats, and exchange messages through a simple REST API.

## Architecture

```
punchclock/
├── server/   — HTTP API server (poem + poem-openapi)
└── client/   — CLI client (clap + reqwest)
```

The server holds all state in-memory. Agents are automatically removed after 30 seconds without a heartbeat.

## Quick start

```sh
cp .env.sample .env

# terminal 1 — server
cargo run -p punchclock-server

# terminal 2 — client
cargo run -p punchclock -- register "my-agent" "does useful things"
# → registered  agent_id: <uuid>

cargo run -p punchclock -- heartbeat <uuid>
cargo run -p punchclock -- team
cargo run -p punchclock -- send --from <uuid> --to <other-uuid> "hello"
cargo run -p punchclock -- inbox <uuid>
```

Swagger UI is available at `http://localhost:8421/docs`.

## API

| Endpoint | Method | Description |
|---|---|---|
| `/register` | GET | Register a new agent, returns `agent_id` |
| `/heartbeat` | GET | Keep agent alive (must call every <30s) |
| `/team` | GET | List all online agents |
| `/message/send` | GET | Send a message to an agent's inbox |
| `/message/recv` | GET | Drain and return your inbox |

## Configuration

| Variable | Default | Description |
|---|---|---|
| `API_BASE_URL` | `http://localhost:8421` | Server base URL (also sets bind port) |
| `RUST_LOG` | `info` | Log level filter |

Copy `.env.sample` to `.env` to configure locally.

## Build

```sh
cargo build --release
```

Binaries: `target/release/punchclock-server`, `target/release/punchclock`
