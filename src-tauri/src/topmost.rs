//! Keep the pet above ordinary windows.
//!
//! `alwaysOnTop` is applied once, at window creation. On Windows the pet can
//! still end up below normal windows later while keeping its WS_EX_TOPMOST
//! flag (seen with the Claude desktop app and Chrome windows stacked above
//! it). A light poll puts it back on top of the topmost band when that
//! happens. It reacts only to *non*-topmost windows above the pet, so it never
//! fights menus, overlays or other always-on-top apps, and it never takes focus.

use tauri::AppHandle;

/// A window stacked above the pet.
#[derive(Clone, Copy, Debug)]
pub struct Above {
    pub visible: bool,
    pub topmost: bool,
}

/// Raise the pet if any visible *non*-topmost window sits above it. Other
/// topmost windows (menus, overlays, always-on-top apps) are left alone.
pub fn needs_raise(above: impl IntoIterator<Item = Above>) -> bool {
    above.into_iter().any(|w| w.visible && !w.topmost)
}

#[cfg(windows)]
mod win {
    use super::{needs_raise, Above};
    use windows_sys::Win32::Foundation::HWND;
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        GetTopWindow, GetWindow, GetWindowLongW, IsWindowVisible, SetWindowPos, GWL_EXSTYLE,
        GW_HWNDNEXT, HWND_TOPMOST, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, WS_EX_TOPMOST,
    };

    /// Windows above `ours` in the z-order, topmost first.
    fn windows_above(ours: HWND) -> Vec<Above> {
        let mut out = Vec::new();
        unsafe {
            let mut h = GetTopWindow(std::ptr::null_mut());
            while !h.is_null() && h != ours {
                out.push(Above {
                    visible: IsWindowVisible(h) != 0,
                    topmost: GetWindowLongW(h, GWL_EXSTYLE) as u32 & WS_EX_TOPMOST != 0,
                });
                h = GetWindow(h, GW_HWNDNEXT);
            }
        }
        out
    }

    /// Put `hwnd` back on top of the topmost band (no focus change) if a
    /// normal window covers it. Returns true if it was raised.
    pub fn raise_if_covered(hwnd: HWND) -> bool {
        if !needs_raise(windows_above(hwnd)) {
            return false;
        }
        unsafe { SetWindowPos(hwnd, HWND_TOPMOST, 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE) };
        true
    }
}

#[cfg(windows)]
pub use win::raise_if_covered;

#[cfg(windows)]
pub fn spawn(app: AppHandle) {
    use tauri::Manager;
    std::thread::spawn(move || loop {
        std::thread::sleep(std::time::Duration::from_millis(500));
        let Some(win) = app.get_webview_window("main") else { continue };
        let Ok(h) = win.hwnd() else { continue };
        raise_if_covered(h.0 as _);
    });
}

#[cfg(not(windows))]
pub fn spawn(_app: AppHandle) {}
