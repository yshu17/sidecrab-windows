//! Live Claude Code sessions (sessions.d/, written by sidecrab-hook) and the
//! plugin lifecycle: when launched by the plugin, the pet quits once the last
//! Claude Code session is gone.
//!
//! A session ends either via its SessionEnd hook (the hook deletes its record)
//! or by its claude.exe process disappearing — terminal close, kill, crash —
//! which the poller detects by (pid, creation time) and prunes the record.

use crate::claude_proc::{self, ProcId};
use serde_json::{json, Value};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter};

const POLL: Duration = Duration::from_secs(2);
/// How long Claude must be completely gone before a plugin-launched pet quits:
/// rides out the desktop app restarting (auto-update) and /clear's end-then-start.
const CLAUDE_GONE_GRACE: Duration = Duration::from_secs(15);

/// How long Claude has been continuously absent.
#[derive(Default)]
pub struct Absence(Option<Instant>);

impl Absence {
    /// Record one observation; true once Claude has been absent for `grace`.
    pub fn gone_for(&mut self, present: bool, now: Instant, grace: Duration) -> bool {
        if present {
            self.0 = None;
            return false;
        }
        now.duration_since(*self.0.get_or_insert(now)) >= grace
    }
}

/// Quit when the last session ends (set by a `--plugin` launch).
static EXIT_WITH_CLAUDE: AtomicBool = AtomicBool::new(false);
/// A new `--plugin` launch arrived (new session starting): wait for its record
/// before judging "no sessions left", even if the old ones are all gone.
static REARM: AtomicBool = AtomicBool::new(false);

pub fn exit_with_claude() {
    EXIT_WITH_CLAUDE.store(true, Ordering::SeqCst);
    REARM.store(true, Ordering::SeqCst);
}

fn proc_id(rec: &Value) -> Option<ProcId> {
    Some((rec["claudePid"].as_u64()? as u32, rec["claudeStart"].as_u64()?))
}

/// Live session records, newest activity first. Deletes records whose Claude
/// process is gone, and unreadable/pre-JSON legacy files. Records without a
/// process id (non-Windows, or claude.exe not found) live until SessionEnd.
pub fn scan(dir: &Path, alive: impl Fn(ProcId) -> bool) -> Vec<Value> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut live = Vec::new();
    for e in entries.flatten() {
        let name = e.file_name().to_string_lossy().into_owned();
        if name.ends_with(".tmp") {
            continue; // hook's atomic write in flight
        }
        let path = e.path();
        let rec = std::fs::read_to_string(&path)
            .ok()
            .and_then(|s| serde_json::from_str::<Value>(&s).ok())
            .filter(Value::is_object);
        let Some(mut rec) = rec else {
            let _ = std::fs::remove_file(&path);
            continue;
        };
        if proc_id(&rec).is_some_and(|p| !alive(p)) {
            let _ = std::fs::remove_file(&path);
            continue;
        }
        rec["id"] = json!(name);
        live.push(rec);
    }
    live.sort_by_key(|r| std::cmp::Reverse(r["ts"].as_i64().unwrap_or(0)));
    live
}

fn sessions_dir() -> std::path::PathBuf {
    crate::paths::home().join("sessions.d")
}

fn read_json(path: &Path) -> Value {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or(Value::Null)
}

/// Everything the status bar needs, for the webview to pull once its listeners
/// are attached (events emitted before that are lost).
#[tauri::command]
pub fn status_snapshot() -> Value {
    let state = read_json(&crate::paths::state_path());
    json!({
        "state": if state.is_null() { json!({ "state": "idle" }) } else { state },
        "limits": read_json(&crate::paths::home().join("limits.json")),
        "cache": crate::usage_cache::load(),
        "sessions": scan(&sessions_dir(), claude_proc::is_alive),
    })
}

pub fn spawn(app: AppHandle) {
    std::thread::spawn(move || {
        let dir = sessions_dir();
        let mut last = Value::Null;
        let mut seen = false; // Claude observed since (re)arming
        let mut absence = Absence::default();
        loop {
            if REARM.swap(false, Ordering::SeqCst) {
                seen = false;
            }
            let live = scan(&dir, claude_proc::is_alive);
            // On Windows an empty sessions.d does not mean Claude is closed: the
            // desktop app restarts its per-session CLI processes (update, resume),
            // which prunes records until that session's next hook. Only the
            // absence of every claude.exe — desktop app or CLI — counts.
            let claude_up = if cfg!(windows) { claude_proc::any_running() } else { !live.is_empty() };
            seen |= claude_up;
            let gone = absence.gone_for(claude_up, Instant::now(), CLAUDE_GONE_GRACE);
            if EXIT_WITH_CLAUDE.load(Ordering::SeqCst) && seen && gone {
                // Claude is gone. Exiting the process tears down the webview
                // (its timers/listeners) and every poller thread.
                app.exit(0);
                return;
            }
            // Newest session with a token count is the active one: remember its
            // context so the next start can show it before any hook has run.
            if let Some(active) = live.iter().find(|r| r["tokens"].is_number()) {
                crate::usage_cache::update_context(active);
            }
            let payload = Value::Array(live);
            if payload != last {
                let _ = app.emit("claude-sessions", &payload);
                last = payload;
            }
            std::thread::sleep(POLL);
        }
    });
}
