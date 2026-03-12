# Extract shared types to `punchclock-common` crate

`RegisterResponse`, `AgentSummary`, `MessageItem`, `InboxResponse`, etc. are
duplicated between server and client. Drift between them will cause bugs.

- Add `common/` crate to the workspace
- Move shared structs there, derive `Serialize + Deserialize` on all of them
- Server and client both depend on `punchclock-common`

## Result

Clean build. Only the pre-existing `tui.rs` dead-code warning remains (unrelated to this change).

Done. Here's what was done:

**New `common/` crate (`punchclock-common`):**
- `RegisterResponse`, `AgentSummary`, `TeamResponse`, `MessageItem`, `InboxResponse`, `BroadcastResponse`, `TaskItem`, `TaskListResponse`, `TaskSyncItem`, `TaskSyncRequest`, `ErrorBody`
- All derive `Serialize + Deserialize`
- `#[cfg_attr(feature = "poem-openapi", derive(poem_openapi::Object))]` on each — server enables this feature, client doesn't need it

**Server changes:**
- Removed all local type definitions
- Imports from `punchclock_common`
- `TaskItem::from_record` (can't add methods to foreign types) → standalone `task_item_from_record` helper

**Client changes (`agent.rs` and `main.rs`):**
- Removed local duplicates of all response types
- `TaskSyncPayload` → `TaskSyncRequest` from common
- `AgentSummary` now has the full 4-field definition (was 2 in client); serde ignores no fields, so all existing accesses still work
