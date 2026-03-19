# Implement `punchclock rm` command

Add a CLI command to remove a repo from the managed set.

## Requirements

- Add `Rm { name: String }` variant to `Cmd` enum
- In the match arm: load repos, remove by name, error if not found, save
- Print confirmation: `removed agent "<name>"`

## Files to edit
- `client/src/main.rs`

## Dependencies
- Task 01
