// Windows 10 before build 20348 draws a yellow border around every window
// captured with Windows.Graphics.Capture, and apps can't turn it off; on
// newer Windows a Store copy may only capture windows, and without the
// border, with the user's yes (store::capture_allowed); and Windows' capture
// service can get stuck (wgc.rs). There Deonyx copies
// the screen instead (DXGI Desktop Duplication). Its layers
// are hidden from screen capture, so the copy shows the real windows under
// them, and each layer takes the part of the screen its window covers.

use std::time::{Duration, Instant};

use windows::core::{Interface, Result};
use windows::Win32::Foundation::RECT;
use windows::Win32::Graphics::Direct3D11::*;
use windows::Win32::Graphics::Dxgi::Common::DXGI_MODE_ROTATION_IDENTITY;
use windows::Win32::Graphics::Dxgi::*;

use crate::gpu::Gpu;
use crate::win::{height, width};

/// Builds where Windows.Graphics.Capture can leave out its border.
const BORDERLESS_BUILD: u32 = 20348;

/// Whether layers copy the screen rather than capture their window.
/// DEONYX_CAPTURE=desktop or =window chooses, for testing.
pub fn needed() -> bool {
    match std::env::var("DEONYX_CAPTURE").as_deref() {
        Ok("desktop") => true,
        Ok("window") => false,
        _ => windows_build() < BORDERLESS_BUILD || !crate::store::capture_allowed(),
    }
}

pub fn windows_build() -> u32 {
    use windows::core::w;
    use windows::Win32::System::Registry::{RegGetValueW, HKEY_LOCAL_MACHINE, RRF_RT_REG_SZ};
    let mut buf = [0u16; 16];
    let mut len = (buf.len() * 2) as u32;
    let r = unsafe {
        RegGetValueW(
            HKEY_LOCAL_MACHINE,
            w!(r"SOFTWARE\Microsoft\Windows NT\CurrentVersion"),
            w!("CurrentBuildNumber"),
            RRF_RT_REG_SZ,
            None,
            Some(buf.as_mut_ptr() as *mut _),
            Some(&mut len),
        )
    };
    if r.is_err() {
        return 0;
    }
    let n = (len as usize / 2).saturating_sub(1).min(buf.len());
    String::from_utf16_lossy(&buf[..n]).trim().parse().unwrap_or(0)
}

struct Output {
    output: IDXGIOutput1,
    /// Where it sits on the desktop.
    rect: RECT,
    dup: Option<IDXGIOutputDuplication>,
    /// The last picture of it.
    tex: Option<ID3D11Texture2D>,
    /// When to try duplicating it again after a failure (the sign-in
    /// screen and UAC prompts take it away for a while).
    retry: Instant,
}

pub struct Desktop {
    outputs: Vec<Output>,
}

impl Desktop {
    /// The screens of the GPU Deonyx draws with.
    pub fn new(gpu: &Gpu) -> Result<Desktop> {
        let dxgi: IDXGIDevice = gpu.device.cast()?;
        let adapter = unsafe { dxgi.GetAdapter()? };
        let mut outputs = Vec::new();
        let mut i = 0;
        while let Ok(o) = unsafe { adapter.EnumOutputs(i) } {
            i += 1;
            let Ok(output) = o.cast::<IDXGIOutput1>() else { continue };
            let Ok(desc) = (unsafe { output.GetDesc() }) else { continue };
            if !desc.AttachedToDesktop.as_bool() || desc.Rotation != DXGI_MODE_ROTATION_IDENTITY {
                // A turned screen comes out of the duplication on its side.
                continue;
            }
            outputs.push(Output { output, rect: desc.DesktopCoordinates, dup: None, tex: None, retry: Instant::now() });
        }
        Ok(Desktop { outputs })
    }

