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

1. **`sidecrab-hook`** (`src-tauri/hook/src/main.rs`) is a standalone binary with no Tauri dependencies. Claude Code runs it for each hook event as `sidecrab-hook <prompt|pre|post|notify|permreq|stop|start|end>`, with the event JSON on stdin. It atomically writes `state.json` and keeps `sessions.d/<session_id>` under `SIDECRAB_HOME` (default `dirs::config_dir()/sidecrab`, which is `%APPDATA%\sidecrab` on Windows). It lives in its own crate so it builds fast and avoids the externalBin chicken-and-egg.
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

**Status label.** A strip under the crab (`src/status.js`, `#status`, `STATUS_H` in `os_actions.rs`, which must match the CSS height) shows the hook's `label` plus the 5-hour rate limit. Limits come from `sidecrab-hook statusline`, configured as the Claude Code `statusLine` command in `~/.claude/settings.json` (plugins cannot ship a main statusLine). That command writes `limits.json`, which `state_watcher.rs` forwards as `claude-limits`. `rate_limits` exists only for Pro/Max, after the first API response.

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
