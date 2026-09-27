//! Plan usage (5-hour window) from Anthropic's OAuth usage endpoint — the same
//! source as Claude Code's `/usage`. The statusLine's `rate_limits` only exists
//! in terminal sessions; the desktop app never runs a statusLine, so this is the
//! only way to get the limit there.
//!
//! Best effort by design: the endpoint is undocumented and rate limited. Any
//! failure (no credentials, expired token, 429, changed shape) leaves the last
//! good limits.json in place — the webview hides it once `resetsAt` passes —
//! and backs off. The token is read per request and never stored or logged.

use crate::debug_log;
use serde_json::{json, Value};
use std::io::Write;
use std::sync::{Condvar, Mutex};
use std::process::{Command, Stdio};
use std::time::Duration;

const URL: &str = "https://api.anthropic.com/api/oauth/usage";
/// OAuth usage is fetched at startup, right after the window's reset time, on a
/// manual refresh, and otherwise only this rarely (5h usage also moves with other
/// sessions and devices, but the endpoint is rate limited).
const BACKGROUND: u64 = 30 * 60;
const MAX_BACKOFF: u64 = 60 * 60;
/// After the reset time, wait this long before asking for the new window.
const RESET_GRACE: i64 = 5;

/// Outcome of one request. The endpoint answers 429 with Retry-After, and
/// asking again early seems to extend the block, so the wait is honoured and
/// persisted (the pet restarts with every Claude Code session).
pub enum Fetch {
    Ok(Value),
    RetryAfter(u64),
    Failed,
}

