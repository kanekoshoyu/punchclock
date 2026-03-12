# Register agent as an OS-level daemon (launchd / systemd)

## Problem

`punchclock agent start` spawns the daemon as a detached child process. This
works but has no automatic restart on failure or on system boot. For a machine
that runs agents long-term, the daemon should be registered with the OS init
system so it:

- starts on login / boot automatically
- restarts if it crashes
- logs to the system journal

## What to add

`punchclock agent install` — generate and register a platform service unit.

### macOS (launchd)

Write `~/Library/LaunchAgents/com.punchclock.<repo-name>.plist`:

```xml
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" ...>
<plist version="1.0"><dict>
  <key>Label</key>             <string>com.punchclock.myrepo</string>
  <key>ProgramArguments</key>  <array><string>/path/to/punchclock</string>
                                       <string>agent</string><string>run</string></array>
  <key>WorkingDirectory</key>  <string>/path/to/repo</string>
  <key>RunAtLoad</key>         <true/>
  <key>KeepAlive</key>         <true/>
  <key>StandardOutPath</key>   <string>/path/to/repo/.punchclock/daemon.log</string>
  <key>StandardErrorPath</key> <string>/path/to/repo/.punchclock/daemon.log</string>
</dict></plist>
```

Then run `launchctl load ~/Library/LaunchAgents/com.punchclock.<repo>.plist`.

### Linux (systemd user)

Write `~/.config/systemd/user/punchclock-<repo>.service`:

```ini
[Unit]
Description=punchclock agent for <repo>
After=network.target

[Service]
ExecStart=/path/to/punchclock agent run
WorkingDirectory=/path/to/repo
Restart=on-failure
StandardOutput=append:/path/to/repo/.punchclock/daemon.log
StandardError=append:/path/to/repo/.punchclock/daemon.log

[Install]
WantedBy=default.target
```

Then run `systemctl --user enable --now punchclock-<repo>`.

## Companion commands

- `punchclock agent install` — write and load/enable the unit
- `punchclock agent uninstall` — unload/disable and remove the unit
- `punchclock agent logs` — tail `.punchclock/daemon.log`

## Result

All three commands are visible and the build is clean. Here's a summary of what was added:

**`punchclock agent install`**
- macOS: generates `~/Library/LaunchAgents/com.punchclock.<repo>.plist` and runs `launchctl load -w`
- Linux: generates `~/.config/systemd/user/punchclock-<repo>.service` and runs `systemctl --user enable --now`
- Logs go to `~/.punchclock/<repo-hash>/daemon.log` (persistent across reboots; avoids the `.punchclock` config file path conflict)

**`punchclock agent uninstall`**
- macOS: `launchctl unload -w` then removes the plist
- Linux: `systemctl --user disable --now` then removes the unit file

**`punchclock agent logs`**
- Tails the install log (`~/.punchclock/<hash>/daemon.log`), falling back to the `start` log (`/tmp/punchclock-<hash>/daemon.log`) if the install log doesn't exist yet
