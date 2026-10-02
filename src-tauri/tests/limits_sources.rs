// Who may write limits.json when: the plugin's engine figure ("session"),
// OAuth, the desktop app's samples; and reading a file the plugin left
// half-written.
use serde_json::{json, Value};
use sidecrab_lib::usage_api::{
    desktop_merge, fresh_session, oauth_may_replace, read_existing, skip_wait, Existing, SESSION_FRESH_S,
};

const NOW: i64 = 1_790_000_000;

fn session(pct: f64, ts: i64, resets: i64) -> Value {
    json!({ "fiveHour": { "usedPercentage": pct, "resetsAt": resets }, "source": "session", "ts": ts })
}

fn oauth(pct: f64, ts: i64, resets: i64) -> Value {
    json!({ "fiveHour": { "usedPercentage": pct, "resetsAt": resets }, "source": "oauth", "ts": ts })
}

fn desktop(pct: f64, ts: i64, resets: i64) -> Value {
    json!({ "fiveHour": { "usedPercentage": pct, "resetsAt": resets }, "estimated": true, "source": "desktop", "ts": ts })
}

// --- skipping the scheduled OAuth request ------------------------------------

#[test]
fn a_fresh_session_value_skips_scheduled_oauth() {
    assert_eq!(fresh_session(&session(41.0, NOW - 60, NOW + 3600), NOW), Some(60));
}

#[test]
fn a_session_value_older_than_the_threshold_does_not() {
    let l = session(41.0, NOW - SESSION_FRESH_S, NOW + 3600);
    assert_eq!(fresh_session(&l, NOW), None, "idle: OAuth polls as before");
    assert_eq!(fresh_session(&session(41.0, NOW - SESSION_FRESH_S + 1, NOW + 3600), NOW), Some(SESSION_FRESH_S - 1));
}

#[test]
fn a_reset_window_does_not() {
    assert_eq!(fresh_session(&session(41.0, NOW - 60, NOW), NOW), None, "asks for the new window");
}

#[test]
fn other_sources_never_skip() {
    assert_eq!(fresh_session(&oauth(41.0, NOW - 60, NOW + 3600), NOW), None);
    assert_eq!(fresh_session(&desktop(41.0, NOW - 60, NOW + 3600), NOW), None);
    assert_eq!(fresh_session(&json!({ "source": "session" }), NOW), None);
}

#[test]
fn a_session_value_from_the_future_does_not_count_as_fresh() {
    // Clock skew between the engine and the pet: don't trust it, poll.
    assert_eq!(fresh_session(&session(41.0, NOW + 30, NOW + 3600), NOW), None);
}

#[test]
fn after_a_skip_the_loop_wakes_when_the_value_turns_stale_or_the_window_resets() {
    let l = session(41.0, NOW - 60, NOW + 3600);
    assert_eq!(skip_wait(&l, 60, NOW), 5 * 60, "capped at the usual interval");
    assert_eq!(skip_wait(&l, SESSION_FRESH_S - 20, NOW), 20, "turns stale first");
    let resetting = session(41.0, NOW - 60, NOW + 40);
    assert_eq!(skip_wait(&resetting, 60, NOW), 45, "window reset + grace first");
}

// --- OAuth vs a newer session value ------------------------------------------

#[test]
fn oauth_keeps_a_session_value_written_during_its_request() {
    let started = NOW - 2;
    let existing = Existing::Limits(session(41.0, NOW - 1, NOW + 3600));
    assert!(!oauth_may_replace(&existing, started, NOW));
}

#[test]
fn oauth_replaces_an_older_session_value() {
    // A manual refresh while the plugin's value is a few minutes old.
    let existing = Existing::Limits(session(41.0, NOW - 300, NOW + 3600));
    assert!(oauth_may_replace(&existing, NOW, NOW));
}

#[test]
fn oauth_replaces_a_session_value_whose_window_reset() {
    let existing = Existing::Limits(session(99.0, NOW, NOW - 10));
    assert!(oauth_may_replace(&existing, NOW, NOW));
}

#[test]
fn oauth_replaces_everything_else() {
    assert!(oauth_may_replace(&Existing::Missing, NOW, NOW));
    assert!(oauth_may_replace(&Existing::Unreadable, NOW, NOW));
    assert!(oauth_may_replace(&Existing::Limits(oauth(10.0, NOW, NOW + 3600)), NOW, NOW));
    assert!(oauth_may_replace(&Existing::Limits(desktop(10.0, NOW, NOW + 3600)), NOW, NOW));
}

// --- the desktop samples never override the engine -----------------------------

#[test]
fn desktop_never_touches_a_session_value_in_its_window() {
    // The case that showed a wrong 0%: a newer desktop sample.
    let existing = Existing::Limits(session(41.0, NOW - 3600, NOW + 3600));
    assert_eq!(desktop_merge(&existing, &desktop(0.0, NOW, NOW + 18_000), NOW), None);
}

#[test]
fn desktop_may_follow_a_session_value_whose_window_reset() {
    let existing = Existing::Limits(session(41.0, NOW - 7200, NOW - 10));
    let l = desktop(3.0, NOW, NOW + 18_000);
    assert_eq!(desktop_merge(&existing, &l, NOW), Some(l));
}

#[test]
fn desktop_leaves_a_half_written_file_alone() {
    assert_eq!(desktop_merge(&Existing::Unreadable, &desktop(5.0, NOW, NOW + 18_000), NOW), None);
}

#[test]
fn desktop_keeps_its_old_rules_for_oauth_and_missing() {
    let l = desktop(30.0, NOW, NOW + 18_000);
    assert_eq!(desktop_merge(&Existing::Missing, &l, NOW), Some(l.clone()));
    // Within an OAuth window only the newer percentage is taken.
    let o = oauth(20.0, NOW - 60, NOW + 3600);
    let merged = desktop_merge(&Existing::Limits(o.clone()), &l, NOW).unwrap();
    assert_eq!(merged["fiveHour"]["usedPercentage"], 30.0);
    assert_eq!(merged["fiveHour"]["resetsAt"], NOW + 3600);
    assert_eq!(merged["source"], "oauth");
    // An older sample changes nothing.
    assert_eq!(desktop_merge(&Existing::Limits(o), &desktop(30.0, NOW - 600, NOW + 18_000), NOW), None);
}

// --- reading a file the plugin left half-written --------------------------------

fn tmp_file(name: &str, text: Option<&str>) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("sidecrab-limits-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let p = dir.join("limits.json");
    if let Some(t) = text {
        std::fs::write(&p, t).unwrap();
    }
    p
}

#[test]
fn a_missing_file_is_missing() {
    assert_eq!(read_existing(&tmp_file("missing", None)), Existing::Missing);
}

#[test]
fn an_empty_or_truncated_file_is_unreadable_not_empty() {
    assert_eq!(read_existing(&tmp_file("empty", Some(""))), Existing::Unreadable);
    let cut = r#"{"fiveHour":{"usedPercentage":41,"resetsAt":17909"#;
    assert_eq!(read_existing(&tmp_file("cut", Some(cut))), Existing::Unreadable);
    assert_eq!(read_existing(&tmp_file("scalar", Some("41"))), Existing::Unreadable);
}

#[test]
fn the_finished_file_reads_again() {
    let p = tmp_file("finish", Some(r#"{"fiveHour":{"usedPercent"#));
    assert_eq!(read_existing(&p), Existing::Unreadable);
    let full = session(41.0, NOW, NOW + 3600);
    std::fs::write(&p, full.to_string()).unwrap();
    assert_eq!(read_existing(&p), Existing::Limits(full));
}
