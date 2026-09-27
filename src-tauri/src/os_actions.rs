//! Tauri commands the webview invokes: window control, host activation, config.

use crate::config::{self, Config};
use tauri::{AppHandle, Manager, WebviewWindow};

// Window logical sizes per setting. Height keeps the sprite's 51:48 aspect so the
// crab is never distorted.
const SIZES: [(&str, f64, f64); 3] = [
    ("S", 102.0, 96.0),
    ("M", 153.0, 144.0),
    ("L", 204.0, 192.0),
];

/// Logical height of the status label strip under the crab (see styles.css).
pub const STATUS_H: f64 = 42.0;

pub fn logical_size(size: &str) -> (f64, f64) {
    let (w, h) = SIZES
        .iter()
        .find(|(s, _, _)| *s == size)
        .map(|&(_, w, h)| (w, h))
        .unwrap_or((153.0, 144.0));
    (w, h + STATUS_H)
}

/// Physical position of a monitor corner ("tl"|"tr"|"bl"|"br") for the current
/// window size (computed from config — outer_size may be stale mid-resize).
pub fn corner_pos(window: &WebviewWindow, corner: &str) -> Option<(i32, i32)> {
    let mon = window.current_monitor().ok()??;
    let m = mon.size();
    let mp = mon.position();
    let scale = mon.scale_factor();
    let (lw, lh) = logical_size(&config::load().size);
    let (w, h) = ((lw * scale).round() as i32, (lh * scale).round() as i32);
    let margin = (24.0 * scale) as i32;
    let x = match corner {
        "tl" | "bl" => mp.x + margin,
        _ => mp.x + m.width as i32 - w - margin,
    };
    let y = match corner {
        "tl" | "tr" => mp.y + margin,
        _ => mp.y + m.height as i32 - h - margin,
    };
    Some((x, y))
}

/// Which corner the crab's HOME currently is, if any (drag spots return None).
pub fn current_corner(window: &WebviewWindow) -> Option<&'static str> {
    let home = config::load()
        .position
        .or_else(|| window.outer_position().ok().map(|p| (p.x, p.y)))?;
    ["tl", "tr", "bl", "br"].into_iter().find(|c| {
        corner_pos(window, c)
            .is_some_and(|(x, y)| (x - home.0).abs() <= 10 && (y - home.1).abs() <= 10)
    })
}

/// Place the window in a corner; when `persist` is set the spot becomes home.
pub fn place_corner(window: &WebviewWindow, corner: &str, persist: bool) {
    let Some((x, y)) = corner_pos(window, corner) else { return };
    let _ = window.set_position(tauri::PhysicalPosition::new(x, y));
    if persist {
        let mut c = config::load();
        c.position = Some((x, y));
        let _ = config::save(&c);
    }
}


/// Absolute placement (wander walking / teleport home).
#[tauri::command]
pub fn set_window_pos(window: WebviewWindow, x: i32, y: i32) {
    let _ = window.set_position(tauri::PhysicalPosition::new(x, y));
}

/// Global cursor position (physical px) — the mad-chase target.
#[tauri::command]
pub fn cursor_pos(app: AppHandle) -> Option<(f64, f64)> {
    app.cursor_position().ok().map(|p| (p.x, p.y))
}

/// Window + current-monitor rects in physical px, for wander pathing bounds.
#[tauri::command]
pub fn get_geometry(window: WebviewWindow) -> Option<serde_json::Value> {
    let pos = window.outer_position().ok()?;
    let size = window.outer_size().ok()?;
    let mon = window.current_monitor().ok()??;
    let mp = mon.position();
    let ms = mon.size();
    Some(serde_json::json!({
        "winX": pos.x, "winY": pos.y,
        "winW": size.width, "winH": size.height,
        "monX": mp.x, "monY": mp.y,
        "monW": ms.width, "monH": ms.height,
    }))
}

/// Persist the current window position as home (called on drag end).
#[tauri::command]
pub fn persist_position(window: WebviewWindow) {
    if let Ok(pos) = window.outer_position() {
        let mut c = config::load();
        c.position = Some((pos.x, pos.y));
        let _ = config::save(&c);
        crate::refresh_app_menu(window.app_handle()); // corner checkmarks may change
    }
}