fn access_token() -> Option<String> {
    let path = dirs::home_dir()?.join(".claude").join(".credentials.json");
    let v: Value = serde_json::from_str(&std::fs::read_to_string(path).ok()?).ok()?;
    let o = &v["claudeAiOauth"];
    // expiresAt is epoch ms; 0/absent when the desktop app manages refresh.
    let exp = o["expiresAt"].as_i64().unwrap_or(0);
    if exp > 0 && exp <= now_ms() {
        return None;
    }
    o["accessToken"].as_str().map(str::to_owned)
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// Headers go through stdin (`-H @-`) so the token never appears on a command line.
/// Always called from a background thread (see `spawn`/`request_refresh`), never
/// from Tauri's setup() or the main thread — this can take up to the curl
/// `--max-time` below, which must never be on the startup path.
fn fetch(token: &str) -> Fetch {
    let home = crate::paths::home();
    let _t = debug_log::Timer::start(&home, "usage_api.fetch");
    let mut cmd = Command::new("curl");
    cmd.args(["-s", "--max-time", "15", "-w", "\n%{http_code} %header{retry-after}", "-H", "@-", URL])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    let Ok(mut child) = cmd.spawn() else {
        debug_log::log(&home, "usage_api.fetch error=spawn_failed");
        return Fetch::Failed;
    };
    let headers = format!(
        "Authorization: Bearer {token}\nanthropic-beta: oauth-2025-04-20\nUser-Agent: sidecrab\n"
    );
    if let Some(mut stdin) = child.stdin.take() {
        let _ = stdin.write_all(headers.as_bytes());
    }
    let Ok(out) = child.wait_with_output() else {
        debug_log::log(&home, "usage_api.fetch error=curl_wait_failed");
        return Fetch::Failed;
    };
    let text = String::from_utf8_lossy(&out.stdout);
    let Some((body, status)) = text.rsplit_once('\n') else {
        debug_log::log(&home, "usage_api.fetch error=no_status_line");
        return Fetch::Failed;
    };
    let mut status = status.split_whitespace();
    let result = match (status.next(), status.next().and_then(|s| s.parse::<u64>().ok())) {
        (Some("200"), _) => serde_json::from_str(body).map_or(Fetch::Failed, Fetch::Ok),
        (Some("429"), Some(secs)) => Fetch::RetryAfter(secs),
        _ => Fetch::Failed,
    };
    debug_log::log(&home, &format!("usage_api.fetch outcome={}", match result {
        Fetch::Ok(_) => "ok",
        Fetch::RetryAfter(_) => "retry_after",
        Fetch::Failed => "failed",
    }));
    result
}

/// Days since 1970-01-01 for a proleptic Gregorian date (Howard Hinnant).
fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// "2026-09-26T17:00:00.123+00:00" / "...Z" -> epoch seconds.
pub fn parse_rfc3339(s: &str) -> Option<i64> {
    let n = |a: usize, b: usize| s.get(a..b)?.parse::<i64>().ok();
    let (y, mo, d, h, mi, se) = (n(0, 4)?, n(5, 7)?, n(8, 10)?, n(11, 13)?, n(14, 16)?, n(17, 19)?);
    let mut t = days_from_civil(y, mo, d) * 86_400 + h * 3600 + mi * 60 + se;
    // Offset: last "+hh:mm"/"-hh:mm" after the time part, or Z.
    let tail = &s[19..];
    if let Some(i) = tail.rfind(['+', '-']) {
        let off = &tail[i..];
        let sign = if off.starts_with('-') { -1 } else { 1 };
        let oh: i64 = off.get(1..3)?.parse().ok()?;
        let om: i64 = off.get(4..6).and_then(|x| x.parse().ok()).unwrap_or(0);
        t -= sign * (oh * 3600 + om * 60);
    }
    Some(t)
}

/// Map the endpoint's `five_hour` window onto the limits.json shape the hook's
/// statusLine mode also writes. Accepts `utilization` or `used_percentage`, and
/// an RFC 3339 string or epoch seconds for `resets_at`.
pub fn to_limits(resp: &Value, now_s: i64) -> Option<Value> {
    let w = &resp["five_hour"];
    let pct = w["utilization"].as_f64().or_else(|| w["used_percentage"].as_f64())?;
    let resets = match &w["resets_at"] {
        Value::String(s) => parse_rfc3339(s)?,
        v => v.as_i64()?,
    };
    Some(json!({
        "fiveHour": { "usedPercentage": pct, "resetsAt": resets },
        "source": "oauth",
        "ts": now_s,
    }))
}

/// Earliest time (epoch s) the next request may go out; survives restarts.
fn not_before_path() -> std::path::PathBuf {
    crate::paths::home().join("usage_api.json")
}

fn load_not_before() -> i64 {
    std::fs::read_to_string(not_before_path())
        .ok()
        .and_then(|s| serde_json::from_str::<Value>(&s).ok())
        .and_then(|v| v["notBefore"].as_i64())
        .unwrap_or(0)
}

fn save_not_before(t: i64) {
    let _ = std::fs::write(not_before_path(), json!({ "notBefore": t }).to_string());
}

static REFRESH: (Mutex<bool>, Condvar) = (Mutex::new(false), Condvar::new());

/// Manual refresh (menu). Still honours a server-imposed Retry-After.
pub fn request_refresh() {
    *REFRESH.0.lock().unwrap_or_else(|e| e.into_inner()) = true;
    REFRESH.1.notify_all();
}

#[tauri::command]
pub fn refresh_usage() {
    request_refresh();
}

/// Sleep up to `secs`, waking early for a manual refresh.
fn wait(secs: u64) {
    let g = REFRESH.0.lock().unwrap_or_else(|e| e.into_inner());
    let (mut g, _) = REFRESH
        .1
        .wait_timeout_while(g, Duration::from_secs(secs.max(1)), |flag| !*flag)
        .unwrap_or_else(|e| e.into_inner());
    *g = false;
}

/// The last fetch failed: keep the last limits but flag them stale so the UI
/// says so. A later successful write (any source) clears the flag.
fn mark_stale(path: &std::path::Path) {
    let Some(mut l) = std::fs::read_to_string(path)
        .ok()
        .and_then(|s| serde_json::from_str::<Value>(&s).ok())
        .filter(|l| l["stale"] != true && l.is_object())
    else {
        return;
    };
    l["stale"] = json!(true);
    write_limits(path, &l);
}

fn write_limits(path: &std::path::Path, l: &Value) {
    let _ = std::fs::create_dir_all(path.parent().unwrap_or(path));
    let tmp = path.with_extension("json.tmp");
    if std::fs::write(&tmp, l.to_string()).is_ok() {
        let _ = std::fs::rename(&tmp, path); // watcher emits claude-limits
    }
}

pub fn spawn() {
    std::thread::spawn(|| {
        let path = crate::paths::home().join("limits.json");
        let mut backoff = BACKGROUND / 6; // 5 min, doubling per consecutive failure
        loop {
            let now = now_ms() / 1000;
            let not_before = load_not_before();
            if now < not_before {
                wait((not_before - now) as u64);
                continue;
            }
            let result = match access_token() {
                Some(t) => fetch(&t),
                None => Fetch::Failed,
            };
            let next = match result {
                Fetch::Ok(r) => match to_limits(&r, now) {
                    Some(l) => {
                        write_limits(&path, &l);
                        backoff = BACKGROUND / 6;
                        // Next: just after this window resets, else the rare background refresh.
                        let until_reset = l["fiveHour"]["resetsAt"].as_i64().unwrap_or(0) - now + RESET_GRACE;
                        if until_reset > 0 { (until_reset as u64).min(BACKGROUND) } else { BACKGROUND }
                    }
                    None => {
                        mark_stale(&path); // shape changed
                        backoff = (backoff * 2).min(MAX_BACKOFF);
                        backoff
                    }
                },
                // Server-dictated wait, never earlier.
                Fetch::RetryAfter(secs) => {
                    mark_stale(&path);
                    secs + 30
                }
                // Expired token / offline / 5xx.
                Fetch::Failed => {
                    mark_stale(&path);
                    backoff = (backoff * 2).min(MAX_BACKOFF);
                    backoff
                }
            };
            save_not_before(now + next as i64);
            wait(next);
        }
    });
}

// ---------------------------------------------------------------------------
// Local source: the Claude desktop app samples the plan usage itself every ~15
// minutes into %APPDATA%\Claude\plan-usage-history.json ({t: ms, u: {fh, sd}},
// percentages). It has no reset time, so that is estimated from the window's
// first sample. This needs no network and no token, so it keeps working while
// the OAuth endpoint is rate limited.

const WINDOW_S: i64 = 5 * 3600;
/// Newer than this counts as current (the app samples every ~15 min).
const STALE_S: i64 = 45 * 60;

/// Build limits.json content from the desktop app's sample history.
pub fn from_history(history: &Value, now_s: i64) -> Option<Value> {
    let samples = history["samples"].as_array()?;
    let last = samples.last()?;
    let t_last = last["t"].as_i64()? / 1000;
    let pct = last["u"]["fh"].as_f64()?;
    if now_s - t_last > STALE_S {
        return None;
    }
    let at = |i: usize| samples[i]["t"].as_i64().map(|t| t / 1000);
    let fh = |i: usize| samples[i]["u"]["fh"].as_f64();

    // Walk back through the current window: it ends at the previous reset
    // (usage dropped), an all-zero sample, or a gap longer than a window.
    let mut first = samples.len() - 1;
    while first > 0 {
        let (prev_pct, prev_t) = (fh(first - 1)?, at(first - 1)?);
        if prev_pct == 0.0 || prev_pct > fh(first)? || at(first)? - prev_t > WINDOW_S {
            break;
        }
        first -= 1;
    }
    // The window opened somewhere between the last quiet sample and the first
    // one showing usage; take the midpoint.
    let start = if first > 0 && fh(first)? > 0.0 {
        (at(first - 1)? + at(first)?) / 2
    } else {
        at(first)?
    };
    let resets = if pct > 0.0 { start + WINDOW_S } else { now_s + WINDOW_S };
    Some(json!({
        "fiveHour": { "usedPercentage": pct, "resetsAt": resets },
        "estimated": true,
        "source": "desktop",
        "ts": t_last,
    }))
}

/// Keep limits.json fed from the desktop history. Within a window whose reset
/// time a precise source (OAuth, statusLine) already gave, only the percentage
/// is refreshed (when the sample is newer); otherwise the estimate is written.
pub fn spawn_desktop_history() {
    std::thread::spawn(|| loop {
        let now = now_ms() / 1000;
        let limits = crate::paths::home().join("limits.json");
        let existing: Value = std::fs::read_to_string(&limits)
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or(Value::Null);
        let history = dirs::config_dir()
            .map(|d| d.join("Claude").join("plan-usage-history.json"))
            .and_then(|p| std::fs::read_to_string(p).ok())
            .and_then(|s| serde_json::from_str::<Value>(&s).ok());
        if let Some(l) = history.and_then(|h| from_history(&h, now)) {
            let precise_window = !existing.is_null()
                && existing["source"].as_str() != Some("desktop")
                && existing["estimated"] != true
                && existing["fiveHour"]["resetsAt"].as_i64().unwrap_or(0) > now;
            if precise_window {
                if l["ts"].as_i64() > existing["ts"].as_i64() {
                    let mut m = existing.clone();
                    m["fiveHour"]["usedPercentage"] = l["fiveHour"]["usedPercentage"].clone();
                    m["ts"] = l["ts"].clone();
                    m.as_object_mut().map(|o| o.remove("stale")); // fresh percentage
                    if m["fiveHour"] != existing["fiveHour"] {
                        write_limits(&limits, &m);
                    }
                }
            } else if l["fiveHour"] != existing["fiveHour"] || existing["stale"] == true {
                write_limits(&limits, &l);
            }
        }
        std::thread::sleep(Duration::from_secs(30));
    });
}
