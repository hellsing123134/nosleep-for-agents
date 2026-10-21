//! The decision loop: merge hook events with the process scan, decide whether to stay awake
//! and hold the lid, and announce finished turns and agents that are waiting for you.

use crate::{agents::Scanner, config::Config, notify, power, tray};
use std::collections::HashMap;
use std::sync::{mpsc::Receiver, Mutex};
use std::time::{Duration, Instant};

const TICK: Duration = Duration::from_secs(2);
/// A session that sent nothing for this long is forgotten (agent crashed, terminal closed...).
const SESSION_TTL: Duration = Duration::from_secs(30 * 60);

#[derive(Clone, Copy, PartialEq)]
pub enum Mode {
    Auto,
    Always,
    Off,
}

#[derive(Clone, Copy, PartialEq)]
pub enum Phase {
    Idle,
    Working,
    Waiting,
}

/// What the tray shows and changes.
pub struct Shared {
    pub mode: Mode,
    pub lid: bool,
    pub phase: Phase,
    pub awake: bool,
    pub summary: String,
}

pub static SHARED: Mutex<Shared> =
    Mutex::new(Shared { mode: Mode::Auto, lid: true, phase: Phase::Idle, awake: false, summary: String::new() });

pub struct Event {
    pub kind: String,
    pub agent: String,
    pub session: String,
    pub project: String,
    pub message: String,
}

struct Session {
    agent: String,
    project: String,
    phase: Phase,
    started: Instant,
    last: Instant,
}

impl Session {
    fn label(&self) -> String {
        if self.project.is_empty() {
            self.agent.clone()
        } else {
            format!("{} ({})", self.agent, self.project)
        }
    }
}

fn dur(d: Duration) -> String {
    let s = d.as_secs();
    match s {
        0..=59 => format!("{s}s"),
        60..=3599 => format!("{}m {}s", s / 60, s % 60),
        _ => format!("{}h {}m", s / 3600, s / 60 % 60),
    }
}

struct Engine {
    cfg: Config,
    sessions: HashMap<String, Session>,
    last_active: Option<Instant>,
    awake: bool,
    lid_held: bool,
    lid_failed: bool,
    battery_warned: bool,
}

impl Engine {
    fn on_event(&mut self, ev: Event) {
        let now = Instant::now();
        self.last_active = Some(now);
        let known = self.sessions.contains_key(&ev.session);
        let s = self.sessions.entry(ev.session.clone()).or_insert_with(|| Session {
            agent: ev.agent.clone(),
            project: ev.project.clone(),
            phase: Phase::Idle,
            started: now,
            last: now,
        });
        s.last = now;
        match ev.kind.as_str() {
            "prompt" => {
                s.phase = Phase::Working;
                s.started = now;
            }
            "tool" => {
                if s.phase == Phase::Idle {
                    s.started = now;
                }
                s.phase = Phase::Working;
            }
            "notification" => {
                // Claude also nags "waiting for your input" a while after a turn ends; the
                // finished-turn notice already covered that.
                if known && s.phase == Phase::Idle {
                    return;
                }
                s.phase = Phase::Waiting;
                let body = if ev.message.is_empty() { "Waiting for your input".into() } else { ev.message };
                let title = format!("🔔 {} needs you", s.label());
                notify::send(&self.cfg, &title, &body);
            }
            "stop" => {
                let ran = now - s.started;
                // Unknown sessions (Codex, or nosleep started mid-turn) have no start time.
                let announce = !known || (s.phase != Phase::Idle && ran.as_secs() >= self.cfg.notify_after_seconds);
                s.phase = Phase::Idle;
                if announce {
                    let title = if known {
                        format!("✅ {} finished in {}", s.label(), dur(ran))
                    } else {
                        format!("✅ {} finished", s.label())
                    };
                    let body = if ev.message.is_empty() { "Turn complete".into() } else { ev.message };
                    notify::send(&self.cfg, &title, &body);
                }
            }
            "end" => {
                self.sessions.remove(&ev.session);
            }
            _ => {}
        }
    }

