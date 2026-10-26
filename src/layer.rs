// One darkened window: its capture, and the click-through overlay that shows
// the result right above it.

use std::ffi::c_void;
use std::time::{Duration, Instant};

use windows::core::{w, Interface, Result};
use windows::Graphics::SizeInt32;
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Direct3D::D3D_PRIMITIVE_TOPOLOGY_TRIANGLELIST;
use windows::Win32::Graphics::Direct3D11::*;
use windows::Win32::Graphics::DirectComposition::{IDCompositionTarget, IDCompositionVisual};
use windows::Win32::Graphics::Dxgi::Common::*;
use windows::Win32::Graphics::Dxgi::*;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::WinRT::Direct3D11::IDirect3DDxgiInterfaceAccess;
use windows::Win32::UI::WindowsAndMessaging::*;

use crate::desk::Desktop;
use crate::wgc::Capture;
use crate::gpu::{Gpu, Params, MAX_RECTS};
use crate::photos;
use crate::win::{frame_rect, height, width};

pub const OVERLAY_CLASS: &str = "DeonyxOverlay";

/// How often a window is looked at for photos while it changes; while the
/// answer stays the same, less and less often, down to PHOTO_CALM.
const PHOTO_EVERY: Duration = Duration::from_millis(60);
const PHOTO_CALM: Duration = Duration::from_millis(480);

unsafe extern "system" fn wndproc(h: HWND, m: u32, w: WPARAM, l: LPARAM) -> LRESULT {
    match m {
        WM_NCHITTEST => LRESULT(-1),    // HTTRANSPARENT
        WM_MOUSEACTIVATE => LRESULT(3), // MA_NOACTIVATE
        _ => DefWindowProcW(h, m, w, l),
    }
}

pub fn register_class() -> Result<()> {
    unsafe {
        let hinst = GetModuleHandleW(None)?;
        let wc = WNDCLASSW {
            lpfnWndProc: Some(wndproc),
            hInstance: hinst.into(),
            lpszClassName: w!("DeonyxOverlay"),
            ..Default::default()
        };
        RegisterClassW(&wc);
    }
    Ok(())
}

pub struct Look {
    pub lmin: f32,
    pub lmax: f32,
    pub warm: f32,
    /// Lifts faint text: 1 leaves it, lower lifts more (see shader.hlsl).
    pub boost: f32,
    /// A window that is dark already is dimmed this much (0 leaves it).
    pub dim: f32,
    pub split: bool,
    pub owner: bool,
    /// Look for photos in the window.
    pub photos: bool,
    /// Hide the layer from screen sharing and screenshots.
    pub hidden: bool,
    /// How strongly to show the darkening (0 while real colours are peeked).
    pub strength: f32,
    /// Fade in from the real colours once the first frame is there, rather
    /// than appear at once (when the user just switched something on).
    pub fade_in: bool,
    /// Draw at most this often (on battery).
    pub gap: Option<Duration>,
}

/// A CPU-readable copy of a small mip level, for measuring lightness.
struct Probe {
    tex: ID3D11Texture2D,
    level: u32,
    w: u32,
    h: u32,
}

/// The photo rating of the window, one byte per 4x4 block (photos.rs).
struct PhotoPass {
    rtv: ID3D11RenderTargetView,
    tex: ID3D11Texture2D,
    staging: ID3D11Texture2D,
    w: u32,
    h: u32,
    /// A copy to the staging texture is on its way.
    waiting: bool,
    last: Instant,
    /// The current wait between looks, and a fingerprint of the last rating.
    every: Duration,
    seen: u64,
}

/// Darkening strength moving from one value to another.
#[derive(Clone, Copy)]
struct Fade {
    from: f32,
    to: f32,
    start: Instant,
    ms: f32,
}

impl Fade {
    fn value(&self) -> f32 {
        let t = if self.ms <= 0.0 { 1.0 } else { (self.start.elapsed().as_secs_f32() * 1000.0 / self.ms).clamp(0.0, 1.0) };
        // Eases in and out.
        self.from + (self.to - self.from) * t * t * (3.0 - 2.0 * t)
    }

    fn done(&self) -> bool {
        self.ms <= 0.0 || self.start.elapsed().as_secs_f32() * 1000.0 >= self.ms
    }
}

