// Programs with open windows, for the "Add an app" list: file name, the
// name the program gives itself, a window title and its icon.

use std::collections::HashSet;
use std::ffi::c_void;
use std::mem::size_of;

use serde::Serialize;
use windows::core::{w, HSTRING, PWSTR};
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::Storage::FileSystem::{GetFileVersionInfoSizeW, GetFileVersionInfoW, VerQueryValueW};
use windows::Win32::System::Threading::*;
use windows::Win32::UI::Shell::ExtractIconExW;
use windows::Win32::UI::WindowsAndMessaging::*;

use crate::win::{is_cloaked, is_top_level, top_level_windows, window_pid};

#[derive(Clone, Serialize)]
pub struct Running {
    pub exe: String,
    pub name: String,
    pub title: String,
    pub icon: Option<String>,
}

/// Windows' own shell parts, which have nothing to darken.
const SKIP: &[&str] = &[
    "explorer.exe",
    "applicationframehost.exe",
    "textinputhost.exe",
    "searchhost.exe",
    "shellexperiencehost.exe",
    "startmenuexperiencehost.exe",
    "systemsettings.exe",
    "deonyx.exe",
];

fn exe_path(pid: u32) -> Option<String> {
    unsafe {
        let p = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid).ok()?;
        let mut buf = [0u16; 1024];
        let mut len = buf.len() as u32;
        let ok = QueryFullProcessImageNameW(p, PROCESS_NAME_WIN32, PWSTR(buf.as_mut_ptr()), &mut len);
        let _ = CloseHandle(p);
        ok.ok()?;
        Some(String::from_utf16_lossy(&buf[..len as usize]))
    }
}

fn title(h: HWND) -> String {
    let mut buf = [0u16; 512];
    let n = unsafe { GetWindowTextW(h, &mut buf) };
    String::from_utf16_lossy(&buf[..n.max(0) as usize])
}

pub fn running() -> Vec<Running> {
    let me = std::process::id();
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for h in top_level_windows() {
        unsafe {
            if !IsWindowVisible(h).as_bool() || !is_top_level(h) || is_cloaked(h) || GetWindow(h, GW_OWNER).is_ok() {
                continue;
            }
            if GetWindowLongW(h, GWL_EXSTYLE) as u32 & WS_EX_TOOLWINDOW.0 != 0 {
                continue;
            }
        }
        let t = title(h);
        let pid = window_pid(h);
        if t.trim().is_empty() || pid == me {
            continue;
        }
        let Some(path) = exe_path(pid) else { continue };
        let mut r = describe(&path);
        let key = r.exe.to_ascii_lowercase();
        if SKIP.contains(&key.as_str()) || !seen.insert(key) {
            continue;
        }
        r.title = t;
        out.push(r);
    }
    out.sort_by_key(|r| r.name.to_lowercase());
    out
}

/// Name and icon of a program file.
pub fn describe(path: &str) -> Running {
    let exe = path.rsplit(['\\', '/']).next().unwrap_or(path).to_string();
    let stem = exe.rsplit_once('.').map(|(s, _)| s).unwrap_or(&exe).to_string();
    // "SketchUp Application" reads better as "SketchUp".
    let name = file_description(path)
        .map(|d| d.strip_suffix(" Application").map(str::to_string).unwrap_or(d))
        .unwrap_or(stem);
    Running { exe, name, title: String::new(), icon: icon_data_url(path) }
}

