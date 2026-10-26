// Windows.Graphics.Capture calls that talk to Windows' capture service.
// That service can get stuck (seen after a Store copy's first permission
// questions): from then on every capture call, in every program, waits for
// good until Windows restarts. So those calls run on a thread of their own:
// starting a capture waits a few seconds at most, after which stuck() says
// so and the engine copies the screen instead (desk.rs); closing a capture
// or changing its pace doesn't wait at all. Frames are still taken from the
// pool right where they're drawn.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{channel, Sender};
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use windows::core::{factory, Result};
use windows::Foundation::{TimeSpan, TypedEventHandler};
use windows::Graphics::Capture::{Direct3D11CaptureFramePool, GraphicsCaptureItem, GraphicsCaptureSession};
use windows::Graphics::DirectX::Direct3D11::IDirect3DDevice;
use windows::Graphics::DirectX::DirectXPixelFormat;
use windows::Graphics::SizeInt32;
use windows::Win32::Foundation::{E_ACCESSDENIED, HANDLE, HWND};
use windows::Win32::System::Com::{CoInitializeEx, COINIT_MULTITHREADED};
use windows::Win32::System::Threading::SetEvent;
use windows::Win32::System::WinRT::Graphics::Capture::IGraphicsCaptureItemInterop;

/// How long starting a capture may take before the service counts as stuck.
const WAIT: Duration = Duration::from_secs(3);

static STUCK: AtomicBool = AtomicBool::new(false);

/// Windows' capture service stopped answering; it stays so until Windows
/// restarts.
pub fn stuck() -> bool {
    STUCK.load(Ordering::Relaxed)
}

type Job = Box<dyn FnOnce() + Send>;

fn later(job: impl FnOnce() + Send + 'static) {
    static JOBS: OnceLock<Sender<Job>> = OnceLock::new();
    let jobs = JOBS.get_or_init(|| {
        let (tx, rx) = channel::<Job>();
        let _ = std::thread::Builder::new().name("deonyx-capture".into()).spawn(move || {
            unsafe {
                let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
            }
            for job in rx {
                job();
            }
        });
        tx
    });
    let _ = jobs.send(Box::new(job));
}

/// The D3D11 device behind it is free-threaded; only the wrapper isn't marked.
struct Device(IDirect3DDevice);
unsafe impl Send for Device {}

/// One window's capture. New frames land in pool() and set the event.
pub struct Capture {
    inner: Option<(Direct3D11CaptureFramePool, GraphicsCaptureSession)>,
}

impl Capture {
    /// Starts capturing `target`. Fails if the window can't be captured, or
    /// if the service doesn't answer in time (stuck() then says so).
    /// DEONYX_CAPTURE_STUCK=1 makes a test copy's service never answer.
    pub fn start(target: HWND, device: &IDirect3DDevice, frame_event: HANDLE) -> Result<(Capture, SizeInt32)> {
        if stuck() {
            return Err(E_ACCESSDENIED.into());
        }
        let hang = std::env::var_os("DEONYX_CONFIG_DIR").is_some() && std::env::var_os("DEONYX_CAPTURE_STUCK").is_some();
        let device = Device(device.clone());
        let (window, event) = (target.0 as isize, frame_event.0 as isize);
        let (tx, rx) = channel();
        let started = Instant::now();
        later(move || {
            if hang {
                std::thread::sleep(Duration::from_secs(3600));
            }
            let go = move || -> Result<(Capture, SizeInt32)> {
                let device = device;
                let interop = factory::<GraphicsCaptureItem, IGraphicsCaptureItemInterop>()?;
                let item: GraphicsCaptureItem = unsafe { interop.CreateForWindow(HWND(window as *mut _))? };
                let size = item.Size()?;
                let pool = Direct3D11CaptureFramePool::CreateFreeThreaded(&device.0, DirectXPixelFormat::B8G8R8A8UIntNormalized, 2, size)?;
                pool.FrameArrived(&TypedEventHandler::new(move |_, _| {
                    unsafe {
                        let _ = SetEvent(HANDLE(event as *mut _));
                    }
                    Ok(())
                }))?;
                let session = pool.CreateCaptureSession(&item)?;
                let _ = session.SetIsCursorCaptureEnabled(false);
                let _ = session.SetIsBorderRequired(false);
                session.StartCapture()?;
                Ok((Capture { inner: Some((pool, session)) }, size))
            };
            let _ = tx.send(go());
        });
        match rx.recv_timeout(WAIT) {
            Ok(r) => r,
            Err(_) => {
                crate::note!(
                    "window capture didn't answer in {} s: Windows' capture service is stuck until Windows restarts",
                    started.elapsed().as_secs()
                );
                STUCK.store(true, Ordering::Relaxed);
                Err(E_ACCESSDENIED.into())
            }
        }
    }

    pub fn pool(&self) -> &Direct3D11CaptureFramePool {
        &self.inner.as_ref().unwrap().0
    }

    /// New frames at most every `ticks` (100 ns), 0 = as they come.
    pub fn set_gap(&self, ticks: i64) {
        let session = self.inner.as_ref().unwrap().1.clone();
        later(move || {
            let _ = session.SetMinUpdateInterval(TimeSpan { Duration: ticks });
        });
    }

    /// Frames at a new size from now on.
    pub fn resize(&self, device: &IDirect3DDevice, size: SizeInt32) {
        let pool = self.pool().clone();
        let device = Device(device.clone());
        later(move || {
            let device = device;
            let _ = pool.Recreate(&device.0, DirectXPixelFormat::B8G8R8A8UIntNormalized, 2, size);
        });
    }
}

impl Drop for Capture {
    fn drop(&mut self) {
        if let Some((pool, session)) = self.inner.take() {
            later(move || {
                let _ = session.Close();
                let _ = pool.Close();
            });
        }
    }
}
