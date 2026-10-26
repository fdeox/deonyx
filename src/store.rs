// Deonyx from the Microsoft Store runs as a package: the Store updates it,
// Windows starts it with the user through the package's startup task (not
// the Run key), and the administrator task isn't there. The same exe works
// both ways; this tells which one it is.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::RecvTimeoutError;
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use windows::core::{factory, Result, HSTRING};
use windows::ApplicationModel::Activation::ActivationKind;
use windows::ApplicationModel::{AppInstance, StartupTask, StartupTaskState};
use windows::Graphics::Capture::{Direct3D11CaptureFramePool, GraphicsCaptureAccess, GraphicsCaptureAccessKind, GraphicsCaptureItem};
use windows::Graphics::DirectX::Direct3D11::IDirect3DDevice;
use windows::Graphics::DirectX::DirectXPixelFormat;
use windows::Security::Authorization::AppCapabilityAccess::{AppCapability, AppCapabilityAccessStatus};
use windows::Win32::Foundation::{APPMODEL_ERROR_NO_PACKAGE, HWND};
use windows::Win32::System::WinRT::Graphics::Capture::IGraphicsCaptureItemInterop;
use windows::Win32::Storage::Packaging::Appx::GetCurrentPackageFullName;
use windows::Win32::System::Com::{CoInitializeEx, COINIT_MULTITHREADED};

/// The startup task's id in the package manifest (store/AppxManifest.xml).
const TASK: &str = "DeonyxStartup";

/// Running from a package, so from the Microsoft Store.
pub fn packaged() -> bool {
    static PACKAGED: OnceLock<bool> = OnceLock::new();
    *PACKAGED.get_or_init(|| {
        let mut len = 0u32;
        unsafe { GetCurrentPackageFullName(&mut len, None) != APPMODEL_ERROR_NO_PACKAGE }
    })
}

/// Runs a Windows Runtime call on a thread of its own with COM ready, the
/// way those calls want it.
fn on_own_thread<T: Send + 'static>(f: impl FnOnce() -> Result<T> + Send + 'static) -> Result<T> {
    std::thread::spawn(move || {
        unsafe {
            let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
        }
        f()
    })
    .join()
    .unwrap_or_else(|_| Err(windows::core::Error::empty()))
}

fn task() -> Result<StartupTask> {
    StartupTask::GetAsync(&HSTRING::from(TASK))?.get()
}

fn on(s: StartupTaskState) -> bool {
    s == StartupTaskState::Enabled || s == StartupTaskState::EnabledByPolicy
}

/// Whether windows can be captured one by one (Windows.Graphics.Capture)
/// without the yellow border Windows draws around them. Programs outside a
/// package always may. A Store copy needs the user's yes twice (Settings >
/// Privacy & security > Screenshots and apps, and Screenshot borders): each
/// asked once, the first time, and the answers stick. Without both, layers
/// copy the screen instead (desk.rs), which needs neither and shows no
/// border. A window must never be captured without the first yes: Windows
/// then holds the call while it waits for an answer it never asks for.
pub fn capture_allowed() -> bool {
    static ALLOWED: OnceLock<bool> = OnceLock::new();
    *ALLOWED.get_or_init(|| {
        if !packaged() {
            return true;
        }
        // Test copies never wait on a question nobody is there to answer.
        let ask = std::env::var_os("DEONYX_CONFIG_DIR").is_none();
        // Nobody answering the question within a few minutes counts as no.
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let _ = tx.send(on_own_thread(move || {
                let access = |name: &str, kind: GraphicsCaptureAccessKind| -> Result<AppCapabilityAccessStatus> {
                    let now = AppCapability::Create(&HSTRING::from(name))?.CheckAccess()?;
                    if now == AppCapabilityAccessStatus::UserPromptRequired && ask {
                        return GraphicsCaptureAccess::RequestAccessAsync(kind)?.get();
                    }
                    Ok(now)
                };
                let programmatic = access("graphicsCaptureProgrammatic", GraphicsCaptureAccessKind::Programmatic)?;
                let borderless = access("graphicsCaptureWithoutBorder", GraphicsCaptureAccessKind::Borderless)?;
                Ok((programmatic, borderless))
            }));
        });
        let status = rx.recv_timeout(Duration::from_secs(180)).unwrap_or_else(|_| Err(windows::core::Error::new(
            windows::Win32::Foundation::E_ACCESSDENIED,
            "no answer",
        )));
        match &status {
            Ok((p, b)) => crate::note!("capture access: windows {}, without the border {}", p.0, b.0),
            Err(e) => crate::note!("capture access unknown: {e}"),
        }
        status.is_ok_and(|(p, b)| p == AppCapabilityAccessStatus::Allowed && b == AppCapabilityAccessStatus::Allowed)
    })
}

