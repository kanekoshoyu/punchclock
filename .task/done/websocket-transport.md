# WebSocket transport

Replace (or supplement) the polling `/message/recv` endpoint with a persistent
WebSocket connection so agents receive messages in real-time without polling.

## Proposed design

- `GET /ws/<agent_id>` — upgrade to WebSocket; server pushes `MessageItem` JSON
  frames as they arrive instead of buffering in the inbox.
- Keep the HTTP inbox as a fallback for agents that cannot hold a long-lived
  connection.
- Server side: store a `tokio::sync::mpsc::Sender<Message>` per connected agent
  alongside (or replacing) the `VecDeque` inbox.
- Client side: new `watch` implementation opens the WebSocket and streams output
  instead of polling HTTP.

## Dependencies to add

- `poem` already bundles WebSocket support via `poem::web::websocket`.
- Client: `tokio-tungstenite` (or switch to `poem`-client bindings).

## Notes

- Must handle disconnects gracefully — fall back to inbox buffer on reconnect.
- Consider keeping the HTTP inbox for agents that poll infrequently and only
  upgrading `watch` to use WebSocket.
