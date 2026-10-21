// The last things Deonyx had to say, for "Copy diagnostics": a bug report
// then shows what went wrong without anyone digging through a console.

use std::collections::VecDeque;
use std::sync::Mutex;

const KEEP: usize = 60;

static LINES: Mutex<VecDeque<String>> = Mutex::new(VecDeque::new());

/// Prints a line and keeps it.
#[macro_export]
macro_rules! note {
    ($($t:tt)*) => {{
        let s = format!($($t)*);
        eprintln!("{s}");
        $crate::log::push(s);
    }};
}

pub fn push(s: String) {
    let mut l = LINES.lock().unwrap();
    if l.len() == KEEP {
        l.pop_front();
    }
    let t = unsafe { windows::Win32::System::SystemInformation::GetLocalTime() };
    l.push_back(format!("{:02}:{:02}:{:02} {s}", t.wHour, t.wMinute, t.wSecond));
}

pub fn lines() -> Vec<String> {
    LINES.lock().unwrap().iter().cloned().collect()
}

/// Like lines(), without waiting: the panic hook may run while the list is
/// held.
fn try_lines() -> Vec<String> {
    LINES.try_lock().map(|l| l.iter().cloned().collect()).unwrap_or_default()
}

fn crash_file() -> std::path::PathBuf {
    crate::config::dir().join("crash.txt")
}

/// When something fails, a report goes to crash.txt next to the settings,
/// and the settings window offers it for a bug report.
pub fn keep_crashes() {
    let default = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let t = unsafe { windows::Win32::System::SystemInformation::GetLocalTime() };
        let thread = std::thread::current().name().unwrap_or("main").to_string();
        let mut text = format!(
            "Deonyx {} on Windows build {} ({})\n{}-{:02}-{:02} {:02}:{:02}:{:02}, thread {thread}\n{info}\n",
            env!("CARGO_PKG_VERSION"),
            crate::desk::windows_build(),
            std::env::consts::ARCH,
            t.wYear,
            t.wMonth,
            t.wDay,
            t.wHour,
            t.wMinute,
            t.wSecond
        );
        let recent = try_lines();
        if !recent.is_empty() {
            text.push_str("\nBefore that:\n");
            for l in recent.iter().rev().take(20).rev() {
                text.push_str(&format!("  {l}\n"));
            }
        }
        let _ = std::fs::write(crash_file(), text);
        default(info);
    }));
}

/// The last crash's report, until it's dismissed.
pub fn crash_report() -> Option<String> {
    std::fs::read_to_string(crash_file()).ok().filter(|s| !s.trim().is_empty())
}

pub fn forget_crash() {
    let _ = std::fs::remove_file(crash_file());
}
