pub mod claude_proc;
pub mod config;
pub mod debug_log;
pub mod hook_installer;
pub mod idle_monitor;
pub mod os_actions;
pub mod paths;
pub mod screen;
pub mod sessions;
pub mod state_watcher;
pub mod topmost;
pub mod updates;
pub mod usage_api;
pub mod usage_cache;

use std::sync::Mutex;
use tauri::menu::{CheckMenuItemBuilder, MenuBuilder, MenuItemBuilder, SubmenuBuilder};
use tauri::{AppHandle, Emitter, Manager, WebviewWindow};
use tauri_plugin_dialog::{DialogExt, MessageDialogButtons};

const CONSENT_TEXT: &str = "To react to Claude Code activity, Sidecrab adds hooks to \
~/.claude/settings.json.\n\nYour file is backed up to settings.json.bak first, existing \
hooks are kept untouched, and you can remove ours anytime via right-click → \
\"Remove Claude Code hooks\".\n\nEnable activity detection?";

/// First-run disclaimer. No settings.json edit happens without an explicit yes;
/// declining leaves the crab inert (idle only) until enabled from the menu.
fn maybe_ask_consent(app: &AppHandle) {
    let cfg = config::load();
    if cfg.consent_asked
        || cfg.plugin_managed
        || cfg.hooks_consent
        || hook_installer::hooks_installed(&paths::claude_settings_path())
    {
        return;
    }
    let handle = app.clone();
    app.dialog()
        .message(CONSENT_TEXT)
        .title("Sidecrab")
        .buttons(MessageDialogButtons::OkCancelCustom(
            "Enable".into(),
            "Not now".into(),
        ))
        .show(move |accepted| {
            let mut c = config::load();
            c.consent_asked = true;
            let _ = config::save(&c);
            if accepted {
                let _ = os_actions::hooks_install(handle);
            }
        });
}

/// Should a freshly started process become the running pet? `--plugin` alone
/// (SessionStart's re-arm) and `--quit` (`/pet off`) only mean something to an
/// instance that is already running, so on their own they exit at once.
/// `--show` (`/pet on`) and a bare launch always start; `--autostart`
/// (SessionStart) starts only while "Start with Claude Code" is on.
pub fn should_start(args: &[String], auto_start: bool) -> bool {
    let has = |f: &str| args.iter().any(|a| a == f);
    if has("--show") {
        return true;
    }
    if has("--quit") {
        return false;
    }
    if has("--autostart") {
        return auto_start;
    }
    !has("--plugin")
}

/// Opaque region of the sprite as fractions of the window (x0,y0,x1,y1), pushed by
/// the frontend. Used by the click-through poller — the crab is boxy, so a rect is
/// an accurate hit shape.
pub struct OpaqueRect(pub Mutex<(f64, f64, f64, f64)>);
/// While the user drags the crab we must not flip ignore_cursor_events mid-drag.
pub struct DragLock(pub Mutex<bool>);

#[tauri::command]
fn set_opaque_rect(state: tauri::State<OpaqueRect>, x0: f64, y0: f64, x1: f64, y1: f64) {
    *state.0.lock().unwrap() = (x0, y0, x1, y1);
}

#[tauri::command]
fn set_drag_lock(state: tauri::State<DragLock>, locked: bool) {
    *state.0.lock().unwrap() = locked;
}

