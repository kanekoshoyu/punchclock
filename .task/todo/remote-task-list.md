# Support remote servers: task/list must not read local filesystem

## Problem

`GET /task/list` currently reads `.task/{todo,done,blocked}/` directly from the
filesystem using the `repo_path` stored on `AgentRecord`. This only works when
the server and all agent repos are on the same machine. A remote server (VPS,
container, cloud) cannot access the agent's local disk.

## Options

**A — Daemon pushes task state to server on each transition**
The daemon calls server endpoints (`task/push`, `task/finish`, `task/block`) to
mirror every file-system state change into server memory. `task/list` reads from
memory as before. Simple, but re-introduces server-side task storage and the
source-of-truth question.

**B — Daemon exposes a local HTTP endpoint that the server proxies**
Each agent runs a small HTTP server (e.g. `0.0.0.0:8422`) that serves its own
`.task/` directory. `repo_path` on `AgentRecord` becomes a `task_url` (e.g.
`http://agent-host:8422`). The server fetches from that URL when `task/list` is
called. Keeps the filesystem as source of truth, but requires agents to be
network-reachable from the server.

**C — Daemon polls and pushes diffs only**
On each daemon tick, diff `.task/` against last-known state and push only
changes. Server stays thin. Works behind NAT (agent initiates all connections).
Most complex to implement correctly.

## Recommendation

Start with **A** as the simplest unblock for remote deployments, with the
understanding that the filesystem remains the source of truth for the daemon
and `task/list` is eventually-consistent. Remove `repo_path`-based filesystem
reads from the server entirely.
