//! Plan usage (5-hour window) from Anthropic's OAuth usage endpoint — the same
//! source as Claude Code's `/usage`. The statusLine's `rate_limits` only exists
//! in terminal sessions; the desktop app never runs a statusLine, so this is the
//! only way to get the limit there.
//!
//! Best effort by design: the endpoint is undocumented and rate limited. Any
//! failure (no credentials, expired token, 429, changed shape) leaves the last
//! good limits.json in place — the webview hides it once `resetsAt` passes —
//! and backs off. The token is read per request and never stored or logged.

use serde_json::{json, Value};
use std::io::Write;
use std::process::{Command, Stdio};
use std::time::Duration;

const URL: &str = "https://api.anthropic.com/api/oauth/usage";
const INTERVAL: Duration = Duration::from_secs(90);
const MAX_BACKOFF: Duration = Duration::from_secs(15 * 60);

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
fn fetch(token: &str) -> Option<Value> {
    let mut cmd = Command::new("curl");
    cmd.args(["-s", "--max-time", "15", "-w", "\n%{http_code}", "-H", "@-", URL])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    let mut child = cmd.spawn().ok()?;
    let headers = format!(
        "Authorization: Bearer {token}\nanthropic-beta: oauth-2025-04-20\nUser-Agent: sidecrab\n"
    );
    child.stdin.take()?.write_all(headers.as_bytes()).ok()?;
    let out = child.wait_with_output().ok()?;
    let text = String::from_utf8_lossy(&out.stdout);
    let (body, code) = text.rsplit_once('\n')?;
    if code.trim() != "200" {
        return None;
    }
    serde_json::from_str(body).ok()
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

pub fn spawn() {
    std::thread::spawn(|| {
        let path = crate::paths::home().join("limits.json");
        let mut wait = INTERVAL;
        loop {
            let limits = access_token()
                .and_then(|t| fetch(&t))
                .and_then(|r| to_limits(&r, now_ms() / 1000));
            match limits {
                Some(l) => {
                    let _ = std::fs::create_dir_all(path.parent().unwrap_or(&path));
                    let tmp = path.with_extension("json.tmp");
                    if std::fs::write(&tmp, l.to_string()).is_ok() {
                        let _ = std::fs::rename(&tmp, &path); // watcher emits claude-limits
                    }
                    wait = INTERVAL;
                }
                // 429 / expired / offline / shape change: keep last data, back off.
                None => wait = (wait * 2).min(MAX_BACKOFF),
            }
            std::thread::sleep(wait);
        }
    });
}
