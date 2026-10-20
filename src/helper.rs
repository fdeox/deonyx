// Running as administrator, Deonyx darkens normal apps from a second copy of
// itself that runs without those rights, the "helper".
//
// A layer sits on its window as that window's owned window, which ties the
// two together for input. If the layer belongs to an administrator program,
// Windows then throws away every key and click that normal programs send
// (AutoHotkey, mouse tools, remote control) while that app is in front. The
// helper's layers have the same rights as the apps under them, so nothing is
// lost. The main Deonyx keeps the tray, the settings, the hotkeys and the
// apps that run as administrator; it sends the helper the settings and the
// helper tells it what it darkens. Both talk with WM_COPYDATA.

use std::ffi::c_void;
use std::os::windows::io::IntoRawHandle;
use std::os::windows::process::CommandExt;
use std::path::Path;

use serde::Serialize;
use windows::core::{HSTRING, PCWSTR, PWSTR};
use windows::Win32::Foundation::*;
use windows::Win32::Security::*;
use windows::Win32::System::DataExchange::COPYDATASTRUCT;
use windows::Win32::System::JobObjects::*;
use windows::Win32::System::Threading::*;
use windows::Win32::UI::WindowsAndMessaging::*;

/// The settings, main -> helper.
pub const CONFIG: usize = 0x4458_4346;
/// What the helper darkens, helper -> main.
pub const STATUS: usize = 0x4458_5354;

pub struct Helper {
    pub pid: u32,
    process: HANDLE,
    job: Option<HANDLE>,
    /// The helper's window, once it has said hello.
    pub window: Option<HWND>,
}

impl Helper {
    /// Starts the helper. `parent` is the window it reports to.
    pub fn spawn(parent: HWND) -> Result<Helper, String> {
        let exe = std::env::current_exe().map_err(|e| e.to_string())?;
        let mut args = format!("--helper --parent {} --window {}", std::process::id(), parent.0 as isize);
        if let Some(d) = std::env::var_os("DEONYX_CONFIG_DIR") {
            args += &format!(" --config-dir \"{}\"", d.to_string_lossy());
        }
        let (process, pid) = if crate::admin::elevated() {
            unsafe { start_as_user(&exe, &args)? }
        } else {
            // Tests (DEONYX_HELPER_TEST) run it without any rights to drop.
            let child = std::process::Command::new(&exe).raw_arg(&args).spawn().map_err(|e| e.to_string())?;
            let pid = child.id();
            (HANDLE(child.into_raw_handle()), pid)
        };
        let job = unsafe { kill_with_us(process) };
        Ok(Helper { pid, process, job, window: None })
    }

    pub fn alive(&self) -> bool {
        unsafe { WaitForSingleObject(self.process, 0) == WAIT_TIMEOUT }
    }
}

impl Drop for Helper {
    fn drop(&mut self) {
        unsafe {
            // Closing the job ends the helper; without one, end it ourselves.
            match self.job.take() {
                Some(job) => {
                    let _ = CloseHandle(job);
                }
                None => {
                    let _ = TerminateProcess(self.process, 0);
                }
            }
            let _ = CloseHandle(self.process);
        }
    }
}

