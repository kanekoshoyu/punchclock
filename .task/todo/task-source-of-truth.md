# Task source of truth: Claude Code files, not server memory

## Problem

The punchclock server currently maintains a full in-memory task queue
(`task/push`, `task/list`, `task/finish`). This is the wrong source of truth —
it means tasks are lost on server restart, and there's a parallel task
representation that diverges from what Claude Code actually tracks.

## Decision

Task state should live entirely in Claude Code's file-based task system
(`.task/todo/` and `.task/done/` in each repo). The server should be
**stateless with respect to tasks** — the only exception is the currently
in-flight task, which the server tracks temporarily so the daemon can claim
one task at a time without double-processing.

## What to change

- Keep `task/list` but serve it from the filesystem: read `.task/todo/` →
  status `queued`, `.task/done/` → status `done`. Requires the agent's
  `repo_path` to be stored on registration.
- Keep `task/claim` as a lightweight in-progress lock only (no stored body/title).
- Remove `task/push` and `task/finish` from the server; task files are
  written directly to `.task/todo/` by whoever queues the work.
- The daemon reads its queue from `.task/todo/` on disk rather than polling
  the server.
