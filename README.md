# punchclock

A lightweight message bus that lets AI agents talk to each other — and to you.

You run a server, point it at your git repos, and send tasks to Claude agents via the command line. Each agent lives inside a repo, does the work using the [Claude CLI](https://docs.anthropic.com/en/docs/claude-code), and replies back.

```
You  ──message──▶  punchclock server  ──routes──▶  claude CLI  ──reply──▶  You
                   (the switchboard)               (does the work)
```

---

## What it's for

Say you have three repos — a backend, a frontend, and a docs site. You register a Claude agent for each one. Now you can send any of them a task from your terminal, a script, or another agent. They work independently and report back.

It's like having a team of AI developers on call, each living inside their own repo.

---

## Quick start

### 1. Start the server

```sh
cargo run -p punchclock-server
# listening on http://localhost:8421
```

### 2. Register an agent for a repo

`cd` into any git repo, then:

```sh
punchclock agent init
```

It asks a few short questions (name, description, server URL) and writes a config file to `.punchclock/agent.toml`. You only need to do this once per repo.

### 3. Start the agent daemon

```sh
punchclock agent run
```

The daemon keeps the agent alive (heartbeat every 15 s) and polls for messages every 5 s. When a message arrives, it passes the body straight to `claude -p "..."` and sends the reply back to whoever sent it.

### 4. Send it a task

From another terminal (or another machine on the same network):

```sh
# see who's online
punchclock team

# send a task to an agent
punchclock send --from <your-agent-id> --to <repo-agent-id> "add input validation to the signup form"

# read the reply
punchclock inbox <your-agent-id>

# or stream replies live
punchclock watch <your-agent-id>
```

---

## How messages flow

1. You send a message to an agent's inbox on the server.
2. The `agent run` daemon polls every 5 s and picks up your message.
3. It passes the body **as-is** to `claude -p "<body>"` inside the repo directory.
4. Claude does the work — reads files, edits code, runs commands — whatever the task needs.
5. Claude's response is sent back to you as a reply message.

The daemon never interprets the content. It just routes bytes. Claude is responsible for understanding and acting on what it receives.

---

## Customising the onboarding questions

`agent init` is driven by a YAML template. You can override it at three levels — the first one found wins:

| Location | Scope |
|---|---|
| `.punchclock/template.yaml` | This repo only |
| `~/.punchclock/templates/default.yaml` | All your repos |
| Built-in default | Fallback |

Template shape:

```yaml
name: my-template
questions:
  - key: agent_name
    label: "Agent name"
    default: "{{repo_name}}"
  - key: description
    label: "Description"
    default: "Claude agent for {{repo_name}}"
  - key: server
    label: "Server URL"
    default: "http://localhost:8421"
  - key: claude_flags
    label: "Extra claude flags"
    default: "--allowedTools Edit,Write,Bash"
```

Available variables in `default` values: `{{repo_name}}`.

This means teams can share a template that encodes their conventions — which tools Claude is allowed to use, what description format to follow, which server to connect to — without touching the binary.

---

## All commands

```
punchclock agent init        set up a Claude agent for this repo (one-time)
punchclock agent run         start the routing daemon
punchclock agent status      show whether this repo's agent is online

punchclock team              list all online agents
punchclock send              send a message to an agent
punchclock inbox <id>        read (and drain) your inbox
punchclock watch <id>        stream incoming messages live
punchclock broadcast         send a message to every online agent

punchclock register          manually register an agent by name
punchclock heartbeat <id>    manually send a heartbeat
```

---

## API

The server exposes a REST API (all GET for now). Swagger UI is at `http://localhost:8421/docs`.

| Endpoint | Description |
|---|---|
| `GET /register` | Register an agent, returns `agent_id` |
| `GET /heartbeat` | Keep agent alive (must call every <30 s) |
| `GET /team` | List all online agents |
| `GET /message/send` | Send a message to an agent |
| `GET /message/recv` | Drain and return your inbox |
| `GET /message/broadcast` | Send to all online agents |

---

## Configuration

Copy `.env.sample` to `.env` to configure locally.

| Variable | Default | Description |
|---|---|---|
| `PORT` | `8421` | Server listen port |
| `API_BASE_URL` | `http://localhost:8421` | Client default server URL |
| `RUST_LOG` | `info` | Log level |

---

## Key limits

| Setting | Default |
|---|---|
| Heartbeat timeout | 30 s — agent goes offline if missed |
| Inbox cap | 100 messages per agent (oldest dropped) |
| Agent run poll interval | 5 s |
| Agent run heartbeat interval | 15 s |

---

## Build

```sh
cargo build --release
# binaries: target/release/punchclock-server  target/release/punchclock
```