fn file_description(path: &str) -> Option<String> {
    unsafe {
        let wpath = HSTRING::from(path);
        let size = GetFileVersionInfoSizeW(&wpath, None);
        if size == 0 {
            return None;
        }
        let mut buf = vec![0u8; size as usize];
        GetFileVersionInfoW(&wpath, None, size, buf.as_mut_ptr() as *mut c_void).ok()?;
        let mut p: *mut c_void = std::ptr::null_mut();
        let mut len = 0u32;
        if !VerQueryValueW(buf.as_ptr() as *const c_void, w!("\\VarFileInfo\\Translation"), &mut p, &mut len).as_bool()
            || len < 4
        {
            return None;
        }
        let lang = *(p as *const u16);
        let cp = *(p as *const u16).add(1);
        let q = HSTRING::from(format!("\\StringFileInfo\\{lang:04x}{cp:04x}\\FileDescription"));
        if !VerQueryValueW(buf.as_ptr() as *const c_void, &q, &mut p, &mut len).as_bool() || len == 0 {
            return None;
        }
        let s = String::from_utf16_lossy(std::slice::from_raw_parts(p as *const u16, len as usize));
        let s = s.trim_end_matches('\0').trim().to_string();
        (!s.is_empty()).then_some(s)
    }
}

fn icon_data_url(path: &str) -> Option<String> {
    unsafe {
        let mut large = HICON::default();
        let n = ExtractIconExW(&HSTRING::from(path), 0, Some(&mut large), None, 1);
        if n == 0 || large.is_invalid() {
            return None;
        }
        let png = icon_png(large, 32);
        let _ = DestroyIcon(large);
        png.map(|b| format!("data:image/png;base64,{}", base64(&b)))
    }
}

fn icon_png(icon: HICON, size: i32) -> Option<Vec<u8>> {
    unsafe {
        let dc = CreateCompatibleDC(None);
        let bmi = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: size,
                biHeight: -size,
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                ..Default::default()
            },
            ..Default::default()
        };
        let mut bits: *mut c_void = std::ptr::null_mut();
        let Ok(bmp) = CreateDIBSection(Some(dc), &bmi, DIB_RGB_COLORS, &mut bits, None, 0) else {
            let _ = DeleteDC(dc);
            return None;
        };
        let old = SelectObject(dc, bmp.into());
        let drawn = DrawIconEx(dc, 0, 0, icon, size, size, 0, None, DI_NORMAL).is_ok();
        let n = (size * size * 4) as usize;
        let px = std::slice::from_raw_parts(bits as *const u8, n).to_vec();
        SelectObject(dc, old);
        let _ = DeleteObject(bmp.into());
        let _ = DeleteDC(dc);
        if !drawn {
            return None;
        }
        // BGRA premultiplied -> RGBA straight. Old icons without alpha come
        // out fully transparent; show them opaque.
        let no_alpha = px.chunks(4).all(|p| p[3] == 0);
        let mut rgba = Vec::with_capacity(n);
        for p in px.chunks(4) {
            let a = if no_alpha { 255 } else { p[3] };
            let un = |c: u8| if a == 0 || no_alpha { c } else { ((c as u32 * 255 + a as u32 / 2) / a as u32).min(255) as u8 };
            rgba.extend_from_slice(&[un(p[2]), un(p[1]), un(p[0]), a]);
        }
        let mut out = Vec::new();
        {
            let mut enc = png::Encoder::new(&mut out, size as u32, size as u32);
            enc.set_color(png::ColorType::Rgba);
            enc.set_depth(png::BitDepth::Eight);
            let mut w = enc.write_header().ok()?;
            w.write_image_data(&rgba).ok()?;
        }
        Some(out)
    }
}

fn base64(data: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut s = String::with_capacity(data.len().div_ceil(3) * 4);
    for c in data.chunks(3) {
        let n = (c[0] as u32) << 16 | (*c.get(1).unwrap_or(&0) as u32) << 8 | *c.get(2).unwrap_or(&0) as u32;
        s.push(T[(n >> 18) as usize & 63] as char);
        s.push(T[(n >> 12) as usize & 63] as char);
        s.push(if c.len() > 1 { T[(n >> 6) as usize & 63] as char } else { '=' });
        s.push(if c.len() > 2 { T[n as usize & 63] as char } else { '=' });
    }
    s
}