    fn tick(&mut self, scanner: &mut Scanner) {
        let scan = scanner.scan();
        let now = Instant::now();
        self.sessions.retain(|_, s| now - s.last < SESSION_TTL);

        let working = scan.busy || self.sessions.values().any(|s| s.phase == Phase::Working);
        let waiting = self.sessions.values().any(|s| s.phase == Phase::Waiting);
        if working {
            self.last_active = Some(now);
        }
        let grace = Duration::from_secs(self.cfg.grace_minutes * 60);
        let left = self.last_active.and_then(|t| grace.checked_sub(now - t));

        let (mode, lid_pref) = {
            let s = SHARED.lock().unwrap();
            (s.mode, s.lid)
        };
        let mut want = match mode {
            Mode::Always => true,
            Mode::Off => false,
            Mode::Auto => working || left.is_some(),
        };

        let battery = power::on_battery();
        let low = battery.is_some_and(|p| p < self.cfg.min_battery);
        if low && want {
            want = false;
            if !self.battery_warned {
                self.battery_warned = true;
                let body = format!("Battery at {}%: letting Windows sleep normally.", battery.unwrap_or(0));
                notify::send(&self.cfg, "🪫 nosleep stepped back", &body);
            }
        } else if !low {
            self.battery_warned = false;
        }

        if want != self.awake {
            power::set_awake(want);
            self.awake = want;
        }
        let hold = want && lid_pref;
        if hold && !self.lid_held && !self.lid_failed {
            self.lid_held = power::hold_lid();
            if !self.lid_held {
                self.lid_failed = true;
                notify::send(&self.cfg, "nosleep", "Windows refused to change the lid-close action; closing the lid will still sleep.");
            }
        } else if !hold {
            if self.lid_held {
                power::recover();
                self.lid_held = false;
            }
            self.lid_failed = false;
        }

        // An agent waiting for you matters more than others still working.
        let phase = if waiting {
            Phase::Waiting
        } else if working {
            Phase::Working
        } else {
            Phase::Idle
        };
        let summary = self.summary(&scan.agents, phase, mode, left, low);
        let mut s = SHARED.lock().unwrap();
        let changed = s.phase != phase || s.awake != want || s.summary != summary;
        s.phase = phase;
        s.awake = want;
        s.summary = summary;
        drop(s);
        if changed {
            tray::refresh();
        }
    }

    fn summary(&self, procs: &[String], phase: Phase, mode: Mode, left: Option<Duration>, low: bool) -> String {
        let mut who: Vec<String> = self
            .sessions
            .values()
            .filter(|s| s.phase == phase)
            .map(Session::label)
            .collect();
        if who.is_empty() {
            who = procs.to_vec();
            who.dedup();
        }
        let who = who.join(", ");
        let state = match (mode, phase) {
            (Mode::Off, _) => "Off: sleep allowed".to_string(),
            (Mode::Always, _) => "Always awake".to_string(),
            (_, Phase::Working) => format!("Working: {who}"),
            (_, Phase::Waiting) => format!("Waiting for you: {who}"),
            (_, Phase::Idle) => match left {
                Some(l) => format!("Idle, staying awake {} more", dur(l)),
                None => "No busy agents: sleep allowed".to_string(),
            },
        };
        let mut out = state;
        if low {
            out += " · low battery";
        } else if self.lid_held {
            out += " · lid close ignored";
        }
        out
    }
}

pub fn run(cfg: Config, rx: Receiver<Event>) {
    let mut scanner = Scanner::new(cfg.agents.clone(), cfg.ignore_paths.clone());
    let mut e = Engine {
        cfg,
        sessions: HashMap::new(),
        last_active: None,
        awake: false,
        lid_held: false,
        lid_failed: false,
        battery_warned: false,
    };
    loop {
        let deadline = Instant::now() + TICK;
        while let Ok(ev) = rx.recv_timeout(deadline.saturating_duration_since(Instant::now())) {
            e.on_event(ev);
        }
        e.tick(&mut scanner);
    }
}
