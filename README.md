# punchclock

A lightweight presence + messaging + task-routing bus for Claude agents.

Run a server, register your git repos as agents, drop tasks into `.task/todo/`, and Claude does the work.

```
You  ──task file──▶  .task/todo/   ──claimed by──▶  punchclock daemon  ──▶  claude -p  ──▶  .task/done/
                                                      (heartbeat + poll)
```

---

## What it is

- **Presence** — agents register and heartbeat; `/team` shows who's online
- **Messaging** — per-agent inbox for ad-hoc messages between agents or from scripts
- **Task routing** — daemon polls for `.task/todo/*.md` files, runs each through `claude -p`, moves them to `.task/done/`
- **Tutorial page** — `GET /` serves a quick-start guide in your browser

## What it is not

- **Not a task database** — tasks live as markdown files in the repo; the server reads them from disk, not memory
- **Not durable** — all agent state (presence, inboxes) is in-memory and lost on restart; agents re-register automatically on the next heartbeat
- **Not authenticated** — any caller can use any `from` field; no tokens or signatures
- **Not a scheduler** — no retries, deadlines, or priority queues

---

## Quick start

### 1. Start the server

```sh
cargo run -p punchclock-server
# http://localhost:8421  (tutorial page)
# http://localhost:8421/docs  (Swagger UI)
```

### 2. Register an agent for a repo

`cd` into any git repo, then:

```sh
punchclock agent init
```

Asks for name, description, server URL, and optional extra `claude` flags. Registers with the server and writes `.punchclock/agent.toml`. One-time per repo.

### 3. Start the daemon

```sh
punchclock agent run
```

The daemon:
- sends a heartbeat every 15 s (re-registers automatically if the server restarted)
- polls for messages every 5 s and routes them to `claude -p`
- polls for `.task/todo/*.md` files every 5 s, claims one at a time, runs it through `claude -p`, and `git mv`s it to `.task/done/` with the result appended

### 4. Push a task

Drop a markdown file into `.task/todo/` in the target repo:

```sh
cat > /path/to/repo/.task/todo/my-task.md << 'EOF'
# Add input validation to signup form

The signup form at src/components/SignupForm.tsx has no validation.
Add client-side validation for email format and password length (min 8 chars).
EOF
```

Or use the CLI to push into the server's in-memory queue (useful when the agent has no local repo path set):

```sh
punchclock task push --to <agent-id> "title" "full task body"
```

### 5. Check progress

```sh
punchclock team                         # who's online
punchclock task list <agent-id>         # queued + done tasks (reads from .task/)
```

---

## How tasks flow

1. A markdown file appears in `.task/todo/<id>.md` **inside the agent's repo** (e.g. `~/myrepo/.task/todo/`).
2. The daemon (running inside that repo) claims it and runs `claude -p "<body>"` in the repo directory.
3. On completion the file is `git mv`d to `.task/done/<id>.md` and the result is appended under `## Result`.
4. If Claude outputs `BLOCKED: <reason>` as its first line, the file goes to `.task/blocked/` instead, with the reason appended under `## Blocked`.
5. `GET /task/list` reads all three directories **directly from the agent's repo on disk** using the `repo_path` the agent registered with. The punchclock server holds no task state of its own — it is just reading the remote filesystem path at request time.

---

## Messaging

Agents can also receive freeform messages (not tasks):

```sh
punchclock send --from <your-id> --to <agent-id> "please review src/lib.rs"
punchclock inbox <agent-id>        # drain inbox
punchclock watch <agent-id>        # stream live
punchclock broadcast --from <id> "deploy freeze starting now"
```

Messages are passed to `claude -p` and the reply is sent back to the sender.

---

## Customising init questions

`agent init` is driven by a YAML template. First found wins:

| Location | Scope |
|---|---|
| `.punchclock/template.yaml` | This repo only |
| `~/.punchclock/templates/default.yaml` | All your repos |
| Built-in default | Fallback |

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

---

## All commands

```
punchclock agent init          set up a Claude agent for this repo (one-time)
punchclock agent run           start the routing daemon
punchclock agent status        show whether this repo's agent is online

punchclock team                list all online agents
punchclock send                send a message to an agent
punchclock inbox <id>          drain your inbox
punchclock watch <id>          stream incoming messages live
punchclock broadcast           send to every online agent

punchclock task push           enqueue a task in the server's in-memory queue
punchclock task list <id>      list tasks (reads .task/todo/ + .task/done/)

punchclock register            manually register an agent
punchclock heartbeat <id>      manually send a heartbeat
```

---

## API

Swagger UI: `http://localhost:8421/docs` — Tutorial: `http://localhost:8421/`

| Endpoint | Description |
|---|---|
| `GET /` | Tutorial and quick-start guide |
| `GET /register` | Register an agent; pass `repo_path` for filesystem-backed task list |
| `GET /heartbeat` | Keep alive; re-registers if reaped when `name`+`description` supplied |
| `GET /team` | Online agents (heartbeat within last 30 s) |
| `GET /message/send` | Push to an agent's inbox |
| `GET /message/recv` | Drain inbox (destructive) |
| `GET /message/broadcast` | Send to all online agents |
| `GET /task/list` | Read `.task/{todo,done,blocked}/` from the **agent's repo** (path registered at init) |
| `GET /task/claim` | Atomically claim the next queued task |
| `GET /task/push` | Enqueue a task in server memory |
| `GET /task/finish` | Mark a claimed task done or failed |

---

## Configuration

Copy `server/.env.sample` to `server/.env`:

| Variable | Default | Description |
|---|---|---|
| `API_BASE_URL` | `http://localhost:8421` | Advertised base URL (also sets listen port) |
| `RUST_LOG` | `info` | Log level |

---

## Key limits

| Setting | Value |
|---|---|
| Heartbeat timeout | 30 s |
| Reaper poll interval | 10 s |
| Inbox cap | 100 messages per agent (oldest dropped) |
| Daemon poll interval | 5 s (messages + tasks) |
| Daemon heartbeat interval | 15 s |

---

## Build & install

```sh
cargo build --release
# server: target/release/punchclock-server
# client: target/release/punchclock

cargo install --path client   # install punchclock to ~/.cargo/bin
```
