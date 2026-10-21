//! `%APPDATA%\nosleep\config.ini`: plain `key = value` lines, written with defaults on first run.

use std::path::PathBuf;

const DEFAULT: &str = "\
# nosleep config. Restart nosleep after editing.

# Agent executables to watch (their whole process tree counts as the agent working).
agents = claude.exe, codex.exe, opencode.exe, gemini.exe, goose.exe, aider.exe, cursor-agent.exe, crush.exe

# Skip matches whose full path contains one of these (the Claude desktop app is claude.exe too;
# Claude Code sessions it starts live elsewhere and still count).
ignore_paths = \\windowsapps\\claude_, \\anthropicclaude\\

# Switch the lid-close action to \"do nothing\" while agents work, restore it afterwards.
lid_override = true

# Stay awake this many minutes after the last sign of agent activity.
grace_minutes = 5

# On battery, give up (let Windows sleep) below this charge, in percent.
min_battery = 20

# Only announce a finished turn if it ran at least this long.
notify_after_seconds = 30

# Windows notification when an agent finishes or needs you.
toast = true

# Phone pushes via ntfy (install the ntfy app, subscribe to a hard-to-guess topic). Empty = off.
ntfy_server = https://ntfy.sh
ntfy_topic =
";

#[derive(Clone)]
pub struct Config {
    pub agents: Vec<String>,
    pub ignore_paths: Vec<String>,
    pub lid_override: bool,
    pub grace_minutes: u64,
    pub min_battery: u8,
    pub notify_after_seconds: u64,
    pub toast: bool,
    pub ntfy_server: String,
    pub ntfy_topic: String,
}

pub fn dir() -> PathBuf {
    PathBuf::from(std::env::var_os("APPDATA").unwrap_or_default()).join("nosleep")
}

pub fn path() -> PathBuf {
    dir().join("config.ini")
}

fn pairs(text: &str) -> impl Iterator<Item = (&str, &str)> {
    text.lines()
        .map(str::trim)
        .filter(|l| !l.starts_with('#'))
        .filter_map(|l| l.split_once('='))
        .map(|(k, v)| (k.trim(), v.trim()))
}

pub fn load() -> Config {
    let p = path();
    if !p.exists() {
        let _ = std::fs::create_dir_all(dir());
        let _ = std::fs::write(&p, DEFAULT);
    }
    let text = std::fs::read_to_string(&p).unwrap_or_default();
    let mut c = parse(DEFAULT);
    parse_into(&text, &mut c);
    c
}

fn parse(text: &str) -> Config {
    let mut c = Config {
        agents: Vec::new(),
        ignore_paths: Vec::new(),
        lid_override: true,
        grace_minutes: 5,
        min_battery: 20,
        notify_after_seconds: 30,
        toast: true,
        ntfy_server: String::new(),
        ntfy_topic: String::new(),
    };
    parse_into(text, &mut c);
    c
}

fn parse_into(text: &str, c: &mut Config) {
    let yes = |v: &str| matches!(v, "true" | "yes" | "1" | "on");
    for (k, v) in pairs(text) {
        match k {
            "agents" => {
                c.agents = v.split(',').map(|s| s.trim().to_lowercase()).filter(|s| !s.is_empty()).collect()
            }
            "ignore_paths" => {
                c.ignore_paths = v.split(',').map(|s| s.trim().to_lowercase()).filter(|s| !s.is_empty()).collect()
            }
            "lid_override" => c.lid_override = yes(v),
            "grace_minutes" => c.grace_minutes = v.parse().unwrap_or(c.grace_minutes),
            "min_battery" => c.min_battery = v.parse().unwrap_or(c.min_battery),
            "notify_after_seconds" => c.notify_after_seconds = v.parse().unwrap_or(c.notify_after_seconds),
            "toast" => c.toast = yes(v),
            "ntfy_server" => c.ntfy_server = v.trim_end_matches('/').to_string(),
            "ntfy_topic" => c.ntfy_topic = v.to_string(),
            _ => {}
        }
    }
}

/// Rewrites one key in the file, keeping comments and everything else as they are.
pub fn set(key: &str, value: &str) {
    let p = path();
    let text = std::fs::read_to_string(&p).unwrap_or_default();
    let mut found = false;
    let mut out: Vec<String> = text
        .lines()
        .map(|l| match l.split_once('=') {
            Some((k, _)) if k.trim() == key && !l.trim_start().starts_with('#') => {
                found = true;
                format!("{key} = {value}")
            }
            _ => l.to_string(),
        })
        .collect();
    if !found {
        out.push(format!("{key} = {value}"));
    }
    let _ = std::fs::write(&p, out.join("\n") + "\n");
}
