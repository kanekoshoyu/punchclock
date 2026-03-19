# Refactor daemon to manage multiple repos from central config

Replace the single-repo daemon loop with a multi-repo orchestrator.

## Requirements

- Add a new function `pub async fn up(foreground: bool)` in `client/src/agent.rs`
- It should:
  1. Load `ReposConfig` from `~/.config/punchclock/repos.toml`
  2. Filter to `enabled == true` entries
  3. For each repo, spawn a tokio task set that runs the existing 3 loops (heartbeat, task, message) using that repo's path and config
  4. The agent name from repos.toml key becomes the agent's `name` field on register
  5. Write a single PID file to `~/.config/punchclock/daemon.pid`
  6. If `foreground == false`, daemonize (same approach as current `start()`)
  7. On SIGTERM/SIGINT, clean up all loops and remove PID
- Reuse as much of the existing `run_daemon_loop` logic as possible — extract the per-repo loop into a function like `run_agent_loop(name, repo_path, server, claude_flags)` that the new `up()` calls for each repo
- The existing `run()` function can remain for backwards compat but should print a deprecation notice pointing to `punchclock up`

## Files to edit
- `client/src/agent.rs`

## Dependencies
- Task 01
