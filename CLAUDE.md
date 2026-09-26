# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

Sidecrab is a Tauri 2 desktop pet (pixel crab) that reacts to Claude Code activity. Upstream (zvoque/sidecrab) is macOS-only and ships via `brew install zvoque/tap/sidecrab`. The local `windows` branch adds Windows support and a Claude Code plugin mode.

## Commands

All Rust commands run from `src-tauri/`, which is a Cargo workspace containing the app crate (`sidecrab`) and the hook crate (`hook/`, package `sidecrab-hook`).

```bash
cargo test --workspace                     # all tests
cargo test --test installer                # one integration test file (tests/installer.rs)
cargo test -p sidecrab-hook                # hook binary tests (hook/tests/hook.rs)
cargo test --test config round_trips       # filter by test name
cargo test --test sessions                 # session liveness / pruning
```

Build (the hook sidecar must exist before `tauri build`, because tauri-build validates `externalBin`):

```bash
npm run sidecar        # builds sidecrab-hook, copies it to src-tauri/binaries/sidecrab-hook-<host-triple>
npx tauri build        # macOS bundle (app/dmg)
npx tauri build --no-bundle   # Windows: plain exe, no installer
```

On Windows, `install-windows.ps1` does the whole thing: it builds both binaries, installs them to `%LOCALAPPDATA%\Programs\sidecrab` (added to the user PATH), writes `sidecrab-update.cmd`, and syncs the binaries into `plugin/bin`.

Frontend art iteration without Tauri: open `src/index.html` in a browser. Keys 1-6 cycle the states (`main.js` detects the missing `window.__TAURI__`).

## Architecture

There are two processes and they communicate only through files:

1. **`sidecrab-hook`** (`src-tauri/hook/src/main.rs`) is a standalone binary with no Tauri dependencies. Claude Code runs it for each hook event as `sidecrab-hook <prompt|pre|post|notify|permreq|stop|fail|start|end>`, with the event JSON on stdin. It atomically writes `state.json` and keeps `sessions.d/<session_id>` under `SIDECRAB_HOME` (default `dirs::config_dir()/sidecrab`, which is `%APPDATA%\sidecrab` on Windows). It lives in its own crate so it builds fast and avoids the externalBin chicken-and-egg.
2. **The app** (`src-tauri/src/`):
   - `state_watcher.rs` watches the state *directory*, not the file, because tmp+rename replaces the inode. It emits `claude-state` to the webview.
   - `lib.rs` owns the setup, the right-click settings menu, the consent dialog, and the click-through poller. The poller toggles `set_ignore_cursor_events` using the opaque sprite rect that the frontend pushes.
   - `os_actions.rs` holds the Tauri commands the webview invokes.
   - `hook_installer.rs` merges the hook entries into `~/.claude/settings.json`: backup once, additive, idempotent. Entries are identified by the `sidecrab-hook` marker substring.
   - `idle_monitor.rs` detects user idleness for wander mode.
3. **The frontend** (`src/`, plain ES modules, no bundler, `frontendDist: ../src`):
   - `state-machine.js` maps feed states (`idle|thinking|tool|permission|done`) to animations and runs the idle micro-life/sleep scheduler.
   - `behavior.js` runs wander and cursor chase.
   - `input.js` handles drag, double-click to activate the host app, and right-click to open the menu.
   - `sprites.js`/`frames.js` hold the pixel frames.

