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
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};
use std::sync::{Condvar, Mutex, OnceLock};
use std::time::Duration;
use tauri::{AppHandle, Emitter};

const URL: &str = "https://api.anthropic.com/api/oauth/usage";
/// OAuth usage is fetched at startup, right after the window's reset time, on a
/// manual refresh, and otherwise this often (5h usage also moves with other
/// sessions and devices). A 429 Retry-After from the endpoint always wins.
const BACKGROUND: u64 = 5 * 60;
const MAX_BACKOFF: u64 = 60 * 60;
/// After the reset time, wait this long before asking for the new window.
const RESET_GRACE: i64 = 5;

/// Outcome of one request. The endpoint answers 429 with Retry-After, and
/// asking again early seems to extend the block, so the wait is honoured and
/// persisted (the pet restarts with every Claude Code session).
pub enum Fetch {
    Ok(Value),
    RetryAfter(u64),
    /// Why, for the log (never the token or the response body).
    Failed(String),
}

/// The OAuth token from Claude Code's credentials file, or why there is none.
#[derive(Debug, PartialEq)]
pub enum Token {
    Ok(String),
    Missing,
    /// Only the Claude Code CLI rewrites this file; the desktop app keeps its own
    /// login, so with the CLI unused for a while the stored token ages out.
    Expired,
}

/// Read the token out of a parsed `.credentials.json`.
pub fn token_from(creds: &Value, now_ms: i64) -> Token {
    let o = &creds["claudeAiOauth"];
    let Some(t) = o["accessToken"].as_str().filter(|t| !t.is_empty()) else {
        return Token::Missing;
    };
    // expiresAt is epoch ms; 0/absent when the desktop app manages refresh.
    let exp = o["expiresAt"].as_i64().unwrap_or(0);
    if exp > 0 && exp <= now_ms {
        return Token::Expired;
    }
    Token::Ok(t.to_owned())
}

/// Only the Claude Code CLI renews the stored login, and only when it talks to
/// the API itself. `claude mcp list` does (it lists the account's claude.ai
/// connectors), so Sidecrab asks Claude Code to renew its own login — it never
/// touches the credentials itself. Gap between attempts: an hour on its own,
/// 5 minutes when the user pressed Refresh.
const RENEW_GAP_S: i64 = 60 * 60;
const RENEW_GAP_MANUAL_S: i64 = 5 * 60;
const RENEW_TIMEOUT: Duration = Duration::from_secs(90);
static LAST_RENEW_S: AtomicI64 = AtomicI64::new(0);

pub fn renew_allowed(now_s: i64, last_s: i64, manual: bool) -> bool {
    now_s - last_s >= if manual { RENEW_GAP_MANUAL_S } else { RENEW_GAP_S }
}

fn config_renew_on() -> bool {
    crate::config::load().renew_login
}

/// Run `claude mcp list` (fixed arguments, no window, output discarded) and
/// re-read the token. `claude` is an npm `.cmd` shim on Windows, hence cmd /C.
fn renew_login() -> Token {
    log("login expired: asking Claude Code to renew it (claude mcp list)");
    let mut cmd = if cfg!(windows) {
        let mut c = Command::new("cmd");
        c.args(["/D", "/C", "claude", "mcp", "list"]);
        c
    } else {
        let mut c = Command::new("claude");
        c.args(["mcp", "list"]);
        c
    };
    cmd.stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    let t0 = std::time::Instant::now();
    match cmd.spawn() {
        Err(e) => log(&format!("login renew: could not start claude ({e})")),
        Ok(mut child) => loop {
            match child.try_wait() {
                Ok(Some(_)) => break,
                Ok(None) if t0.elapsed() < RENEW_TIMEOUT => std::thread::sleep(Duration::from_millis(250)),
                _ => {
                    let _ = child.kill();
                    log("login renew: claude did not finish in time");
                    break;
                }
            }
        },
    }
    let token = access_token();
    log(&format!(
        "login renew: {} after {} ms",
        if matches!(token, Token::Ok(_)) { "renewed" } else { "still not valid" },
        t0.elapsed().as_millis()
    ));
    token
}

