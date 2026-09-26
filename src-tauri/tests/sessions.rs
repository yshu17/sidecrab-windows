// sessions::scan decides which Claude Code sessions are alive — and therefore
// whether a plugin-launched pet should quit.
use serde_json::json;
use sidecrab_lib::sessions::scan;

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
