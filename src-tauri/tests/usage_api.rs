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