pub struct Layer {
    pub target: HWND,
    /// Lower-case exe name of the program this window belongs to.
    pub exe: String,
    pub overlay: HWND,
    /// Windows.Graphics.Capture; None when the layer copies the screen
    /// instead (see desk.rs).
    capture: Option<Capture>,
    swap: IDXGISwapChain1,
    /// Kept alive while the layer lives (see-through layers only).
    _comp: Option<(IDCompositionTarget, IDCompositionVisual)>,
    rtv: Option<ID3D11RenderTargetView>,
    copy: Option<(ID3D11Texture2D, ID3D11ShaderResourceView)>,
    probe: Option<Probe>,
    last_probe: Instant,
    measured: bool,
    size: (u32, u32),
    pub params: Params,
    /// Protected areas relative to the window frame (found by UI Automation).
    pub protect: Vec<RECT>,
    /// Areas the user drew, as distances from the window's edges.
    pub manual: Vec<crate::config::Area>,
    /// Photos and videos found in the window, kept in real colours.
    pub photos: Vec<RECT>,
    photo: Option<PhotoPass>,
    find_photos: bool,
    fade: Fade,
    fade_in: bool,
    /// Drawing waits for `gap` to pass (on battery).
    gap: Option<Duration>,
    last_render: Instant,
    pending: bool,
    last_rect: RECT,
    shown: bool,
    pub born: Instant,
    pub first_frame_ms: Option<f64>,
    /// Whether the owner binding took (it can't across a privilege boundary).
    pub owned: bool,
    pub frames: u32,
    pub work_ms: f64,
}

impl Layer {
    pub fn new(gpu: &Gpu, target: HWND, look: &Look, copy_screen: bool, frame_event: HANDLE) -> Result<Layer> {
        unsafe {
            let r = frame_rect(target);
            let (capture, start) = if copy_screen {
                (None, SizeInt32 { Width: width(&r), Height: height(&r) })
            } else {
                // A Store copy without the user's yes would wait for good
                // (store::capture_allowed), so it doesn't even try.
                if !crate::store::capture_allowed() {
                    return Err(windows::core::Error::from_hresult(windows::Win32::Foundation::E_ACCESSDENIED));
                }
                let (capture, start) = Capture::start(target, &gpu.rt_device, frame_event)?;
                (Some(capture), start)
            };

            let hinst = GetModuleHandleW(None)?;
            let see_through = gpu.dcomp.is_some();
            let mut ex = WS_EX_LAYERED | WS_EX_TRANSPARENT | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE;
            if see_through {
                ex |= WS_EX_NOREDIRECTIONBITMAP;
            }
            let overlay = CreateWindowExW(
                ex,
                w!("DeonyxOverlay"),
                w!("Deonyx"),
                WS_POPUP,
                r.left,
                r.top,
                width(&r).max(1),
                height(&r).max(1),
                None,
                None,
                Some(hinst.into()),
                None,
            )?;
            SetLayeredWindowAttributes(overlay, COLORREF(0), 255, LWA_ALPHA)?;
            if look.owner {
                // An owned window always stays above its owner, so activating
                // the target can't cover us even for one frame.
                SetWindowLongPtrW(overlay, GWLP_HWNDPARENT, target.0 as isize);
            }
            let owned = GetWindow(overlay, GW_OWNER).ok() == Some(target);

            let w0 = start.Width.max(1) as u32;
            let h0 = start.Height.max(1) as u32;
            let desc = DXGI_SWAP_CHAIN_DESC1 {
                Width: w0,
                Height: h0,
                Format: DXGI_FORMAT_B8G8R8A8_UNORM,
                SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
                BufferUsage: DXGI_USAGE_RENDER_TARGET_OUTPUT,
                BufferCount: 2,
                Scaling: DXGI_SCALING_STRETCH,
                SwapEffect: if see_through { DXGI_SWAP_EFFECT_FLIP_SEQUENTIAL } else { DXGI_SWAP_EFFECT_FLIP_DISCARD },
                AlphaMode: if see_through { DXGI_ALPHA_MODE_PREMULTIPLIED } else { DXGI_ALPHA_MODE_IGNORE },
                ..Default::default()
            };
            let (swap, comp) = match &gpu.dcomp {
                Some(dc) => {
                    let swap = gpu.factory.CreateSwapChainForComposition(&gpu.device, &desc, None)?;
                    let target = dc.CreateTargetForHwnd(overlay, true)?;
                    let visual = dc.CreateVisual()?;
                    visual.SetContent(&swap)?;
                    target.SetRoot(&visual)?;
                    dc.Commit()?;
                    (swap, Some((target, visual)))
                }
                None => {
                    let swap = gpu.factory.CreateSwapChainForHwnd(&gpu.device, overlay, &desc, None, None)?;
                    let _ = gpu.factory.MakeWindowAssociation(overlay, DXGI_MWA_NO_ALT_ENTER);
                    (swap, None)
                }
            };

            let mut layer = Layer {
                target,
                exe: String::new(),
                overlay,
                capture,
                swap,
                _comp: comp,
                rtv: None,
                copy: None,
                probe: None,
                last_probe: Instant::now(),
                measured: false,
                size: (w0, h0),
                params: Params {
                    lmin: look.lmin,
                    lmax: look.lmax,
                    warm: look.warm,
                    sat: 0.95,
                    amount: 1.0,
                    fringe: if std::env::var_os("DEONYX_NO_FRINGE").is_some() { 0.0 } else { 1.0 },
                    holes: if see_through { 1.0 } else { 0.0 },
                    strength: look.strength,
                    boost: look.boost,
                    dim: look.dim,
                    ..Default::default()
                },
                protect: Vec::new(),
                manual: Vec::new(),
                photos: Vec::new(),
                photo: None,
                find_photos: look.photos,
                fade: Fade { from: look.strength, to: look.strength, start: Instant::now(), ms: 0.0 },
                fade_in: look.fade_in,
                gap: None,
                last_render: Instant::now(),
                pending: false,
                last_rect: RECT::default(),
                shown: false,
                born: Instant::now(),
                first_frame_ms: None,
                owned,
                frames: 0,
                work_ms: 0.0,
            };
            layer.params.split = if look.split { 1.0 } else { 0.0 };
            // The screen copy must never see our own layers.
            layer.hide_from_capture(look.hidden || copy_screen);
            layer.set_gap(look.gap);
            layer.make_targets(gpu)?;
            Ok(layer)
        }
    }

