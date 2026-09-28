# CLAUDE.md

Sidecrab: a Tauri 2 desktop pet (pixel crab) that reacts to Claude Code activity. Upstream `zvoque/sidecrab` is macOS-only (brew). This is a GitHub fork (`yshu17/sidecrab-windows`, public) of `whorlyknows/sidecrab-windows`; its default branch is `windows` (the fork's `main` stays the upstream snapshot), and it adds Windows support and a Claude Code plugin mode.

## Commands

Rust runs from `src-tauri/` (Cargo workspace: app crate `sidecrab` + hook crate `hook/`, package `sidecrab-hook`).

```bash
cargo test --workspace                          # everything
cargo test --test sessions                      # one file in tests/ (also: topmost, updates, installer, config, ...)
cargo test -p sidecrab-hook                     # hook tests (hook/tests/hook.rs)
cargo test --test updates -- --ignored          # live GitHub release check (network)
```

Windows build + install: `install-windows.ps1` builds both binaries, installs them to `%LOCALAPPDATA%\Programs\sidecrab` (on PATH), writes `sidecrab-update.cmd` (fast-forward `git pull` if the branch tracks a remote, then rerun the script), copies them into `plugin/bin`, and runs `claude plugin marketplace update local` + `claude plugin update sidecrab@local` when `claude` is on PATH.
macOS/manual: `npm run sidecar` (hook must exist before `tauri build` — tauri-build validates `externalBin`), then `npx tauri build` (`--no-bundle` on Windows).

Frontend without Tauri: open `src/index.html` in a browser; keys 1-6 cycle states.

## Versions, releases, update check

- One version everywhere: `src-tauri/tauri.conf.json` (the version the app reports), `src-tauri/Cargo.toml`, `src-tauri/hook/Cargo.toml`, `plugin/.claude-plugin/plugin.json`. `tests/updates.rs` fails if they drift. Bump all four together — Claude Code caches plugins by version and ignores rebuilt binaries otherwise.
- "Check for Updates…" (`updates.rs`) reads `GET /repos/yshu17/sidecrab-windows/releases/latest`, takes the first dotted number in `tag_name` (`Alpha0.1` → 0.1, `v0.1.18` → 0.1.18), and offers the release page only if it is strictly newer. Tag new releases with the version (e.g. `v0.1.18`), or older-numbered tags will never show as updates. Nothing is downloaded in-app. The API is read without login, so the check only works while the repository is public; a private repo answers 404 ("Couldn't reach GitHub").

## Architecture

Two processes, talking only through files under `SIDECRAB_HOME` (default `%APPDATA%\sidecrab` / `dirs::config_dir()/sidecrab`):

1. **`sidecrab-hook`** (`hook/src/main.rs`, no Tauri deps): Claude Code runs `sidecrab-hook <prompt|pre|post|notify|permreq|stop|fail|start|end|statusline>` with event JSON on stdin. Writes `state.json` atomically and `sessions.d/<session_id>`. Local disk I/O only, never network.
2. **App** (`src/*.rs`): `state_watcher.rs` watches the state *directory* (tmp+rename replaces the inode) and emits `claude-state`; `lib.rs` has setup, right-click menu, consent dialog and the 60 ms cursor poller (click-through via the opaque sprite rect, `crab-hover`, `status-hover`); `os_actions.rs` holds webview commands; `hook_installer.rs` merges hooks into `~/.claude/settings.json` (backup once, additive, idempotent, marker `sidecrab-hook`); `idle_monitor.rs` idleness for wander; `topmost.rs` keeps the pet on top (below).
3. **Frontend** (`src/`, plain ES modules, no bundler): `state-machine.js` maps `idle|thinking|tool|permission|done` to animations and runs micro-life/sleep; `sprites.js` draws `frames.js` (29 frames, 51×36, base64 PNG) on a canvas; `behavior.js` wander/chase; `input.js` drag, double-click, right-click; `status.js` the status panel. Rust never draws — it only sends events.

**Rendering is on demand** (`sprites.js`): the loop redraws only when a render key changes (anim, step, facing, hat/rotor phase, blink, thought phase, canvas size, image ready) and sleeps until the next due change (`_nextDelay`) instead of running `requestAnimationFrame` every refresh; `play`/`setFacing`/`setHat`/`setThought`/resize/image load call `_kick()` to render at once. Anything new that changes the picture must be in the key and wake the loop. (Continuous rAF cost ~30% of a core in WebView2; now ~6% total.)

**Security:** strict CSP in `tauri.conf.json` (self scripts/styles, `data:` images for the frames, Tauri IPC only). The UI writes text only via `textContent` — keep it that way, hook data (tool/model names) is untrusted. The hook reduces `session_id` to `[A-Za-z0-9_-]` before using it as a file name.

**Launcher handles:** `main.rs` clears HANDLE_FLAG_INHERIT on its own std handles before spawning the detached pet; otherwise the pet inherits the caller's stdout pipe and anything reading the launcher's output to EOF waits for the pet's whole lifetime.

`paths::home()` (app) and `home()` (hook) are duplicated on purpose; keep them in sync. `claude_proc.rs` and `debug_log.rs` are shared into the hook crate via `#[path]`.

**Status panel** (`#status`, `status.js`; `STATUS_H` in `os_actions.rs` must match the CSS height): activity dot + model + tokens, context meter, 5-hour meter with local reset time; `--` until data arrives.
- Activity: hook events (green thinking/tool, yellow permission, red error or 10 min silent while working, grey idle).
- Context/model: the session's `sessions.d` record — from `sidecrab-hook statusline` in terminals, else from the latest `usage` in `transcript_path` (desktop app has no statusLine). Input-side tokens; window 1M except Haiku (200K).
- 5-hour limit: `limits.json` (watcher emits `claude-limits`). Sources by precision: `usage_api.rs` (OAuth endpoint; at startup, after reset, on "Refresh usage", else every 30 min; honours and persists 429 `Retry-After`; keeps last value flagged `stale` on failure; token from `~/.claude/.credentials.json`, passed to curl via stdin), the terminal statusLine, and the desktop app's `%APPDATA%\Claude\plan-usage-history.json` (exact %, reset time estimated, shown `~14:30`).
- `usage_cache.rs` → `usage_cache.json`: fast start/fallback only, never truth; unconfirmed values are drawn dimmed with `?`. The webview pulls `status_snapshot` after attaching listeners (earlier events are lost).
- Compact mode (`Config.compact_status`, default on; old key `autoHideStatus` accepted): collapses to a mini plate that grows from the centre on hover. Hover comes from the Rust poller because the panel is click-through. `config.json` is read with a UTF-8 BOM stripped (a rejected file resets all settings).

**Sessions and lifecycle** (`sessions.rs`, `claude_proc.rs`): the hook stamps each record with its nearest `claude.exe` ancestor (pid + creation time via ToolHelp, once per session). A 2 s poller deletes records whose process is gone (SessionEnd doesn't fire on kill/close) and emits `claude-sessions`. A `--plugin` launch arms `ExitWatch`: quit only after Claude was seen and then stayed gone for 15 s. On Windows "gone" = no `claude.exe` process at all (desktop app or CLI), **not** an empty `sessions.d` — the desktop app restarts its per-session `claude.exe --resume=<id>` on its own, which prunes records while Claude is still open. Elsewhere: no live records. A manual launch never quits with Claude.

**Always on top** (`topmost.rs`, Windows): `alwaysOnTop` is applied only at creation, and Windows can later stack normal windows (Claude desktop app, Chrome) above the pet while it keeps WS_EX_TOPMOST. Every 500 ms, if a visible *non-topmost* window is above it, `SetWindowPos(HWND_TOPMOST, SWP_NOACTIVATE)`. Other topmost windows (menus, overlays, always-on-top apps) are ignored so it never fights them or steals focus.

**Recovering from an invalid monitor** (`screen.rs`, `os_actions::is_stranded`/`recover_offscreen`): a saved position can predate the current display layout — most often a monitor that was connected when it was last saved is gone on wake from sleep (undocked laptop, unplugged projector). `screen::is_onscreen` (pure, unit-tested) checks the window rect against `available_monitors()`; the same 500 ms Windows poll that reasserts topmost also checks this first and, if the window isn't sufficiently on any monitor, snaps it to the bottom-right corner of the nearest one and re-persists that as home — leaving the stale one in place would just have the wander ticker's "walk home" drag it back off-screen. `setup()` runs the same check once before showing the window, so a stale saved position doesn't start the pet invisible either.

## Windows notes

- Idle time: `GetLastInputInfo` (macOS: `ioreg`). App menu bar only on macOS.
- Hook paths in `settings.json` use forward slashes without the `\\?\` prefix; Claude Code runs Windows hooks through Git Bash.
- Child processes (`curl`, `powershell`) use `CREATE_NO_WINDOW` (release exe is `windows` subsystem).
- `main.rs` re-spawns itself detached with `--foreground`, forwarding args.

## Plugin mode

This repo is a local marketplace (`.claude-plugin/marketplace.json`, name `local`); plugin `sidecrab@local` is `plugin/`, its hooks call `${CLAUDE_PLUGIN_ROOT}/bin/sidecrab-hook.exe`. Don't also install hooks into `settings.json` while the plugin is on (events fire twice).

**Off by default.** Only `/pet on` / `/sidecrab on` (`plugin/commands/`) start it; `/pet off` quits it fully (no threads, no usage calls); bare `/pet` reports ON/OFF via `tasklist` (no persisted flag). `SessionStart` runs `sidecrab-hook.exe start` plus a `tasklist`-guarded `sidecrab.exe --plugin`, so nothing is spawned while off.

CLI flags, handled in the `tauri_plugin_single_instance` callback (only in the already-running instance, so never a duplicate):
- `--show`: raise/show the window (not a toggle).
- `--quit`: exit.
- `--plugin`: arm the quit-with-Claude watch; sets `Config.plugin_managed` (no consent dialog, no hooks menu item).

`--plugin`/`--quit` reaching a *fresh* process (pet was off) exit at the top of `.setup()` before any window or thread starts — only a bare launch or `--show` really starts Sidecrab.

## Hook latency (Claude Code must never wait on us)

- `SessionStart`'s `sidecrab.exe --plugin` only decides foreground/background, spawns a detached child with null stdio and returns; all window/network work happens later in that child's background threads.
- The hook does local disk I/O only; `transcript_usage` reads the last 512 KB; `claude_ancestor()` runs once per session.
- `usage_api::fetch` (`curl --max-time 15`) only runs on background threads.
- If a session still stalls at open, suspect antivirus scanning of freshly rebuilt unsigned exes (exclusion for `%LOCALAPPDATA%\Programs\sidecrab` is a manual, user-approved step) or Git Bash startup per hook.
- Diagnostics: `SIDECRAB_DEBUG=1` → timestamped `<home>/debug.log` (never stdout — hook stdout becomes Claude Code context) for `launcher.main`, `hook.<event>`, `app.setup`, `usage_api.fetch`.

## Elsewhere

- `future/new-crab/`: planned redesigned pet (not wired in). Turn frames in `turn/`, rebuild GIFs/sheet with `py build_turn.py`; plan in its README.
- Design spec: `docs/superpowers/specs/2026-07-10-sidecrab-design.md`.