fn access_token() -> Token {
    let creds = dirs::home_dir()
        .map(|h| h.join(".claude").join(".credentials.json"))
        .and_then(|p| std::fs::read_to_string(p).ok())
        .and_then(|s| serde_json::from_str::<Value>(&s).ok());
    match creds {
        Some(c) => token_from(&c, now_ms()),
        None => Token::Missing,
    }
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
        return Fetch::Failed("curl could not be started".into());
    };
    let headers = format!(
        "Authorization: Bearer {token}\nanthropic-beta: oauth-2025-04-20\nUser-Agent: sidecrab\n"
    );
    if let Some(mut stdin) = child.stdin.take() {
        let _ = stdin.write_all(headers.as_bytes());
    }
    let Ok(out) = child.wait_with_output() else {
        return Fetch::Failed("curl did not finish".into());
    };
    let text = String::from_utf8_lossy(&out.stdout);
    let Some((body, status)) = text.rsplit_once('\n') else {
        return Fetch::Failed("no response (offline or timed out)".into());
    };
    classify(body, status)
}

/// curl's output (body, then "<status> <retry-after>") -> outcome.
pub fn classify(body: &str, status_line: &str) -> Fetch {
    let mut status = status_line.split_whitespace();
    match (status.next(), status.next().and_then(|s| s.parse::<u64>().ok())) {
        (Some("200"), _) => serde_json::from_str(body)
            .map_or_else(|_| Fetch::Failed("HTTP 200 with unreadable JSON".into()), Fetch::Ok),
        (Some("429"), Some(secs)) => Fetch::RetryAfter(secs),
        (Some("000") | None, _) => Fetch::Failed("no response (offline or timed out)".into()),
        (Some(code), _) => Fetch::Failed(format!("HTTP {code}")),
    }
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
/// `server` = the wait came from a 429 Retry-After, which a manual refresh must
/// respect; otherwise it is only our own schedule, which a manual refresh skips.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Gate {
    pub not_before: i64,
    pub server: bool,
}

/// May a request go out now?
pub fn may_fetch(now_s: i64, gate: Gate, manual: bool) -> bool {
    now_s >= gate.not_before || (manual && !gate.server)
}

/// Manual refreshes closer together than this are dropped (menu mashing).
pub const MANUAL_GAP_MS: i64 = 10_000;

/// Accept a manual refresh? Not while one is still pending or running, and not
/// within `MANUAL_GAP_MS` of the last accepted one.
pub fn manual_allowed(now_ms: i64, last_accepted_ms: i64, busy: bool) -> bool {
    !busy && now_ms - last_accepted_ms >= MANUAL_GAP_MS
}

fn gate_path() -> std::path::PathBuf {
    crate::paths::home().join("usage_api.json")
}

fn load_gate() -> Gate {
    let v = std::fs::read_to_string(gate_path())
        .ok()
        .and_then(|s| serde_json::from_str::<Value>(&s).ok())
        .unwrap_or(Value::Null);
    Gate { not_before: v["notBefore"].as_i64().unwrap_or(0), server: v["server"] == true }
}

fn save_gate(g: Gate) {
    let _ = std::fs::write(gate_path(), json!({ "notBefore": g.not_before, "server": g.server }).to_string());
}

static APP: OnceLock<AppHandle> = OnceLock::new();
static REFRESH: (Mutex<bool>, Condvar) = (Mutex::new(false), Condvar::new());
static MANUAL_BUSY: AtomicBool = AtomicBool::new(false);
static LAST_MANUAL_MS: AtomicI64 = AtomicI64::new(0);
/// limits.json is read-modified-written by the OAuth and desktop-history
/// threads; one lock so neither overwrites the other from a stale read.
static LIMITS_LOCK: Mutex<()> = Mutex::new(());

fn log(msg: &str) {
    debug_log::event(&crate::paths::home(), msg);
}

/// Tell the status bar how a manual refresh is going ("running"/"ok"/"failed").
/// `login` = it failed because the stored Claude Code login is expired/missing.
fn emit_refresh(phase: &str, detail: &str, login: bool) {
    if let Some(app) = APP.get() {
        let _ = app.emit("usage-refresh", json!({ "phase": phase, "detail": detail, "login": login }));
    }
}

