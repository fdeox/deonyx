// Programs with open windows, for the "Add an app" list: file name, the
// name the program gives itself, a window title and its icon.

use std::collections::{HashMap, HashSet};
use std::ffi::c_void;
use std::mem::size_of;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use serde::Serialize;
use windows::core::{w, Interface, HSTRING, PCWSTR, PWSTR};
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::Storage::FileSystem::{GetFileVersionInfoSizeW, GetFileVersionInfoW, VerQueryValueW};
use windows::Win32::System::Com::{CoCreateInstance, CoInitializeEx, IPersistFile, CLSCTX_INPROC_SERVER, COINIT_MULTITHREADED, STGM_READ};
use windows::Win32::System::Registry::*;
use windows::Win32::System::Threading::*;
use windows::Win32::UI::Shell::{ExtractIconExW, IShellLinkW, ShellLink};
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

pub fn exe_path(pid: u32) -> Option<String> {
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

/// A program with a bright window open, for the first start: what to show
/// and how bright it is.
#[derive(Clone, Serialize)]
pub struct Bright {
    pub exe: String,
    pub name: String,
    pub title: String,
    pub icon: Option<String>,
    /// A small picture of the window, as a PNG data URL.
    pub thumb: String,
    /// Mean cube-root luminance, like the engine measures (0..1).
    pub light: f32,
}

/// How wide the pictures are, and how long one window may take to draw
/// itself (a hung program never does).
const THUMB_W: i32 = 480;
const PRINT_WAIT: Duration = Duration::from_millis(1500);

/// The open programs whose biggest window is light, brightest picture
/// first: the ones worth darkening. A program's own window only, at most 8.
pub fn bright() -> Vec<Bright> {
    let me = std::process::id();
    let mut best: HashMap<String, (i64, HWND, String)> = HashMap::new();
    for h in top_level_windows() {
        unsafe {
            if !IsWindowVisible(h).as_bool() || IsIconic(h).as_bool() || !is_top_level(h) || is_cloaked(h) || GetWindow(h, GW_OWNER).is_ok() {
                continue;
            }
            if GetWindowLongW(h, GWL_EXSTYLE) as u32 & WS_EX_TOOLWINDOW.0 != 0 || IsHungAppWindow(h).as_bool() {
                continue;
            }
        }
        let pid = window_pid(h);
        if pid == me || title(h).trim().is_empty() {
            continue;
        }
        let r = crate::win::frame_rect(h);
        let (w, hh) = (crate::win::width(&r), crate::win::height(&r));
        if w < 240 || hh < 160 {
            continue;
        }
        let Some(path) = exe_path(pid) else { continue };
        let exe = path.rsplit('\\').next().unwrap_or(&path).to_ascii_lowercase();
        if SKIP.contains(&exe.as_str()) {
            continue;
        }
        let area = w as i64 * hh as i64;
        if best.get(&exe).is_none_or(|(a, _, _)| area > *a) {
            best.insert(exe, (area, h, path));
        }
    }
    // The biggest ones, all drawn at the same time; whatever isn't done by
    // the deadline is left out.
    let mut found: Vec<(i64, HWND, String)> = best.into_values().collect();
    found.sort_by_key(|(a, _, _)| -a);
    found.truncate(12);
    let jobs: Vec<_> = found
        .into_iter()
        .map(|(_, h, path)| {
            let (tx, rx) = std::sync::mpsc::channel();
            let hwnd = h.0 as isize;
            std::thread::spawn(move || {
                let _ = tx.send(unsafe { print(HWND(hwnd as *mut c_void)) });
            });
            (h, path, rx)
        })
        .collect();
    let deadline = Instant::now() + PRINT_WAIT;
    let mut out: Vec<Bright> = jobs
        .into_iter()
        .filter_map(|(h, path, rx)| {
            let (px, w, hh) = rx.recv_timeout(deadline.saturating_duration_since(Instant::now())).ok().flatten()?;
            let light = lightness(&px);
            if light < 0.5 {
                return None;
            }
            let d = describe(&path);
            Some(Bright { exe: d.exe, name: d.name, title: title(h), icon: d.icon, thumb: thumb_png(&px, w, hh)?, light })
        })
        .collect();
    out.sort_by(|a, b| b.light.total_cmp(&a.light));
    out.truncate(8);
    out
}

/// The window drawn into a small BGRA picture, THUMB_W wide; None if it
/// doesn't draw or comes out blank. A hung program never draws: callers
/// wait only so long (bright).
unsafe fn print(h: HWND) -> Option<(Vec<u8>, i32, i32)> {
    let mut r = RECT::default();
    GetWindowRect(h, &mut r).ok()?;
    let (w, hh) = (r.right - r.left, r.bottom - r.top);
    // The picture covers the window's invisible resize borders too; only
    // the part that shows is kept.
    let f = crate::win::frame_rect(h);
    let (ox, oy) = ((f.left - r.left).clamp(0, w - 1), (f.top - r.top).clamp(0, hh - 1));
    let (cw, ch) = ((f.right - f.left).clamp(1, w - ox), (f.bottom - f.top).clamp(1, hh - oy));
    let screen = GetDC(None);
    let dc = CreateCompatibleDC(Some(screen));
    let bmi = BITMAPINFO {
        bmiHeader: BITMAPINFOHEADER {
            biSize: size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: w,
            biHeight: -hh,
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB.0,
            ..Default::default()
        },
        ..Default::default()
    };
    let mut bits: *mut c_void = std::ptr::null_mut();
    let bmp = CreateDIBSection(Some(dc), &bmi, DIB_RGB_COLORS, &mut bits, None, 0);
    ReleaseDC(None, screen);
    let Ok(bmp) = bmp else {
        let _ = DeleteDC(dc);
        return None;
    };
    let old = SelectObject(dc, bmp.into());
    // PW_RENDERFULLCONTENT: also windows drawn with the GPU.
    let ok = windows::Win32::Storage::Xps::PrintWindow(h, dc, windows::Win32::Storage::Xps::PRINT_WINDOW_FLAGS(2)).as_bool();
    let full = std::slice::from_raw_parts(bits as *const u8, (w * hh * 4) as usize);
    // Box-averaged down to THUMB_W wide.
    let tw = THUMB_W.min(cw);
    let th = (ch as i64 * tw as i64 / cw as i64).max(1) as i32;
    let mut small = vec![0u8; (tw * th * 4) as usize];
    for y in 0..th {
        let (y0, y1) = (y * ch / th, ((y + 1) * ch / th).max(y * ch / th + 1));
        for x in 0..tw {
            let (x0, x1) = (x * cw / tw, ((x + 1) * cw / tw).max(x * cw / tw + 1));
            let mut sum = [0u32; 3];
            let mut n = 0u32;
            for yy in (y0..y1).step_by(2) {
                for xx in (x0..x1).step_by(2) {
                    let i = (((oy + yy) * w + ox + xx) * 4) as usize;
                    for c in 0..3 {
                        sum[c] += full[i + c] as u32;
                    }
                    n += 1;
                }
            }
            let o = ((y * tw + x) * 4) as usize;
            for c in 0..3 {
                small[o + c] = (sum[c] / n.max(1)) as u8;
            }
            small[o + 3] = 255;
        }
    }
    SelectObject(dc, old);
    let _ = DeleteObject(bmp.into());
    let _ = DeleteDC(dc);
    // A window that didn't draw (or drew nothing) is all one colour.
    let first = &small[..3];
    if !ok || small.chunks(4).all(|p| &p[..3] == first) {
        return None;
    }
    Some((small, tw, th))
}

fn lightness(bgra: &[u8]) -> f32 {
    let lin = |v: u8| (v as f64 / 255.0).powf(2.2);
    let (mut sum, mut n) = (0.0f64, 0u32);
    for p in bgra.chunks(4) {
        sum += (0.0722 * lin(p[0]) + 0.7152 * lin(p[1]) + 0.2126 * lin(p[2])).cbrt();
        n += 1;
    }
    (sum / n.max(1) as f64) as f32
}

fn thumb_png(bgra: &[u8], w: i32, h: i32) -> Option<String> {
    let rgb: Vec<u8> = bgra.chunks(4).flat_map(|p| [p[2], p[1], p[0]]).collect();
    let mut out = Vec::new();
    {
        let mut enc = png::Encoder::new(&mut out, w as u32, h as u32);
        enc.set_color(png::ColorType::Rgb);
        enc.set_depth(png::BitDepth::Eight);
        let mut wr = enc.write_header().ok()?;
        wr.write_image_data(&rgb).ok()?;
    }
    Some(format!("data:image/png;base64,{}", base64(&out)))
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

/// Exe names that are never what a user wants to darken.
fn is_junk(name: &str, exe: &str) -> bool {
    let n = name.to_lowercase();
    let e = exe.to_lowercase();
    const NAME: &[&str] = &[
        "uninstall", "readme", "read me", "help", "documentation", "release notes", "website", "license", "support",
        "manual", "changelog", "what's new", "repair", "service", "recovery", "administrative tools",
    ];
    const EXE: &[&str] = &[
        "unins", "uninstall", "setup", "installer", "update", "crash", "report", "helper", "elevat", "maintenance",
        "remove", "service", "msiexec", "rundll32", "control.exe", "cmd.exe", "powershell", "pwsh", "wsl.exe", "conhost",
    ];
    NAME.iter().any(|w| n.contains(w)) || EXE.iter().any(|w| e.contains(w)) || SKIP.contains(&e.as_str())
}

/// Expands %NAME% parts the way Windows does.
fn expand(s: &str) -> String {
    let mut out = String::new();
    let mut rest = s;
    while let Some(a) = rest.find('%') {
        let Some(b) = rest[a + 1..].find('%') else { break };
        out.push_str(&rest[..a]);
        let var = &rest[a + 1..a + 1 + b];
        out.push_str(&std::env::var(var).unwrap_or_else(|_| format!("%{var}%")));
        rest = &rest[a + 2 + b..];
    }
    out.push_str(rest);
    out
}

fn reg_string(root: HKEY, path: &str, value: &str) -> Option<String> {
    unsafe {
        let mut buf = [0u16; 1024];
        let mut len = (buf.len() * 2) as u32;
        let r = RegGetValueW(
            root,
            &HSTRING::from(path),
            if value.is_empty() { PCWSTR::null() } else { PCWSTR(HSTRING::from(value).as_ptr()) },
            RRF_RT_REG_SZ | RRF_RT_REG_EXPAND_SZ,
            None,
            Some(buf.as_mut_ptr() as *mut c_void),
            Some(&mut len),
        );
        if r != ERROR_SUCCESS {
            return None;
        }
        let n = (len as usize / 2).saturating_sub(1);
        let v = String::from_utf16_lossy(&buf[..n.min(buf.len())]);
        (!v.trim().is_empty()).then(|| v.trim().to_string())
    }
}

fn reg_dword(root: HKEY, path: &str, value: &str) -> Option<u32> {
    unsafe {
        let mut v = 0u32;
        let mut len = 4u32;
        let r = RegGetValueW(
            root,
            &HSTRING::from(path),
            &HSTRING::from(value),
            RRF_RT_REG_DWORD,
            None,
            Some(&mut v as *mut u32 as *mut c_void),
            Some(&mut len),
        );
        (r == ERROR_SUCCESS).then_some(v)
    }
}

fn reg_subkeys(root: HKEY, path: &str) -> Vec<String> {
    let mut out = Vec::new();
    unsafe {
        let mut key = HKEY::default();
        if RegOpenKeyExW(root, &HSTRING::from(path), None, KEY_READ, &mut key) != ERROR_SUCCESS {
            return out;
        }
        for i in 0.. {
            let mut buf = [0u16; 256];
            let mut len = buf.len() as u32;
            if RegEnumKeyExW(key, i, Some(PWSTR(buf.as_mut_ptr())), &mut len, None, None, None, None) != ERROR_SUCCESS {
                break;
            }
            out.push(String::from_utf16_lossy(&buf[..len as usize]));
        }
        let _ = RegCloseKey(key);
    }
    out
}

/// "C:\x\app.exe",0  ->  C:\x\app.exe
fn icon_path(v: &str) -> Option<PathBuf> {
    let v = v.trim();
    let v = v.rsplit_once(',').filter(|(_, n)| n.trim().parse::<i32>().is_ok()).map(|(p, _)| p).unwrap_or(v);
    let p = expand(v.trim().trim_matches('"'));
    p.to_lowercase().ends_with(".exe").then(|| PathBuf::from(p))
}

/// Lower-case words of a program name worth matching against file names.
fn name_words(name: &str) -> Vec<String> {
    name.split(|c: char| !c.is_alphanumeric())
        .map(str::to_lowercase)
        .filter(|w| w.len() >= 3 && !w.chars().all(|c| c.is_ascii_digit()))
        .collect()
}

/// The exe in the install folder named most like the program. Looks one
/// level down too, but only in folders that carry the name ("SketchUp 2026\
/// SketchUp") or hold versioned app builds ("Discord\app-1.0.9").
fn exe_in(dir: &Path, name: &str) -> Option<PathBuf> {
    let words = name_words(name);
    if words.is_empty() {
        return None;
    }
    let squash = |s: &str| s.to_lowercase().replace([' ', '-', '_', '.'], "");
    let score = |stem: &str| {
        let st = squash(stem);
        words.iter().filter(|w| st.starts_with(w.as_str())).map(|w| w.len() * 2 + usize::from(st == **w) * 100).max()
    };
    let mut best: Option<(usize, PathBuf)> = None;
    let look = |d: &Path, best: &mut Option<(usize, PathBuf)>| -> Vec<PathBuf> {
        let mut subdirs = Vec::new();
        let Ok(rd) = std::fs::read_dir(d) else { return subdirs };
        for e in rd.flatten() {
            let p = e.path();
            let file = e.file_name().to_string_lossy().into_owned();
            if e.file_type().map(|t| t.is_dir()).unwrap_or(false) {
                let f = squash(&file);
                if f.starts_with("app") || words.iter().any(|w| f.contains(w.as_str())) {
                    subdirs.push(p);
                }
            } else if file.to_lowercase().ends_with(".exe") && !is_junk(name, &file) {
                let stem = file[..file.len() - 4].to_string();
                if let Some(sc) = score(&stem) {
                    if best.as_ref().is_none_or(|(b, _)| sc > *b) {
                        *best = Some((sc, p));
                    }
                }
            }
        }
        subdirs
    };
    for sub in look(dir, &mut best) {
        look(&sub, &mut best);
    }
    best.map(|(_, p)| p)
}

fn shortcut_target(lnk: &Path) -> Option<PathBuf> {
    unsafe {
        let link: IShellLinkW = CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER).ok()?;
        let file: IPersistFile = link.cast().ok()?;
        file.Load(&HSTRING::from(lnk.as_os_str()), STGM_READ).ok()?;
        let mut buf = [0u16; 1024];
        link.GetPath(&mut buf, std::ptr::null_mut(), 0).ok()?;
        let n = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
        let p = expand(&String::from_utf16_lossy(&buf[..n]));
        p.to_lowercase().ends_with(".exe").then(|| PathBuf::from(p))
    }
}

fn shortcuts(dir: &Path, out: &mut Vec<(String, PathBuf)>) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    for e in rd.flatten() {
        let p = e.path();
        if p.is_dir() {
            shortcuts(&p, out);
        } else if p.extension().is_some_and(|x| x.eq_ignore_ascii_case("lnk")) {
            if let (Some(name), Some(target)) = (p.file_stem(), shortcut_target(&p)) {
                out.push((name.to_string_lossy().into_owned(), target));
            }
        }
    }
}

