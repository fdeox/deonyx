// The screens, with names people recognise ("DELL U2720Q") and ids that stay
// the same across restarts, so each can keep its own look.

use std::mem::size_of;

use serde::Serialize;
use windows::core::BOOL;
use windows::Win32::Devices::Display::*;
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::UI::WindowsAndMessaging::MONITORINFOF_PRIMARY;

#[derive(Clone, Debug, Serialize)]
pub struct Info {
    /// The monitor's device path: the same screen on the same port keeps it.
    pub id: String,
    /// The monitor's own name; empty when it has none (the settings then
    /// call it "Built-in screen" or "Screen 2").
    pub name: String,
    pub primary: bool,
    /// A laptop's own panel.
    pub builtin: bool,
    /// Windows' name for it right now, like \\.\DISPLAY1.
    #[serde(skip)]
    pub gdi: String,
}

fn wide(s: &[u16]) -> String {
    let n = s.iter().position(|&c| c == 0).unwrap_or(s.len());
    String::from_utf16_lossy(&s[..n])
}

/// GDI name, device path, friendly name and whether it's built in, from the
/// display configuration.
fn targets() -> Vec<(String, String, String, bool)> {
    let mut out = Vec::new();
    unsafe {
        let (mut np, mut nm) = (0u32, 0u32);
        if GetDisplayConfigBufferSizes(QDC_ONLY_ACTIVE_PATHS, &mut np, &mut nm) != ERROR_SUCCESS {
            return out;
        }
        let mut paths = vec![DISPLAYCONFIG_PATH_INFO::default(); np as usize];
        let mut modes = vec![DISPLAYCONFIG_MODE_INFO::default(); nm as usize];
        if QueryDisplayConfig(QDC_ONLY_ACTIVE_PATHS, &mut np, paths.as_mut_ptr(), &mut nm, modes.as_mut_ptr(), None)
            != ERROR_SUCCESS
        {
            return out;
        }
        for p in paths.iter().take(np as usize) {
            let mut src = DISPLAYCONFIG_SOURCE_DEVICE_NAME::default();
            src.header.r#type = DISPLAYCONFIG_DEVICE_INFO_GET_SOURCE_NAME;
            src.header.size = size_of::<DISPLAYCONFIG_SOURCE_DEVICE_NAME>() as u32;
            src.header.adapterId = p.sourceInfo.adapterId;
            src.header.id = p.sourceInfo.id;
            if DisplayConfigGetDeviceInfo(&mut src.header) != 0 {
                continue;
            }
            let mut tgt = DISPLAYCONFIG_TARGET_DEVICE_NAME::default();
            tgt.header.r#type = DISPLAYCONFIG_DEVICE_INFO_GET_TARGET_NAME;
            tgt.header.size = size_of::<DISPLAYCONFIG_TARGET_DEVICE_NAME>() as u32;
            tgt.header.adapterId = p.targetInfo.adapterId;
            tgt.header.id = p.targetInfo.id;
            if DisplayConfigGetDeviceInfo(&mut tgt.header) != 0 {
                continue;
            }
            let tech = tgt.outputTechnology;
            let builtin = tech == DISPLAYCONFIG_OUTPUT_TECHNOLOGY_INTERNAL
                || tech == DISPLAYCONFIG_OUTPUT_TECHNOLOGY_LVDS
                || tech == DISPLAYCONFIG_OUTPUT_TECHNOLOGY_DISPLAYPORT_EMBEDDED
                || tech == DISPLAYCONFIG_OUTPUT_TECHNOLOGY_UDI_EMBEDDED;
            out.push((wide(&src.viewGdiDeviceName), wide(&tgt.monitorDevicePath), wide(&tgt.monitorFriendlyDeviceName), builtin));
        }
    }
    out
}

unsafe extern "system" fn collect(m: HMONITOR, _: HDC, _: *mut RECT, l: LPARAM) -> BOOL {
    let v = &mut *(l.0 as *mut Vec<HMONITOR>);
    v.push(m);
    TRUE
}

/// Windows' GDI name for a monitor, like \\.\DISPLAY1.
pub fn gdi_name(m: HMONITOR) -> Option<(String, bool)> {
    let mut mi = MONITORINFOEXW::default();
    mi.monitorInfo.cbSize = size_of::<MONITORINFOEXW>() as u32;
    unsafe {
        if !GetMonitorInfoW(m, &mut mi as *mut MONITORINFOEXW as *mut MONITORINFO).as_bool() {
            return None;
        }
    }
    Some((wide(&mi.szDevice), mi.monitorInfo.dwFlags & MONITORINFOF_PRIMARY != 0))
}

/// Every screen in use.
pub fn list() -> Vec<Info> {
    let mut mons: Vec<HMONITOR> = Vec::new();
    unsafe {
        let _ = EnumDisplayMonitors(None, None, Some(collect), LPARAM(&mut mons as *mut Vec<HMONITOR> as isize));
    }
    let names = targets();
    let mut out: Vec<Info> = Vec::new();
    for m in mons {
        let Some((gdi, primary)) = gdi_name(m) else { continue };
        let (id, name, builtin) = match names.iter().find(|(g, _, _, _)| *g == gdi) {
            Some((_, path, friendly, builtin)) => (path.clone(), friendly.trim().to_string(), *builtin),
            None => (gdi.clone(), String::new(), false),
        };
        out.push(Info { id, name, primary, builtin, gdi });
    }
    // The main screen first.
    out.sort_by_key(|i| !i.primary);
    out
}

/// The id of the screen showing most of a window, from a list().
pub fn of_window(h: HWND, screens: &[Info]) -> Option<String> {
    let m = unsafe { MonitorFromWindow(h, MONITOR_DEFAULTTONEAREST) };
    let (gdi, _) = gdi_name(m)?;
    screens.iter().find(|s| s.gdi == gdi).map(|s| s.id.clone())
}

