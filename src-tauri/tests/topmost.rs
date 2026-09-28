// Keeping the pet on top: the rule (pure) and, on Windows, the real z-order.
use sidecrab_lib::topmost::{needs_raise, Above};

const NORMAL: Above = Above { visible: true, topmost: false };
const TOPMOST: Above = Above { visible: true, topmost: true };
const HIDDEN: Above = Above { visible: false, topmost: false };

#[test]
fn raises_only_when_a_normal_window_covers_the_pet() {
    assert!(!needs_raise([]), "nothing above: already on top");
    assert!(!needs_raise([TOPMOST, TOPMOST]), "taskbar / menus / overlays above are fine");
    assert!(!needs_raise([HIDDEN, TOPMOST]), "invisible windows don't count");
    assert!(needs_raise([TOPMOST, NORMAL]), "a normal window above the pet: raise");
    assert!(needs_raise([NORMAL]));
}

/// Real windows: a topmost "pet" pushed under a normal window is put back on top,
/// without disturbing a pet that is already on top.
#[cfg(windows)]
#[test]
fn pushed_under_a_normal_window_the_pet_is_raised_again() {
    use sidecrab_lib::topmost::raise_if_covered;
    use windows_sys::Win32::Foundation::HWND;
    use windows_sys::Win32::UI::WindowsAndMessaging::*;

    let class: Vec<u16> = "SidecrabTopmostTest\0".encode_utf16().collect();
    unsafe {
        let wc = WNDCLASSW {
            lpfnWndProc: Some(DefWindowProcW),
            lpszClassName: class.as_ptr(),
            ..std::mem::zeroed()
        };
        RegisterClassW(&wc);
        // Far off-screen, tool windows: visible for the z-order, never on the taskbar.
        let make = |ex: u32| -> HWND {
            CreateWindowExW(
                ex | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE,
                class.as_ptr(),
                class.as_ptr(),
                WS_POPUP | WS_VISIBLE,
                -30000,
                -30000,
                8,
                8,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null(),
            )
        };
        let pet = make(WS_EX_TOPMOST);
        let normal = make(0);
        assert!(!pet.is_null() && !normal.is_null());
        let is_topmost = |h: HWND| GetWindowLongW(h, GWL_EXSTYLE) as u32 & WS_EX_TOPMOST != 0;
        let above = |a: HWND, b: HWND| {
            let mut h = GetTopWindow(std::ptr::null_mut());
            while !h.is_null() {
                if h == a {
                    return true;
                }
                if h == b {
                    return false;
                }
                h = GetWindow(h, GW_HWNDNEXT);
            }
            false
        };

        assert!(is_topmost(pet) && above(pet, normal));
        assert!(!raise_if_covered(pet), "already on top: must not touch it");

        // What Windows / another app does: slot the pet below a normal window.
        SetWindowPos(pet, normal, 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE);
        assert!(above(normal, pet), "setup: pet is now under the normal window");

        assert!(raise_if_covered(pet), "covered: must raise");
        assert!(is_topmost(pet), "topmost flag restored");
        assert!(above(pet, normal), "pet is above the normal window again");

        DestroyWindow(normal);
        DestroyWindow(pet);
    }
}

/// The exit watch's "is Claude running" probe finds processes by exe name.
#[cfg(windows)]
#[test]
fn process_probe_finds_running_exes_by_name() {
    use sidecrab_lib::claude_proc::any_named;
    let me = std::env::current_exe().unwrap();
    let me = me.file_name().unwrap().to_str().unwrap();
    assert!(any_named(me), "this test binary is running");
    assert!(any_named(&me.to_uppercase()), "case-insensitive");
    assert!(!any_named("no-such-process-sidecrab-test.exe"));
}
