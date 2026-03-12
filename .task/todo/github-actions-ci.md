# GitHub Actions CI

Run build and tests on every push and pull request.

- Trigger: push + pull_request on `main`
- Steps: checkout, cache `~/.cargo`, `cargo build`, `cargo test`
- Matrix: stable + beta Rust toolchains
- Optional: `cargo clippy --deny warnings`, `cargo fmt --check`
