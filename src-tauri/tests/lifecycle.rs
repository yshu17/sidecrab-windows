// Which launches may turn a fresh process into the running pet. A launch that
// reaches an already running pet is forwarded by the single-instance plugin
// and never gets here, so this only decides "start or exit at once".
use sidecrab_lib::should_start;

fn args(a: &[&str]) -> Vec<String> {
    std::iter::once("sidecrab.exe").chain(a.iter().copied()).map(String::from).collect()
}

#[test]
fn session_start_launches_the_pet_while_auto_start_is_on() {
    assert!(should_start(&args(&["--plugin", "--autostart", "--foreground"]), true));
}

#[test]
fn session_start_does_nothing_when_auto_start_is_off() {
    assert!(!should_start(&args(&["--plugin", "--autostart", "--foreground"]), false));
}

#[test]
fn pet_on_always_starts() {
    assert!(should_start(&args(&["--plugin", "--show", "--foreground"]), false));
    assert!(should_start(&args(&["--plugin", "--show", "--foreground"]), true));
}

#[test]
fn pet_off_never_starts_a_pet_just_to_quit_it() {
    assert!(!should_start(&args(&["--quit", "--foreground"]), true));
}

#[test]
fn bare_plugin_rearm_never_starts() {
    // Older hooks.json (before auto-start) only re-armed a running pet.
    assert!(!should_start(&args(&["--plugin", "--foreground"]), true));
}

#[test]
fn manual_launch_starts() {
    assert!(should_start(&args(&["--foreground"]), false));
    assert!(should_start(&args(&[]), false));
}
