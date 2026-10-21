//! Tells you about agents: a Windows notification from the tray icon and, if configured, a
//! push to your phone through ntfy (https://ntfy.sh, or a self-hosted server).

use crate::{config::Config, tray};
use serde_json::json;

pub fn send(cfg: &Config, title: &str, body: &str) {
    if cfg.toast {
        tray::balloon(title, body);
    }
    if !cfg.ntfy_topic.is_empty() && !cfg.ntfy_server.is_empty() {
        // JSON publishing keeps non-ASCII titles intact (headers would mangle them).
        let payload = json!({ "topic": cfg.ntfy_topic, "title": title, "message": body, "tags": ["robot"] });
        let url = cfg.ntfy_server.clone();
        std::thread::spawn(move || {
            let _ = ureq::post(&url)
                .timeout(std::time::Duration::from_secs(10))
                .set("Content-Type", "application/json")
                .send_string(&payload.to_string());
        });
    }
}
