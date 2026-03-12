# claude-agent-daemon

Add `punchclock agent` subcommands that turn the client into a thin routing
daemon bridging the punchclock message bus and the `claude` CLI. The client
does **not** compose prompts or interpret task content — it only routes bytes.

## Design

```
punchclock server  ←→  punchclock agent (daemon)  ←→  claude CLI
   (message bus)          (routing layer)               (agent brain)
```

### New subcommands

```
punchclock agent init   # one-time setup: register, write .punchclock/agent.toml
punchclock agent run    # blocking daemon: heartbeat + poll + route to claude
punchclock agent status # show this repo's agent entry on /team + unread count
```

### `agent init`

Interactive wizard that asks a small set of questions, then:

1. Registers with the server (`/register`).
2. Writes `.punchclock/agent.toml` with agent_id, name, description, server URL,
   and optional claude flags.

The wizard questions are **defined by a YAML template**, not hard-coded.
Template resolution order (first found wins):

```
.punchclock/template.yaml          ← repo-level override
~/.punchclock/templates/<name>.yaml ← user personal library
built-in "default" bundled in binary
```

Built-in default template questions:
- Agent name (default: `{{repo_name}}` — inferred from git remote or dir name)
- Description (default: `"Claude agent for {{repo_name}}"`)
- Server URL (default: `http://localhost:8421`)
- Extra claude flags (default: `""`)

Context variables available in `{{...}}` defaults:
- `repo_name`, `repo_path`, `repo_remote`

Template YAML shape:
```yaml
name: default
questions:
  - key: agent_name
    label: "Agent name"
    default: "{{repo_name}}"
    type: string          # string | path | multiline | choice | bool
  - key: claude_flags
    label: "Extra claude CLI flags"
    default: ""
    type: string
```

### `agent run`

Blocking daemon. On startup:

1. Load `.punchclock/agent.toml`.
2. Spawn background task: send heartbeat every 15 s.
3. Loop (poll every 5 s):
   a. `GET /message/recv?agent_id=<id>` — drain inbox.
   b. For each message: spawn `claude -p "<body>" <claude_flags>` in repo root.
   c. Capture stdout; send it back to `message.from` via `/message/send`.
   d. Log sent/received to stderr.

No prompt injection, no task awareness — the message body is passed as-is to
`claude -p`. The sender (human or another agent) is responsible for content.

### `agent status`

1. Call `/team`, find agent by saved id.
2. Call `/message/recv` to count pending messages (then re-queue — or just show
   count without draining; decide at implementation time).
3. Print one-line summary.

## Files to create / modify

- `client/src/main.rs` — add `Agent` variant to `Cmd` enum with subcommands
- `client/src/agent.rs` — init, run, status logic
- `client/Cargo.toml` — add `serde_yaml`, `toml`, `dialoguer` dependencies

## Out of scope for v1

- Prompt composition or task file parsing (that is Claude's job)
- Multi-message conversation threading
- Template inheritance (`extends:`)
- Choice/bool question types (string + multiline only for now)
