# Architecture

```
 Claude Code ──hook (stdin JSON)──┐
 Codex ───────notify (argv JSON)──┤   nosleep hook <agent>      (short-lived, exits 0 always)
                                  ▼
                       127.0.0.1:47811  one JSON line   ── ipc.rs
                                  │
                                  ▼
 Toolhelp32 + GetProcessTimes ─► engine.rs (2 s tick) ─► power.rs   SetThreadExecutionState
   agents.rs: agent trees, CPU     sessions, policy,       lid action override + backup
                                   grace, battery          battery status
                                  │
                                  ├─► notify.rs  tray balloon (toast) + ntfy push
                                  └─► tray.rs    dot icon, menu, tooltip  (main thread)
```

## Modules

| file | job |
|---|---|
| `main.rs` | CLI dispatch; `run()` recovers a stale lid backup, binds IPC (single instance), starts the engine thread and the tray loop |
| `agents.rs` | snapshot processes, match agent executables (skipping `ignore_paths`, e.g. the Claude desktop app, which is also `claude.exe`), walk their descendants, sum CPU time deltas → `busy` if over 3% of one core |
| `hook.rs` | translate Claude Code / Codex payloads into `prompt` · `tool` · `notification` · `stop` · `end`; edit `~/.claude/settings.json` |
| `ipc.rs` | localhost TCP, one JSON line per message; `status` gets a reply. Binding doubles as the single-instance lock |
| `engine.rs` | per-session state machine, the keep-awake decision, notifications, summary text |
| `power.rs` | execution state, lid action (read / backup / write / restore), battery |
| `notify.rs` | toast via the tray icon, ntfy via JSON POST on a background thread |
| `tray.rs` | hidden window, generated dot icons, menu, `WM_ENDSESSION` restore |
| `config.rs` | `key = value` file with commented defaults; `set()` edits one key in place |

## Session state machine (per hook session)

```
            prompt / tool
   Idle ────────────────────► Working ◄──── tool ────┐
    ▲                          │   │                   │
    │ stop (announce if ran    │   └── notification ─► Waiting   (announce "needs you")
    │ ≥ notify_after_seconds)  │
    └──────────────────────────┘
   end → forgotten;  no event for 30 min → forgotten
```

A `notification` for an already idle session is Claude's "still waiting for input" nag after
a finished turn. It is dropped, because the finished notice already went out.

## Keep-awake policy

```
working  = any process tree busy  OR  any session Working
awake    = Always | (Auto AND (working OR last activity < grace_minutes ago))
awake    = false if on battery below min_battery
hold lid = awake AND lid_override
```

`SetThreadExecutionState(ES_CONTINUOUS | ES_SYSTEM_REQUIRED)` blocks idle sleep but lets the
display turn off. It does **not** cover the lid, which always runs the power plan's lid
action. So we write `GUID_LIDCLOSE_ACTION = 0` (do nothing) for AC and DC on the active scheme
via `PowerWrite{AC,DC}ValueIndex` + `PowerSetActiveScheme`. Standard users may do this, no UAC.

Crash safety: before the first write the original `(scheme, ac, dc)` goes to
`%APPDATA%\nosleep\lid.backup`. If that write fails we don't touch the setting. The backup is
restored and deleted when agents go idle, on Exit, on `WM_ENDSESSION`, at the next start, or
with `nosleep restore`.

## Roadmap

- **v0.2** · Gemini CLI / OpenCode hooks · winget + scoop packages · autostart toggle ·
  "agent asked for permission" → approve from the phone (ntfy action buttons → Claude Code
  remote control)
- **v0.3** · pause heavy background stuff while on battery (OneDrive, Windows Update
  downloads, indexer) · dim or turn off the display · stats ("agents worked 3h12m tonight")
- **later** · cross-platform core (macOS `caffeinate -s` + `pmset disablesleep`, Linux
  `systemd-inhibit --what=handle-lid-switch`)