/// The one settings menu, built fresh so checkmarks reflect current config.
/// Used as the right-click popup AND (wrapped) as the macOS app menu.
fn build_settings_menu(app: &AppHandle) -> Option<tauri::menu::Submenu<tauri::Wry>> {
    let win = app.get_webview_window("main")?;
    let cfg = config::load();
    let hooks_on = hook_installer::hooks_installed(&paths::claude_settings_path());
    let corner = os_actions::current_corner(&win);

    let size = SubmenuBuilder::new(app, "Size")
        .items(&[
            &CheckMenuItemBuilder::with_id("size-S", "Small").checked(cfg.size == "S").build(app).ok()?,
            &CheckMenuItemBuilder::with_id("size-M", "Medium").checked(cfg.size == "M").build(app).ok()?,
            &CheckMenuItemBuilder::with_id("size-L", "Large").checked(cfg.size == "L").build(app).ok()?,
        ])
        .build()
        .ok()?;

    let position = SubmenuBuilder::new(app, "Position")
        .items(&[
            &CheckMenuItemBuilder::with_id("pos-tl", "Top Left").checked(corner == Some("tl")).build(app).ok()?,
            &CheckMenuItemBuilder::with_id("pos-tr", "Top Right").checked(corner == Some("tr")).build(app).ok()?,
            &CheckMenuItemBuilder::with_id("pos-bl", "Bottom Left").checked(corner == Some("bl")).build(app).ok()?,
            &CheckMenuItemBuilder::with_id("pos-br", "Bottom Right").checked(corner == Some("br")).build(app).ok()?,
        ])
        .separator()
        .items(&[&MenuItemBuilder::with_id("pos-reset", "Reset Position").build(app).ok()?])
        .build()
        .ok()?;

    let hat = SubmenuBuilder::new(app, "Hat")
        .items(&[
            &CheckMenuItemBuilder::with_id("hat-none", "None").checked(cfg.hat == "none").build(app).ok()?,
            &CheckMenuItemBuilder::with_id("hat-top", "Top Hat").checked(cfg.hat == "top").build(app).ok()?,
            &CheckMenuItemBuilder::with_id("hat-chef", "Chef's Hat").checked(cfg.hat == "chef").build(app).ok()?,
            &CheckMenuItemBuilder::with_id("hat-fedora", "Fedora").checked(cfg.hat == "fedora").build(app).ok()?,
            &CheckMenuItemBuilder::with_id("hat-heli", "Helicopter Hat").checked(cfg.hat == "heli").build(app).ok()?,
        ])
        .build()
        .ok()?;

    let wander = CheckMenuItemBuilder::with_id("wander", "Wander when idle")
        .checked(cfg.wander_enabled)
        .build(app)
        .ok()?;

    let compact = CheckMenuItemBuilder::with_id("status-compact", "Compact status bar")
        .checked(cfg.compact_status)
        .build(app)
        .ok()?;

    let claude_start = CheckMenuItemBuilder::with_id("auto-start", "Start with Claude Code")
        .checked(cfg.auto_start)
        .build(app)
        .ok()?;

    let autostart_on = {
        use tauri_plugin_autostart::ManagerExt;
        app.autolaunch().is_enabled().unwrap_or(false)
    };
    let autostart = CheckMenuItemBuilder::with_id("autostart", "Launch at login")
        .checked(autostart_on)
        .build(app)
        .ok()?;

    let hooks = if hooks_on {
        MenuItemBuilder::with_id("hooks-remove", "Remove Claude Code hooks").build(app).ok()?
    } else {
        MenuItemBuilder::with_id("hooks-install", "Enable activity detection…").build(app).ok()?
    };

    let mut menu = SubmenuBuilder::new(app, "Sidecrab")
        .item(&size)
        .item(&position)
        .item(&hat)
        .item(&wander)
        .item(&compact)
        .item(&autostart)
        .separator();
    if cfg.plugin_managed {
        menu = menu.item(&claude_start).separator();
    }
    if !cfg.plugin_managed {
        menu = menu.item(&hooks).separator();
    }
    menu
        .items(&[
            &MenuItemBuilder::with_id("usage-refresh", "Refresh usage").build(app).ok()?,
            &MenuItemBuilder::with_id("update-check", "Check for Updates…").build(app).ok()?,
            &MenuItemBuilder::with_id("quit", "Quit Sidecrab")
                .accelerator("CmdOrCtrl+Q")
                .build(app)
                .ok()?,
        ])
        .build()
        .ok()
}

/// macOS menu bar: the settings live under the app-name menu (no tray icon).
pub(crate) fn refresh_app_menu(app: &AppHandle) {
    // Windows/Linux would attach this as a menu bar to the crab window itself.
    if !cfg!(target_os = "macos") {
        return;
    }
    if let Some(settings) = build_settings_menu(app) {
        if let Ok(menu) = MenuBuilder::new(app).item(&settings).build() {
            let _ = app.set_menu(menu);
        }
    }
}

#[tauri::command]
fn show_menu(window: WebviewWindow) {
    let Some(menu) = build_settings_menu(window.app_handle()) else { return };
    // Anchor high enough that the menu never opens past the screen bottom
    // (macOS renders a clipped, scroll-to-reveal menu otherwise).
    const MENU_H: f64 = 310.0; // generous logical estimate
    let y = match (window.current_monitor(), window.outer_position(), window.scale_factor()) {
        (Ok(Some(mon)), Ok(pos), Ok(scale)) => {
            let below = (mon.position().y + mon.size().height as i32 - pos.y) as f64 / scale;
            (below - MENU_H).min(0.0)
        }
        _ => -150.0,
    };
    let _ = window.popup_menu_at(&menu, tauri::Position::Logical(tauri::LogicalPosition::new(0.0, y)));
}

