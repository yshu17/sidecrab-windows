//! "Check for Updates…": compares the latest GitHub release of this fork with
//! the running build and offers the release page. Nothing is downloaded.

use serde_json::Value;
use tauri::AppHandle;
use tauri_plugin_dialog::{DialogExt, MessageDialogButtons};
use tauri_plugin_opener::OpenerExt;

pub const REPO: &str = "whorlyknows/sidecrab-windows";

#[derive(Debug)]
pub struct Release {
    pub tag: String,
    pub version: Vec<u32>,
    pub url: String,
}

/// First dotted number in a free-form tag: "Alpha0.1" -> [0, 1], "v0.1.18" -> [0, 1, 18].
pub fn parse_version(tag: &str) -> Option<Vec<u32>> {
    let start = tag.find(|c: char| c.is_ascii_digit())?;
    let run: String = tag[start..].chars().take_while(|c| c.is_ascii_digit() || *c == '.').collect();
    let parts: Vec<u32> = run.split('.').filter(|s| !s.is_empty()).map(|s| s.parse().ok()).collect::<Option<_>>()?;
    (!parts.is_empty()).then_some(parts)
}

/// Strictly newer; missing trailing parts count as 0 (0.1 == 0.1.0).
pub fn is_newer(latest: &[u32], current: &[u32]) -> bool {
    let n = latest.len().max(current.len());
    let at = |v: &[u32], i| v.get(i).copied().unwrap_or(0);
    for i in 0..n {
        if at(latest, i) != at(current, i) {
            return at(latest, i) > at(current, i);
        }
    }
    false
}

/// `GET /repos/{REPO}/releases/latest` body -> Release. Rejects error bodies,
/// version-less tags and any page URL outside this repo (it gets opened).
pub fn latest_release(body: &Value) -> Option<Release> {
    let tag = body["tag_name"].as_str()?.to_string();
    let url = body["html_url"].as_str()?.to_string();
    if !url.starts_with(&format!("https://github.com/{REPO}/")) {
        return None;
    }
    Some(Release { version: parse_version(&tag)?, tag, url })
}

fn fetch_latest() -> Option<Release> {
    let mut cmd = std::process::Command::new("curl");
    cmd.args([
        "-s",
        "--max-time",
        "10",
        "-H",
        "Accept: application/vnd.github+json",
        &format!("https://api.github.com/repos/{REPO}/releases/latest"),
    ]);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    let out = cmd.output().ok()?;
    latest_release(&serde_json::from_slice(&out.stdout).ok()?)
}

pub fn check(app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        let current = app.package_info().version.to_string();
        let releases = format!("https://github.com/{REPO}/releases");
        let Some(latest) = fetch_latest() else {
            app.dialog()
                .message(format!("Couldn't reach GitHub to check for updates.\n\nReleases: {releases}"))
                .title("Update check failed")
                .show(|_| {});
            return;
        };
        let cur = parse_version(&current).unwrap_or_default();
        if !is_newer(&latest.version, &cur) {
            app.dialog()
                .message(format!("You're on the latest version ({current})."))
                .title("No updates")
                .show(|_| {});
            return;
        }
        let url = latest.url.clone();
        let opener = app.clone();
        app.dialog()
            .message(format!(
                "{} is available (you have {current}).\n\nOpen the release page to download it?\n\
                 Built from source? Run `sidecrab-update` instead.",
                latest.tag
            ))
            .title("Update available")
            .buttons(MessageDialogButtons::OkCancelCustom("Open release".into(), "Later".into()))
            .show(move |open| {
                if open {
                    let _ = opener.opener().open_url(url, None::<&str>);
                }
            });
    });
}
