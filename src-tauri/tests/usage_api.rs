// The OAuth usage endpoint is undocumented: parsing must accept the known
// shapes and reject anything else (so the poller keeps the last good data).
use serde_json::json;
use sidecrab_lib::usage_api::{parse_rfc3339, to_limits};

#[test]
fn rfc3339_utc_and_offsets() {
    assert_eq!(parse_rfc3339("1970-01-01T00:00:00Z"), Some(0));
    assert_eq!(parse_rfc3339("2026-09-26T17:00:00.123456+00:00"), Some(1_790_442_000));
    assert_eq!(parse_rfc3339("2026-09-26T19:00:00+02:00"), Some(1_790_442_000));
    assert_eq!(parse_rfc3339("garbage"), None);
}

#[test]
fn five_hour_window_mapped() {
    let r = json!({"five_hour": {"utilization": 15.0, "resets_at": "2026-09-26T17:00:00Z"},
                   "seven_day": {"utilization": 3.0, "resets_at": null}});
    let l = to_limits(&r, 42).unwrap();
    assert_eq!(l["fiveHour"]["usedPercentage"], 15.0);
    assert_eq!(l["fiveHour"]["resetsAt"], 1_790_442_000i64);
    assert_eq!(l["source"], "oauth");
}

#[test]
fn epoch_reset_and_alt_field_accepted() {
    let r = json!({"five_hour": {"used_percentage": 7.5, "resets_at": 1_790_442_000i64}});
    assert_eq!(to_limits(&r, 0).unwrap()["fiveHour"]["usedPercentage"], 7.5);
}

#[test]
fn unknown_shape_rejected() {
    assert!(to_limits(&json!({"error": {"type": "rate_limit_error"}}), 0).is_none());
    assert!(to_limits(&json!({"five_hour": null}), 0).is_none());
}

// --- desktop app history (no network, no token) ---
use sidecrab_lib::usage_api::from_history;

fn hist(samples: &[(i64, f64)]) -> serde_json::Value {
    json!({"version": 2, "samples": samples.iter()
        .map(|(t, fh)| json!({"t": t * 1000, "u": {"fh": fh, "sd": 50}})).collect::<Vec<_>>()})
}

#[test]
fn history_percent_and_estimated_reset() {
    // quiet at 1000, first usage seen at 2800 -> window opened ~1900 -> resets 1900+5h
    let h = hist(&[(100, 40.0), (200, 0.0), (1000, 0.0), (2800, 3.0), (3700, 15.0), (4600, 26.0)]);
    let l = from_history(&h, 4700).unwrap();
    assert_eq!(l["fiveHour"]["usedPercentage"], 26.0);
    assert_eq!(l["fiveHour"]["resetsAt"], 1900 + 5 * 3600);
    assert_eq!(l["estimated"], true);
    assert_eq!(l["source"], "desktop");
}

#[test]
fn history_window_break_on_usage_drop() {
    // 90 -> 4 is a reset: the window starts between those two samples
    let h = hist(&[(0, 80.0), (900, 90.0), (1800, 4.0), (2700, 10.0)]);
    let l = from_history(&h, 2800).unwrap();
    assert_eq!(l["fiveHour"]["resetsAt"], (900 + 1800) / 2 + 5 * 3600);
}

#[test]
fn history_zero_usage_has_no_window_yet() {
    let l = from_history(&hist(&[(0, 5.0), (900, 0.0)]), 1000).unwrap();
    assert_eq!(l["fiveHour"]["usedPercentage"], 0.0);
    assert_eq!(l["fiveHour"]["resetsAt"], 1000 + 5 * 3600);
}

#[test]
fn history_stale_or_empty_rejected() {
    assert!(from_history(&hist(&[(0, 10.0)]), 3 * 3600).is_none());
    assert!(from_history(&json!({"samples": []}), 0).is_none());
    assert!(from_history(&json!({"nope": 1}), 0).is_none());
}
