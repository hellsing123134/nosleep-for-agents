//! The `nosleep hook <agent>` entry point that agents call, and installing it into Claude Code.
//!
//! Claude Code runs it for each hooked event with a JSON payload on stdin. Codex runs its
//! `notify` program with the JSON as the last argument. Either way we translate the payload
//! into one small event for the tray instance and exit 0 without printing anything, so a
//! missing or broken nosleep never gets in the agent's way.

use crate::ipc;
use serde_json::{json, Value};
use std::io::Read;
use std::path::{Path, PathBuf};

/// Claude Code events we listen to.
const CLAUDE_EVENTS: [&str; 5] = ["UserPromptSubmit", "PostToolUse", "Notification", "Stop", "SessionEnd"];

fn project(cwd: &str) -> String {
    Path::new(cwd).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()
}

pub fn run(args: &[String]) {
    let agent = args.first().map(String::as_str).unwrap_or("claude");
    let payload: Value = match args.get(1) {
        Some(arg) => serde_json::from_str(arg).unwrap_or_default(),
        None => {
            let mut s = String::new();
            let _ = std::io::stdin().take(1 << 20).read_to_string(&mut s);
            serde_json::from_str(&s).unwrap_or_default()
        }
    };
    if let Some(log) = std::env::var_os("NOSLEEP_DEBUG") {
        use std::io::Write;
        if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(log) {
            let _ = writeln!(f, "{agent}: {payload}");
        }
    }
    let get = |k: &str| payload[k].as_str().unwrap_or_default();

    let (kind, session, message) = match agent {
        "codex" => {
            let kind = if get("type") == "agent-turn-complete" { "stop" } else { "" };
            (kind, format!("codex:{}", get("cwd")), get("last-assistant-message"))
        }
        _ => {
            let kind = match get("hook_event_name") {
                "UserPromptSubmit" => "prompt",
                "PreToolUse" | "PostToolUse" => "tool",
                "Notification" => "notification",
                "Stop" => "stop",
                "SessionEnd" => "end",
                _ => "",
            };
            (kind, get("session_id").to_string(), get("message"))
        }
    };
    if kind.is_empty() {
        return;
    }
    let message: String = message.chars().take(300).collect();
    ipc::request(&json!({
        "kind": kind,
        "agent": agent,
        "session": session,
        "project": project(get("cwd")),
        "message": message,
    }));
}

fn settings_path() -> PathBuf {
    PathBuf::from(std::env::var_os("USERPROFILE").unwrap_or_default()).join(".claude").join("settings.json")
}

fn is_ours(entry: &Value) -> bool {
    entry["hooks"].as_array().is_some_and(|hs| {
        hs.iter().any(|h| h["command"].as_str().is_some_and(|c| c.contains("nosleep")))
    })
}

fn edit_settings(add: bool) -> std::io::Result<PathBuf> {
    let path = settings_path();
    let text = std::fs::read_to_string(&path).unwrap_or_else(|_| "{}".into());
    let mut root: Value = serde_json::from_str(&text)
        .map_err(|e| std::io::Error::other(format!("{} is not valid JSON: {e}", path.display())))?;
    if path.exists() {
        std::fs::copy(&path, path.with_extension("json.nosleep.bak"))?;
    }
    // Claude Code runs hooks through a shell (Git Bash on Windows): forward slashes are safest.
    let exe = std::env::current_exe()?.to_string_lossy().replace('\\', "/");
    let command = format!("\"{exe}\" hook claude");

    if !root["hooks"].is_object() {
        root["hooks"] = json!({});
    }
    for ev in CLAUDE_EVENTS {
        let list = &mut root["hooks"][ev];
        if !list.is_array() {
            *list = json!([]);
        }
        let arr = list.as_array_mut().unwrap();
        arr.retain(|e| !is_ours(e));
        if add {
            arr.push(json!({ "hooks": [{ "type": "command", "command": command, "timeout": 5 }] }));
        }
        if arr.is_empty() {
            root["hooks"].as_object_mut().unwrap().remove(ev);
        }
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&path, serde_json::to_string_pretty(&root)? + "\n")?;
    Ok(path)
}

pub fn install() {
    match edit_settings(true) {
        Ok(p) => {
            let exe = std::env::current_exe().map(|e| e.to_string_lossy().replace('\\', "/")).unwrap_or_default();
            println!("Claude Code hooks added to {} (backup: settings.json.nosleep.bak)", p.display());
            println!("\nFor Codex, add this line to ~/.codex/config.toml:");
            println!("  notify = [\"{exe}\", \"hook\", \"codex\"]");
        }
        Err(e) => println!("could not update Claude Code settings: {e}"),
    }
}

pub fn uninstall() {
    match edit_settings(false) {
        Ok(p) => println!("nosleep hooks removed from {}", p.display()),
        Err(e) => println!("could not update Claude Code settings: {e}"),
    }
}
