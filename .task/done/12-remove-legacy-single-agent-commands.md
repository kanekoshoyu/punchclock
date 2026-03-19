# Remove legacy single-agent commands

Remove these subcommands that the daemon now handles automatically:
- `register` — daemon registers on heartbeat
- `heartbeat` — daemon sends heartbeats
- `inbox` — daemon drains inbox
- `watch` — daemon routes messages

Delete the corresponding match arms in `main.rs` and any helper functions that become dead code.
Clean up unused imports from `punchclock_common`.
