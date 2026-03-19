# Group commands into local vs server sections in help output

Use clap's `#[command(subcommand_help_heading = "...")]` or manual grouping to display commands in two sections:

## Local (no server needed)
- `add` — Add a repo to the managed set
- `rm` — Remove a repo from the managed set
- `ls` — List all registered repos
- `pause` — Pause an agent
- `resume` — Resume a paused agent
- `up` — Start the daemon
- `down` — Stop the daemon
- `import` — Import legacy configs

## Server
- `ps` — List online agents
- `send` — Send a message to an agent
- `broadcast` — Broadcast to all agents
- `task` — Manage tasks

This makes the CLI self-documenting about what needs a server.