    /// Screen sharing and screenshots see the real window, not the layer.
    pub fn hide_from_capture(&self, on: bool) {
        let to = if on { WDA_EXCLUDEFROMCAPTURE } else { WDA_NONE };
        unsafe {
            let _ = SetWindowDisplayAffinity(self.overlay, to);
        }
    }

    /// Draw at most every `gap` (on battery). Newer Windows can also capture
    /// less often, which saves more.
    pub fn set_gap(&mut self, gap: Option<Duration>) {
        self.gap = gap;
        if let Some(c) = &self.capture {
            c.set_gap(gap.map_or(0, |g| g.as_nanos() as i64 / 100));
        }
    }

    pub fn set_find_photos(&mut self, on: bool) {
        self.find_photos = on;
        if !on {
            self.photos.clear();
        }
    }

    fn make_targets(&mut self, gpu: &Gpu) -> Result<()> {
        unsafe {
            self.rtv = None;
            gpu.ctx.OMSetRenderTargets(None, None);
            let (w, h) = self.size;
            self.swap.ResizeBuffers(0, w, h, DXGI_FORMAT_UNKNOWN, DXGI_SWAP_CHAIN_FLAG(0))?;
            let bb: ID3D11Texture2D = self.swap.GetBuffer(0)?;
            let mut rtv = None;
            gpu.device.CreateRenderTargetView(&bb, None, Some(&mut rtv))?;
            self.rtv = rtv;
            // Full mip chain: the small levels give the window's average
            // lightness for almost nothing.
            let td = D3D11_TEXTURE2D_DESC {
                Width: w,
                Height: h,
                MipLevels: 0,
                ArraySize: 1,
                Format: DXGI_FORMAT_B8G8R8A8_UNORM,
                SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
                Usage: D3D11_USAGE_DEFAULT,
                BindFlags: (D3D11_BIND_SHADER_RESOURCE.0 | D3D11_BIND_RENDER_TARGET.0) as u32,
                MiscFlags: D3D11_RESOURCE_MISC_GENERATE_MIPS.0 as u32,
                ..Default::default()
            };
            let mut tex = None;
            gpu.device.CreateTexture2D(&td, None, Some(&mut tex))?;
            let tex = tex.unwrap();
            let mut srv = None;
            gpu.device.CreateShaderResourceView(&tex, None, Some(&mut srv))?;
            self.copy = Some((tex, srv.unwrap()));

            let mut lvl = 0u32;
            while (w >> (lvl + 1)) >= 48 && (h >> (lvl + 1)) >= 16 {
                lvl += 1;
            }
            let (pw, ph) = ((w >> lvl).max(1), (h >> lvl).max(1));
            let sd = D3D11_TEXTURE2D_DESC {
                Width: pw,
                Height: ph,
                MipLevels: 1,
                ArraySize: 1,
                Format: DXGI_FORMAT_B8G8R8A8_UNORM,
                SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
                Usage: D3D11_USAGE_STAGING,
                CPUAccessFlags: D3D11_CPU_ACCESS_READ.0 as u32,
                ..Default::default()
            };
            let mut st = None;
            gpu.device.CreateTexture2D(&sd, None, Some(&mut st))?;
            self.probe = st.map(|t| Probe { tex: t, level: lvl, w: pw, h: ph });

            // The photo rating, a quarter of the size each way.
            let (mw, mh) = (w.div_ceil(photos::BLOCK as u32), h.div_ceil(photos::BLOCK as u32));
            let md = D3D11_TEXTURE2D_DESC {
                Width: mw,
                Height: mh,
                MipLevels: 1,
                ArraySize: 1,
                Format: DXGI_FORMAT_R8_UNORM,
                SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
                Usage: D3D11_USAGE_DEFAULT,
                BindFlags: D3D11_BIND_RENDER_TARGET.0 as u32,
                ..Default::default()
            };
            let mut mt = None;
            gpu.device.CreateTexture2D(&md, None, Some(&mut mt))?;
            let mt = mt.unwrap();
            let mut mrtv = None;
            gpu.device.CreateRenderTargetView(&mt, None, Some(&mut mrtv))?;
            let staging_desc = D3D11_TEXTURE2D_DESC {
                BindFlags: 0,
                Usage: D3D11_USAGE_STAGING,
                CPUAccessFlags: D3D11_CPU_ACCESS_READ.0 as u32,
                ..md
            };
            let mut ms = None;
            gpu.device.CreateTexture2D(&staging_desc, None, Some(&mut ms))?;
            self.photo = Some(PhotoPass {
                rtv: mrtv.unwrap(),
                tex: mt,
                staging: ms.unwrap(),
                w: mw,
                h: mh,
                waiting: false,
                last: Instant::now() - PHOTO_EVERY,
                every: PHOTO_EVERY,
                seen: 0,
            });
            self.photos.clear();
        }
        Ok(())
    }

