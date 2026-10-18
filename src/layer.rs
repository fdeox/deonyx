// One darkened window: its capture, and the click-through overlay that shows
// the result right above it.

use std::ffi::c_void;
use std::time::Instant;

use windows::core::{factory, w, Interface, Result};
use windows::Graphics::Capture::{Direct3D11CaptureFramePool, GraphicsCaptureItem, GraphicsCaptureSession};
use windows::Graphics::DirectX::DirectXPixelFormat;
use windows::Graphics::SizeInt32;
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Direct3D::D3D_PRIMITIVE_TOPOLOGY_TRIANGLELIST;
use windows::Win32::Graphics::Direct3D11::*;
use windows::Win32::Graphics::Dxgi::Common::*;
use windows::Win32::Graphics::Dxgi::*;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::WinRT::Direct3D11::IDirect3DDxgiInterfaceAccess;
use windows::Win32::System::WinRT::Graphics::Capture::IGraphicsCaptureItemInterop;
use windows::Win32::UI::WindowsAndMessaging::*;

use crate::gpu::{Gpu, Params, MAX_RECTS};
use crate::win::{frame_rect, height, width};

pub const OVERLAY_CLASS: &str = "DeonyxOverlay";

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
    pub split: bool,
    pub owner: bool,
}

/// A CPU-readable copy of a small mip level, for measuring lightness.
struct Probe {
    tex: ID3D11Texture2D,
    level: u32,
    w: u32,
    h: u32,
}

pub struct Layer {
    pub target: HWND,
    pub overlay: HWND,
    pool: Direct3D11CaptureFramePool,
    session: GraphicsCaptureSession,
    swap: IDXGISwapChain1,
    rtv: Option<ID3D11RenderTargetView>,
    copy: Option<(ID3D11Texture2D, ID3D11ShaderResourceView)>,
    probe: Option<Probe>,
    last_probe: Instant,
    measured: bool,
    size: (u32, u32),
    pub params: Params,
    /// Protected areas relative to the window frame.
    pub protect: Vec<RECT>,
    last_rect: RECT,
    shown: bool,
    pub born: Instant,
    pub first_frame_ms: Option<f64>,
    pub frames: u32,
    pub work_ms: f64,
}

impl Layer {
    pub fn new(gpu: &Gpu, target: HWND, look: &Look) -> Result<Layer> {
        unsafe {
            let interop = factory::<GraphicsCaptureItem, IGraphicsCaptureItemInterop>()?;
            let item: GraphicsCaptureItem = interop.CreateForWindow(target)?;
            let start = item.Size()?;
            let pool = Direct3D11CaptureFramePool::CreateFreeThreaded(
                &gpu.rt_device,
                DirectXPixelFormat::B8G8R8A8UIntNormalized,
                2,
                start,
            )?;
            let session = pool.CreateCaptureSession(&item)?;
            let _ = session.SetIsCursorCaptureEnabled(false);
            let _ = session.SetIsBorderRequired(false);

            let r = frame_rect(target);
            let hinst = GetModuleHandleW(None)?;
            let overlay = CreateWindowExW(
                WS_EX_LAYERED | WS_EX_TRANSPARENT | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE,
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
                SwapEffect: DXGI_SWAP_EFFECT_FLIP_DISCARD,
                AlphaMode: DXGI_ALPHA_MODE_IGNORE,
                ..Default::default()
            };
            let swap = gpu.factory.CreateSwapChainForHwnd(&gpu.device, overlay, &desc, None, None)?;
            let _ = gpu.factory.MakeWindowAssociation(overlay, DXGI_MWA_NO_ALT_ENTER);

            let mut layer = Layer {
                target,
                overlay,
                pool,
                session,
                swap,
                rtv: None,
                copy: None,
                probe: None,
                last_probe: Instant::now(),
                measured: false,
                size: (w0, h0),
                params: Params { lmin: look.lmin, lmax: look.lmax, sat: 0.95, amount: 1.0, ..Default::default() },
                protect: Vec::new(),
                last_rect: RECT::default(),
                shown: false,
                born: Instant::now(),
                first_frame_ms: None,
                frames: 0,
                work_ms: 0.0,
            };
            layer.params.split = if look.split { 1.0 } else { 0.0 };
            layer.make_targets(gpu)?;
            layer.session.StartCapture()?;
            Ok(layer)
        }
    }

    pub fn on_frame_arrived(&self, ev: HANDLE) -> Result<()> {
        let evv = ev.0 as isize;
        self.pool.FrameArrived(&windows::Foundation::TypedEventHandler::new(move |_, _| {
            unsafe {
                let _ = windows::Win32::System::Threading::SetEvent(HANDLE(evv as *mut c_void));
            }
            Ok(())
        }))?;
        Ok(())
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
        }
        Ok(())
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
        let mut latest = None;
        while let Ok(f) = self.pool.TryGetNextFrame() {
            latest = Some(f);
        }
        let Some(frame) = latest else { return Ok(false) };
        let t = Instant::now();
        let cs = frame.ContentSize()?;
        let want = (cs.Width.max(1) as u32, cs.Height.max(1) as u32);
        if want != self.size {
            self.size = want;
            self.pool.Recreate(
                &gpu.rt_device,
                DirectXPixelFormat::B8G8R8A8UIntNormalized,
                2,
                SizeInt32 { Width: want.0 as i32, Height: want.1 as i32 },
            )?;
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
        if !self.shown || self.last_probe.elapsed().as_millis() >= 500 {
            self.measure(gpu);
        }
        self.render(gpu)?;
        if !self.shown {
            self.shown = true;
            self.first_frame_ms = Some(self.born.elapsed().as_secs_f64() * 1000.0);
            self.sync(enabled);
        }
        self.frames += 1;
        self.work_ms += t.elapsed().as_secs_f64() * 1000.0;
        Ok(true)
    }

    fn render(&mut self, gpu: &Gpu) -> Result<()> {
        let n = self.protect.len().min(MAX_RECTS);
        self.params.count = n as u32;
        for (i, r) in self.protect.iter().take(n).enumerate() {
            self.params.rects[i] = [r.left as f32, r.top as f32, r.right as f32, r.bottom as f32];
        }
        let mut p = self.params;
        p.split = if self.params.split > 0.0 { self.size.0 as f32 / 2.0 } else { 0.0 };
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
                    if self.protect.iter().any(|r| fx >= r.left && fx < r.right && fy >= r.top && fy < r.bottom) {
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
            let on = if !self.measured {
                avg >= 0.46
            } else if was > 0.0 {
                avg >= 0.42
            } else {
                avg > 0.50
            };
            self.params.amount = if on { 1.0 } else { 0.0 };
            if !on && (was > 0.0 || !self.measured) {
                println!("  zaten koyu, dokunulmadi: {} (aydinlik {:.2})", crate::win::class_name(self.target), avg);
            }
            self.measured = true;
        }
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
        let _ = self.session.Close();
        let _ = self.pool.Close();
        unsafe {
            let _ = DestroyWindow(self.overlay);
        }
    }
}
