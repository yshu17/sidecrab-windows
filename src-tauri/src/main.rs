// Prevents an extra console window on Windows in release. DO NOT REMOVE!!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

/// Terminal-friendly launch: typing the command detaches the pet and returns
/// the shell immediately. `--foreground` (or the daemon-child env marker)
/// runs attached — used for debugging and by launchd/brew services.
fn main() {
    // Diagnostic only (SIDECRAB_DEBUG=1): this is the exact process Claude Code's
    // SessionStart hook runs and waits on. It must return in well under a second —
    // the `spawn()` below hands the real work to a detached child and returns
    // immediately, so nothing here talks to Claude Code, the network, or waits on
    // the child. If SessionStart ever looks slow, this line's elapsed_ms is the
    // first thing to check (see debug_log.rs for how to turn it on).
    let debug_home = sidecrab_lib::paths::home();
    let _t = sidecrab_lib::debug_log::Timer::start(&debug_home, "launcher.main");
    let foreground = std::env::args().any(|a| a == "--foreground")
        || std::env::var_os("SIDECRAB_CHILD").is_some();
    if !foreground {
        if let Ok(exe) = std::env::current_exe() {
            let ok = std::process::Command::new(exe)
                .args(std::env::args().skip(1))
                .arg("--foreground")
                .env("SIDECRAB_CHILD", "1")
                .stdin(std::process::Stdio::null())
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .spawn()
                .is_ok();
            if ok {
                println!("pet is running (single instance enforced)");
                return;
            }
        }
        // Spawn failed — fall through and run attached rather than not at all.
    }
    sidecrab_lib::run()
}
