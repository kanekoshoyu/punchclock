# Task directory — rules for Claude agents

This directory is managed by the punchclock daemon. Read this file before touching anything else in `.task/`.

---

## Directory layout

```
.task/
  todo/       tasks waiting to be picked up by the daemon
  done/       completed tasks (daemon moves here, result appended)
  blocked/    tasks waiting for human input (daemon moves here, question appended)
  AGENTS.md   this file — do not modify
```

---

## Task file format

Every task is a markdown file. The daemon writes it; you read it and act on it.

```markdown
# Task title

Full task description. Everything after the title is the body — treat it as
your instructions. Do not modify the title or the original body.

## Blocked
(added by you if you need clarification — see below)

## Result
(added by the daemon after you finish — do not write this yourself)
```

---

## How to complete a task

Just do the work described in the body. Output a concise summary of what you
did — what changed, what files were touched, what the outcome was.

The daemon will append your output as `## Result` and `git mv` the file to
`.task/done/`. You do not need to move any files yourself.

---

## How to signal you are blocked

If you cannot complete the task without human input — ambiguous requirements,
a decision that requires judgement, a missing credential, a conflict with
existing code that changes the scope — do the following:

1. **First line of your output must be:** `BLOCKED: <one-line summary>`
2. The rest of your output should explain the specific question or dispute in
   full. Be concrete: quote the relevant code or requirement, state what you
   tried, and state exactly what you need to proceed.

The daemon detects `BLOCKED:` as the first line, appends your output under
`## Blocked` in the task file, and moves it to `.task/blocked/`.
The human will read it, add clarification, and re-queue the task.

**Example output when blocked:**

```
BLOCKED: Unclear whether to overwrite or merge the existing config

The task says "update the config" but src/config.rs already has a hand-written
`max_retries` field that isn't in the schema. If I regenerate from the schema
it will be lost. Should I:
  a) keep the hand-written field and merge manually, or
  b) overwrite and let the caller add it back?
```

---

## Rules

- **Never delete task files.** They are the audit trail. Only edit, never remove.
- **Never modify `.task/done/` files.** They are sealed records.
- **Never write `## Result` yourself.** The daemon appends it after you exit.
- **One question per block.** If you have multiple blockers, ask the most
  critical one. Unblock incrementally.
- **Use `git mv`** (not `mv`) if you ever need to move a task file manually.
- **Do not invent task IDs.** Task filenames are UUIDs assigned by the server.
  Do not create new task files; only respond to what the daemon gives you.
