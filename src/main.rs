//! nosleep: keeps Windows awake while AI coding agents work, even with the lid closed,
//! and pings you (toast + phone) when an agent finishes or needs you.

#![windows_subsystem = "windows"]

mod agents;
mod config;
mod engine;
mod hook;
mod ipc;
mod notify;
mod power;
mod tray;

const USAGE: &str = "nosleep — keep Windows awake while AI agents work

usage:
  nosleep                   run in the tray (the normal way)
  nosleep install-hooks     wire Claude Code hooks into ~/.claude/settings.json
  nosleep uninstall-hooks   remove them again
  nosleep status            print what the running instance is doing
  nosleep restore           put the original lid-close action back (after a crash)
  nosleep hook <agent>      called by agent hooks, not by you";

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        None | Some("run") => run(),
        Some("hook") => hook::run(&args[1..]),
        Some(cmd) => {
            console();
            match cmd {
                "install-hooks" => hook::install(),
                "uninstall-hooks" => hook::uninstall(),
                "status" => match ipc::request(&serde_json::json!({"kind": "status"})) {
                    Some(s) => println!("{s}"),
                    None => println!("nosleep is not running"),
                },
                "restore" => {
                    if power::recover() {
                        println!("lid-close action restored");
                    } else {
                        println!("nothing to restore");
                    }
                }
                _ => println!("{USAGE}"),
            }
        }
    }
}

fn run() {
    power::recover(); // a previous run may have died while holding the lid
    let cfg = config::load();
    let (tx, rx) = std::sync::mpsc::channel();
    if !ipc::serve(tx) {
        tray::message("nosleep is already running (look in the tray).");
        return;
    }
    engine::SHARED.lock().unwrap().lid = cfg.lid_override;
    tray::create();
    std::thread::spawn(move || engine::run(cfg, rx));
    tray::message_loop();
}

/// GUI-subsystem binaries have no console; borrow the parent's so CLI commands can print.
fn console() {
    unsafe {
        windows_sys::Win32::System::Console::AttachConsole(windows_sys::Win32::System::Console::ATTACH_PARENT_PROCESS);
    }
}