/// Set once window capture didn't answer in time (capture_answers).
static STUCK: AtomicBool = AtomicBool::new(false);

pub fn capture_stuck() -> bool {
    STUCK.load(Ordering::Relaxed)
}

/// Whether capturing a window really answers. Windows' capture service can
/// get stuck (seen after a Store copy's first permission questions): from
/// then on every capture, in every program, waits for good until Windows
/// restarts, and it would take the whole engine with it. So the first
/// capture is tried on a thread of its own; if it hasn't started within a
/// few seconds, layers copy the screen instead (the engine switches when it
/// sees capture_stuck). DEONYX_CAPTURE_PROBE=hang in a test copy pretends
/// Windows never answers.
pub fn capture_answers(target: HWND, device: &IDirect3DDevice) -> bool {
    static ANSWERED: AtomicBool = AtomicBool::new(false);
    if STUCK.load(Ordering::Relaxed) {
        return false;
    }
    if ANSWERED.load(Ordering::Relaxed) {
        return true;
    }
    let hang = std::env::var_os("DEONYX_CONFIG_DIR").is_some() && std::env::var("DEONYX_CAPTURE_PROBE").as_deref() == Ok("hang");
    struct Device(IDirect3DDevice);
    // The D3D11 device behind it is free-threaded; only the wrapper isn't marked.
    unsafe impl Send for Device {}
    let device = Device(device.clone());
    let window = target.0 as isize;
    let (tx, rx) = std::sync::mpsc::channel();
    let started = Instant::now();
    std::thread::spawn(move || {
        unsafe {
            let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
        }
        if hang {
            std::thread::sleep(Duration::from_secs(3600));
        }
        let try_it = move || -> Result<()> {
            let device = device;
            let interop = factory::<GraphicsCaptureItem, IGraphicsCaptureItemInterop>()?;
            let item: GraphicsCaptureItem = unsafe { interop.CreateForWindow(HWND(window as *mut _))? };
            let pool = Direct3D11CaptureFramePool::CreateFreeThreaded(&device.0, DirectXPixelFormat::B8G8R8A8UIntNormalized, 1, item.Size()?)?;
            // This is where a stuck service holds the call.
            let session = pool.CreateCaptureSession(&item)?;
            let _ = session.SetIsBorderRequired(false);
            session.StartCapture()?;
            let _ = session.Close();
            let _ = pool.Close();
            Ok(())
        };
        let _ = tx.send(try_it());
    });
    match rx.recv_timeout(Duration::from_secs(4)) {
        Ok(Ok(())) => {
            crate::note!("window capture answers in {} ms", started.elapsed().as_millis());
            ANSWERED.store(true, Ordering::Relaxed);
            true
        }
        // A window that can't be captured says so at once: not a hang.
        Ok(Err(_)) | Err(RecvTimeoutError::Disconnected) => true,
        Err(RecvTimeoutError::Timeout) => {
            crate::note!("window capture didn't answer in 4 s (Windows' capture service is stuck until Windows restarts)");
            STUCK.store(true, Ordering::Relaxed);
            false
        }
    }
}

/// Windows started the package with the user, through its startup task:
/// Deonyx goes to the tray without opening its settings.
pub fn started_with_windows() -> bool {
    packaged() && on_own_thread(|| Ok(AppInstance::GetActivatedEventArgs()?.Kind()? == ActivationKind::StartupTask)).unwrap_or(false)
}

pub fn starts_with_windows() -> bool {
    on_own_thread(|| task()?.State()).is_ok_and(on)
}

/// Turns starting with Windows on or off. Once the user has turned it off
/// in Task Manager, only they can turn it back on there.
pub fn set_start_with_windows(want: bool) -> std::result::Result<(), String> {
    let state = on_own_thread(move || {
        let t = task()?;
        if want {
            t.RequestEnableAsync()?.get()
        } else {
            t.Disable()?;
            t.State()
        }
    })
    .map_err(|e| e.to_string())?;
    if on(state) == want {
        Ok(())
    } else {
        Err("Starting with Windows is turned off in Task Manager > Startup apps.".into())
    }
}
