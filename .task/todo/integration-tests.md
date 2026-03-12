# Integration tests

Spin up a real server in a test harness and exercise the full flow.

- Use `tokio::test` + bind server on a random port
- Test: register → heartbeat → team → send → recv round-trip
- Test: reaper removes agent after heartbeat timeout
- Test: inbox cap (100 messages) drops oldest on overflow
- Live in `server/tests/` or a top-level `tests/` crate
