# Broadcast endpoint

`POST /message/broadcast` — send one message to all currently-online agents at once.

Useful for coordination signals like "shutdown", "new task available", etc.

Reuse existing inbox delivery logic; iterate over all live agent IDs.
