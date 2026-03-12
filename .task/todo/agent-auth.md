# Agent authentication

Any caller can spoof the `from` field or issue heartbeats/recvs for another agent's ID.

- Return a `secret_token` alongside `agent_id` at `/register`
- Require the token as a header or query param on `/heartbeat`, `/message/send`, `/message/recv`
- Server validates token before acting
