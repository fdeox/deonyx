// The command line, for Stream Deck buttons, AutoHotkey scripts and
// shortcuts: a second deonyx.exe hands the command to the running one and
// quits.
//
//   deonyx --on | --off | --toggle      darkening
//   deonyx --front                      the app in front, like Ctrl+Alt+F
//   deonyx --window                     the window under the mouse
//   deonyx --app SketchUp.exe [on|off]  one app (switched when on/off is left out)
//   deonyx --pause [minutes] | --resume a pause (an hour if no minutes are given)
//   deonyx --profile Night              a saved profile
//   deonyx --status                     prints the state as JSON
//   deonyx --settings                   opens the settings

use windows::core::PCWSTR;
use windows::Win32::Foundation::HWND;
use windows::Win32::Storage::FileSystem::{GetFileType, FILE_TYPE_UNKNOWN};
use windows::Win32::System::Console::{AttachConsole, GetStdHandle, ATTACH_PARENT_PROCESS, STD_OUTPUT_HANDLE};
use windows::Win32::UI::WindowsAndMessaging::{FindWindowExW, FindWindowW, GetWindowThreadProcessId};

use crate::engine::{status_bits, Order};

const HELP: &str = "Deonyx command line:
  deonyx --on | --off | --toggle      darkening on, off, or switched
  deonyx --front                      the app in front, like the hotkey
  deonyx --window                     the window under the mouse
  deonyx --app SketchUp.exe [on|off]  one app on, off, or switched
  deonyx --pause [minutes]            pause (an hour if no minutes are given)
  deonyx --resume                     end a pause
  deonyx --profile Night              switch to a saved profile
  deonyx --status                     the state, as JSON
  deonyx --settings                   open the settings";

/// Prints where the command came from: a pipe or file (scripts, Stream
/// Deck), else the console that started it.
fn say(text: &str) {
    unsafe {
        let piped = GetStdHandle(STD_OUTPUT_HANDLE).is_ok_and(|h| !h.is_invalid() && GetFileType(h) != FILE_TYPE_UNKNOWN);
        if piped {
            println!("{text}");
        } else if AttachConsole(ATTACH_PARENT_PROCESS).is_ok() {
            println!("\n{text}");
        }
    }
}

/// The command in the arguments, if there is one.
fn order(args: &[String]) -> Option<Result<Order, ()>> {
    let flag = args.iter().skip(1).find(|a| a.starts_with("--"))?;
    let at = |name: &str| args.iter().position(|a| a == name);
    let next = |i: usize| args.get(i + 1).filter(|a| !a.starts_with("--")).cloned();
    let plain = |cmd: &str| Some(Ok(Order { cmd: cmd.into(), ..Default::default() }));
    match flag.as_str() {
        "--on" => plain("on"),
        "--off" => plain("off"),
        "--toggle" => plain("toggle"),
        "--front" => plain("front"),
        "--window" => plain("window"),
        "--settings" => plain("settings"),
        "--resume" => plain("resume"),
        "--status" => plain("status"),
        "--pause" => {
            let minutes = at("--pause").and_then(next).map(|m| m.parse::<u64>().map_err(|_| ()));
            match minutes {
                Some(Err(())) => Some(Err(())),
                Some(Ok(m)) => Some(Ok(Order { cmd: "pause".into(), minutes: Some(m), ..Default::default() })),
                None => Some(Ok(Order { cmd: "pause".into(), minutes: Some(60), ..Default::default() })),
            }
        }
        "--profile" => match at("--profile").and_then(next) {
            Some(name) => Some(Ok(Order { cmd: "profile".into(), profile: name, ..Default::default() })),
            None => Some(Err(())),
        },
        "--app" => {
            let i = at("--app")?;
            let Some(app) = next(i) else { return Some(Err(())) };
            let on = match args.get(i + 2).map(|s| s.to_ascii_lowercase()).as_deref() {
                Some("on") => Some(true),
                Some("off") => Some(false),
                _ => None,
            };
            Some(Ok(Order { cmd: "app".into(), app, on, ..Default::default() }))
        }
        "--help" | "--?" => Some(Err(())),
        _ => None,
    }
}

