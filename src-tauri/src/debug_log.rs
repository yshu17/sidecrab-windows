//! Optional timestamped diagnostics for tracking down Claude Code startup
//! stalls (issue: "sometimes the first message after opening a session is
//! slow / the session looks stuck"). Off by default — a single env var read
//! per call, so hooks and the app stay non-blocking with it unset.
//!
//! Enable for one Claude Code session with:
//!   SIDECRAB_DEBUG=1 claude
//! (or `set SIDECRAB_DEBUG=1` first on cmd). Writes timestamped lines to
//! `<SIDECRAB_HOME>/debug.log`, appended, never to stdout: hook stdout can
//! become Claude Code hook "additionalContext" (see plugin/hooks/hooks.json),
//! so nothing here may print there.
//!
//! Shared by the hook binary (pulled in with `#[path]`, like claude_proc.rs,
//! so that crate stays free of tauri deps) and the app.
#![allow(dead_code)]

use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

fn enabled() -> bool {
    std::env::var_os("SIDECRAB_DEBUG").is_some()
}

fn log_path(home: &Path) -> PathBuf {
    home.join("debug.log")
}

/// One timestamped line, only when SIDECRAB_DEBUG is set. Best effort: a
/// failed write (e.g. locked file) is silently dropped, same as the rest of
/// this app's file I/O — diagnostics must never be able to affect behaviour.
pub fn log(home: &Path, msg: &str) {
    if !enabled() {
        return;
    }
    let ms = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis()).unwrap_or(0);
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(log_path(home)) {
        let _ = writeln!(f, "[{ms}] pid={} {msg}", std::process::id());
    }
}

/// `sidecrab.log` is rotated to `sidecrab.log.1` past this size.
const EVENT_LOG_MAX: u64 = 256 * 1024;

/// Always-on app event log (`<home>/sidecrab.log`): usage refreshes and their
/// outcome, so "Refresh usage did nothing" can be answered without a debug
/// session. App only — the hook never calls it (and never writes stdout).
/// Never logs tokens or response bodies.
pub fn event(home: &Path, msg: &str) {
    let path = home.join("sidecrab.log");
    if std::fs::metadata(&path).is_ok_and(|m| m.len() > EVENT_LOG_MAX) {
        let _ = std::fs::rename(&path, home.join("sidecrab.log.1"));
    }
    let secs = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(&path) {
        let _ = writeln!(f, "[{secs}] {msg}");
    }
}

/// RAII span: logs "<label> start" immediately and "<label> end elapsed_ms=N"
/// on drop. Declaring it at the top of a function times the whole call,
/// including every early `return` in scope, without touching each one.
pub struct Timer<'a> {
    home: &'a Path,
    label: String,
    t0: Instant,
    on: bool,
}

impl<'a> Timer<'a> {
    pub fn start(home: &'a Path, label: impl Into<String>) -> Self {
        let on = enabled();
        let label = label.into();
        if on {
            log(home, &format!("{label} start"));
        }
        Timer { home, label, t0: Instant::now(), on }
    }
}

impl Drop for Timer<'_> {
    fn drop(&mut self) {
        if self.on {
            log(self.home, &format!("{} end elapsed_ms={}", self.label, self.t0.elapsed().as_millis()));
        }
    }
}
