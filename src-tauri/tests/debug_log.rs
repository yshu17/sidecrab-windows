// SIDECRAB_DEBUG gate: off by default (no file, no cost beyond one env read),
// on writes timestamped lines to <home>/debug.log — never to stdout, since hook
// stdout can become Claude Code hook "additionalContext". One test (not two):
// SIDECRAB_DEBUG is process-global and cargo runs tests in one binary concurrently.
use sidecrab_lib::debug_log::{log, Timer};

fn tmp(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("sidecrab-debuglog-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn off_by_default_then_enabled_logs_timestamped_lines() {
    std::env::remove_var("SIDECRAB_DEBUG");
    let off = tmp("off");
    log(&off, "hello");
    {
        let _t = Timer::start(&off, "span");
    }
    assert!(!off.join("debug.log").exists());

    let on = tmp("on");
    std::env::set_var("SIDECRAB_DEBUG", "1");
    {
        let _t = Timer::start(&on, "hook.prompt");
    }
    log(&on, "hook.prompt outcome=ok");
    std::env::remove_var("SIDECRAB_DEBUG");

    let content = std::fs::read_to_string(on.join("debug.log")).unwrap();
    let lines: Vec<&str> = content.lines().collect();
    assert_eq!(lines.len(), 3);
    for l in &lines {
        assert!(l.starts_with('['), "line should start with a [timestamp]: {l}");
        assert!(l.contains("pid="));
    }
    assert!(lines[0].contains("hook.prompt start"));
    assert!(lines[1].contains("hook.prompt end elapsed_ms="));
    assert!(lines[2].contains("hook.prompt outcome=ok"));
}
