//! Hooks talk to the tray instance over one JSON line on localhost TCP. Binding the port also
//! doubles as the single-instance check.

use crate::engine::{self, Event};
use serde_json::Value;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{Shutdown, TcpListener, TcpStream};
use std::sync::mpsc::Sender;
use std::time::Duration;

const ADDR: &str = "127.0.0.1:47811";

/// Starts listening; false if another instance already owns the port.
pub fn serve(tx: Sender<Event>) -> bool {
    let Ok(listener) = TcpListener::bind(ADDR) else { return false };
    std::thread::spawn(move || {
        for s in listener.incoming().flatten() {
            handle(s, &tx);
        }
    });
    true
}

fn handle(mut s: TcpStream, tx: &Sender<Event>) {
    let _ = s.set_read_timeout(Some(Duration::from_secs(1)));
    let mut line = String::new();
    if BufReader::new(&s).take(256 * 1024).read_line(&mut line).is_err() {
        return;
    }
    let Ok(v) = serde_json::from_str::<Value>(&line) else { return };
    let str_of = |k: &str| v[k].as_str().unwrap_or_default().to_string();
    if str_of("kind") == "status" {
        let _ = s.write_all(engine::SHARED.lock().unwrap().summary.as_bytes());
        return;
    }
    let _ = tx.send(Event {
        kind: str_of("kind"),
        agent: str_of("agent"),
        session: str_of("session"),
        project: str_of("project"),
        message: str_of("message"),
    });
}

/// Sends one message and returns the reply. Short timeouts: a hook must never stall an agent.
pub fn request(v: &Value) -> Option<String> {
    let mut s = TcpStream::connect_timeout(&ADDR.parse().ok()?, Duration::from_millis(300)).ok()?;
    let _ = s.set_read_timeout(Some(Duration::from_millis(500)));
    s.write_all(format!("{v}\n").as_bytes()).ok()?;
    let _ = s.shutdown(Shutdown::Write);
    let mut out = String::new();
    let _ = s.read_to_string(&mut out);
    Some(out)
}
