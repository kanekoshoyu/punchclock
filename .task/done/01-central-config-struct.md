# Add central config structs (ReposConfig, GlobalConfig)

Create a new file `client/src/config.rs` with the data structures for the central configuration.

## Requirements

- `GlobalConfig` struct with fields: `server: String` (default `http://localhost:8421`)
- `RepoEntry` struct with fields: `path: PathBuf`, `description: String`, `enabled: bool` (default true), `claude_flags: String` (default empty)
- `ReposConfig` is a `BTreeMap<String, RepoEntry>` (key = agent name)
- Both structs derive `Serialize, Deserialize, Debug, Clone`
- Add functions:
  - `config_dir() -> PathBuf` → `~/.config/punchclock/`
  - `load_global() -> GlobalConfig` (create default if missing)
  - `load_repos() -> ReposConfig` (return empty map if missing)
  - `save_repos(repos: &ReposConfig)` (write to `~/.config/punchclock/repos.toml`)
- Add `mod config;` to `client/src/main.rs`
- Use `serde`, `toml`, `anyhow` (already in deps)

## Files to edit
- Create `client/src/config.rs`
- Edit `client/src/main.rs` (add `mod config;`)
