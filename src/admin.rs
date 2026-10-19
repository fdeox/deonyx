// Running Deonyx as administrator, the way PowerToys does it: a scheduled
// task with the highest run level starts it. Windows asks the user once,
// when the task is created or deleted; after that it starts without asking.
// Only then can Deonyx put its layer on windows that run as administrator.

use std::os::windows::process::CommandExt;
use std::process::Command;

use windows::core::{w, HSTRING, PCWSTR};
use windows::Win32::Foundation::*;
use windows::Win32::System::Threading::{GetExitCodeProcess, WaitForSingleObject, INFINITE};
use windows::Win32::UI::Shell::{ShellExecuteExW, SEE_MASK_NOCLOSEPROCESS, SHELLEXECUTEINFOW};
use windows::Win32::UI::WindowsAndMessaging::*;

pub const TASK: &str = "Deonyx";
const NO_WINDOW: u32 = 0x0800_0000;
/// Class of the hidden window every running Deonyx keeps, so a second launch
/// can find it and ask it to open its settings.
pub const SIGNAL_CLASS: PCWSTR = w!("DeonyxSignal");
pub const OPEN_MSG: PCWSTR = w!("DeonyxOpenSettings");

pub fn elevated() -> bool {
    crate::win::is_elevated(std::process::id())
}

fn schtasks(args: &[&str]) -> bool {
    Command::new("schtasks").args(args).creation_flags(NO_WINDOW).output().map(|o| o.status.success()).unwrap_or(false)
}

pub fn task_exists() -> bool {
    schtasks(&["/Query", "/TN", TASK])
}

pub fn run_task() -> bool {
    task_exists() && schtasks(&["/Run", "/TN", TASK])
}

fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

/// Creates or replaces the task. Needs administrator rights.
pub fn create_task(at_logon: bool) -> Result<(), String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let user = format!(
        "{}\\{}",
        std::env::var("USERDOMAIN").unwrap_or_default(),
        std::env::var("USERNAME").unwrap_or_default()
    );
    let (name, level) = if std::env::var_os("DEONYX_TASK_TEST").is_some() {
        ("DeonyxTest", "LeastPrivilege")
    } else {
        (TASK, "HighestAvailable")
    };
    let trigger = if at_logon {
        format!("<LogonTrigger><Enabled>true</Enabled><UserId>{}</UserId></LogonTrigger>", xml_escape(&user))
    } else {
        String::new()
    };
    let xml = format!(
        r#"<?xml version="1.0" encoding="UTF-16"?>
<Task version="1.2" xmlns="http://schemas.microsoft.com/windows/2004/02/mit/task">
  <RegistrationInfo>
    <Author>Deonyx</Author>
    <Description>Starts Deonyx with administrator rights, so it can darken apps that run as administrator.</Description>
  </RegistrationInfo>
  <Triggers>{trigger}</Triggers>
  <Principals>
    <Principal id="Author">
      <UserId>{user}</UserId>
      <LogonType>InteractiveToken</LogonType>
      <RunLevel>{level}</RunLevel>
    </Principal>
  </Principals>
  <Settings>
    <MultipleInstancesPolicy>IgnoreNew</MultipleInstancesPolicy>
    <DisallowStartIfOnBatteries>false</DisallowStartIfOnBatteries>
    <StopIfGoingOnBatteries>false</StopIfGoingOnBatteries>
    <AllowHardTerminate>true</AllowHardTerminate>
    <StartWhenAvailable>false</StartWhenAvailable>
    <AllowStartOnDemand>true</AllowStartOnDemand>
    <Enabled>true</Enabled>
    <ExecutionTimeLimit>PT0S</ExecutionTimeLimit>
    <Priority>5</Priority>
  </Settings>
  <Actions Context="Author">
    <Exec>
      <Command>{exe}</Command>
      <Arguments>--hidden --from-task</Arguments>
    </Exec>
  </Actions>
</Task>
"#,
        user = xml_escape(&user),
        exe = xml_escape(&exe.to_string_lossy()),
    );
    // schtasks reads task XML as UTF-16.
    let mut bytes = vec![0xFF, 0xFE];
    for u in xml.encode_utf16() {
        bytes.extend(u.to_le_bytes());
    }
    let path = std::env::temp_dir().join("deonyx-task.xml");
    std::fs::write(&path, bytes).map_err(|e| e.to_string())?;
    let ok = schtasks(&["/Create", "/TN", name, "/XML", &path.to_string_lossy(), "/F"]);
    let _ = std::fs::remove_file(&path);
    if ok {
        Ok(())
    } else {
        Err("Windows didn't let Deonyx create its task.".into())
    }
}

pub fn delete_task() -> Result<(), String> {
    if !task_exists() || schtasks(&["/Delete", "/TN", TASK, "/F"]) {
        Ok(())
    } else {
        Err("Windows didn't let Deonyx remove its task.".into())
    }
}

/// Runs this exe again as administrator with `args` and waits for it.
/// Windows shows its permission prompt first.
pub fn run_elevated(args: &str) -> Result<(), String> {
    let exe = HSTRING::from(std::env::current_exe().map_err(|e| e.to_string())?.as_os_str());
    let params = HSTRING::from(args);
    let mut info = SHELLEXECUTEINFOW {
        cbSize: std::mem::size_of::<SHELLEXECUTEINFOW>() as u32,
        fMask: SEE_MASK_NOCLOSEPROCESS,
        lpVerb: w!("runas"),
        lpFile: PCWSTR(exe.as_ptr()),
        lpParameters: PCWSTR(params.as_ptr()),
        nShow: SW_HIDE.0,
        ..Default::default()
    };
    unsafe {
        if ShellExecuteExW(&mut info).is_err() || info.hProcess.is_invalid() {
            return Err("Windows didn't allow it.".into());
        }
        WaitForSingleObject(info.hProcess, INFINITE);
        let mut code = 1u32;
        let _ = GetExitCodeProcess(info.hProcess, &mut code);
        let _ = CloseHandle(info.hProcess);
        if code == 0 {
            Ok(())
        } else {
            Err("Something went wrong while asking Windows.".into())
        }
    }
}

/// A moment after we quit, start Deonyx through the task, as administrator.
pub fn relaunch_elevated() {
    let _ = Command::new("cmd")
        .raw_arg(format!("/c ping -n 2 127.0.0.1 >nul & schtasks /Run /TN {TASK}"))
        .creation_flags(NO_WINDOW)
        .spawn();
}

/// A moment after we quit, start Deonyx without administrator rights:
/// Explorer starts it as the signed-in user.
pub fn relaunch_normal() {
    if let Ok(exe) = std::env::current_exe() {
        let _ = Command::new("cmd")
            .raw_arg(format!("/c ping -n 2 127.0.0.1 >nul & explorer.exe \"{}\"", exe.to_string_lossy()))
            .creation_flags(NO_WINDOW)
            .spawn();
    }
}

/// Asks an already running Deonyx to open its settings. False if none runs.
pub fn ask_running_to_open() -> bool {
    unsafe {
        let Ok(h) = FindWindowW(SIGNAL_CLASS, PCWSTR::null()) else { return false };
        let msg = RegisterWindowMessageW(OPEN_MSG);
        PostMessageW(Some(h), msg, WPARAM(0), LPARAM(0)).is_ok()
    }
}
