# Initial implementation

- Server: register, heartbeat, team, send, recv endpoints
- Reaper task removes agents after 30 s heartbeat timeout
- Inbox capped at 100 messages (oldest dropped on overflow)
- Swagger UI at /docs, OpenAPI JSON at /openapi.json
- CLI client with clap subcommands (register, heartbeat, team, send, inbox)
- .env.sample with API_BASE_URL and RUST_LOG
