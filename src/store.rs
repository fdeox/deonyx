// Deonyx from the Microsoft Store runs as a package: the Store updates it,
// Windows starts it with the user through the package's startup task (not
// the Run key), and the administrator task isn't there. The same exe works
// both ways; this tells which one it is.

use std::sync::OnceLock;

use windows::core::{Result, HSTRING};
use windows::ApplicationModel::Activation::ActivationKind;
use windows::ApplicationModel::{AppInstance, StartupTask, StartupTaskState};
use windows::Win32::Foundation::APPMODEL_ERROR_NO_PACKAGE;
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