static INSTALLED: Mutex<Option<(Instant, Vec<Running>)>> = Mutex::new(None);
/// One search at a time: a second caller waits and gets the cached result.
static SEARCHING: Mutex<()> = Mutex::new(());

/// Installed programs, from the Start menu, the uninstall list and App Paths.
/// Cached for two minutes: icons take a moment.
pub fn installed() -> Vec<Running> {
    let _one = SEARCHING.lock().unwrap();
    if let Some((t, v)) = INSTALLED.lock().unwrap().as_ref() {
        if t.elapsed() < Duration::from_secs(120) {
            return v.clone();
        }
    }
    unsafe {
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
    }
    let debug = std::env::var_os("DEONYX_DEBUG").is_some();
    let clock = Instant::now();
    let lap = |what: &str| {
        if debug {
            eprintln!("installed programs: {what} {} ms", clock.elapsed().as_millis());
        }
    };
    let mut found: Vec<(String, PathBuf)> = Vec::new();
    for base in [std::env::var_os("ProgramData"), std::env::var_os("APPDATA")].into_iter().flatten() {
        shortcuts(&PathBuf::from(base).join(r"Microsoft\Windows\Start Menu\Programs"), &mut found);
    }
    lap("kisayollar");
    const UNINSTALL: &str = r"SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall";
    const UNINSTALL32: &str = r"SOFTWARE\WOW6432Node\Microsoft\Windows\CurrentVersion\Uninstall";
    for (root, base) in [(HKEY_LOCAL_MACHINE, UNINSTALL), (HKEY_LOCAL_MACHINE, UNINSTALL32), (HKEY_CURRENT_USER, UNINSTALL)] {
        for sub in reg_subkeys(root, base) {
            let path = format!(r"{base}\{sub}");
            let Some(name) = reg_string(root, &path, "DisplayName") else { continue };
            if reg_dword(root, &path, "SystemComponent") == Some(1) || reg_string(root, &path, "ParentKeyName").is_some() {
                continue;
            }
            let exe = reg_string(root, &path, "DisplayIcon").and_then(|v| icon_path(&v)).filter(|p| p.exists()).or_else(|| {
                let dir = reg_string(root, &path, "InstallLocation")?;
                exe_in(Path::new(expand(dir.trim_matches('"')).as_str()), &name)
            });
            if let Some(exe) = exe {
                found.push((name, exe));
            }
        }
    }
    lap("kaldirma kayitlari");
    const APP_PATHS: &str = r"SOFTWARE\Microsoft\Windows\CurrentVersion\App Paths";
    for root in [HKEY_LOCAL_MACHINE, HKEY_CURRENT_USER] {
        for sub in reg_subkeys(root, APP_PATHS) {
            if let Some(exe) = reg_string(root, &format!(r"{APP_PATHS}\{sub}"), "").and_then(|v| icon_path(&v)) {
                let stem = exe.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
                found.push((stem, exe));
            }
        }
    }

    lap("app paths");
    let me = std::env::current_exe().ok().and_then(|p| p.file_name().map(|n| n.to_string_lossy().to_lowercase()));
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for (name, path) in found {
        let Some(file) = path.file_name().map(|n| n.to_string_lossy().into_owned()) else { continue };
        let key = file.to_lowercase();
        if seen.contains(&key) || Some(&key) == me.as_ref() || is_junk(&name, &file) || !path.exists() {
            continue;
        }
        seen.insert(key);
        let full = path.to_string_lossy().into_owned();
        // Names and paths only: reading the exe files (icons, descriptions)
        // is the slow part, and icons() does that afterwards.
        out.push(Running { exe: file, name, title: full, icon: None });
    }
    lap("liste");
    out.sort_by_key(|r| r.name.to_lowercase());
    *INSTALLED.lock().unwrap() = Some((Instant::now(), out.clone()));
    out
}

static ICONS: Mutex<Option<HashMap<String, Option<String>>>> = Mutex::new(None);

/// Icons for program files, cached. The first look at an exe can be slow
/// (the virus scanner checks it), so the settings window asks in batches.
pub fn icons(paths: &[String]) -> Vec<Option<String>> {
    paths
        .iter()
        .map(|p| {
            if let Some(hit) = ICONS.lock().unwrap().get_or_insert_with(HashMap::new).get(p) {
                return hit.clone();
            }
            let icon = icon_data_url(p);
            ICONS.lock().unwrap().get_or_insert_with(HashMap::new).insert(p.clone(), icon.clone());
            icon
        })
        .collect()
}

/// Gets the list and every icon ready in the background, quietly, so the
/// picker opens at once.
pub fn warm_up() {
    std::thread::spawn(|| {
        unsafe {
            use windows::Win32::System::Threading::{GetCurrentThread, SetThreadPriority, THREAD_PRIORITY_LOWEST};
            let _ = SetThreadPriority(GetCurrentThread(), THREAD_PRIORITY_LOWEST);
        }
        let list = installed();
        let paths: Vec<String> = list.into_iter().map(|r| r.title).collect();
        icons(&paths);
    });
}
