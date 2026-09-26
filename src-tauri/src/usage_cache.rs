//! Last-known usage, persisted in `usage_cache.json` for instant display at
//! startup and as a fallback when a live source is unavailable.
//!
//! It is only a cache, never the source of truth: 5-hour usage also moves with
//! other Claude sessions and devices, so live data (OAuth usage API, statusLine,
//! desktop samples, hooks) always replaces it, and the webview marks anything
//! shown from the cache as stale.
//!
//! Fields (camelCase): fiveHourUsed, fiveHourRemaining (percent), fiveHourResetTime
//! (epoch s), fiveHourEstimated, fiveHourStale, fiveHourUpdated, contextUsed,
//! contextMax, contextPercentage, model, contextUpdated, lastUpdated.

use serde_json::{json, Value};
use std::path::PathBuf;
use std::sync::Mutex;

/// Writers live on different threads (limits watcher, sessions poller).
static LOCK: Mutex<()> = Mutex::new(());

fn path() -> PathBuf {
    crate::paths::home().join("usage_cache.json")
}

pub fn load() -> Value {
    std::fs::read_to_string(path())
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .filter(Value::is_object)
        .unwrap_or_else(|| json!({}))
}

fn save(c: &Value) {
    let p = path();
    if let Some(dir) = p.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let tmp = p.with_extension("json.tmp");
    if std::fs::write(&tmp, c.to_string()).is_ok() {
        let _ = std::fs::rename(&tmp, &p);
    }
}

/// Merge `fields` into the cache; write only when a value actually changed.
fn merge(fields: Value, updated: i64) {
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut c = load();
    let before = c.clone();
    if let (Some(dst), Some(src)) = (c.as_object_mut(), fields.as_object()) {
        for (k, v) in src {
            dst.insert(k.clone(), v.clone());
        }
    }
    if c == before {
        return;
    }
    let newest = c["lastUpdated"].as_i64().unwrap_or(0).max(updated);
    c["lastUpdated"] = json!(newest);
    save(&c);
}

/// From a limits.json document (any source).
pub fn update_five_hour(limits: &Value) {
    let f = &limits["fiveHour"];
    let (Some(used), Some(resets)) = (f["usedPercentage"].as_f64(), f["resetsAt"].as_i64()) else {
        return;
    };
    merge(
        json!({
            "fiveHourUsed": used,
            "fiveHourRemaining": (100.0 - used).max(0.0),
            "fiveHourResetTime": resets,
            "fiveHourEstimated": limits["estimated"] == true,
            "fiveHourStale": limits["stale"] == true,
            "fiveHourUpdated": limits["ts"].as_i64().unwrap_or(0),
        }),
        limits["ts"].as_i64().unwrap_or(0),
    );
}

/// From a sessions.d record (the active session's context usage).
pub fn update_context(session: &Value) {
    let Some(used) = session["tokens"].as_f64() else { return };
    let max = session["contextSize"].as_f64();
    let pct = session["contextPct"]
        .as_f64()
        .or_else(|| max.filter(|m| *m > 0.0).map(|m| used / m * 100.0));
    let mut fields = json!({
        "contextUsed": used,
        "contextUpdated": session["ts"].as_i64().unwrap_or(0),
    });
    if let Some(m) = max {
        fields["contextMax"] = json!(m);
    }
    if let Some(p) = pct {
        fields["contextPercentage"] = json!(p);
    }
    if let Some(m) = session["model"].as_str() {
        fields["model"] = json!(m);
    }
    merge(fields, session["ts"].as_i64().unwrap_or(0));
}