/// The state as JSON, from the bits the running Deonyx answers with, and
/// from its settings: the profile in use and each app on or off (for
/// Stream Deck keys that show them).
fn status_json(bits: Option<isize>, cfg: &crate::config::Config) -> String {
    let Some(b) = bits else { return r#"{"running":false}"#.into() };
    let s = status_bits::read(b);
    let apps: serde_json::Map<String, serde_json::Value> = cfg.apps.iter().map(|a| (a.key(), a.enabled.into())).collect();
    serde_json::json!({
        "running": true,
        "on": s.on,
        "paused": s.paused,
        "waiting": s.waiting,
        "battery": s.battery,
        "game": s.game,
        "sharing": s.sharing,
        "windows": s.windows,
        "profile": cfg.profile,
        "profiles": cfg.profiles.iter().map(|p| p.name.clone()).collect::<Vec<_>>(),
        "apps": apps,
    })
    .to_string()
}

/// The running Deonyx's signal window. A test copy (DEONYX_CONFIG_DIR) only
/// talks to the copy DEONYX_TARGET_PID names, so tests never reach the real
/// one.
pub fn signal_window() -> Option<HWND> {
    use crate::admin::SIGNAL_CLASS;
    if std::env::var_os("DEONYX_CONFIG_DIR").is_none() {
        return unsafe { FindWindowW(SIGNAL_CLASS, PCWSTR::null()) }.ok();
    }
    let pid: u32 = std::env::var("DEONYX_TARGET_PID").ok()?.parse().ok()?;
    let mut prev: Option<HWND> = None;
    loop {
        let h = unsafe { FindWindowExW(None, prev, SIGNAL_CLASS, PCWSTR::null()) }.ok()?;
        let mut owner = 0u32;
        unsafe { GetWindowThreadProcessId(h, Some(&mut owner)) };
        if owner == pid {
            return Some(h);
        }
        prev = Some(h);
    }
}

/// Handles a command line run. Returns the exit code, or None when the
/// arguments aren't a command and Deonyx should start as usual.
pub fn run(args: &[String]) -> Option<i32> {
    let o = match order(args)? {
        Ok(o) => o,
        Err(()) => {
            say(HELP);
            return Some(2);
        }
    };
    let found = signal_window();
    if o.cmd == "status" {
        let bits = found.and_then(|h| crate::helper::ask(h, crate::helper::ORDER, &o));
        say(&status_json(bits, &crate::config::load()));
        return Some(if bits.is_some() { 0 } else { 1 });
    }
    let Some(h) = found else {
        // From the jump list or a shortcut: start Deonyx, settings open.
        if o.cmd == "settings" {
            return None;
        }
        say("Deonyx isn't running.");
        return Some(1);
    };
    if crate::helper::send(h, None, crate::helper::ORDER, &o) {
        Some(0)
    } else {
        say("Deonyx didn't answer.");
        Some(1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn a(s: &str) -> Vec<String> {
        std::iter::once("deonyx.exe").chain(s.split(' ')).map(String::from).collect()
    }

    #[test]
    fn reads_commands() {
        assert_eq!(order(&a("--toggle")).unwrap().unwrap().cmd, "toggle");
        let o = order(&a("--app SketchUp.exe off")).unwrap().unwrap();
        assert_eq!((o.cmd.as_str(), o.app.as_str(), o.on), ("app", "SketchUp.exe", Some(false)));
        assert_eq!(order(&a("--app Photoshop.exe")).unwrap().unwrap().on, None);
        assert!(order(&a("--app")).unwrap().is_err());
        assert!(order(&a("--hidden")).is_none());
        assert_eq!(order(&a("--pause")).unwrap().unwrap().minutes, Some(60));
        assert_eq!(order(&a("--pause 15")).unwrap().unwrap().minutes, Some(15));
        assert!(order(&a("--pause soon")).unwrap().is_err());
        assert_eq!(order(&a("--profile Night")).unwrap().unwrap().profile, "Night");
        assert!(order(&a("--profile")).unwrap().is_err());
    }

    #[test]
    fn writes_status() {
        use crate::config::{AppRule, Config, Profile};
        let mut cfg = Config { profile: "Night".into(), ..Default::default() };
        cfg.profiles.push(Profile { name: "Night".into(), ..Default::default() });
        cfg.apps.push(AppRule { exe: "SketchUp.exe".into(), enabled: true, ..Default::default() });
        assert_eq!(status_json(None, &cfg), r#"{"running":false}"#);
        let b = status_bits::make(true, false, false, false, false, true, 3);
        let v: serde_json::Value = serde_json::from_str(&status_json(Some(b), &cfg)).unwrap();
        assert_eq!(
            v,
            serde_json::json!({
                "running": true, "on": true, "paused": false, "waiting": false, "battery": false, "game": false,
                "sharing": true, "windows": 3, "profile": "Night", "profiles": ["Night"], "apps": { "sketchup.exe": true }
            })
        );
    }
}
