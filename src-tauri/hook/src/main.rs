// Standalone hook handler invoked by Claude Code hooks. Reads the hook JSON payload
// on stdin, maps the event to a pet state, and atomically writes state.json under
// SIDECRAB_HOME (default: ~/Library/Application Support/sidecrab). Also maintains
// sessions.d/ (one JSON file per live session: the owning claude.exe pid/start time
// for liveness, plus model/token usage from the statusLine) and, on session start/end, clears a stale
// frozen state — but only if that state is owned by the same session id (a warmup
// burst from another session must never wipe a live turn).
//
// Usage: sidecrab-hook <prompt|pre|post|notify|permreq|stop|fail|start|end>
//        sidecrab-hook statusline   (Claude Code statusLine command: records rate
//        limits to limits.json and the session's token usage for the pet, and
//        prints a one-line status)

use serde_json::{json, Value};
use std::io::Read;
use std::path::{Path, PathBuf};

#[path = "../../src/claude_proc.rs"]
mod claude_proc;

// Mirror of sidecrab_lib::paths::home() — duplicated so this crate stays free
// of the tauri dependency tree. Keep the two in sync.
fn home() -> PathBuf {
    if let Ok(h) = std::env::var("SIDECRAB_HOME") {
        return PathBuf::from(h);
    }
    dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("/tmp"))
        .join("sidecrab")
}

fn tool_label(tool: &str) -> &'static str {
    match tool {
        "Bash" => "Running command",
        "Edit" | "MultiEdit" | "NotebookEdit" => "Editing",
        "Write" => "Writing",
        "Read" => "Reading",
        "Grep" | "Glob" => "Searching",
        "WebFetch" => "Browsing web",
        "WebSearch" => "Searching web",
        "Task" => "Delegating",
        "TodoWrite" => "Planning",
        _ => "Using tool",
    }
}

fn safe_id(v: &Value) -> String {
    v["session_id"]
        .as_str()
        .unwrap_or("")
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || "_.-".contains(*c))
        .take(64)
        .collect()
}

fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

fn read_state(path: &Path) -> Value {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_else(|| json!({}))
}

fn write_atomic(path: &Path, v: &Value) {
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let tmp = path.with_extension(format!("{}.tmp", std::process::id()));
    if std::fs::write(&tmp, v.to_string()).is_ok() {
        let _ = std::fs::rename(&tmp, path);
    }
}

/// Reset a frozen mid-turn state, but only when `sid` owns it. Force-quit fires
/// SessionEnd with no Stop, which would otherwise freeze the crab mid-animation.
fn clear_stale_state(state_path: &Path, sid: &str) {
    let prev = read_state(state_path);
    if prev["sessionId"].as_str().unwrap_or("") != sid || sid.is_empty() {
        return;
    }
    match prev["state"].as_str().unwrap_or("") {
        "thinking" | "tool" | "permission" => {}
        _ => return,
    }
    let mut out = prev;
    out["state"] = json!("idle");
    out["label"] = json!("");
    out["startedAt"] = json!(0);
    out["ts"] = json!(now());
    write_atomic(state_path, &out);
}

/// "claude-opus-5-5" -> "Opus 5.5", "claude-haiku-4-5-20251001" -> "Haiku 4.5".
fn model_name(id: &str) -> String {
    let mut parts = id.trim_start_matches("claude-").split('-').filter(|p| p.len() < 8);
    let family = parts.next().unwrap_or(id);
    let mut name: String = family[..1].to_uppercase() + &family[1..];
    let version: Vec<&str> = parts.collect();
    if !version.is_empty() {
        name.push(' ');
        name.push_str(&version.join("."));
    }
    name
}

/// Context window by model: every current model has 1M except Haiku (200K).
/// Used only when the statusLine hasn't reported context_window_size.
fn context_size(model_id: &str) -> f64 {
    if model_id.contains("haiku") { 200_000.0 } else { 1_000_000.0 }
}

