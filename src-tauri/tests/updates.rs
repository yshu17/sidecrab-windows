// The update check compares the latest GitHub release of this fork against the
// running build. Tag names are free-form ("Alpha0.1", "v0.1.18"), so the
// version is the first dotted number in the tag.
use serde_json::json;
use sidecrab_lib::updates::{is_newer, latest_release, parse_version, REPO};

#[test]
fn versions_are_read_from_free_form_tags() {
    assert_eq!(parse_version("v0.1.18"), Some(vec![0, 1, 18]));
    assert_eq!(parse_version("Alpha0.1"), Some(vec![0, 1]));
    assert_eq!(parse_version("0.2.0"), Some(vec![0, 2, 0]));
    assert_eq!(parse_version("release-1.4 (windows)"), Some(vec![1, 4]));
    assert_eq!(parse_version("nightly"), None);
}

#[test]
fn only_a_strictly_newer_release_counts() {
    let v = |s| parse_version(s).unwrap();
    assert!(is_newer(&v("0.1.18"), &v("0.1.17")));
    assert!(is_newer(&v("0.2"), &v("0.1.17")));
    assert!(is_newer(&v("1.0.0"), &v("0.9.9")));
    assert!(!is_newer(&v("0.1.17"), &v("0.1.17")));
    assert!(!is_newer(&v("0.1"), &v("0.1.0")), "missing parts count as zero");
    assert!(!is_newer(&v("Alpha0.1"), &v("0.1.17")), "an older release is not an update");
}

#[test]
fn latest_release_parsed_from_github_response() {
    let body = json!({
        "tag_name": "v0.1.18",
        "name": "0.1.18",
        "html_url": "https://github.com/yshu17/sidecrab-windows/releases/tag/v0.1.18",
        "draft": false
    });
    let r = latest_release(&body).unwrap();
    assert_eq!(r.tag, "v0.1.18");
    assert_eq!(r.version, vec![0, 1, 18]);
    assert!(r.url.starts_with(&format!("https://github.com/{REPO}/")));
}

#[test]
fn unusable_responses_are_rejected() {
    // GitHub error body (rate limit, no releases yet) or a tag without a version
    assert!(latest_release(&json!({"message": "Not Found"})).is_none());
    assert!(latest_release(&json!({"tag_name": "nightly", "html_url": "https://x"})).is_none());
    assert!(latest_release(&json!({"tag_name": "v1.0", "html_url": "https://evil.example/x"})).is_none());
}

#[test]
fn check_points_at_this_fork() {
    assert_eq!(REPO, "yshu17/sidecrab-windows");
}

/// Network: `cargo test --test updates -- --ignored`. Proves the real
/// releases/latest response of this repo still parses.
#[test]
#[ignore]
fn live_github_latest_release_parses() {
    let out = std::process::Command::new("curl")
        .args(["-s", "--max-time", "15", &format!("https://api.github.com/repos/{REPO}/releases/latest")])
        .output()
        .expect("curl");
    let body: serde_json::Value = serde_json::from_slice(&out.stdout).expect("json");
    let r = latest_release(&body).unwrap_or_else(|| panic!("unparseable release: {body}"));
    println!("latest release: {} -> {:?} ({})", r.tag, r.version, r.url);
}

/// One version for the whole thing: the app (what the update check compares),
/// both crates and the Claude Code plugin must agree, or the check lies.
#[test]
fn app_crates_and_plugin_share_one_version() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let read = |p: &str| std::fs::read_to_string(root.join(p)).unwrap();
    let json_version = |p: &str| {
        serde_json::from_str::<serde_json::Value>(&read(p)).unwrap()["version"].as_str().unwrap().to_string()
    };
    let toml_version = |p: &str| {
        read(p)
            .lines()
            .find_map(|l| l.strip_prefix("version = \"").map(|v| v.trim_end_matches('"').to_string()))
            .unwrap()
    };
    let app = json_version("tauri.conf.json");
    assert_eq!(toml_version("Cargo.toml"), app, "src-tauri/Cargo.toml");
    assert_eq!(toml_version("hook/Cargo.toml"), app, "src-tauri/hook/Cargo.toml");
    assert_eq!(json_version("../plugin/.claude-plugin/plugin.json"), app, "plugin/.claude-plugin/plugin.json");
}
