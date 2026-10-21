# No Sleep for Agents

**caffeinate for AI agents, on Windows.** Close your laptop, walk away, and let Claude Code /
Codex / OpenCode keep working. nosleep keeps Windows awake only while your agents are actually
busy, ignores the lid while they work, and pings your phone when an agent finishes or needs you.

- 🟠 **Agent-aware.** Watches agent processes *and everything they spawn* (builds, tests, servers)
  plus Claude Code / Codex hooks. Nothing is running → Windows sleeps as usual.
- 💻 **Lid closed? Still working.** Temporarily sets the power plan's lid-close action to
  "do nothing" and restores it afterwards, even after a crash.
- 🔔 **Finished / needs you.** A Windows notification and an optional phone push via
  [ntfy](https://ntfy.sh) when a turn completes or an agent waits for permission.
- 🪫 **Battery guard.** On battery below 20% it steps back and lets the laptop sleep.
- 🪶 **Tiny.** One ~1.7 MB Rust binary, no runtime, no admin rights, no telemetry.

## Install

```bash
cargo install --git https://github.com/hellsing123134/nosleep-for-agents
```

Then run `nosleep` (it lives in the tray) and wire up the hooks once:

```bash
nosleep install-hooks
```

That adds nosleep to `~/.claude/settings.json` (a backup is kept). For Codex it prints the
`notify = [...]` line to put in `~/.codex/config.toml`. Without hooks nosleep still works from
process activity alone, it just can't tell you *why* an agent stopped.

### Phone notifications

Install the ntfy app, subscribe to a long random topic, and put it in
`%APPDATA%\nosleep\config.ini`:

```ini
ntfy_topic = nosleep-7f3a9c1e2b
```

## Tray

The dot tells you the state: 🟠 working · 🔴 waiting for you · 🟡 idle but staying awake (grace
period) · ⚪ sleep allowed. Right-click for **Auto** / **Always awake** / **Off** and the
**Keep working with the lid closed** toggle.

## CLI

| command | |
|---|---|
| `nosleep` | run in the tray |
| `nosleep install-hooks` / `uninstall-hooks` | add or remove the Claude Code hooks |
| `nosleep status` | one line: what the running instance is doing |
| `nosleep restore` | put the original lid action back by hand |

## Config

`%APPDATA%\nosleep\config.ini`, created on first run with comments. Highlights:
`agents` (executables to watch), `grace_minutes` (stay awake after the last activity, default 5),
`min_battery`, `notify_after_seconds` (don't announce turns shorter than this), `lid_override`.

## ⚠️ Lid closed, in a bag

A closed laptop working hard in a backpack gets hot. nosleep only holds the lid while agents
are busy plus the grace period, and gives up on low battery, but put it on a desk, not in a bag.

## How it works

See [docs/architecture.md](docs/architecture.md).

## Troubleshooting

Set `NOSLEEP_DEBUG=C:\path\to\log.txt` in the environment your agent runs in, and every hook
payload nosleep receives is appended there.

## License

MIT
