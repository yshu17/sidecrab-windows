// usage_cache.json: last-known usage for instant startup and as a fallback. One
// test function: the cache location comes from the process-wide SIDECRAB_HOME.
use serde_json::json;
use sidecrab_lib::usage_cache::{load, update_context, update_five_hour};

#[test]
fn cache_round_trip_and_merge() {
    let dir = std::env::temp_dir().join(format!("sidecrab-cache-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::env::set_var("SIDECRAB_HOME", &dir);

    assert_eq!(load(), json!({}), "missing cache loads as empty");

    update_five_hour(&json!({
        "fiveHour": {"usedPercentage": 63.0, "resetsAt": 1_790_442_000i64},
        "estimated": true, "ts": 1000
    }));
    let c = load();
    assert_eq!(c["fiveHourUsed"], 63.0);
    assert_eq!(c["fiveHourRemaining"], 37.0);
    assert_eq!(c["fiveHourResetTime"], 1_790_442_000i64);
    assert_eq!(c["fiveHourEstimated"], true);
    assert_eq!(c["fiveHourStale"], false);
    assert_eq!(c["lastUpdated"], 1000);

    // context arrives independently and must not disturb the 5h fields
    update_context(&json!({"tokens": 34000.0, "contextSize": 200000.0, "contextPct": 17.0, "model": "Sonnet 5", "ts": 2000}));
    let c = load();
    assert_eq!(c["contextUsed"], 34000.0);
    assert_eq!(c["contextMax"], 200000.0);
    assert_eq!(c["contextPercentage"], 17.0);
    assert_eq!(c["model"], "Sonnet 5");
    assert_eq!(c["fiveHourUsed"], 63.0);
    assert_eq!(c["lastUpdated"], 2000);

    // percentage derived when the session record has none
    update_context(&json!({"tokens": 50000.0, "contextSize": 200000.0, "ts": 3000}));
    assert_eq!(load()["contextPercentage"], 25.0);

    // a failed refresh marks the cached 5h value stale
    update_five_hour(&json!({
        "fiveHour": {"usedPercentage": 63.0, "resetsAt": 1_790_442_000i64}, "stale": true, "ts": 1000
    }));
    assert_eq!(load()["fiveHourStale"], true);

    // records without usable data leave the cache alone
    let before = load();
    update_five_hour(&json!({"error": "x"}));
    update_context(&json!({"model": "Opus 5.5"}));
    assert_eq!(load(), before);
}