    /// Darkening strength right now.
    pub fn strength(&self) -> f32 {
        self.fade.value()
    }

    /// Fades to `to` (0 = real colours, 1 = dark) over `ms` milliseconds.
    pub fn fade_to(&mut self, to: f32, ms: f32) {
        if !self.shown {
            // Nothing on screen yet: start there.
            self.fade = Fade { from: to, to, start: Instant::now(), ms: 0.0 };
            self.fade_in = false;
            return;
        }
        self.fade = Fade { from: self.strength(), to, start: Instant::now(), ms };
    }

    pub fn fading(&self) -> bool {
        !self.fade.done()
    }

    /// Keeps the overlay over the target and directly above it in z-order.
    /// Returns false once the target is gone.
    pub fn sync(&mut self, enabled: bool) -> bool {
        unsafe {
            if !IsWindow(Some(self.target)).as_bool() {
                return false;
            }
            let hide = !enabled
                || !self.shown
                || (self.strength() <= 0.0 && !self.fading())
                || IsIconic(self.target).as_bool()
                || !IsWindowVisible(self.target).as_bool()
                || crate::win::is_cloaked(self.target);
            if hide {
                if IsWindowVisible(self.overlay).as_bool() {
                    let _ = ShowWindow(self.overlay, SW_HIDE);
                }
                return true;
            }
            let r = frame_rect(self.target);
            let above = GetWindow(self.target, GW_HWNDPREV).ok();
            let in_place = above == Some(self.overlay);
            if in_place && IsWindowVisible(self.overlay).as_bool() && r == self.last_rect {
                return true;
            }
            let after = match above {
                Some(h) if h != self.overlay => h,
                _ => HWND_TOP,
            };
            let mut flags = SWP_NOACTIVATE | SWP_SHOWWINDOW;
            if in_place {
                flags |= SWP_NOZORDER;
            }
            let _ = SetWindowPos(self.overlay, Some(after), r.left, r.top, width(&r), height(&r), flags);
            self.last_rect = r;
        }
        true
    }

