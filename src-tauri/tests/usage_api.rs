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
    // Anchored to the sample (t=900), so repeated passes write the same value.
    assert_eq!(l["fiveHour"]["resetsAt"], 900 + 5 * 3600);
    let again = from_history(&hist(&[(0, 5.0), (900, 0.0)]), 1030).unwrap();
    assert_eq!(again, l);
}

#[test]
fn history_stale_or_empty_rejected() {
    assert!(from_history(&hist(&[(0, 10.0)]), 3 * 3600).is_none());
    assert!(from_history(&json!({"samples": []}), 0).is_none());
    assert!(from_history(&json!({"nope": 1}), 0).is_none());
}

// --- Manual "Refresh usage" ---------------------------------------------------

use sidecrab_lib::usage_api::{classify, manual_allowed, may_fetch, token_from, Fetch, Gate, Token, MANUAL_GAP_MS};

#[test]
fn manual_refresh_skips_our_own_schedule() {
    // The bug: a fetch 1 min ago scheduled the next one 30 min out, and the
    // menu's refresh was swallowed by that wait.
    let gate = Gate { not_before: 1_000 + 30 * 60, server: false };
    assert!(!may_fetch(1_000, gate, false), "scheduled loop still waits");
    assert!(may_fetch(1_000, gate, true), "manual refresh goes out now");
}

#[test]
fn manual_refresh_respects_server_retry_after() {
    let gate = Gate { not_before: 1_000 + 600, server: true };
    assert!(!may_fetch(1_000, gate, true));
    assert!(may_fetch(1_600, gate, true));
}

#[test]
fn repeated_presses_are_coalesced() {
    let t = 1_000_000;
    assert!(manual_allowed(t, 0, false));
    assert!(!manual_allowed(t, 0, true), "one already running");
    assert!(!manual_allowed(t + MANUAL_GAP_MS - 1, t, false), "too soon");
    assert!(manual_allowed(t + MANUAL_GAP_MS, t, false));
}

#[test]
fn token_states_are_told_apart() {
    let now = 1_790_000_000_000;
    let creds = |tok: &str, exp: i64| json!({ "claudeAiOauth": { "accessToken": tok, "expiresAt": exp } });
    assert_eq!(token_from(&creds("abc", now + 60_000), now), Token::Ok("abc".into()));
    assert_eq!(token_from(&creds("abc", now - 1), now), Token::Expired);
    assert_eq!(token_from(&creds("abc", 0), now), Token::Ok("abc".into()), "0 = refresh managed elsewhere");
    assert_eq!(token_from(&creds("", now + 60_000), now), Token::Missing);
    assert_eq!(token_from(&json!({}), now), Token::Missing);
}

#[test]
fn http_outcomes_carry_a_reason() {
    assert!(matches!(classify(r#"{"five_hour":{}}"#, "200 "), Fetch::Ok(_)));
    assert!(matches!(classify("", "429 120"), Fetch::RetryAfter(120)));
    assert!(matches!(classify("", "401 "), Fetch::Failed(r) if r == "HTTP 401"));
    assert!(matches!(classify("", "000 "), Fetch::Failed(r) if r.contains("offline")));
    assert!(matches!(classify("<html>", "200 "), Fetch::Failed(r) if r.contains("JSON")));
}

#[test]
fn login_renewal_is_rate_limited() {
    use sidecrab_lib::usage_api::renew_allowed;
    let t = 1_790_000_000;
    assert!(renew_allowed(t, 0, false), "never tried");
    assert!(!renew_allowed(t + 59 * 60, t, false), "on its own: at most hourly");
    assert!(renew_allowed(t + 60 * 60, t, false));
    assert!(!renew_allowed(t + 4 * 60, t, true), "Refresh: at most every 5 min");
    assert!(renew_allowed(t + 5 * 60, t, true));
}
