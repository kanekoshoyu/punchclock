# Resolve agent names to UUIDs in CLI commands

Make all commands that take `agent_id` also accept agent names.

## Requirements

- Add a helper function `resolve_agent(client, base, name_or_id) -> Result<String>`:
  1. If input looks like a UUID, return it as-is
  2. Otherwise, call `/team` to get the agent list, find the agent whose `name` matches, return its `id`
  3. Error if no match found
- Update `resolve_id` closure in `main.rs` to first check repos.toml by name, then fall back to the current logic
- Update the `--to` argument on `send` and `task push` to use `resolve_agent`
- This lets users write `punchclock send --to albatross "hello"` instead of passing UUIDs

## Files to edit
- `client/src/main.rs`

## Dependencies
- Task 01