    /// Takes the newest captured frame, if any, and draws it darkened.
    pub fn poll(&mut self, gpu: &Gpu, enabled: bool) -> Result<bool> {
        let Some(capture) = &self.capture else { return Ok(false) };
        let pool = capture.pool();
        let mut latest = None;
        while let Ok(f) = pool.TryGetNextFrame() {
            latest = Some(f);
        }
        let Some(frame) = latest else { return Ok(false) };
        let t = Instant::now();
        let cs = frame.ContentSize()?;
        let want = (cs.Width.max(1) as u32, cs.Height.max(1) as u32);
        if want != self.size {
            self.size = want;
            capture.resize(&gpu.rt_device, SizeInt32 { Width: want.0 as i32, Height: want.1 as i32 });
            self.make_targets(gpu)?;
        }
        unsafe {
            let access: IDirect3DDxgiInterfaceAccess = frame.Surface()?.cast()?;
            let tex: ID3D11Texture2D = access.GetInterface()?;
            let mut td = D3D11_TEXTURE2D_DESC::default();
            tex.GetDesc(&mut td);
            let bx = D3D11_BOX {
                left: 0,
                top: 0,
                front: 0,
                right: td.Width.min(self.size.0),
                bottom: td.Height.min(self.size.1),
                back: 1,
            };
            let (copy, srv) = self.copy.as_ref().unwrap();
            gpu.ctx.CopySubresourceRegion(copy, 0, 0, 0, 0, &tex, 0, Some(&bx));
            gpu.ctx.GenerateMips(srv);
        }
        let _ = frame.Close();
        self.after_copy(gpu, enabled, t)
    }

    /// Like poll(), for a layer that copies the screen: takes its part of
    /// the screen when that changed or the window moved.
    pub fn poll_screen(&mut self, gpu: &Gpu, desk: &Desktop, changed: &[RECT], enabled: bool) -> Result<bool> {
        let r = frame_rect(self.target);
        let want = (width(&r).max(1) as u32, height(&r).max(1) as u32);
        let touched = changed.iter().any(|c| c.left < r.right && c.right > r.left && c.top < r.bottom && c.bottom > r.top);
        if want == self.size && r == self.last_rect && self.shown && !touched {
            return Ok(false);
        }
        let t = Instant::now();
        if want != self.size {
            self.size = want;
            self.make_targets(gpu)?;
        }
        let (copy, srv) = self.copy.as_ref().unwrap();
        if !desk.copy(gpu, copy, r) {
            return Ok(false);
        }
        unsafe { gpu.ctx.GenerateMips(srv) };
        self.after_copy(gpu, enabled, t)
    }

    /// A new picture of the window is in `copy`: measure it, look for
    /// photos, draw.
    fn after_copy(&mut self, gpu: &Gpu, enabled: bool, t: Instant) -> Result<bool> {
        if !self.shown || self.last_probe.elapsed().as_millis() >= 500 {
            self.measure(gpu);
        }
        self.rate_photos(gpu);
        if self.shown && self.gap.is_some_and(|g| self.last_render.elapsed() < g) {
            // On battery: draw a little later, with whatever is newest then.
            self.pending = true;
        } else {
            self.render(gpu)?;
        }
        if !self.shown {
            self.shown = true;
            self.first_frame_ms = Some(self.born.elapsed().as_secs_f64() * 1000.0);
            if self.fade_in {
                // Appear with the real colours, then darken.
                let to = self.fade.to;
                self.fade = Fade { from: 0.0, to, start: Instant::now(), ms: crate::engine::FADE_MS };
                self.fade_in = false;
            }
            self.sync(enabled);
        }
        self.frames += 1;
        self.work_ms += t.elapsed().as_secs_f64() * 1000.0;
        Ok(true)
    }