fn finish_manual(ok: bool, detail: &str, login: bool) {
    log(&format!("manual refresh {}: {detail}", if ok { "done" } else { "failed" }));
    emit_refresh(if ok { "ok" } else { "failed" }, detail, login);
    MANUAL_BUSY.store(false, Ordering::SeqCst);
}

/// True when OAuth can't run for lack of a usable stored login — the status
/// bar then says "login" and the menu tells how to renew it.
pub fn login_needed() -> bool {
    !matches!(access_token(), Token::Ok(_))
}

/// Manual refresh (menu). Forces a request now, skipping our own schedule but
/// still honouring a server-imposed Retry-After. One at a time; repeats within
/// `MANUAL_GAP_MS` are dropped.
pub fn request_refresh() {
    let now = now_ms();
    let busy = MANUAL_BUSY.load(Ordering::SeqCst);
    if !manual_allowed(now, LAST_MANUAL_MS.load(Ordering::SeqCst), busy) {
        log(if busy {
            "manual refresh ignored: one is already running"
        } else {
            "manual refresh ignored: pressed again too soon"
        });
        return;
    }
    LAST_MANUAL_MS.store(now, Ordering::SeqCst);
    MANUAL_BUSY.store(true, Ordering::SeqCst);
    log("manual refresh requested");
    emit_refresh("running", "", false);
    *REFRESH.0.lock().unwrap_or_else(|e| e.into_inner()) = true;
    REFRESH.1.notify_all();
}

#[tauri::command]
pub fn refresh_usage() {
    request_refresh();
}

/// Sleep up to `secs`, waking early for a manual refresh. True = woken by one.
fn wait(secs: u64) -> bool {
    let g = REFRESH.0.lock().unwrap_or_else(|e| e.into_inner());
    let (mut g, _) = REFRESH
        .1
        .wait_timeout_while(g, Duration::from_secs(secs.max(1)), |flag| !*flag)
        .unwrap_or_else(|e| e.into_inner());
    std::mem::take(&mut *g)
}