    /// Takes the screens' newest pictures. Returns the parts of the desktop
    /// that changed since the last call.
    pub fn poll(&mut self, gpu: &Gpu) -> Vec<RECT> {
        let mut changed = Vec::new();
        for o in &mut self.outputs {
            if o.dup.is_none() {
                if o.retry > Instant::now() {
                    continue;
                }
                match unsafe { o.output.DuplicateOutput(&gpu.device) } {
                    Ok(d) => {
                        o.dup = Some(d);
                        // Everything is new to a fresh duplication.
                        changed.push(o.rect);
                    }
                    Err(_) => {
                        o.retry = Instant::now() + Duration::from_secs(1);
                        continue;
                    }
                }
            }
            let dup = o.dup.clone().unwrap();
            let mut info = DXGI_OUTDUPL_FRAME_INFO::default();
            let mut res = None;
            match unsafe { dup.AcquireNextFrame(0, &mut info, &mut res) } {
                Ok(()) => {}
                Err(e) if e.code() == DXGI_ERROR_WAIT_TIMEOUT => continue,
                Err(_) => {
                    // Lost (a mode change, the secure desktop): start over.
                    o.dup = None;
                    o.tex = None;
                    continue;
                }
            }
            // Only the mouse moved: nothing new on the screen.
            if info.LastPresentTime != 0 {
                if let Some(Ok(tex)) = res.map(|r| r.cast::<ID3D11Texture2D>()) {
                    if o.tex.is_none() {
                        let mut d = D3D11_TEXTURE2D_DESC::default();
                        unsafe { tex.GetDesc(&mut d) };
                        let own = D3D11_TEXTURE2D_DESC {
                            Usage: D3D11_USAGE_DEFAULT,
                            BindFlags: 0,
                            CPUAccessFlags: 0,
                            MiscFlags: 0,
                            MipLevels: 1,
                            ..d
                        };
                        let mut t = None;
                        let _ = unsafe { gpu.device.CreateTexture2D(&own, None, Some(&mut t)) };
                        o.tex = t;
                    }
                    if let Some(own) = &o.tex {
                        unsafe { gpu.ctx.CopyResource(own, &tex) };
                    }
                    changed.extend(dirty_rects(&dup, info.TotalMetadataBufferSize, o.rect));
                }
            }
            let _ = unsafe { dup.ReleaseFrame() };
        }
        changed
    }

    /// Copies the screen under `frame` (desktop pixels) into `dst`, which is
    /// as big as the frame. False if no screen showed any of it.
    pub fn copy(&self, gpu: &Gpu, dst: &ID3D11Texture2D, frame: RECT) -> bool {
        let mut any = false;
        for o in &self.outputs {
            let Some(src) = &o.tex else { continue };
            let i = RECT {
                left: frame.left.max(o.rect.left),
                top: frame.top.max(o.rect.top),
                right: frame.right.min(o.rect.right),
                bottom: frame.bottom.min(o.rect.bottom),
            };
            if width(&i) <= 0 || height(&i) <= 0 {
                continue;
            }
            let bx = D3D11_BOX {
                left: (i.left - o.rect.left) as u32,
                top: (i.top - o.rect.top) as u32,
                right: (i.right - o.rect.left) as u32,
                bottom: (i.bottom - o.rect.top) as u32,
                front: 0,
                back: 1,
            };
            unsafe {
                gpu.ctx.CopySubresourceRegion(dst, 0, (i.left - frame.left) as u32, (i.top - frame.top) as u32, 0, src, 0, Some(&bx))
            };
            any = true;
        }
        any
    }
}

/// The parts of one screen that changed in this frame, in desktop pixels.
fn dirty_rects(dup: &IDXGIOutputDuplication, bytes: u32, origin: RECT) -> Vec<RECT> {
    let whole = RECT { left: origin.left, top: origin.top, right: origin.right, bottom: origin.bottom };
    if bytes == 0 {
        return vec![whole];
    }
    let mut out = Vec::new();
    let cap = bytes as usize / std::mem::size_of::<RECT>() + 1;
    let mut rects = vec![RECT::default(); cap];
    let mut used = 0u32;
    let got = unsafe {
        dup.GetFrameDirtyRects((cap * std::mem::size_of::<RECT>()) as u32, rects.as_mut_ptr(), &mut used)
    };
    if got.is_ok() {
        out.extend(rects.iter().take(used as usize / std::mem::size_of::<RECT>()).copied());
    }
    let mcap = bytes as usize / std::mem::size_of::<DXGI_OUTDUPL_MOVE_RECT>() + 1;
    let mut moves = vec![DXGI_OUTDUPL_MOVE_RECT::default(); mcap];
    let got = unsafe {
        dup.GetFrameMoveRects((mcap * std::mem::size_of::<DXGI_OUTDUPL_MOVE_RECT>()) as u32, moves.as_mut_ptr(), &mut used)
    };
    if got.is_ok() {
        out.extend(moves.iter().take(used as usize / std::mem::size_of::<DXGI_OUTDUPL_MOVE_RECT>()).map(|m| m.DestinationRect));
    }
    // If Windows didn't say which parts, take the whole screen.
    if out.is_empty() {
        out.push(RECT { left: 0, top: 0, right: width(&origin), bottom: height(&origin) });
    }
    out.into_iter()
        .map(|r| RECT { left: r.left + origin.left, top: r.top + origin.top, right: r.right + origin.left, bottom: r.bottom + origin.top })
        .collect()
}
