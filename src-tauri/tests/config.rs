// Config round-trip + defaults. Explicit-path variants are tested (the env-based
// wrappers just point at SIDECRAB_HOME/config.json).
use sidecrab_lib::config::{load_from, save_to, Config};

fn tmp(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("sidecrab-config-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir.join("config.json")
}

#[test]
fn round_trips() {
    let path = tmp("rt");
    let c = Config {
        position: Some((2200, 1300)),
        size: "L".into(),
        wander_enabled: true,
        hooks_consent: true,
        consent_asked: true,
        plugin_managed: true,
        hat: "fedora".into(),
        compact_status: false,
        auto_start: false,
    };
    save_to(&path, &c).unwrap();
    assert_eq!(load_from(&path), c);
}

#[test]
fn compact_defaults_on_and_survives_old_configs() {
    assert!(Config::default().compact_status);
    // A config.json written before the setting existed has no such key.
    let path = tmp("old");
    std::fs::write(&path, r#"{"size":"S","hat":"chef"}"#).unwrap();
    let c = load_from(&path);
    assert_eq!(c.size, "S");
    assert!(c.compact_status);
    // The previous "auto-hide" toggle carries over...
    std::fs::write(&path, r#"{"autoHideStatus":false}"#).unwrap();
    assert!(!load_from(&path).compact_status);
    // ...and is saved under the new name only.
    save_to(&path, &load_from(&path)).unwrap();
    let raw = std::fs::read_to_string(&path).unwrap();
    assert!(raw.contains("compactStatus") && !raw.contains("autoHideStatus"));
}

#[test]
fn utf8_bom_is_tolerated() {
    let path = tmp("bom");
    std::fs::write(&path, "\u{feff}{\"size\":\"L\",\"compactStatus\":false}").unwrap();
    let c = load_from(&path);
    assert_eq!(c.size, "L");
    assert!(!c.compact_status);
}

#[test]
fn missing_file_yields_defaults() {
    let path = tmp("missing");
    let c = load_from(&path);
    assert_eq!(c.position, None);
    assert_eq!(c.size, "M");
    assert!(!c.wander_enabled);
    assert!(!c.hooks_consent);
}

#[test]
fn corrupt_file_yields_defaults() {
    let path = tmp("corrupt");
    std::fs::write(&path, "{not json").unwrap();
    assert_eq!(load_from(&path), Config::default());
}

#[test]
fn auto_start_defaults_on_and_survives_old_configs() {
    assert!(Config::default().auto_start);
    // Configs written before the setting existed start the pet with Claude Code.
    let path = tmp("autostart");
    std::fs::write(&path, r#"{"size":"S","pluginManaged":true}"#).unwrap();
    assert!(load_from(&path).auto_start);
    // An explicit "off" from the menu is kept.
    std::fs::write(&path, r#"{"autoStart":false}"#).unwrap();
    assert!(!load_from(&path).auto_start);
}