/// The last fetch failed: keep the last limits but flag them stale so the UI
/// says so. A later successful write (any source) clears the flag.
fn mark_stale(path: &std::path::Path) {
    let _lock = LIMITS_LOCK.lock().unwrap_or_else(|e| e.into_inner());
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

/// Atomic replace; the watcher then emits `claude-limits`. Callers hold LIMITS_LOCK.
fn write_limits(path: &std::path::Path, l: &Value) {
    let _ = std::fs::create_dir_all(path.parent().unwrap_or(path));
    let tmp = path.with_extension("json.tmp");
    if std::fs::write(&tmp, l.to_string()).is_ok() && std::fs::rename(&tmp, path).is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
}

pub fn spawn(app: AppHandle) {
    let _ = APP.set(app);
    std::thread::spawn(|| {
        let path = crate::paths::home().join("limits.json");
        let mut backoff = BACKGROUND; // doubling per consecutive failure, up to MAX_BACKOFF
        let mut manual = false;
        // A schedule saved by an older, slower version (or before a long
        // sleep) must not hold the first request back past the current
        // interval; a server Retry-After is kept as is.
        let g = load_gate();
        let start = now_ms() / 1000;
        if !g.server && g.not_before > start + BACKGROUND as i64 {
            save_gate(Gate { not_before: start, server: false });
        }
        loop {
            let now = now_ms() / 1000;
            let gate = load_gate();
            if !may_fetch(now, gate, manual) {
                if manual {
                    // Asking during a 429 block seems to extend it: refresh the
                    // local source only, and say how long the server wants.
                    sync_desktop_history();
                    finish_manual(false, &format!("server asked to wait {}s more (HTTP 429)", gate.not_before - now), false);
                }
                manual = wait((gate.not_before - now).max(1) as u64);
                continue;
            }
            log(&format!("oauth request started ({})", if manual { "manual" } else { "scheduled" }));
            let mut token = access_token();
            if token == Token::Expired && config_renew_on() && renew_allowed(now, LAST_RENEW_S.load(Ordering::SeqCst), manual) {
                LAST_RENEW_S.store(now, Ordering::SeqCst);
                token = renew_login();
            }
            let login = !matches!(token, Token::Ok(_));
            let result = match token {
                Token::Ok(t) => fetch(&t),
                Token::Missing => Fetch::Failed("no Claude Code login in ~/.claude/.credentials.json".into()),
                Token::Expired => Fetch::Failed(
                    "stored login expired (renewed when Claude Code in a terminal next talks to the API)".into(),
                ),
            };
            let mut server = false;
            let (next, outcome): (u64, Result<String, String>) = match result {
                Fetch::Ok(r) => match to_limits(&r, now) {
                    Some(l) => {
                        {
                            let _lock = LIMITS_LOCK.lock().unwrap_or_else(|e| e.into_inner());
                            write_limits(&path, &l);
                        }
                        backoff = BACKGROUND;
                        let resets = l["fiveHour"]["resetsAt"].as_i64().unwrap_or(0);
                        let msg = format!(
                            "5h used={}% resets_at={resets}, limits.json written",
                            l["fiveHour"]["usedPercentage"]
                        );
                        // Next: just after this window resets, else the rare background refresh.
                        let until_reset = resets - now + RESET_GRACE;
                        let next = if until_reset > 0 { (until_reset as u64).min(BACKGROUND) } else { BACKGROUND };
                        (next, Ok(msg))
                    }
                    None => {
                        mark_stale(&path);
                        backoff = (backoff * 2).min(MAX_BACKOFF);
                        (backoff, Err("response has no five_hour window (shape changed?)".into()))
                    }
                },
                // Server-dictated wait, never earlier.
                Fetch::RetryAfter(secs) => {
                    mark_stale(&path);
                    server = true;
                    (secs + 30, Err(format!("HTTP 429, retry after {secs}s")))
                }
                // Expired token / offline / 5xx.
                Fetch::Failed(why) => {
                    mark_stale(&path);
                    backoff = (backoff * 2).min(MAX_BACKOFF);
                    (backoff, Err(why))
                }
            };
            match &outcome {
                Ok(msg) => log(&format!("oauth ok: {msg}")),
                Err(why) => log(&format!("oauth error: {why}; keeping last value, next try in {next}s")),
            }
            save_gate(Gate { not_before: now + next as i64, server });
            if manual {
                match outcome {
                    Ok(msg) => finish_manual(true, &msg, false),
                    Err(why) => {
                        // OAuth failed: at least pick up the desktop app's latest sample now.
                        sync_desktop_history();
                        finish_manual(false, &why, login);
                    }
                }
            }
            manual = wait(next);
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
    // No usage yet = no window yet. Anchor the placeholder to the sample, not to
    // "now": a now-based time slid forward on every 30 s pass, rewriting
    // limits.json (and the log) each time and never settling.
    let resets = if pct > 0.0 { start + WINDOW_S } else { t_last + WINDOW_S };
    Some(json!({
        "fiveHour": { "usedPercentage": pct, "resetsAt": resets },
        "estimated": true,
        "source": "desktop",
        "ts": t_last,
    }))
}

/// One pass of the desktop-history source. Within a window whose reset time a
/// precise source (OAuth, statusLine) already gave, only the percentage is
/// refreshed (when the sample is newer); otherwise the estimate is written.
fn sync_desktop_history() {
    let now = now_ms() / 1000;
    let limits = crate::paths::home().join("limits.json");
    let history = dirs::config_dir()
        .map(|d| d.join("Claude").join("plan-usage-history.json"))
        .and_then(|p| std::fs::read_to_string(p).ok())
        .and_then(|s| serde_json::from_str::<Value>(&s).ok());
    let Some(l) = history.and_then(|h| from_history(&h, now)) else { return };
    let _lock = LIMITS_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let existing: Value = std::fs::read_to_string(&limits)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or(Value::Null);
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

/// Keep limits.json fed from the desktop history every 30 s (a manual refresh
/// whose OAuth request fails also runs one pass at once).
pub fn spawn_desktop_history() {
    std::thread::spawn(|| loop {
        sync_desktop_history();
        std::thread::sleep(Duration::from_secs(30));
    });
}
