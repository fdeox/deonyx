// Small Win32 helpers shared by the rest of the app.

use std::ffi::c_void;
use std::mem::size_of;

use windows::core::{BOOL, PWSTR};
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Dwm::{DwmGetWindowAttribute, DWMWA_CLOAKED, DWMWA_EXTENDED_FRAME_BOUNDS};
use windows::Win32::Security::{
    GetSidSubAuthority, GetSidSubAuthorityCount, GetTokenInformation, TokenElevation, TokenIntegrityLevel, TOKEN_ELEVATION,
    TOKEN_MANDATORY_LABEL, TOKEN_QUERY,
};
use windows::Win32::System::Threading::*;
use windows::Win32::UI::WindowsAndMessaging::*;

pub fn exe_name(pid: u32) -> Option<String> {
    unsafe {
        let p = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid).ok()?;
        let mut buf = [0u16; 520];
        let mut len = buf.len() as u32;
        let ok = QueryFullProcessImageNameW(p, PROCESS_NAME_WIN32, PWSTR(buf.as_mut_ptr()), &mut len);
        let _ = CloseHandle(p);
        ok.ok()?;
        let path = String::from_utf16_lossy(&buf[..len as usize]);
        path.rsplit('\\').next().map(str::to_string)
    }
}

/// Whether a process runs as administrator. When we can't even read its
/// token, it runs above us, which counts the same.
pub fn is_elevated(pid: u32) -> bool {
    unsafe {
        let Ok(p) = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) else { return false };
        let mut tok = HANDLE::default();
        let elevated = if OpenProcessToken(p, TOKEN_QUERY, &mut tok).is_ok() {
            let mut e = TOKEN_ELEVATION::default();
            let mut len = 0u32;
            let ok = GetTokenInformation(
                tok,
                TokenElevation,
                Some(&mut e as *mut TOKEN_ELEVATION as *mut c_void),
                size_of::<TOKEN_ELEVATION>() as u32,
                &mut len,
            )
            .is_ok();
            let _ = CloseHandle(tok);
            ok && e.TokenIsElevated != 0
        } else {
            true
        };
        let _ = CloseHandle(p);
        elevated
    }
}

pub const LEVEL_MEDIUM: u32 = 0x2000;
pub const LEVEL_SYSTEM: u32 = 0x4000;

/// Windows integrity level of a process: 0x2000 normal, 0x3000 administrator,
/// 0x4000 system. Windows lets no window be drawn over by a process with a
/// lower level. A token we can't read belongs to something above us.
pub fn integrity(pid: u32) -> u32 {
    unsafe {
        let Ok(p) = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) else { return u32::MAX };
        let mut tok = HANDLE::default();
        let mut level = u32::MAX;
        if OpenProcessToken(p, TOKEN_QUERY, &mut tok).is_ok() {
            let mut buf = [0u8; 128];
            let mut len = 0u32;
            if GetTokenInformation(
                tok,
                TokenIntegrityLevel,
                Some(buf.as_mut_ptr() as *mut c_void),
                buf.len() as u32,
                &mut len,
            )
            .is_ok()
            {
                let label = &*(buf.as_ptr() as *const TOKEN_MANDATORY_LABEL);
                let sid = label.Label.Sid;
                let n = *GetSidSubAuthorityCount(sid);
                if n > 0 {
                    level = *GetSidSubAuthority(sid, (n - 1) as u32);
                }
            }
            let _ = CloseHandle(tok);
        }
        let _ = CloseHandle(p);
        level
    }
}

pub fn window_pid(h: HWND) -> u32 {
    let mut pid = 0u32;
    unsafe { GetWindowThreadProcessId(h, Some(&mut pid)) };
    pid
}

/// The visible frame, without the invisible resize borders.
pub fn frame_rect(h: HWND) -> RECT {
    let mut r = RECT::default();
    unsafe {
        let got = DwmGetWindowAttribute(
            h,
            DWMWA_EXTENDED_FRAME_BOUNDS,
            &mut r as *mut RECT as *mut c_void,
            size_of::<RECT>() as u32,
        );
        if got.is_err() {
            let _ = GetWindowRect(h, &mut r);
        }
    }
    r
}

