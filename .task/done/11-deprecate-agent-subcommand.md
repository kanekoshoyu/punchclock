# Deprecate `punchclock agent` subcommand

Mark the old per-repo agent commands as deprecated in favor of the new centralized commands.

## Requirements

- Keep `AgentCmd` enum and all its match arms working (don't break anything)
- Add `#[command(hide = true)]` to the `Agent` variant in `Cmd` so it doesn't show in `--help`
- In each `AgentCmd` match arm, print a deprecation warning to stderr before executing:
  - `agent init` → `use "punchclock add <path>" instead`
  - `agent run` → `use "punchclock up" instead`
  - `agent start` → `use "punchclock up --daemon" instead`
  - `agent stop` → `use "punchclock down" instead`
  - `agent status` → `use "punchclock ls" instead`
  - `agent install` → `use "punchclock up --daemon" instead`
  - `agent logs` → `use "punchclock logs" instead`

## Files to edit
- `client/src/main.rs`

## Dependencies
- Tasks 02, 07 (new commands must exist before deprecating old ones)
