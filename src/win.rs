// Small Win32 helpers shared by the rest of the app.

use std::ffi::c_void;
use std::mem::size_of;

use windows::core::{BOOL, PWSTR};
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Dwm::{DwmGetWindowAttribute, DWMWA_CLOAKED, DWMWA_EXTENDED_FRAME_BOUNDS};
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
