// Integration tests for the sidecrab-hook binary: spawn it exactly as Claude Code
// hooks would (event arg + JSON payload on stdin) against a temp SIDECRAB_HOME.
use std::io::Write;
use std::path::Path;
use std::process::{Command, Stdio};

const BIN: &str = env!("CARGO_BIN_EXE_sidecrab-hook");

fn run_hook(home: &Path, event: &str, payload: &str, envs: &[(&str, &str)]) {
    let mut cmd = Command::new(BIN);
    cmd.arg(event)
        .env("SIDECRAB_HOME", home)
        .env_remove("TERM_PROGRAM")
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    for (k, v) in envs {
        cmd.env(k, v);
    }
    let mut child = cmd.spawn().expect("spawn hook");
    child
        .stdin
        .take()
        .unwrap()
        .write_all(payload.as_bytes())
        .unwrap();
    let status = child.wait().unwrap();
    assert!(status.success(), "hook exited nonzero for event {event}");
}

fn state(home: &Path) -> serde_json::Value {
    let raw = std::fs::read_to_string(home.join("state.json")).expect("state.json");
    serde_json::from_str(&raw).unwrap()
}

fn tmp_home(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("sidecrab-test-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn pre_bash_writes_tool_state() {
    let home = tmp_home("pre");
    run_hook(
        &home,
        "pre",
        r#"{"tool_name":"Bash","session_id":"abc","cwd":"/x/doom"}"#,
        &[],
    );
    let s = state(&home);
    assert_eq!(s["state"], "tool");
    assert_eq!(s["label"], "Running command");
    assert_eq!(s["tool"], "Bash");
    assert_eq!(s["project"], "doom");
    assert_eq!(s["sessionId"], "abc");
    assert!(s["ts"].as_i64().unwrap() > 0);
}

#[test]
fn prompt_writes_thinking() {
    let home = tmp_home("prompt");
    run_hook(&home, "prompt", r#"{"session_id":"abc","cwd":"/x/p"}"#, &[]);
    let s = state(&home);
    assert_eq!(s["state"], "thinking");
    assert_eq!(s["label"], "Thinking…");
    assert!(s["startedAt"].as_i64().unwrap() > 0);
}

#[test]
fn unknown_tool_gets_generic_label() {
    let home = tmp_home("mcp");
    run_hook(
        &home,
        "pre",
        r#"{"tool_name":"mcp__server__thing","session_id":"abc","cwd":"/x/p"}"#,
        &[],
    );
    let s = state(&home);
    assert_eq!(s["state"], "tool");
    assert_eq!(s["label"], "Using tool");
}

#[test]
fn notify_non_permission_is_ignored() {
    let home = tmp_home("notify-idle");
    run_hook(
        &home,
        "notify",
        r#"{"session_id":"abc","message":"Claude is waiting for your input"}"#,
        &[],
    );
    assert!(!home.join("state.json").exists(), "no state should be written");
}

#[test]
fn notify_permission_writes_permission() {
    let home = tmp_home("notify-perm");
    run_hook(
        &home,
        "notify",
        r#"{"session_id":"abc","message":"Claude needs your permission to use Bash"}"#,
        &[],
    );
    let s = state(&home);
    assert_eq!(s["state"], "permission");
    assert_eq!(s["label"], "Awaiting permission");
}

#[test]
fn stop_writes_done() {
    let home = tmp_home("stop");
    run_hook(&home, "stop", r#"{"session_id":"abc"}"#, &[]);
    let s = state(&home);
    assert_eq!(s["state"], "done");
    assert_eq!(s["startedAt"], 0);
}

#[test]
fn start_registers_session_and_records_host() {
    let home = tmp_home("start");
    run_hook(
        &home,
        "start",
        r#"{"session_id":"abc"}"#,
        &[("TERM_PROGRAM", "iTerm.app")],
    );
    assert!(home.join("sessions.d").join("abc").exists());
    // A subsequent state write carries the host from the hook's environment.
    run_hook(
        &home,
        "prompt",
        r#"{"session_id":"abc","cwd":"/x/p"}"#,
        &[("TERM_PROGRAM", "iTerm.app")],
    );
    assert_eq!(state(&home)["host"], "iTerm.app");
}

#[test]
fn missing_term_program_defaults_to_claude_host() {
    let home = tmp_home("host-default");
    run_hook(&home, "prompt", r#"{"session_id":"abc","cwd":"/x/p"}"#, &[]);
    assert_eq!(state(&home)["host"], "Claude");
}

#[test]
fn end_removes_session_and_clears_own_stale_state() {
    let home = tmp_home("end-own");
    run_hook(&home, "start", r#"{"session_id":"abc"}"#, &[]);
    run_hook(
        &home,
        "pre",
        r#"{"tool_name":"Bash","session_id":"abc","cwd":"/x/p"}"#,
        &[],
    );
    run_hook(&home, "end", r#"{"session_id":"abc"}"#, &[]);
    assert!(!home.join("sessions.d").join("abc").exists());
    // Frozen "tool" state owned by the ending session resets to idle.
    assert_eq!(state(&home)["state"], "idle");
}

#[test]
fn end_of_other_session_leaves_live_state_alone() {
    let home = tmp_home("end-other");
    run_hook(
        &home,
        "pre",
        r#"{"tool_name":"Bash","session_id":"abc","cwd":"/x/p"}"#,
        &[],
    );
    run_hook(&home, "end", r#"{"session_id":"zzz"}"#, &[]);
    // Live turn owned by "abc" must survive another session's end.
    assert_eq!(state(&home)["state"], "tool");
}

#[test]
fn statusline_records_five_hour_limit() {
    let home = tmp_home("statusline");
    run_hook(
        &home,
        "statusline",
        r#"{"rate_limits":{"five_hour":{"used_percentage":42.4,"resets_at":4102444800}}}"#,
        &[],
    );
    let raw = std::fs::read_to_string(home.join("limits.json")).expect("limits.json");
    let l: serde_json::Value = serde_json::from_str(&raw).unwrap();
    assert_eq!(l["fiveHour"]["usedPercentage"], 42.4);
    assert_eq!(l["fiveHour"]["resetsAt"], 4102444800i64);
    assert!(!home.join("state.json").exists(), "statusline must not touch pet state");
}

#[test]
fn statusline_without_rate_limits_keeps_previous_limits() {
    let home = tmp_home("statusline-absent");
    std::fs::write(home.join("limits.json"), r#"{"fiveHour":{"usedPercentage":7}}"#).unwrap();
    run_hook(&home, "statusline", r#"{"model":{"display_name":"Opus"}}"#, &[]);
    let raw = std::fs::read_to_string(home.join("limits.json")).unwrap();
    assert!(raw.contains("\"usedPercentage\":7"));
}

fn session(home: &Path, sid: &str) -> serde_json::Value {
    let raw = std::fs::read_to_string(home.join("sessions.d").join(sid)).expect("session record");
    serde_json::from_str(&raw).unwrap()
}

#[test]
fn session_record_is_json_with_owner_process() {
    let home = tmp_home("session-json");
    run_hook(&home, "start", r#"{"session_id":"s1"}"#, &[]);
    let rec = session(&home, "s1");
    assert!(rec["ts"].as_i64().unwrap() > 0);
    // The owner is only found when an ancestor is claude.exe (e.g. cargo run from
    // a Claude Code session); when present it carries the PID-reuse guard.
    assert_eq!(rec["claudePid"].is_null(), rec["claudeStart"].is_null());
}

#[test]
fn statusline_records_session_token_usage() {
    let home = tmp_home("statusline-usage");
    run_hook(&home, "start", r#"{"session_id":"s1"}"#, &[]);
    run_hook(
        &home,
        "statusline",
        r#"{"session_id":"s1","model":{"display_name":"Opus 5.5"},
            "context_window":{"total_input_tokens":12040,"total_output_tokens":500,"used_percentage":6}}"#,
        &[],
    );
    let rec = session(&home, "s1");
    assert_eq!(rec["model"], "Opus 5.5");
    assert_eq!(rec["tokens"], 12040.0); // input side only
    assert_eq!(rec["contextPct"], 6.0);
    // Before the first API response context_window is null: keep the last value.
    run_hook(&home, "statusline", r#"{"session_id":"s1","context_window":{"total_input_tokens":null}}"#, &[]);
    assert_eq!(session(&home, "s1")["tokens"], 12040.0);
}

#[test]
fn stop_failure_sets_error_state() {
    let home = tmp_home("fail");
    run_hook(&home, "prompt", r#"{"session_id":"s1"}"#, &[]);
    run_hook(&home, "fail", r#"{"session_id":"s1","error_type":"rate_limit"}"#, &[]);
    let s = state(&home);
    assert_eq!(s["state"], "error");
    assert_eq!(s["label"], "Error: rate_limit");
}

#[test]
fn transcript_fallback_fills_context_usage() {
    let home = tmp_home("transcript");
    let t = home.join("t.jsonl");
    let lines = [
        r#"{"type":"assistant","isSidechain":false,"message":{"model":"claude-opus-5-5","usage":{"input_tokens":2,"cache_creation_input_tokens":1000,"cache_read_input_tokens":299000,"output_tokens":50}}}"#,
        r#"{"type":"assistant","isSidechain":true,"message":{"model":"claude-haiku-4-5","usage":{"input_tokens":9,"cache_read_input_tokens":9}}}"#,
        r#"{"type":"user","message":{"content":"x"}}"#,
    ];
    std::fs::write(&t, lines.join("\n")).unwrap();
    let payload = serde_json::json!({"session_id": "s1", "tool_name": "Read", "transcript_path": t}).to_string();
    run_hook(&home, "post", &payload, &[]);
    let rec = session(&home, "s1");
    assert_eq!(rec["model"], "Opus 5.5");
    assert_eq!(rec["tokens"], 300002.0); // input side only; subagent line ignored
    assert_eq!(rec["contextSize"], 1000000.0);
    assert_eq!(rec["source"], "transcript");
}

#[test]
fn statusline_session_is_not_overwritten_by_transcript() {
    let home = tmp_home("statusline-wins");
    run_hook(&home, "statusline",
        r#"{"session_id":"s1","context_window":{"total_input_tokens":5000,"context_window_size":200000,"used_percentage":2.5}}"#, &[]);
    let t = home.join("t.jsonl");
    std::fs::write(&t, r#"{"type":"assistant","message":{"model":"claude-opus-5-5","usage":{"input_tokens":999999}}}"#).unwrap();
    let payload = serde_json::json!({"session_id": "s1", "transcript_path": t}).to_string();
    run_hook(&home, "post", &payload, &[]);
    assert_eq!(session(&home, "s1")["tokens"], 5000.0);
}