/// Context usage from the transcript's most recent main-chain API response —
/// the fallback when no statusLine runs (the Claude desktop app never runs one).
/// Reads only the tail of the file.
fn transcript_usage(path: &str) -> Option<Value> {
    use std::io::{Seek, SeekFrom};
    const TAIL: u64 = 512 * 1024;
    let mut f = std::fs::File::open(path).ok()?;
    let len = f.metadata().ok()?.len();
    f.seek(SeekFrom::Start(len.saturating_sub(TAIL))).ok()?;
    let mut buf = Vec::new();
    f.read_to_end(&mut buf).ok()?;
    let text = String::from_utf8_lossy(&buf);
    text.lines().rev().find_map(|line| {
        let e: Value = serde_json::from_str(line).ok()?;
        if e["type"] != "assistant" || e["isSidechain"] == true {
            return None;
        }
        let m = &e["message"];
        let u = &m["usage"];
        let input: f64 = ["input_tokens", "cache_creation_input_tokens", "cache_read_input_tokens"]
            .iter()
            .filter_map(|k| u[*k].as_f64())
            .sum();
        let model = m["model"].as_str()?;
        if input == 0.0 || model.starts_with('<') {
            return None; // synthetic/error entries carry no real usage
        }
        let size = context_size(model);
        Some(json!({
            "model": model_name(model),
            "tokens": input,
            "contextSize": size,
            "contextPct": input / size * 100.0,
            "source": "transcript",
        }))
    })
}

/// Create/refresh sessions.d/<sid>: merge `extra` into the existing record and
/// stamp the owning Claude Code process, which the app polls for liveness
/// (SessionEnd never fires on terminal close or kill).
fn touch_session(sess_dir: &Path, sid: &str, extra: Value) {
    if sid.is_empty() {
        return;
    }
    let path = sess_dir.join(sid);
    let mut rec = read_state(&path);
    if !rec.is_object() {
        rec = json!({});
    }
    if rec["claudePid"].is_null() {
        if let Some((pid, start)) = claude_proc::claude_ancestor() {
            rec["claudePid"] = json!(pid);
            rec["claudeStart"] = json!(start);
        }
    }
    if let (Some(r), Some(e)) = (rec.as_object_mut(), extra.as_object()) {
        for (k, v) in e {
            r.insert(k.clone(), v.clone());
        }
    }
    rec["ts"] = json!(now());
    write_atomic(&path, &rec);
}

/// 12540 -> "12.5k", 148000 -> "148k" (same rule as status.js).
fn compact(n: f64) -> String {
    if n >= 1e6 {
        format!("{:.1}M", n / 1e6)
    } else if n >= 99_950.0 {
        format!("{:.0}k", n / 1e3)
    } else if n >= 1e3 {
        format!("{:.1}k", n / 1e3)
    } else {
        format!("{n:.0}")
    }
}

/// "2h10m" / "45m" until `resets_at` (epoch seconds).
fn until(resets_at: i64) -> String {
    let s = (resets_at - now()).max(0);
    let (h, m) = (s / 3600, (s % 3600) / 60);
    if h > 0 {
        format!("{h}h{m:02}m")
    } else {
        format!("{m}m")
    }
}

/// statusLine mode: runs after every assistant message (and on refreshInterval)
/// with the session's live data. Token usage goes into that session's registry
/// record; rate limits are account-wide, so they go to limits.json. rate_limits
/// only exists for Pro/Max after the first API response, and context_window
/// usage is null until then — absent values keep the previous record.
fn statusline(dir: &Path, p: &Value) {
    let mut parts = Vec::new();
    let cw = &p["context_window"];
    // Input side only (incl. cache reads/writes): what occupies the window, and
    // the same basis as used_percentage and Claude Code's "Context window".
    let tokens = cw["total_input_tokens"].as_f64();
    let mut usage = json!({ "source": "statusline" });
    if let Some(size) = cw["context_window_size"].as_f64() {
        usage["contextSize"] = json!(size);
    }
    if let Some(m) = p["model"]["display_name"].as_str() {
        usage["model"] = json!(m);
        parts.push(m.to_string());
    }
    if let Some(t) = tokens {
        usage["tokens"] = json!(t);
        let mut s = format!("{} ctx", compact(t));
        if let Some(pct) = cw["used_percentage"].as_f64() {
            usage["contextPct"] = json!(pct);
            s.push_str(&format!(" ({pct:.0}%)"));
        }
        parts.push(s);
    }
    touch_session(&dir.join("sessions.d"), &safe_id(p), usage);

    let five = &p["rate_limits"]["five_hour"];
    if let (Some(pct), Some(resets)) = (five["used_percentage"].as_f64(), five["resets_at"].as_i64()) {
        write_atomic(
            &dir.join("limits.json"),
            &json!({ "fiveHour": { "usedPercentage": pct, "resetsAt": resets }, "ts": now() }),
        );
        parts.push(format!("5h {:.0}% (resets in {})", pct, until(resets)));
    }
    print!("{}", parts.join(" \u{00b7} "));
}

