# Implement `punchclock ls` command

Add a CLI command to list all registered repos and their status.

## Requirements

- Add `Ls` variant to `Cmd` enum (no args)
- In the match arm: load repos, print a table with columns: `NAME`, `PATH`, `ENABLED`
- If no repos registered, print: `no repos registered — use "punchclock add <path>" to add one`

## Files to edit
- `client/src/main.rs`

## Dependencies
- Task 01
