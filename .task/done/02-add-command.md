# Implement `punchclock add` command

Add a CLI command to register a repo directory as a managed agent.

## Requirements

- Add `Add` variant to the `Cmd` enum in `client/src/main.rs`:
  ```
  Add {
      path: PathBuf,        // repo directory (required)
      #[arg(long)]
      name: Option<String>, // defaults to directory basename
      #[arg(long)]
      description: Option<String>, // defaults to empty
      #[arg(long)]
      claude_flags: Option<String>,
  }
  ```
- In the match arm: resolve `path` to absolute, verify it exists and contains a `.git/` dir, derive name from basename if not provided, insert into `ReposConfig`, save
- Print confirmation: `added agent "<name>" → <path>`
- Error if name already exists (suggest `--name` to pick a different one)

## Files to edit
- `client/src/main.rs` (add variant + match arm)

## Dependencies
- Task 01 (central config structs) must be done first