fn on_menu(app: &AppHandle, id: &str) {
    let win = app.get_webview_window("main");
    match id {
        "size-S" | "size-M" | "size-L" => {
            if let Some(w) = win {
                os_actions::resize_window(w, id.trim_start_matches("size-").to_string());
            }
        }
        // Corner placement becomes the new home; reset = default bottom-right.
        "pos-tl" | "pos-tr" | "pos-bl" | "pos-br" => {
            if let Some(w) = win {
                os_actions::place_corner(&w, id.trim_start_matches("pos-"), true);
            }
        }
        "pos-reset" => {
            if let Some(w) = win {
                os_actions::place_corner(&w, "br", true);
            }
        }
        "wander" => {
            let enabled = !config::load().wander_enabled;
            os_actions::set_wander(enabled);
            let _ = app.emit("wander-changed", enabled);
        }
        "status-compact" => {
            let mut c = config::load();
            c.compact_status = !c.compact_status;
            let _ = config::save(&c);
            let _ = app.emit("status-compact-changed", c.compact_status);
        }
        "auto-start" => {
            let mut c = config::load();
            c.auto_start = !c.auto_start;
            let _ = config::save(&c);
        }
        "autostart" => {
            use tauri_plugin_autostart::ManagerExt;
            let al = app.autolaunch();
            if al.is_enabled().unwrap_or(false) {
                let _ = al.disable();
            } else {
                let _ = al.enable();
            }
        }
        id if id.starts_with("hat-") => {
            let hat = id.trim_start_matches("hat-").to_string();
            let mut c = config::load();
            c.hat = hat.clone();
            let _ = config::save(&c);
            let _ = app.emit("hat-changed", hat);
        }
        // Menu action = explicit user intent = consent.
        "hooks-install" => {
            let _ = os_actions::hooks_install(app.clone());
        }
        "hooks-remove" => {
            let _ = os_actions::hooks_remove();
        }
        "usage-refresh" => usage_api::request_refresh(),
        "update-check" => updates::check(app.clone()),
        "quit" => app.exit(0),
        _ => {}
    }
    refresh_app_menu(app); // keep menu-bar checkmarks in sync with the change
}

/// Poll the global cursor; make empty window pixels click-through. When the cursor
/// is outside the sprite's opaque rect the window ignores mouse events, so clicks
/// land on whatever is underneath.
fn spawn_click_through_poller(app: AppHandle) {
    std::thread::spawn(move || {
        let mut ignoring = false;
        let mut hovering = false;
        let mut over_status_prev = false;
        loop {
            // Short enough that the auto-hiding status bar answers a hover at once.
            std::thread::sleep(std::time::Duration::from_millis(60));
            let Some(win) = app.get_webview_window("main") else { continue };
            if *app.state::<DragLock>().0.lock().unwrap() {
                continue;
            }
            let (Ok(cursor), Ok(pos), Ok(size)) =
                (app.cursor_position(), win.outer_position(), win.outer_size())
            else {
                continue;
            };
            let (lx, ly) = (cursor.x - pos.x as f64, cursor.y - pos.y as f64);
            let inside_window =
                lx >= 0.0 && ly >= 0.0 && lx < size.width as f64 && ly < size.height as f64;
            let over_crab = if inside_window {
                let (fx, fy) = (lx / size.width as f64, ly / size.height as f64);
                let (x0, y0, x1, y1) = *app.state::<OpaqueRect>().0.lock().unwrap();
                fx >= x0 && fx <= x1 && fy >= y0 && fy <= y1
            } else {
                false
            };
            // The same over_crab signal drives the hover reaction in the frontend.
            if over_crab != hovering {
                hovering = over_crab;
                let _ = app.emit("crab-hover", hovering);
            }
            // The status bar's strip (bottom STATUS_H of the window) is click-through,
            // so its hover comes from this poll too — it keeps working while the bar
            // is collapsed, and the frontend uses it to expand the mini plate.
            let strip = os_actions::STATUS_H * win.scale_factor().unwrap_or(1.0);
            let over_status = inside_window && ly >= size.height as f64 - strip;
            if over_status != over_status_prev {
                over_status_prev = over_status;
                let _ = app.emit("status-hover", over_status);
            }
            // Ignore events only while the cursor is over empty pixels of our window;
            // outside the window the flag is irrelevant, so reset it for safety.
            let want_ignore = inside_window && !over_crab;
            if want_ignore != ignoring {
                ignoring = want_ignore;
                let _ = win.set_ignore_cursor_events(ignoring);
            }
        }
    });
}