    /// Between frames: draws again while fading, when a delayed frame is
    /// due, or when the photo search found something new. True if it drew.
    pub fn tick(&mut self, gpu: &Gpu) -> bool {
        if !self.shown {
            return false;
        }
        let photos_changed = self.read_photos(gpu);
        let due = self.pending && self.gap.is_none_or(|g| self.last_render.elapsed() >= g);
        if self.fading() || due || photos_changed {
            let _ = self.render(gpu);
            return true;
        }
        false
    }

    /// Rates the window's blocks for looking like a photo, now and then.
    fn rate_photos(&mut self, gpu: &Gpu) {
        if !self.find_photos || self.params.amount <= 0.0 {
            return;
        }
        let Some(p) = self.photo.as_mut() else { return };
        if p.waiting || p.last.elapsed() < p.every {
            return;
        }
        let Some((_, srv)) = &self.copy else { return };
        unsafe {
            gpu.ctx.OMSetRenderTargets(Some(&[Some(p.rtv.clone())]), None);
            let vp = D3D11_VIEWPORT { Width: p.w as f32, Height: p.h as f32, MaxDepth: 1.0, ..Default::default() };
            gpu.ctx.RSSetViewports(Some(&[vp]));
            gpu.ctx.IASetPrimitiveTopology(D3D_PRIMITIVE_TOPOLOGY_TRIANGLELIST);
            gpu.ctx.VSSetShader(&gpu.vs, None);
            gpu.ctx.PSSetShader(&gpu.photo, None);
            gpu.ctx.PSSetShaderResources(0, Some(&[Some(srv.clone())]));
            gpu.ctx.Draw(3, 0);
            gpu.ctx.CopyResource(&p.staging, &p.tex);
        }
        p.waiting = true;
        p.last = Instant::now();
    }

    /// Picks up the photo rating once the GPU has it, without waiting for
    /// it. True if the photos found changed.
    fn read_photos(&mut self, gpu: &Gpu) -> bool {
        let Some(p) = self.photo.as_mut() else { return false };
        if !p.waiting {
            return false;
        }
        let mut m = D3D11_MAPPED_SUBRESOURCE::default();
        let mapped = unsafe { gpu.ctx.Map(&p.staging, 0, D3D11_MAP_READ, D3D11_MAP_FLAG_DO_NOT_WAIT.0 as u32, Some(&mut m)) };
        if mapped.is_err() {
            return false;
        }
        let cells = unsafe { std::slice::from_raw_parts(m.pData as *const u8, (m.RowPitch * p.h) as usize) };
        // The same rating as last time (the usual case once a window sits
        // still): nothing to work out, and look less often.
        let seen = cells.iter().fold(0xcbf29ce484222325u64, |h, b| (h ^ *b as u64).wrapping_mul(0x100000001b3));
        if seen == p.seen {
            unsafe { gpu.ctx.Unmap(&p.staging, 0) };
            p.waiting = false;
            p.every = (p.every * 2).min(PHOTO_CALM);
            return false;
        }
        p.seen = seen;
        p.every = PHOTO_EVERY;
        let found = photos::find(cells, p.w as usize, p.h as usize, m.RowPitch as usize);
        unsafe { gpu.ctx.Unmap(&p.staging, 0) };
        p.waiting = false;
        if !self.find_photos || found == self.photos {
            return false;
        }
        self.photos = found;
        true
    }