pub fn class_name(h: HWND) -> String {
    let mut buf = [0u16; 256];
    let n = unsafe { GetClassNameW(h, &mut buf) };
    String::from_utf16_lossy(&buf[..n.max(0) as usize])
}

pub fn is_cloaked(h: HWND) -> bool {
    let mut v = 0u32;
    unsafe {
        DwmGetWindowAttribute(h, DWMWA_CLOAKED, &mut v as *mut u32 as *mut c_void, size_of::<u32>() as u32).is_ok()
            && v != 0
    }
}

pub fn is_top_level(h: HWND) -> bool {
    unsafe { GetAncestor(h, GA_PARENT) == GetDesktopWindow() }
}

pub fn width(r: &RECT) -> i32 {
    r.right - r.left
}

pub fn height(r: &RECT) -> i32 {
    r.bottom - r.top
}

unsafe extern "system" fn collect_cb(h: HWND, l: LPARAM) -> BOOL {
    let v = &mut *(l.0 as *mut Vec<HWND>);
    v.push(h);
    TRUE
}

/// Every top-level window, front to back.
pub fn top_level_windows() -> Vec<HWND> {
    let mut v: Vec<HWND> = Vec::new();
    unsafe {
        let _ = EnumWindows(Some(collect_cb), LPARAM(&mut v as *mut Vec<HWND> as isize));
    }
    v
}

/// The language Windows shows itself in, if Deonyx speaks it, else English.
pub fn system_language() -> &'static str {
    let id = unsafe { windows::Win32::Globalization::GetUserDefaultUILanguage() };
    match id & 0x3ff {
        0x1f => "tr",
        0x07 => "de",
        0x0c => "fr",
        0x0a => "es",
        0x10 => "it",
        0x16 => "pt",
        0x19 => "ru",
        0x22 => "uk",
        0x13 => "nl",
        0x11 => "ja",
        0x12 => "ko",
        0x04 => "zh",
        _ => "en",
    }
}

/// Windows is set to dark mode for apps.
pub fn apps_use_dark_theme() -> bool {
    use windows::core::w;
    use windows::Win32::System::Registry::{RegGetValueW, HKEY_CURRENT_USER, RRF_RT_REG_DWORD};
    let mut v = 1u32;
    let mut len = 4u32;
    let r = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            w!(r"Software\Microsoft\Windows\CurrentVersion\Themes\Personalize"),
            w!("AppsUseLightTheme"),
            RRF_RT_REG_DWORD,
            None,
            Some(&mut v as *mut u32 as *mut c_void),
            Some(&mut len),
        )
    };
    r == ERROR_SUCCESS && v == 0
}

/// Minutes since local midnight.
pub fn local_minutes() -> u32 {
    let t = unsafe { windows::Win32::System::SystemInformation::GetLocalTime() };
    t.wHour as u32 * 60 + t.wMinute as u32
}

/// The same with the seconds, for changes that run over many minutes.
pub fn local_minutes_exact() -> f64 {
    let t = unsafe { windows::Win32::System::SystemInformation::GetLocalTime() };
    t.wHour as f64 * 60.0 + t.wMinute as f64 + t.wSecond as f64 / 60.0
}

/// A window without a title bar that covers its whole screen: a game or a
/// video played full screen.
pub fn is_fullscreen(h: HWND) -> bool {
    use windows::Win32::Graphics::Gdi::{GetMonitorInfoW, MonitorFromWindow, MONITORINFO, MONITOR_DEFAULTTONEAREST};
    unsafe {
        let style = GetWindowLongW(h, GWL_STYLE) as u32;
        if style & WS_CAPTION.0 == WS_CAPTION.0 {
            return false;
        }
        let mon = MonitorFromWindow(h, MONITOR_DEFAULTTONEAREST);
        let mut mi = MONITORINFO { cbSize: size_of::<MONITORINFO>() as u32, ..Default::default() };
        if !GetMonitorInfoW(mon, &mut mi).as_bool() {
            return false;
        }
        let mut r = RECT::default();
        let _ = GetWindowRect(h, &mut r);
        r.left <= mi.rcMonitor.left && r.top <= mi.rcMonitor.top && r.right >= mi.rcMonitor.right && r.bottom >= mi.rcMonitor.bottom
    }
}
