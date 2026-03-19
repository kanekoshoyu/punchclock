# Add `punchclock up` CLI command

Wire the new multi-agent daemon to the CLI.

## Requirements

- Add `Up` variant to `Cmd` enum:
  ```
  Up {
      #[arg(long)]
      daemon: bool,  // run in background
  }
  ```
- In the match arm: call `agent::up(!daemon)` (foreground = !daemon)
- Add `Down` variant (no args) that reads `~/.config/punchclock/daemon.pid`, sends SIGTERM, removes PID file
- Print appropriate status messages

## Files to edit
- `client/src/main.rs`

## Dependencies
- Task 06