fn main() {
    let event = std::env::args().nth(1).unwrap_or_default();
    let mut raw = String::new();
    let _ = std::io::stdin().read_to_string(&mut raw);
    let p: Value = serde_json::from_str(&raw).unwrap_or_else(|_| json!({}));

    let dir = home();
    if event == "statusline" {
        statusline(&dir, &p);
        return;
    }
    let state_path = dir.join("state.json");
    let sess_dir = dir.join("sessions.d");
    let sid = safe_id(&p);

    // Session lifecycle events only maintain the registry + stale-state guard.
    match event.as_str() {
        "start" => {
            // A resumed/continued session already has usage in its transcript.
            let usage = p["transcript_path"].as_str().and_then(transcript_usage).unwrap_or(json!({}));
            touch_session(&sess_dir, &sid, usage);
            clear_stale_state(&state_path, &sid);
            return;
        }
        "end" => {
            if !sid.is_empty() {
                let _ = std::fs::remove_file(sess_dir.join(&sid));
            }
            clear_stale_state(&state_path, &sid);
            return;
        }
        _ => {}
    }

    // Register the session on any activity too, so sessions predating hook install
    // are tracked once they do anything. Without a statusLine feeding this
    // session (the desktop app never runs one), refresh context usage from the
    // transcript after each API response.
    let mut usage = json!({});
    if matches!(event.as_str(), "prompt" | "pre" | "post" | "stop" | "fail") {
        let fed_by_statusline =
            read_state(&sess_dir.join(&sid))["source"].as_str() == Some("statusline");
        if !fed_by_statusline {
            if let Some(u) = p["transcript_path"].as_str().and_then(transcript_usage) {
                usage = u;
            }
        }
    }
    touch_session(&sess_dir, &sid, usage);

    let prev = read_state(&state_path);
    let ts = now();
    let mut started_at = prev["startedAt"].as_i64().unwrap_or(0);
    let project = p["cwd"]
        .as_str()
        .and_then(|c| Path::new(c).file_name())
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| prev["project"].as_str().unwrap_or("").to_string());

    let (state, label) = match event.as_str() {
        "prompt" => {
            started_at = ts;
            ("thinking", "Thinking…".to_string())
        }
        "pre" => {
            let tool = p["tool_name"].as_str().unwrap_or("");
            if started_at == 0 {
                started_at = ts;
            }
            ("tool", tool_label(tool).to_string())
        }
        "post" => {
            if started_at == 0 {
                started_at = ts;
            }
            ("thinking", "Thinking…".to_string())
        }
        "notify" => {
            // Only a permission prompt drives the pet; other notifications
            // (esp. "waiting for your input") must not park it on a stale state.
            let msg = p["message"].as_str().unwrap_or("").to_lowercase();
            let is_perm = p["notification_type"].as_str() == Some("permission_prompt")
                || msg.contains("permission")
                || msg.contains("approve")
                || msg.contains("allow");
            if !is_perm {
                return;
            }
            started_at = 0;
            ("permission", "Awaiting permission".to_string())
        }
        "permreq" => {
            started_at = 0;
            ("permission", "Awaiting permission".to_string())
        }
        "stop" => {
            started_at = 0;
            ("done", "Done".to_string())
        }
        // StopFailure: the turn died on an API error (rate limit, overload, auth…).
        "fail" => {
            started_at = 0;
            let kind = ["error_type", "error", "reason"]
                .iter()
                .find_map(|k| p[*k].as_str())
                .unwrap_or("unknown");
            ("error", format!("Error: {kind}"))
        }
        _ => return,
    };

    // Host app for double-click activation: the hook inherits the terminal's env;
    // no TERM_PROGRAM means a non-terminal surface (Claude Desktop).
    let host = std::env::var("TERM_PROGRAM").unwrap_or_else(|_| "Claude".to_string());

    let out = json!({
        "state": state,
        "label": label,
        "tool": p["tool_name"].as_str().unwrap_or(""),
        "project": project,
        "sessionId": p["session_id"].as_str().unwrap_or(""),
        "transcript": p["transcript_path"].as_str().unwrap_or_else(|| prev["transcript"].as_str().unwrap_or("")),
        "host": host,
        "startedAt": started_at,
        "ts": ts,
    });
    write_atomic(&state_path, &out);
}
