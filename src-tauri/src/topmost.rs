//! Keep the pet above ordinary windows.
//!
//! `alwaysOnTop` is applied once, at window creation. On Windows the pet can
//! still end up below normal windows later while keeping its WS_EX_TOPMOST
//! flag (seen with the Claude desktop app and Chrome windows stacked above
//! it). A light poll puts it back on top of the topmost band when that
//! happens. It reacts only to *non*-topmost windows above the pet, so it never
//! fights menus, overlays or other always-on-top apps, and it never takes focus.

use tauri::AppHandle;

#[cfg(windows)]
pub fn spawn(app: AppHandle) {
    use std::time::Duration;
    use tauri::Manager;
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        SetWindowPos, HWND_TOPMOST, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE,
    };

    std::thread::spawn(move || loop {
        std::thread::sleep(Duration::from_millis(500));
        let Some(win) = app.get_webview_window("main") else { continue };
        let Ok(h) = win.hwnd() else { continue };
        let hwnd = h.0 as windows_sys::Win32::Foundation::HWND;
        unsafe {
            if below_a_normal_window(hwnd) {
                SetWindowPos(hwnd, HWND_TOPMOST, 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE);
            }
        }
    });
}

/// True if any visible non-topmost window sits above `ours` in the z-order.
#[cfg(windows)]
unsafe fn below_a_normal_window(ours: windows_sys::Win32::Foundation::HWND) -> bool {
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        GetTopWindow, GetWindow, GetWindowLongW, IsWindowVisible, GWL_EXSTYLE, GW_HWNDNEXT,
        WS_EX_TOPMOST,
    };
    let mut h = GetTopWindow(std::ptr::null_mut());
    while !h.is_null() && h != ours {
        let topmost = GetWindowLongW(h, GWL_EXSTYLE) as u32 & WS_EX_TOPMOST != 0;
        if IsWindowVisible(h) != 0 && !topmost {
            return true;
        }
        h = GetWindow(h, GW_HWNDNEXT);
    }
    false
}

#[cfg(not(windows))]
pub fn spawn(_app: AppHandle) {}
