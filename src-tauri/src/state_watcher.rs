//! Watches state.json (written by sidecrab-hook) and forwards each change to the
//! webview as a `claude-state` event. The pet is a pure consumer of that file.

use notify::{RecursiveMode, Watcher};
use serde_json::{json, Value};
use std::path::Path;
use std::sync::mpsc;
use std::time::Duration;
use tauri::{AppHandle, Emitter};

const DEBOUNCE: Duration = Duration::from_millis(120);

fn current_state(path: &Path) -> Value {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_else(|| json!({ "state": "idle" }))
}

pub fn spawn(app: AppHandle) {
    std::thread::spawn(move || {
        let dir = crate::paths::home();
        let _ = std::fs::create_dir_all(&dir);
        let state_path = crate::paths::state_path();

        let limits_path = dir.join("limits.json");
        let read_limits = || {
            std::fs::read_to_string(&limits_path)
                .ok()
                .and_then(|s| serde_json::from_str::<Value>(&s).ok())
        };

        // Initial emit so the crab reflects reality on launch.
        let _ = app.emit("claude-state", current_state(&state_path));
        if let Some(l) = read_limits() {
            crate::usage_cache::update_five_hour(&l);
            let _ = app.emit("claude-limits", l);
        }

        let (tx, rx) = mpsc::channel();
        let mut watcher = match notify::recommended_watcher(tx) {
            Ok(w) => w,
            Err(_) => return,
        };
        // Watch the dir, not the file: atomic tmp+rename replaces the inode.
        if watcher.watch(&dir, RecursiveMode::NonRecursive).is_err() {
            return;
        }

        let mut last_logged: Option<Value> = None;
        let touches = |ev: &notify::Result<notify::Event>, name: &str| {
            matches!(ev, Ok(e) if e.paths.iter().any(|p| p.file_name().is_some_and(|n| n == name)))
        };
        while let Ok(ev) = rx.recv() {
            let mut state = touches(&ev, "state.json");
            let mut limits = touches(&ev, "limits.json");
            if !state && !limits {
                continue;
            }
            // Debounce: coalesce the write burst, then emit once per file.
            while let Ok(more) = rx.recv_timeout(DEBOUNCE) {
                state |= touches(&more, "state.json");
                limits |= touches(&more, "limits.json");
            }
            if state {
                let _ = app.emit("claude-state", current_state(&state_path));
            }
            if limits {
                if let Some(l) = read_limits() {
                    crate::usage_cache::update_five_hour(&l);
                    // Log only real changes: the terminal statusLine rewrites
                    // limits.json on every update with the same numbers.
                    let shown = (&l["fiveHour"], &l["source"], l["stale"] == true);
                    if last_logged.as_ref() != Some(&json!([shown.0, shown.1, shown.2])) {
                        last_logged = Some(json!([shown.0, shown.1, shown.2]));
                        crate::debug_log::event(
                            &dir,
                            &format!(
                                "limits changed: source={} 5h used={}% resets_at={} stale={}; cache updated, UI notified",
                                l["source"], l["fiveHour"]["usedPercentage"], l["fiveHour"]["resetsAt"], shown.2
                            ),
                        );
                    }
                    let _ = app.emit("claude-limits", l);
                }
            }
        }
    });
}