**Status panel.** A fixed-size, always-visible 3-row panel under the crab (`#status` in `index.html`, `src/status.js`; `STATUS_H` in `os_actions.rs` must match the CSS height): dot + activity/model + `tokens/window`, a context meter (`CTX 34k/200k 17%`), and a 5-hour meter with the local reset clock time (`5H 63% RESET 14:30`). Values show `--` until data arrives. Data is hybrid:
- Dot and activity come from hook events. Green means thinking/tool, yellow permission, red `error` (StopFailure) or 10 min without events while working, grey idle.
- Context tokens and model live in the session's `sessions.d` record. `sidecrab-hook statusline` fills it in terminal sessions (the `statusLine` command in `~/.claude/settings.json`; plugins cannot ship one). The desktop app never runs a statusLine, so hook events read the latest main-chain `usage` from `transcript_path`. That fallback is skipped once a statusLine has fed the session. Tokens are input-side only, matching `used_percentage`. The window size is 1M except Haiku (200K).
- The 5-hour limit is in `limits.json` (the live transport; watcher emits `claude-limits`). Writers, in order of precision: `usage_api.rs` (OAuth usage endpoint: fetched at startup, right after the window's reset time, on the "Refresh usage" menu item, else every 30 min; a 429 `Retry-After` is honoured and persisted in `usage_api.json`; on failure the last limits are kept and flagged `stale`; token read from `~/.claude/.credentials.json` per request and passed to curl via stdin), the terminal statusLine, and `usage_api::spawn_desktop_history`, which reads the desktop app's own samples (`%APPDATA%\Claude\plan-usage-history.json`, `{t, u:{fh, sd}}` every ~15 min): exact percentage, reset time estimated from the window's first sample (`estimated: true`, shown `~14:30`). Within a window whose reset time a precise source gave, the desktop samples only refresh the percentage.
- `usage_cache.rs` persists last-known values in `usage_cache.json` (fiveHourUsed/Remaining/ResetTime/Stale/Updated, contextUsed/Max/Percentage, model, lastUpdated), updated by the limits watcher and the sessions poller. It is a fast-start and fallback only, never the source of truth (5h usage also moves with other sessions/devices): `status_snapshot` returns it, `status.js` shows it instantly at startup and draws anything not confirmed by a live source dimmed with a trailing `?`. A cached 5h window past its reset time shows `--`.

The webview pulls `status_snapshot` once its listeners are attached, because events emitted before that are lost.

**Auto-hide.** `Config.auto_hide_status` (default on; menu item "Auto-hide status bar", event `status-autohide-changed`) makes `status.js` set `data-hidden` on `#status`: the panel slides down (`transform`, so layout and the window never change) and leaves a 4px handle tinted with the state colour. The panel is click-through, so hover comes from the Rust cursor poller in `lib.rs`, which emits `status-hover` while the cursor is in the bottom `STATUS_H` of the window (it polls every 60 ms and works while the panel is hidden). The panel hides 350 ms after the cursor leaves and peeks for 3 s at startup and when the setting is switched on. `config.json` is parsed with any UTF-8 BOM stripped: a rejected file would silently reset all settings.

**Sessions and lifecycle** (`sessions.rs`, `claude_proc.rs`). Each `sessions.d/<session_id>` record is JSON. The hook stamps it with the nearest `claude.exe` ancestor (pid + creation time, found through ToolHelp). `sessions.rs` polls every 2 s: records whose process is gone are deleted, because SessionEnd does not fire on terminal close or kill. It emits `claude-sessions`. When launched with `--plugin` (or when a later `--plugin` launch is forwarded by single-instance), the pet exits once a session has been seen and none remain. A manual launch without `--plugin` is not bound to Claude. `claude_proc.rs` is compiled into the hook crate via `#[path]`. On non-Windows there is no owner pid, so records live until SessionEnd.

`paths::home()` in the app and `home()` in the hook are intentionally duplicated. Keep them in sync.

The hook records `TERM_PROGRAM` as `host`. `activate_host` uses `host` on double-click to focus the Claude/terminal window: it runs `open -a` on macOS and PowerShell `AppActivate` on Windows.

## Platform notes (Windows branch)

- Platform code is split by `#[cfg(windows)]` / `cfg!(target_os = "macos")`: idle time comes from `GetLastInputInfo` (via `windows-sys`) and from `ioreg` on macOS. The app menu bar is set only on macOS, because on Windows it would attach to the crab window.
- The hook command path written to `settings.json` uses forward slashes and strips the `\\?\` verbatim prefix. Claude Code runs Windows hooks through Git Bash.
- Child processes (`curl`, `powershell`) use `CREATE_NO_WINDOW`, because the release exe uses the `windows` subsystem.
- `main.rs` re-spawns itself detached with `--foreground` and forwards the original args.

## Plugin mode

This repo is also a local Claude Code marketplace (`.claude-plugin/marketplace.json`, name `local`); the plugin `sidecrab@local` lives in `plugin/`. Its `hooks/hooks.json` calls `${CLAUDE_PLUGIN_ROOT}/bin/sidecrab-hook.exe`. Its `SessionStart` also launches `sidecrab.exe --plugin`. That sets `Config.plugin_managed`, which suppresses the consent dialog and the install/remove-hooks menu item. Do not also install hooks into `settings.json` while the plugin is enabled: every event would fire twice.

Claude Code caches plugins by version. After rebuilding, bump `version` in `plugin/.claude-plugin/plugin.json`, then run `claude plugin marketplace update local` and `claude plugin update sidecrab@local`.

The design spec is in `docs/superpowers/specs/2026-07-10-sidecrab-design.md` (state feed contract, animation states, window behavior, persistence).
