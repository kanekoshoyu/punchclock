# Extract shared types to `punchclock-common` crate

`RegisterResponse`, `AgentSummary`, `MessageItem`, `InboxResponse`, etc. are
duplicated between server and client. Drift between them will cause bugs.

- Add `common/` crate to the workspace
- Move shared structs there, derive `Serialize + Deserialize` on all of them
- Server and client both depend on `punchclock-common`
