// sessions::scan decides which Claude Code sessions are alive — and therefore
// whether a plugin-launched pet should quit.
use serde_json::json;
use sidecrab_lib::sessions::{scan, ExitWatch};
use std::time::{Duration, Instant};

const GRACE: Duration = Duration::from_secs(15);

fn clock() -> impl Fn(u64) -> Instant {
    let t0 = Instant::now();
    move |s| t0 + Duration::from_secs(s)
}

#[test]
fn quits_only_after_claude_stays_gone_for_the_whole_grace() {
    let at = clock();
    let mut w = ExitWatch::new(GRACE);
    w.arm();
    assert!(!w.observe(true, at(0)));
    assert!(!w.observe(false, at(2)), "a brief gap (desktop app restart) must not quit");
    assert!(!w.observe(false, at(16)));
    assert!(!w.observe(true, at(17)), "Claude came back: the timer resets");
    assert!(!w.observe(false, at(20)));
    assert!(!w.observe(false, at(34)));
    assert!(w.observe(false, at(35)), "gone for the whole grace period: quit");
}

#[test]
fn never_quits_unless_launched_by_the_plugin() {
    let at = clock();
    let mut w = ExitWatch::new(GRACE); // manual `sidecrab` launch: never armed
    assert!(!w.observe(true, at(0)));
    for s in (2..=120).step_by(2) {
        assert!(!w.observe(false, at(s)), "unarmed pet quit at {s}s");
    }
}

#[test]
fn never_quits_before_claude_was_seen() {
    let at = clock();
    let mut w = ExitWatch::new(GRACE);
    w.arm();
    for s in (0..=60).step_by(2) {
        assert!(!w.observe(false, at(s)), "quit at {s}s without ever seeing Claude");
    }
    assert!(!w.observe(true, at(62)));
    assert!(!w.observe(false, at(64)));
    assert!(w.observe(false, at(79)), "seen, then gone for the grace period: quit");
}

#[test]
fn rearming_waits_for_claude_again() {
    let at = clock();
    let mut w = ExitWatch::new(GRACE);
    w.arm();
    assert!(!w.observe(true, at(0)));
    assert!(!w.observe(false, at(2)));
    w.arm(); // a new --plugin launch while Claude looks gone
    assert!(!w.observe(false, at(40)), "re-armed: must see Claude first");
    assert!(!w.observe(true, at(42)));
    assert!(!w.observe(false, at(44)));
    assert!(w.observe(false, at(59)), "seen after re-arm, then gone for the grace period: quit");
}

fn tmp_dir(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("sidecrab-sessions-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn dead_owner_is_pruned_live_owner_kept() {
    let dir = tmp_dir("owners");
    std::fs::write(dir.join("live"), json!({"claudePid": 1, "claudeStart": 10, "ts": 5}).to_string()).unwrap();
    std::fs::write(dir.join("dead"), json!({"claudePid": 2, "claudeStart": 20, "ts": 9}).to_string()).unwrap();
    let live = scan(&dir, |(pid, _)| pid == 1);
    assert_eq!(live.len(), 1);
    assert_eq!(live[0]["id"], "live");
    assert!(!dir.join("dead").exists(), "dead session record must be deleted");
}

#[test]
fn record_without_owner_lives_until_session_end() {
    let dir = tmp_dir("no-owner");
    std::fs::write(dir.join("s"), json!({"ts": 1}).to_string()).unwrap();
    assert_eq!(scan(&dir, |_| false).len(), 1);
}

#[test]
fn legacy_empty_files_removed_and_tmp_ignored() {
    let dir = tmp_dir("legacy");
    std::fs::write(dir.join("old"), "").unwrap();
    std::fs::write(dir.join("s.123.tmp"), "{").unwrap();
    assert!(scan(&dir, |_| true).is_empty());
    assert!(!dir.join("old").exists());
    assert!(dir.join("s.123.tmp").exists());
}

#[test]
fn newest_activity_first() {
    let dir = tmp_dir("order");
    std::fs::write(dir.join("a"), json!({"ts": 1}).to_string()).unwrap();
    std::fs::write(dir.join("b"), json!({"ts": 3}).to_string()).unwrap();
    let ids: Vec<_> = scan(&dir, |_| true).into_iter().map(|r| r["id"].clone()).collect();
    assert_eq!(ids, [json!("b"), json!("a")]);
}
