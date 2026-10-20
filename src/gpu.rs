// The one D3D11 device every layer shares, and the dark-mode shader.

use std::ffi::c_void;
use std::mem::size_of;

use windows::core::{Interface, Result, PCSTR};
use windows::Graphics::DirectX::Direct3D11::IDirect3DDevice;
use windows::Win32::Foundation::HMODULE;
use windows::Win32::Graphics::Direct3D::Fxc::D3DCompile;
use windows::Win32::Graphics::Direct3D::*;
use windows::Win32::Graphics::Direct3D11::*;
use windows::Win32::Graphics::DirectComposition::{DCompositionCreateDevice, IDCompositionDevice};
use windows::Win32::Graphics::Dxgi::*;
use windows::Win32::System::WinRT::Direct3D11::CreateDirect3D11DeviceFromDXGIDevice;

const SHADER: &str = include_str!("shader.hlsl");
pub const MAX_RECTS: usize = 8;

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
    pub pad: [f32; 3],
}

pub struct Gpu {
    pub device: ID3D11Device,
    pub ctx: ID3D11DeviceContext,
    pub rt_device: IDirect3DDevice,
    pub factory: IDXGIFactory2,
    pub vs: ID3D11VertexShader,
    pub ps: ID3D11PixelShader,
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
            let mut vs = None;
            let mut ps = None;
            device.CreateVertexShader(bytes(&vsb), None, Some(&mut vs))?;
            device.CreatePixelShader(bytes(&psb), None, Some(&mut ps))?;
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
                cb: cb.unwrap(),
                dcomp,
            })
        }
    }
}
