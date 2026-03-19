# Implement `punchclock pause` and `punchclock resume` commands

Toggle the `enabled` flag on a registered repo.

## Requirements

- Add `Pause { name: String }` and `Resume { name: String }` variants to `Cmd`
- `pause`: set `enabled = false`, save, print `paused "<name>"`
- `resume`: set `enabled = true`, save, print `resumed "<name>"`
- Error if name not found in repos.toml

## Files to edit
- `client/src/main.rs`

## Dependencies
- Task 01
