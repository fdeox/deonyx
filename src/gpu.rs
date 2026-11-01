// The one D3D11 device every layer shares, and the dark-mode shaders.

use std::ffi::c_void;
use std::mem::size_of;

use windows::core::{Interface, Result, PCSTR};
use windows::Graphics::DirectX::Direct3D11::IDirect3DDevice;
use windows::Win32::Foundation::HMODULE;
use windows::Win32::Graphics::Direct3D::Fxc::D3DCompile;
use windows::Win32::Graphics::Direct3D::*;
use windows::Win32::Graphics::Direct3D11::*;
use windows::Win32::Graphics::DirectComposition::{DCompositionCreateDevice, IDCompositionDevice};
use windows::Win32::Graphics::Dxgi::Common::{DXGI_FORMAT_R32_FLOAT, DXGI_SAMPLE_DESC};
use windows::Win32::Graphics::Dxgi::*;
use windows::Win32::System::WinRT::Direct3D11::CreateDirect3D11DeviceFromDXGIDevice;

const SHADER: &str = include_str!("shader.hlsl");
pub const MAX_RECTS: usize = 32;

#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct Params {
    pub rects: [[f32; 4]; MAX_RECTS],
    pub count: u32,
    pub lmin: f32,
    pub lmax: f32,
    pub sat: f32,
    pub split: f32,
    pub amount: f32,
    pub fringe: f32,
    pub holes: f32,
    pub warm: f32,
    /// 0..1, how strongly the darkening shows (fades in and out).
    pub strength: f32,
    /// Curve on the flipped lightness: 1 leaves it, below 1 lifts faint text.
    pub boost: f32,
    /// Windows that are dark already (amount 0) are dimmed this much rather
    /// than shown as they are; 0 leaves them.
    pub dim: f32,
    /// 1: a frame that is clearly dark or clearly light on average decides
    /// at once, on the GPU (see shader.hlsl).
    pub auto: f32,
    pub _pad: [f32; 3],
    /// A colour theme (config::tone): background a, b and text a, b in
    /// OKLab; zeros keep the greys grey.
    pub tone: [f32; 4],
}

pub struct Gpu {
    pub device: ID3D11Device,
    pub ctx: ID3D11DeviceContext,
    pub rt_device: IDirect3DDevice,
    pub factory: IDXGIFactory2,
    pub vs: ID3D11VertexShader,
    pub ps: ID3D11PixelShader,
    /// Rates 4x4 blocks for looking like a photo (see photos.rs).
    pub photo: ID3D11PixelShader,
    /// Measures a frame's lightness into one pixel, which the main pass
    /// reads (shader.hlsl level, amount_now). Shared: layers draw one after
    /// another.
    pub level: ID3D11PixelShader,
    pub level_rtv: ID3D11RenderTargetView,
    pub level_srv: ID3D11ShaderResourceView,
    pub cb: ID3D11Buffer,
    /// See-through layers go through DirectComposition. None falls back to
    /// opaque layers (DEONYX_OPAQUE=1, or if it isn't available).
    pub dcomp: Option<IDCompositionDevice>,
}

fn compile(entry: &[u8], target: &[u8]) -> Result<ID3DBlob> {
    let mut code = None;
    let mut err = None;
    let r = unsafe {
        D3DCompile(
            SHADER.as_ptr() as *const c_void,
            SHADER.len(),
            PCSTR::null(),
            None,
            None,
            PCSTR(entry.as_ptr()),
            PCSTR(target.as_ptr()),
            0,
            0,
            &mut code,
            Some(&mut err),
        )
    };
    if let Err(e) = r {
        if let Some(b) = err {
            eprintln!("{}", String::from_utf8_lossy(bytes(&b)));
        }
        return Err(e);
    }
    Ok(code.unwrap())
}

fn bytes(b: &ID3DBlob) -> &[u8] {
    unsafe { std::slice::from_raw_parts(b.GetBufferPointer() as *const u8, b.GetBufferSize()) }
}

impl Gpu {
    pub fn new() -> Result<Gpu> {
        unsafe {
            let mut device = None;
            let mut ctx = None;
            D3D11CreateDevice(
                None,
                D3D_DRIVER_TYPE_HARDWARE,
                HMODULE::default(),
                D3D11_CREATE_DEVICE_BGRA_SUPPORT,
                None,
                D3D11_SDK_VERSION,
                Some(&mut device),
                None,
                Some(&mut ctx),
            )?;
            let device: ID3D11Device = device.unwrap();
            let dxgi: IDXGIDevice = device.cast()?;
            let rt_device: IDirect3DDevice = CreateDirect3D11DeviceFromDXGIDevice(&dxgi)?.cast()?;
            let factory: IDXGIFactory2 = CreateDXGIFactory2(DXGI_CREATE_FACTORY_FLAGS(0))?;
            let dcomp = if std::env::var_os("DEONYX_OPAQUE").is_some() {
                None
            } else {
                DCompositionCreateDevice::<_, IDCompositionDevice>(&dxgi).ok()
            };

            let vsb = compile(b"vs\0", b"vs_5_0\0")?;
            let psb = compile(b"ps\0", b"ps_5_0\0")?;
            let phb = compile(b"photo\0", b"ps_5_0\0")?;
            let lvb = compile(b"level\0", b"ps_5_0\0")?;
            let mut level = None;
            device.CreatePixelShader(bytes(&lvb), None, Some(&mut level))?;
            let ld = D3D11_TEXTURE2D_DESC {
                Width: 1,
                Height: 1,
                MipLevels: 1,
                ArraySize: 1,
                Format: DXGI_FORMAT_R32_FLOAT,
                SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
                Usage: D3D11_USAGE_DEFAULT,
                BindFlags: (D3D11_BIND_SHADER_RESOURCE.0 | D3D11_BIND_RENDER_TARGET.0) as u32,
                ..Default::default()
            };
            let mut lt = None;
            device.CreateTexture2D(&ld, None, Some(&mut lt))?;
            let lt = lt.unwrap();
            let (mut level_rtv, mut level_srv) = (None, None);
            device.CreateRenderTargetView(&lt, None, Some(&mut level_rtv))?;
            device.CreateShaderResourceView(&lt, None, Some(&mut level_srv))?;
            let mut vs = None;
            let mut ps = None;
            let mut photo = None;
            device.CreateVertexShader(bytes(&vsb), None, Some(&mut vs))?;
            device.CreatePixelShader(bytes(&psb), None, Some(&mut ps))?;
            device.CreatePixelShader(bytes(&phb), None, Some(&mut photo))?;
            let bd = D3D11_BUFFER_DESC {
                ByteWidth: size_of::<Params>() as u32,
                Usage: D3D11_USAGE_DEFAULT,
                BindFlags: D3D11_BIND_CONSTANT_BUFFER.0 as u32,
                ..Default::default()
            };
            let mut cb = None;
            device.CreateBuffer(&bd, None, Some(&mut cb))?;
            Ok(Gpu {
                device,
                ctx: ctx.unwrap(),
                rt_device,
                factory,
                vs: vs.unwrap(),
                ps: ps.unwrap(),
                photo: photo.unwrap(),
                level: level.unwrap(),
                level_rtv: level_rtv.unwrap(),
                level_srv: level_srv.unwrap(),
                cb: cb.unwrap(),
                dcomp,
            })
        }
    }
}
