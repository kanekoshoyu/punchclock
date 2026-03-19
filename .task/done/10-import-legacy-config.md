# Implement `punchclock import` for legacy .punchclock files

Auto-import repos that have the old per-repo `.punchclock` config file.

## Requirements

- Add `Import { paths: Vec<PathBuf> }` variant to `Cmd`
- If `paths` is empty, scan common locations (home dir, ~/Documents, ~/Projects — one level deep) for dirs containing `.punchclock`
- For each found `.punchclock` file: parse the old `AgentConfig`, add to repos.toml using the `name` field as key
- Skip if name already exists in repos.toml
- Print summary: `imported N agent(s), skipped M already registered`

## Files to edit
- `client/src/main.rs`

## Dependencies
- Task 01
