# Update TUI to show all managed agents

Adapt the existing ratatui TUI to display multiple agents from repos.toml.

## Requirements

- Load repos.toml on TUI start to know which agents are "ours"
- Show a left sidebar or tab bar listing all registered agent names
- Highlight which agents are online (cross-reference with `/team` response)
- Allow selecting an agent to see its tasks, logs, and inbox in the main pane
- The TUI already exists in `client/src/tui.rs` — extend it, don't rewrite

## Files to edit
- `client/src/tui.rs`
- `client/src/main.rs` (if TUI launch needs new args)

## Dependencies
- Task 01, Task 06