/// Bring the app hosting the active Claude Code session to the front.
/// `host` comes from TERM_PROGRAM recorded by the hook; "Claude" = desktop app.
#[tauri::command]
pub fn activate_host(host: String) {
    let app_name = match host.as_str() {
        "" | "Claude" => "Claude",
        "Apple_Terminal" => "Terminal",
        "iTerm.app" => "iTerm",
        "vscode" => "Visual Studio Code",
        "ghostty" => "Ghostty",
        other => other, // best-effort: try the raw TERM_PROGRAM value
    };
    #[cfg(not(windows))]
    let _ = std::process::Command::new("open")
        .args(["-a", app_name])
        .spawn();
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        let title = match app_name {
            "Claude" => "Claude",
            "Visual Studio Code" => "Visual Studio Code",
            _ => "Terminal",
        };
        let script = format!(
            "(New-Object -ComObject WScript.Shell).AppActivate('{title}') | Out-Null"
        );
        let _ = std::process::Command::new("powershell")
            .args(["-NoProfile", "-NonInteractive", "-Command", &script])
            .creation_flags(CREATE_NO_WINDOW)
            .spawn();
    }
}

#[tauri::command]
pub fn resize_window(window: WebviewWindow, size: String) {
    let (lw, lh) = logical_size(&size);
    // Anchor the bottom-right corner. Compute the new physical size instead of
    // querying outer_size() after set_size — the query returns the STALE size
    // (resize applies asynchronously), which made S/L drift.
    let before = (window.outer_position().ok(), window.outer_size().ok());
    let scale = window.scale_factor().unwrap_or(2.0);
    let _ = window.set_size(tauri::LogicalSize::new(lw, lh));
    let mut c = config::load();
    if let (Some(pos), Some(old)) = before {
        let (new_w, new_h) = ((lw * scale).round() as i32, (lh * scale).round() as i32);
        let (x, y) = (
            pos.x + old.width as i32 - new_w,
            pos.y + old.height as i32 - new_h,
        );
        let _ = window.set_position(tauri::PhysicalPosition::new(x, y));
        // Update the persisted HOME only if the crab was actually at home when
        // resized. Resizing a displaced (wandering/stranded) crab must not turn
        // its current spot into the new home — only dragging redefines home.
        let at_home = c
            .position
            .map(|(hx, hy)| (hx - pos.x).abs() <= 12 && (hy - pos.y).abs() <= 12)
            .unwrap_or(true);
        if at_home {
            c.position = Some((x, y));
        }
    }
    c.size = size;
    let _ = config::save(&c);
}

/// `/pet on` support (a `sidecrab.exe --show` launch, forwarded here by
/// tauri-plugin-single-instance when Sidecrab is already running instead of
/// starting a second process): raise and show the window, unconditionally —
/// never hides it, unlike a toggle, so calling `/pet on` twice can't
/// accidentally turn the pet off. Never steals focus (`focus: false` in
/// tauri.conf.json is deliberate for a pet that sits over whatever the user is
/// doing), and touches nothing else — position, size, hat, status bar, hooks
/// and session state are all untouched. When Sidecrab wasn't running at all,
/// this never runs: the freshly created window is visible by default.
pub fn ensure_visible(app: &AppHandle) {
    let Some(win) = app.get_webview_window("main") else { return };
    let _ = win.unminimize();
    let _ = win.show();
}

#[tauri::command]
pub fn get_config() -> Config {
    config::load()
}

#[tauri::command]
pub fn set_wander(enabled: bool) {
    let mut c = config::load();
    c.wander_enabled = enabled;
    let _ = config::save(&c);
}

/// Consent-gated hook management (consent recorded by the dialog flow).
#[tauri::command]
pub fn hooks_install(app: AppHandle) -> Result<(), String> {
    let bin = hook_bin_path(&app).ok_or("hook binary not found")?;
    crate::hook_installer::install_hooks(&crate::paths::claude_settings_path(), &bin)
        .map_err(|e| e.to_string())?;
    let mut c = config::load();
    c.hooks_consent = true;
    let _ = config::save(&c);
    Ok(())
}

#[tauri::command]
pub fn hooks_remove() -> Result<(), String> {
    crate::hook_installer::remove_hooks(&crate::paths::claude_settings_path())
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn hooks_status() -> bool {
    crate::hook_installer::hooks_installed(&crate::paths::claude_settings_path())
}

/// The bundled sidecrab-hook sits next to the app executable.
fn hook_bin_path(app: &AppHandle) -> Option<String> {
    let exe = tauri::process::current_binary(&app.env()).ok()?;
    let name = if cfg!(windows) { "sidecrab-hook.exe" } else { "sidecrab-hook" };
    let p = exe.parent()?.join(name).to_string_lossy().into_owned();
    // Claude Code runs hooks through Git Bash on Windows: forward slashes are
    // safe there and in cmd, backslashes are not.
    // current_binary() may return a verbatim `\\?\C:\...` path; strip the prefix.
    Some(if cfg!(windows) {
        p.trim_start_matches(r"\\?\").replace('\\', "/")
    } else {
        p
    })
}

