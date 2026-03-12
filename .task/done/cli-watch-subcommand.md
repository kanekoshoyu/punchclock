# `watch` subcommand in CLI

Poll inbox on a configurable interval and print new messages as they arrive.

```
punchclock watch <agent_id> [--interval 5s]
```

Loop: call `/message/recv`, print any messages, sleep, repeat.
