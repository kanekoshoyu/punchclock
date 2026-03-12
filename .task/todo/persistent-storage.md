# Persistent storage

Replace in-memory `HashMap` with SQLite via `sqlx` so agents and messages
survive server restarts.

- Add `sqlx` with `sqlite` feature + `sqlx-cli` for migrations
- Schema: `agents` table, `messages` table
- Reaper becomes a `DELETE WHERE last_heartbeat < ?` query
- Consider: keep in-memory as a fast path, persist async