/// Pre-rename installs used the "clawd-pet" config dir; carry it over so
/// position/hat/consent survive the upgrade.
fn migrate_legacy_home() {
    let new = paths::home();
    if new.exists() {
        return;
    }
    if let Some(cfg_dir) = dirs::config_dir() {
        let old = cfg_dir.join("clawd-pet");
        if old.exists() {
            let _ = std::fs::rename(old, new);
        }
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    migrate_legacy_home();
    // Launched by the Claude Code plugin's SessionStart hook.
    if std::env::args().any(|a| a == "--plugin") {
        sessions::exit_with_claude();
        let mut c = config::load();
        if !c.plugin_managed {
            c.plugin_managed = true;
            let _ = config::save(&c);
        }
    }
    tauri::Builder::default()
        // Second launch = no twin crabs; the existing instance just stays.
        // A `--plugin` launch (SessionStart, or `/pet on`) re-arms the
        // Claude-bound lifecycle. `--show` (`/pet on`) raises the window.
        // `--quit` (`/pet off`) exits it. A fresh process is gated by
        // `should_start` in `setup()` below.
        .plugin(tauri_plugin_single_instance::init(|app, args, _cwd| {
            if args.iter().any(|a| a == "--plugin") {
                sessions::exit_with_claude();
            }
            if args.iter().any(|a| a == "--show") {
                os_actions::ensure_visible(app);
            }
            if args.iter().any(|a| a == "--quit") {
                app.exit(0);
            }
        }))
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            Some(vec!["--foreground"]), // agent child must not re-daemonize
        ))
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .manage(OpaqueRect(Mutex::new((0.0, 0.0, 1.0, 1.0))))
        .manage(DragLock(Mutex::new(false)))
        .invoke_handler(tauri::generate_handler![
            set_opaque_rect,
            set_drag_lock,
            show_menu,
            os_actions::set_window_pos,
            os_actions::get_geometry,
            os_actions::cursor_pos,
            os_actions::persist_position,
            os_actions::activate_host,
            os_actions::resize_window,
            os_actions::get_config,
            os_actions::set_wander,
            os_actions::hooks_install,
            os_actions::hooks_remove,
            os_actions::hooks_status,
            idle_monitor::user_is_idle,
            sessions::status_snapshot,
            usage_api::refresh_usage,
        ])
        .setup(|app| {
            // Reaching setup() at all means no other Sidecrab was found running:
            // the single-instance plugin above would have forwarded the args and
            // ended this process otherwise. `should_start` decides whether this
            // fresh process becomes the pet; if not, exit before any window is
            // shown or any background thread (usage API, watchers, pollers) starts.
            let args: Vec<String> = std::env::args().collect();
            if !should_start(&args, config::load().auto_start) {
                app.handle().exit(0);
                return Ok(());
            }
            // Diagnostic only (SIDECRAB_DEBUG=1): times the rest of this closure,
            // which runs on Tauri's own startup — never anything Claude Code's
            // hook runner waits on, since the SessionStart hook already returned
            // before this process was even fully created (main.rs's launcher
            // spawns detached), and by the point above this process is only ever
            // reached via a bare launch or an explicit /pet on.
            let debug_home = paths::home();
            let _t = debug_log::Timer::start(&debug_home, "app.setup");
            let win = app.get_webview_window("main").expect("main window");
            // Float above other apps on every Space.
            let _ = win.set_visible_on_all_workspaces(true);

            let cfg = config::load();
            let (lw, lh) = os_actions::logical_size(&cfg.size);
            let _ = win.set_size(tauri::LogicalSize::new(lw, lh));
            match cfg.position {
                Some((x, y)) => {
                    let _ = win.set_position(tauri::PhysicalPosition::new(x, y));
                    // The saved spot can predate the current monitor layout (a
                    // display unplugged since last run) — recover instead of
                    // starting up invisible.
                    if os_actions::is_stranded(&win) {
                        os_actions::recover_offscreen(&win);
                    }
                }
                None => {
                    // Default resting spot: bottom-right corner (not persisted — a
                    // saved home only comes from dragging or the Position menu).
                    os_actions::place_corner(&win, "br", false);
                }
            }

            app.on_menu_event(|app, event| on_menu(app, event.id().as_ref()));
            refresh_app_menu(app.handle()); // settings under the app-name menu too
            state_watcher::spawn(app.handle().clone());
            sessions::spawn(app.handle().clone());
            usage_api::spawn(app.handle().clone());
            usage_api::spawn_desktop_history();
            spawn_click_through_poller(app.handle().clone());
            topmost::spawn(app.handle().clone());
            idle_monitor::spawn(app.handle().clone());
            // Pre-rename hook entries point at a binary that no longer exists —
            // reinstall silently (consent was already given for those hooks).
            if hook_installer::has_legacy_hooks(&paths::claude_settings_path()) {
                let _ = os_actions::hooks_install(app.handle().clone());
            }
            maybe_ask_consent(app.handle());
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
