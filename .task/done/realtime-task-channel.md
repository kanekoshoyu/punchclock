# Real-time bidirectional task channel

Replace the polling task model with a persistent WebSocket so task assignment
and status reporting are instantaneous in both directions.

## Problem with current model

- Operator pushes task via `POST /task/push` → agent polls `/task/claim` every
  5 s → up to 5 s latency before work starts.
- Agent calls `/task/finish` or `/task/block` → operator polls `/task/list` to
  find out → no push notification, no acknowledgement.
- No way for operator to cancel or reprioritise an in-flight task.

## Proposed design

### Single WebSocket endpoint

```
GET /ws/{agent_id}   — agent connects; full-duplex JSON frames
GET /ws/observe      — operator/browser connects; receives all task events (read-only)
```

Both endpoints share the same internal event bus (`tokio::broadcast`).

### Frame types (JSON envelope)

**Server → agent**
```json
{ "type": "task.assign",    "task": { ...TaskItem } }
{ "type": "task.cancel",    "task_id": "..." }
{ "type": "ping" }
```

**Agent → server**
```json
{ "type": "task.claimed",   "task_id": "..." }
{ "type": "task.progress",  "task_id": "...", "message": "..." }
{ "type": "task.done",      "task_id": "...", "result": "..." }
{ "type": "task.failed",    "task_id": "...", "error": "..." }
{ "type": "task.blocked",   "task_id": "...", "reason": "..." }
{ "type": "pong" }
```

**Server → observer (browser)**
```json
{ "type": "task.assigned",  "agent_id": "...", "task": { ...TaskItem } }
{ "type": "task.status",    "task_id": "...", "status": "...", "detail": "..." }
{ "type": "agent.online",   "agent_id": "..." }
{ "type": "agent.offline",  "agent_id": "..." }
```

### Server-side state changes

- Replace per-agent `VecDeque<Message>` inbox for tasks with an
  `mpsc::Sender<WsFrame>` stored on `AgentRecord` when connected.
- `POST /task/push` (or operator sends via `/ws/observe`) → if agent is
  connected, push `task.assign` frame directly; otherwise buffer to a small
  pending queue (fall back to existing `/task/claim` poll for offline agents).
- On receiving `task.done` / `task.failed` / `task.blocked`, update
  `TaskRecord` in state and broadcast a `task.status` event to all observers.

### Client daemon changes

- `agent run` opens `GET /ws/{agent_id}` on startup, reconnects with
  exponential back-off on disconnect.
- Task loop becomes event-driven: wait for `task.assign` frame instead of
  polling `/task/claim`.
- Send `task.progress` frames periodically while `claude -p` is running (tail
  stdout lines).
- Keep HTTP polling as fallback when WebSocket is unavailable.

### Browser / operator use

- Swagger UI observer tab or a thin HTML page at `/observe` connects to
  `/ws/observe` and renders live task events.
- Operator can POST new tasks via the existing REST endpoint; they appear in
  the observer stream in real-time.

## Dependencies

- `poem` already has WebSocket via `poem::web::websocket` — no new server dep.
- Client: add `tokio-tungstenite` for the daemon's WS client.
- Subsumes the messaging WebSocket from `websocket-transport.md` — implement
  together and close that todo too.

## Implementation order

1. Add `tokio::broadcast` bus and `/ws/observe` (read-only, easiest to test)
2. Add `/ws/{agent_id}` with `task.assign` push + status frames from agent
3. Wire daemon task loop to WebSocket, keep HTTP fallback
4. Add `task.progress` streaming from `claude -p` stdout