/// Starts the exe as the signed-in user without administrator rights, with
/// Explorer's own token.
unsafe fn start_as_user(exe: &Path, args: &str) -> Result<(HANDLE, u32), String> {
    let shell = GetShellWindow();
    if shell.is_invalid() {
        return Err("no shell window".into());
    }
    let mut pid = 0u32;
    GetWindowThreadProcessId(shell, Some(&mut pid));
    let process = OpenProcess(PROCESS_QUERY_INFORMATION, false, pid).map_err(|e| e.to_string())?;
    let mut token = HANDLE::default();
    let opened = OpenProcessToken(process, TOKEN_DUPLICATE, &mut token);
    let _ = CloseHandle(process);
    opened.map_err(|e| e.to_string())?;
    let mut primary = HANDLE::default();
    let access = TOKEN_QUERY | TOKEN_DUPLICATE | TOKEN_ASSIGN_PRIMARY | TOKEN_ADJUST_DEFAULT | TOKEN_ADJUST_SESSIONID;
    let duplicated = DuplicateTokenEx(token, access, None, SecurityImpersonation, TokenPrimary, &mut primary);
    let _ = CloseHandle(token);
    duplicated.map_err(|e| e.to_string())?;

    let app = HSTRING::from(exe.as_os_str());
    let mut line: Vec<u16> = format!("\"{}\" {}", exe.display(), args).encode_utf16().chain(Some(0)).collect();
    let si = STARTUPINFOW { cb: size_of::<STARTUPINFOW>() as u32, ..Default::default() };
    let mut pi = PROCESS_INFORMATION::default();
    let started = CreateProcessWithTokenW(
        primary,
        CREATE_PROCESS_LOGON_FLAGS(0),
        PCWSTR(app.as_ptr()),
        Some(PWSTR(line.as_mut_ptr())),
        PROCESS_CREATION_FLAGS(0),
        None,
        PCWSTR::null(),
        &si,
        &mut pi,
    );
    let _ = CloseHandle(primary);
    started.map_err(|e| e.to_string())?;
    let _ = CloseHandle(pi.hThread);
    Ok((pi.hProcess, pi.dwProcessId))
}

/// A job that ends the helper when we end, even if we crash.
unsafe fn kill_with_us(process: HANDLE) -> Option<HANDLE> {
    let job = CreateJobObjectW(None, PCWSTR::null()).ok()?;
    let mut info = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
    info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
    let set = SetInformationJobObject(
        job,
        JobObjectExtendedLimitInformation,
        &info as *const _ as *const c_void,
        size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
    );
    if set.is_err() || AssignProcessToJobObject(job, process).is_err() {
        let _ = CloseHandle(job);
        return None;
    }
    Some(job)
}

/// Sends `value` as JSON to the other copy's window.
pub fn send<T: Serialize>(to: HWND, from: Option<HWND>, kind: usize, value: &T) -> bool {
    let Ok(bytes) = serde_json::to_vec(value) else { return false };
    let data = COPYDATASTRUCT { dwData: kind, cbData: bytes.len() as u32, lpData: bytes.as_ptr() as *mut c_void };
    let from = from.map(|h| h.0 as usize).unwrap_or(0);
    unsafe {
        SendMessageTimeoutW(to, WM_COPYDATA, WPARAM(from), LPARAM(&data as *const _ as isize), SMTO_ABORTIFHUNG, 1000, None).0 != 0
    }
}

/// What a WM_COPYDATA brought: its kind, the sender's process and the bytes.
pub unsafe fn received(w: WPARAM, l: LPARAM) -> Option<(usize, u32, Vec<u8>)> {
    let data = (l.0 as *const COPYDATASTRUCT).as_ref()?;
    if data.lpData.is_null() || data.cbData == 0 {
        return None;
    }
    let bytes = std::slice::from_raw_parts(data.lpData as *const u8, data.cbData as usize).to_vec();
    let mut pid = 0u32;
    GetWindowThreadProcessId(HWND(w.0 as *mut c_void), Some(&mut pid));
    Some((data.dwData, pid, bytes))
}

/// The helper's main thread: waits until the main Deonyx is gone.
pub fn wait_for_parent(pid: u32, window: HWND) {
    unsafe {
        let parent = OpenProcess(PROCESS_SYNCHRONIZE, false, pid).ok();
        loop {
            match parent {
                Some(p) if WaitForSingleObject(p, 1000) != WAIT_TIMEOUT => break,
                Some(_) => {}
                None => std::thread::sleep(std::time::Duration::from_secs(1)),
            }
            if !IsWindow(Some(window)).as_bool() {
                break;
            }
        }
        if let Some(p) = parent {
            let _ = CloseHandle(p);
        }
    }
}
