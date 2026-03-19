# Rename server-facing commands for clarity

Rename the remaining server-facing commands to make it clear they talk to the server:

- `team` → `ps` — list all online agents (like `docker ps`)
- `send` — keep name, but update help text: "Send a message to a named agent via the server"
- `broadcast` — keep name, update help text similarly
- `task` — keep as-is (server-mediated task management)

Update help descriptions to mention "(requires server)" so the user knows.
