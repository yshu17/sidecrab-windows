//! Identity and liveness of the Claude Code process behind a session.
//! Shared by the hook binary (pulled in with `#[path]`, so that crate stays free
//! of tauri deps) and the app, so both agree on the (pid, start time) format.
//! Windows only for now; elsewhere liveness falls back to SessionEnd.
#![allow(dead_code, unused_imports)]

/// (pid, process creation time). The creation time guards against PID reuse.
pub type ProcId = (u32, u64);

#[cfg(windows)]
mod imp {
    use super::ProcId;
    use std::collections::HashMap;
    use windows_sys::Win32::Foundation::{
        CloseHandle, GetLastError, ERROR_ACCESS_DENIED, FILETIME, HANDLE, INVALID_HANDLE_VALUE,
        STILL_ACTIVE,
    };
    use windows_sys::Win32::System::Diagnostics::ToolHelp::{
        CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W,
        TH32CS_SNAPPROCESS,
    };
    use windows_sys::Win32::System::Threading::{
        GetExitCodeProcess, GetProcessTimes, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION,
    };

    /// Claude Code CLI binary. Hooks and the statusLine run as its descendants
    /// (claude.exe -> bash/cmd -> sidecrab-hook), in a terminal and in the desktop app.
    const CLAUDE_EXE: &str = "claude.exe";

    /// pid -> (parent pid, exe name) for every process on the system.
    fn process_table() -> Option<HashMap<u32, (u32, String)>> {
        unsafe {
            let snap = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0);
            if snap == INVALID_HANDLE_VALUE {
                return None;
            }
            let mut table = HashMap::new();
            let mut e: PROCESSENTRY32W = std::mem::zeroed();
            e.dwSize = std::mem::size_of::<PROCESSENTRY32W>() as u32;
            let mut ok = Process32FirstW(snap, &mut e) != 0;
            while ok {
                let len = e.szExeFile.iter().position(|&c| c == 0).unwrap_or(e.szExeFile.len());
                let name = String::from_utf16_lossy(&e.szExeFile[..len]);
                table.insert(e.th32ProcessID, (e.th32ParentProcessID, name));
                ok = Process32NextW(snap, &mut e) != 0;
            }
            CloseHandle(snap);
            Some(table)
        }
    }

    fn creation_time(h: HANDLE) -> Option<u64> {
        let zero = FILETIME { dwLowDateTime: 0, dwHighDateTime: 0 };
        let (mut created, mut exited, mut kernel, mut user) = (zero, zero, zero, zero);
        let ok = unsafe { GetProcessTimes(h, &mut created, &mut exited, &mut kernel, &mut user) };
        (ok != 0).then(|| (u64::from(created.dwHighDateTime) << 32) | u64::from(created.dwLowDateTime))
    }

    /// Nearest ancestor of this process that is the Claude Code CLI.
    pub fn claude_ancestor() -> Option<ProcId> {
        let table = process_table()?;
        let mut pid = std::process::id();
        for _ in 0..16 {
            let &(parent, _) = table.get(&pid)?;
            let (_, name) = table.get(&parent)?;
            if name.eq_ignore_ascii_case(CLAUDE_EXE) {
                let h = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, parent) };
                if h.is_null() {
                    return None;
                }
                let start = creation_time(h);
                unsafe { CloseHandle(h) };
                return Some((parent, start?));
            }
            pid = parent;
        }
        None
    }

    /// Any Claude process at all: the desktop app (Claude.exe) or a CLI.
    /// Errs on "running" if the process list can't be read.
    pub fn any_running() -> bool {
        any_named(CLAUDE_EXE)
    }

    /// Any process with this exe name (case-insensitive). Errs on "running"
    /// if the process list can't be read.
    pub fn any_named(exe: &str) -> bool {
        process_table().is_none_or(|t| t.values().any(|(_, n)| n.eq_ignore_ascii_case(exe)))
    }

    /// True while that exact process (same pid AND same creation time) runs.
    pub fn is_alive((pid, start): ProcId) -> bool {
        unsafe {
            let h = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
            if h.is_null() {
                // Exists but not ours to query (e.g. elevated): don't declare it dead.
                return GetLastError() == ERROR_ACCESS_DENIED;
            }
            let mut code = 0u32;
            let running = GetExitCodeProcess(h, &mut code) != 0 && code == STILL_ACTIVE as u32;
            let same = creation_time(h) == Some(start);
            CloseHandle(h);
            running && same
        }
    }
}

#[cfg(not(windows))]
mod imp {
    use super::ProcId;

    pub fn claude_ancestor() -> Option<ProcId> {
        None
    }

    pub fn is_alive(_: ProcId) -> bool {
        true
    }

    pub fn any_running() -> bool {
        true
    }
}

pub use imp::{any_running, claude_ancestor, is_alive};
#[cfg(windows)]
pub use imp::any_named;