    fn render(&mut self, gpu: &Gpu) -> Result<()> {
        let (w, h) = (self.size.0 as i32, self.size.1 as i32);
        let drawn = self.manual.iter().map(|a| RECT { left: a.left, top: a.top, right: w - a.right, bottom: h - a.bottom });
        let all: Vec<RECT> = self
            .protect
            .iter()
            .copied()
            .chain(drawn)
            .chain(self.photos.iter().copied())
            .filter(|r| r.right > r.left && r.bottom > r.top)
            .collect();
        let n = all.len().min(MAX_RECTS);
        self.params.count = n as u32;
        for (i, r) in all.iter().take(n).enumerate() {
            self.params.rects[i] = [r.left as f32, r.top as f32, r.right as f32, r.bottom as f32];
        }
        let mut p = self.params;
        p.split = if self.params.split > 0.0 { self.size.0 as f32 / 2.0 } else { 0.0 };
        p.strength = self.strength();
        unsafe {
            gpu.ctx.UpdateSubresource(&gpu.cb, 0, None, &p as *const Params as *const c_void, 0, 0);
            gpu.ctx.OMSetRenderTargets(Some(&[self.rtv.clone()]), None);
            let vp = D3D11_VIEWPORT {
                Width: self.size.0 as f32,
                Height: self.size.1 as f32,
                MaxDepth: 1.0,
                ..Default::default()
            };
            gpu.ctx.RSSetViewports(Some(&[vp]));
            gpu.ctx.IASetPrimitiveTopology(D3D_PRIMITIVE_TOPOLOGY_TRIANGLELIST);
            gpu.ctx.VSSetShader(&gpu.vs, None);
            gpu.ctx.PSSetShader(&gpu.ps, None);
            let srv = self.copy.as_ref().map(|(_, s)| s.clone());
            gpu.ctx.PSSetShaderResources(0, Some(&[srv]));
            gpu.ctx.PSSetConstantBuffers(0, Some(&[Some(gpu.cb.clone())]));
            gpu.ctx.Draw(3, 0);
            self.swap.Present(0, DXGI_PRESENT(0)).ok()?;
        }
        self.last_render = Instant::now();
        self.pending = false;
        Ok(())
    }

    /// Average lightness outside the protected areas decides whether the
    /// window needs darkening at all: a window that is dark already (a dark
    /// themed system dialog, say) is shown as it is. Two thresholds keep the
    /// decision from flickering.
    fn measure(&mut self, gpu: &Gpu) {
        self.last_probe = Instant::now();
        let (Some(p), Some((copy, _))) = (&self.probe, &self.copy) else { return };
        unsafe {
            gpu.ctx.CopySubresourceRegion(&p.tex, 0, 0, 0, 0, copy, p.level, None);
            let mut m = D3D11_MAPPED_SUBRESOURCE::default();
            if gpu.ctx.Map(&p.tex, 0, D3D11_MAP_READ, 0, Some(&mut m)).is_err() {
                return;
            }
            let mut sum = 0.0f64;
            let mut n = 0u32;
            for y in 0..p.h {
                let row = std::slice::from_raw_parts((m.pData as *const u8).add((y * m.RowPitch) as usize), (p.w * 4) as usize);
                for x in 0..p.w {
                    let (fx, fy) = ((x << p.level) as i32, (y << p.level) as i32);
                    let inside = |r: &RECT| fx >= r.left && fx < r.right && fy >= r.top && fy < r.bottom;
                    if self.protect.iter().any(inside) || self.photos.iter().any(inside) {
                        continue;
                    }
                    let px = &row[(x * 4) as usize..(x * 4 + 4) as usize];
                    let lin = |v: u8| (v as f64 / 255.0).powf(2.2);
                    let yl = 0.0722 * lin(px[0]) + 0.7152 * lin(px[1]) + 0.2126 * lin(px[2]);
                    sum += yl.cbrt();
                    n += 1;
                }
            }
            gpu.ctx.Unmap(&p.tex, 0);
            if n == 0 {
                return;
            }
            let avg = sum / n as f64;
            let was = self.params.amount;
            let on = if std::env::var_os("DEONYX_FORCE_DARKEN").is_some() {
                true
            } else if !self.measured {
                avg >= 0.46
            } else if was > 0.0 {
                avg >= 0.42
            } else {
                avg > 0.50
            };
            self.params.amount = if on { 1.0 } else { 0.0 };
            if !on && (was > 0.0 || !self.measured) {
                println!("  dark already, left alone: {} (lightness {:.2})", crate::win::class_name(self.target), avg);
            }
            self.measured = true;
        }
    }

    /// The window was found to be dark already and is shown as it is.
    pub fn left_as_is(&self) -> bool {
        self.measured && self.params.amount <= 0.0
    }

    /// Redraws the last frame, e.g. after the protected areas changed.
    pub fn redraw(&mut self, gpu: &Gpu) {
        if self.shown {
            let _ = self.render(gpu);
        }
    }
}

impl Drop for Layer {
    fn drop(&mut self) {
        // The capture closes itself on its own thread (wgc.rs).
        unsafe {
            let _ = DestroyWindow(self.overlay);
        }
    }
}
