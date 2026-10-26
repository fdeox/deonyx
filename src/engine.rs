// The darkening engine. Runs on its own thread with its own message loop:
// WinEvent hooks, the hotkeys and capture frames all arrive there, so the
// settings window can never slow it down.

use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};
use std::ffi::c_void;
use std::sync::{mpsc, Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use windows::core::{w, Result};
use windows::Win32::Foundation::*;
use windows::Win32::System::Com::{CoCreateInstance, CoInitializeEx, CLSCTX_INPROC_SERVER, COINIT_MULTITHREADED};
use windows::Win32::System::Diagnostics::ToolHelp::{CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W, TH32CS_SNAPPROCESS};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Power::{GetSystemPowerStatus, SYSTEM_POWER_STATUS};
use windows::Win32::System::Threading::{CreateEventW, GetCurrentThreadId};
use windows::Win32::UI::Accessibility::*;
use windows::Win32::UI::Input::KeyboardAndMouse::{RegisterHotKey, UnregisterHotKey, MOD_NOREPEAT, MOD_SHIFT};
use windows::Win32::UI::WindowsAndMessaging::*;

use crate::config::{now_secs, AppRule, Area, Config, Keys, Screen};
use crate::desk::Desktop;
use crate::gpu::Gpu;
use crate::helper::{self, Helper};
use crate::keys::{self, Combo};
use crate::layer::{Layer, Look, OVERLAY_CLASS};
use crate::note;
use crate::presets::{self, Preset};
use crate::protect::Finder;
use crate::screens;
use crate::select;
use crate::sun;
use crate::win::*;

const OBJID_WINDOW: i32 = 0;
const WM_ENGINE: u32 = WM_APP + 1;
const HOTKEY_ALL: i32 = 1;
const HOTKEY_FRONT: i32 = 2;
const HOTKEY_PEEK: i32 = 3;
const HOTKEY_WINDOW: i32 = 4;
/// Apps' own hotkeys get ids from here on.
const HOTKEY_APP: i32 = 100;
/// How much faint text is lifted when "readable" is on (a curve on the
/// flipped lightness, see shader.hlsl).
const READABLE: f32 = 0.78;

/// How long switching darkening on or off fades, in milliseconds.
pub const FADE_MS: f32 = 220.0;
/// Peeking at the real colours is quicker.
const PEEK_MS: f32 = 110.0;
/// With "gradual", darkening comes in over this many minutes after the
/// schedule starts and goes over as many before it ends.
pub const EASE_MIN: f64 = 30.0;
/// The gradual strength moves on in steps at least this big, each one
/// faded over LEVEL_MS so the steps never show.
const LEVEL_STEP: f32 = 0.01;
const LEVEL_MS: f32 = 4000.0;
/// The most a dark window is dimmed.
const DIM_MAX: f32 = 0.6;
/// On battery, "slow" draws a window at most this often.
const BATTERY_GAP: Duration = Duration::from_millis(66);

/// Programs that mean the screen is being shared or recorded right now: Zoom's
/// sharing helper, OBS, Streamlabs, XSplit, Webex's meeting manager. While
/// one runs, "automatic" sharing shows the real colours to the others.
const SHARE_APPS: &[&str] = &["cpthost.exe", "obs64.exe", "obs32.exe", "obs.exe", "streamlabs obs.exe", "xsplit.core.exe", "atmgr.exe"];

/// Lower-case names of every running program.
fn running_programs() -> HashSet<String> {
    let mut out = HashSet::new();
    unsafe {
        let Ok(snap) = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) else { return out };
        let mut e = PROCESSENTRY32W { dwSize: std::mem::size_of::<PROCESSENTRY32W>() as u32, ..Default::default() };
        let mut ok = Process32FirstW(snap, &mut e).is_ok();
        while ok {
            let n = e.szExeFile.iter().position(|&c| c == 0).unwrap_or(e.szExeFile.len());
            out.insert(String::from_utf16_lossy(&e.szExeFile[..n]).to_ascii_lowercase());
            ok = Process32NextW(snap, &mut e).is_ok();
        }
        let _ = CloseHandle(snap);
    }
    out
}

/// Parts of Windows itself, never darkened in "every light window" mode.
const SHELL_CLASSES: &[&str] = &[
    "Progman",
    "WorkerW",
    "Shell_TrayWnd",
    "Shell_SecondaryTrayWnd",
    "NotifyIconOverflowWindow",
    "TopLevelWindowForOverflowXamlIsland",
    "Windows.UI.Core.CoreWindow",
    "XamlExplorerHostIslandWindow",
    "ForegroundStaging",
    "MultitaskingViewFrame",
    "TaskListThumbnailWnd",
    "IME",
    "MSCTFIME UI",
];
const SHELL_EXES: &[&str] = &[
    "dwm.exe",
    "shellexperiencehost.exe",
    "startmenuexperiencehost.exe",
    "searchhost.exe",
    "searchapp.exe",
    "textinputhost.exe",
    "lockapp.exe",
    "screenclippinghost.exe",
    "gamebar.exe",
    "gamebarftserver.exe",
    "xboxgamebarwidgets.exe",
];

/// Which copy of Deonyx this engine runs in (see helper.rs).
#[derive(Clone, Copy, PartialEq)]
pub enum Role {
    /// The one with the tray. Running as administrator, it leaves normal
    /// apps to a helper.
    Main,
    /// The helper, started by the main one (whose process id this is).
    Helper { parent: u32 },
}

pub enum Cmd {
    Config(Config),
    /// The app presets (presets.rs), when they first load and when newer
    /// ones arrive.
    Presets(Vec<Preset>),
    /// Wait for a click on a window, then explain what Deonyx does with it.
    Inspect,
    /// Wait for a click on a part of a window, then keep that part in real
    /// colours.
    PickArea,
    StopInspect,
    /// Let the user draw an area of that app's window to keep in real colours.
    DrawArea { exe: String, hint: String },
    Quit,
}

/// Things the engine asks the app to do.
pub enum Request {
    OpenSettings,
    /// Ctrl+Alt+F: switch darkening of the app in front.
    ToggleApp { exe: String, path: String },
    /// From the command line or an app's hotkey: an app on, off, or
    /// switched (None).
    SetApp { exe: String, on: Option<bool> },
    /// From the command line: pause for a while (0 resumes).
    Pause(u64),
    /// From the command line: switch to a profile.
    Profile(String),
    /// The engine failed and started again: it needs the settings again.
    Restarted,
    Inspected(Report),
    /// The outcome of DrawArea or PickArea: the area, or why there is none.
    Area { exe: String, path: String, area: std::result::Result<Area, String> },
}

/// What a command line run of deonyx.exe asks the running one (see cli.rs).
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Order {
    /// "on", "off", "toggle", "front", "window", "app", "pause", "resume",
    /// "profile", "status" or "settings".
    pub cmd: String,
    pub app: String,
    pub on: Option<bool>,
    /// Minutes for "pause".
    pub minutes: Option<u64>,
    /// The profile's name for "profile".
    pub profile: String,
}

/// What Deonyx does with a window, for "Why isn't this window dark?".
#[derive(Clone, Debug, Default, Serialize)]
pub struct Report {
    pub exe: String,
    pub path: String,
    pub title: String,
    /// darkened, already_dark, not_listed, excluded, app_off, off, waiting,
    /// paused, battery, screen_off, left_alone, admin, service, no_capture,
    /// helper, fullscreen, shell, own, failed or unknown.
    pub reason: String,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Status {
    pub enabled: bool,
    /// On, but the schedule says not now.
    pub waiting: bool,
    /// On, but paused from the tray for a while.
    pub paused: bool,
    /// On, but paused while on battery.
    pub battery: bool,
    /// On, but paused while this program runs (game mode), its exe name.
    pub paused_by: String,
    /// Screen sharing shows the real colours right now because a meeting or
    /// streaming app shares.
    pub sharing: bool,
    /// Lower-case exe name -> number of its main windows being darkened.
    pub running: HashMap<String, usize>,
    /// Lower-case exe name -> main windows left alone because the program
    /// runs as administrator and we don't: Windows won't let a layer sit
    /// on them.
    pub admin: HashMap<String, usize>,
    /// Same, for windows of a Windows service (system level), which nothing
    /// can draw over.
    pub system: HashMap<String, usize>,
    /// Waiting for a click, for "Why isn't this window dark?" or picking an
    /// area.
    pub inspecting: bool,
    /// Hotkeys another program has taken: "all", "front", "peek".
    pub keys_taken: Vec<String>,
    /// "window" (Windows.Graphics.Capture) or "screen" (desk.rs).
    pub capture: String,
    /// From the helper only: its window, and every window it has a layer on
    /// (true when that one is dark already and left as it is).
    #[serde(skip_serializing_if = "is_zero")]
    pub hwnd: isize,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub windows: Vec<(isize, bool)>,
}

fn is_zero(v: &isize) -> bool {
    *v == 0
}

/// The state packed into one number, the answer to `deonyx --status` (the
/// signal window gives it back right away, as the result of WM_COPYDATA).
pub mod status_bits {
    pub struct State {
        pub on: bool,
        pub paused: bool,
        pub waiting: bool,
        pub battery: bool,
        pub game: bool,
        pub sharing: bool,
        pub windows: usize,
    }

    pub fn make(on: bool, paused: bool, waiting: bool, battery: bool, game: bool, sharing: bool, windows: usize) -> isize {
        let flags = [on, paused, waiting, battery, game, sharing];
        let mut b: isize = 1;
        for (i, f) in flags.iter().enumerate() {
            if *f {
                b |= 2 << i;
            }
        }
        b | ((windows.min(0xFFFF) as isize) << 8)
    }

    pub fn read(b: isize) -> State {
        let f = |i: usize| b & (2 << i) != 0;
        State { on: f(0), paused: f(1), waiting: f(2), battery: f(3), game: f(4), sharing: f(5), windows: ((b >> 8) & 0xFFFF) as usize }
    }
}

pub type OnStatus = Arc<dyn Fn(Status) + Send + Sync>;
pub type OnRequest = Arc<dyn Fn(Request) + Send + Sync>;

pub struct Engine {
    tid: u32,
    cmds: Arc<Mutex<Vec<Cmd>>>,
    _thread: JoinHandle<()>,
}

impl Engine {
    pub fn start(cfg: Config, role: Role, on_status: OnStatus, on_request: OnRequest) -> Engine {
        let cmds = Arc::new(Mutex::new(Vec::new()));
        let (tx, rx) = mpsc::channel();
        let queue = cmds.clone();
        let thread = std::thread::Builder::new()
            .name("deonyx-engine".into())
            .spawn(move || {
                unsafe {
                    let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
                    // Creates this thread's message queue before anyone posts to it.
                    let mut msg = MSG::default();
                    let _ = PeekMessageW(&mut msg, None, 0, 0, PM_NOREMOVE);
                    let _ = tx.send(GetCurrentThreadId());
                }
                // If the engine fails, it starts again with a clean slate (and
                // asks for the settings), up to three times in five minutes:
                // darkening carries on rather than the tray staying with a
                // dead engine. The panic hook (main.rs) keeps a report.
                let mut failures: Vec<Instant> = Vec::new();
                loop {
                    let ran = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                        run(cfg.clone(), role, queue.clone(), on_status.clone(), on_request.clone())
                    }));
                    match ran {
                        Ok(Ok(())) => break,
                        Ok(Err(e)) => {
                            note!("engine stopped: {e}");
                            break;
                        }
                        Err(_) => {
                            failures.retain(|t| t.elapsed() < Duration::from_secs(300));
                            failures.push(Instant::now());
                            if failures.len() > 3 {
                                note!("the engine keeps failing; it stays off");
                                break;
                            }
                            note!("the engine failed and starts again");
                            on_request(Request::Restarted);
                        }
                    }
                }
            })
            .expect("engine thread");
        let tid = rx.recv().unwrap_or(0);
        Engine { tid, cmds, _thread: thread }
    }

    pub fn send(&self, c: Cmd) {
        self.cmds.lock().unwrap().push(c);
        unsafe {
            let _ = PostThreadMessageW(self.tid, WM_ENGINE, WPARAM(0), LPARAM(0));
        }
    }
}

thread_local! {
    static EVENTS: RefCell<Vec<(u32, isize)>> = const { RefCell::new(Vec::new()) };
    static PICK: RefCell<Option<Pick>> = const { RefCell::new(None) };
    /// WM_COPYDATA from the other copy: kind, sender's process, bytes.
    static INBOX: RefCell<Vec<(usize, u32, Vec<u8>)>> = const { RefCell::new(Vec::new()) };
    /// The state for `deonyx --status` (status_bits), kept by publish().
    static NOW: Cell<isize> = const { Cell::new(1) };
    /// What a run of the engine set up on this thread, for the next run to
    /// clean up if this one failed half way: WinEvent hooks, hidden window.
    static LEFT: RefCell<(Vec<HWINEVENTHOOK>, Option<HWND>)> = const { RefCell::new((Vec::new(), None)) };
}

unsafe extern "system" fn win_event(_: HWINEVENTHOOK, event: u32, hwnd: HWND, obj: i32, child: i32, _: u32, _: u32) {
    if obj != OBJID_WINDOW || child != 0 {
        return;
    }
    EVENTS.with(|e| e.borrow_mut().push((event, hwnd.0 as isize)));
}

#[derive(Clone, Copy, PartialEq)]
enum Purpose {
    /// "Why isn't this window dark?"
    Inspect,
    /// "Keep a part in real colours": the clicked part becomes an area.
    Area,
}

/// The next click anywhere is caught, not passed on.
struct Pick {
    hook: HHOOK,
    purpose: Purpose,
    at: Option<POINT>,
    released: bool,
    cancelled: bool,
    since: Instant,
}

unsafe extern "system" fn mouse_ll(code: i32, w: WPARAM, l: LPARAM) -> LRESULT {
    if code >= 0 {
        let info = &*(l.0 as *const MSLLHOOKSTRUCT);
        let mut swallow = false;
        PICK.with(|p| {
            if let Some(s) = p.borrow_mut().as_mut() {
                match w.0 as u32 {
                    WM_LBUTTONDOWN if s.at.is_none() => {
                        s.at = Some(info.pt);
                        swallow = true;
                    }
                    WM_LBUTTONUP if s.at.is_some() && !s.released => {
                        s.released = true;
                        swallow = true;
                    }
                    WM_RBUTTONDOWN => {
                        s.cancelled = true;
                        swallow = true;
                    }
                    _ => {}
                }
            }
        });
        if swallow {
            return LRESULT(1);
        }
    }
    CallNextHookEx(None, code, w, l)
}

struct Manager {
    gpu: Gpu,
    rules: HashMap<String, AppRule>,
    /// Every rule, on or off, for the inspector.
    all_rules: HashMap<String, AppRule>,
    everything: bool,
    exclude: HashSet<String>,
    /// Apps whose presets say to leave them alone (render windows).
    leave: HashSet<String>,
    style: String,
    warm: f32,
    /// Kind ("always", "windows", "times", "sun"), from and to minutes, and
    /// the place for "sun".
    schedule: (String, u32, u32),
    place: (f64, f64),
    /// Darkening comes and goes slowly at the ends of the schedule.
    gradual: bool,
    /// How dark it is right now, 0..1 (below 1 only while gradual eases in
    /// or out).
    level: f32,
    /// Windows that are dark already are dimmed this much.
    dim: f32,
    enabled: bool,
    /// The schedule allows darkening right now.
    in_time: bool,
    paused_until: u64,
    /// "keep", "slow" or "pause".
    battery_mode: String,
    on_battery: bool,
    share_original: bool,
    split: bool,
    layers: HashMap<isize, Layer>,
    /// Layers fading out before they go.
    leaving: Vec<Layer>,
    /// New layers fade in until then, after the user switched something on.
    fade_new: Option<Instant>,
    /// The key held for peeking at the real colours, if one is.
    peek: Option<u32>,
    /// The helper peeks when its main copy says so.
    peek_told: bool,
    /// Screens copied instead of windows captured (Windows 10, desk.rs).
    desk: Option<Desktop>,
    keys: [Option<Combo>; 4],
    keys_taken: Vec<String>,
    /// Apps' own hotkeys as registered: exe and key.
    app_keys: Vec<(String, Combo)>,
    /// Faint text lifted (see READABLE).
    readable: bool,
    /// Every running program, looked at every few seconds.
    programs: HashSet<String>,
    /// Pause while one of these runs.
    pause_apps: HashSet<String>,
    /// The one that is running, if any.
    paused_by: Option<String>,
    share_auto: bool,
    /// A sharing app runs (and share_auto is on).
    sharing: bool,
    /// Windows switched on or off one by one (the window hotkey), for as
    /// long as they live.
    window_over: HashMap<isize, bool>,
    screens: Vec<screens::Info>,
    screen_rules: Vec<Screen>,
    /// Layer -> id of the screen its window is on.
    screen_of: HashMap<isize, String>,
    moved: HashSet<isize>,
    exe_of: HashMap<u32, Option<String>>,
    levels: HashMap<u32, u32>,
    my_level: u32,
    admin: HashMap<String, usize>,
    system: HashMap<String, usize>,
    ignored: HashSet<isize>,
    finder: Finder,
    protect: HashMap<isize, Vec<RECT>>,
    frame_event: HANDLE,
    since: Instant,
    on_status: OnStatus,
    on_request: OnRequest,
    last_status: Option<Status>,
    drawing: Option<String>,
    role: Role,
    /// Main copy with a helper: only apps with more rights than a normal
    /// program are ours, the helper darkens the rest.
    upper: bool,
    helper: Option<Helper>,
    /// What the helper last said it darkens.
    helper_status: Option<Status>,
    helper_starts: Vec<Instant>,
    /// The settings as last applied, for the helper.
    last_cfg: Option<Config>,
    /// Our hidden window: the signal window, or the helper's.
    window: Option<HWND>,
}

fn minutes(hhmm: &str) -> u32 {
    let (h, m) = hhmm.split_once(':').unwrap_or(("0", "0"));
    (h.trim().parse::<u32>().unwrap_or(0) % 24) * 60 + m.trim().parse::<u32>().unwrap_or(0) % 60
}

/// Darkening strength at `now` in a stretch from `start` to `end` (minutes
/// since midnight; it may wrap past midnight): 0 at either end, 1 from
/// EASE_MIN in until EASE_MIN before the end, eased in between.
fn ease(start: f64, end: f64, now: f64) -> f32 {
    let len = (end - start).rem_euclid(1440.0);
    let since = (now - start).rem_euclid(1440.0);
    if len <= 0.0 || since > len {
        return 1.0;
    }
    let t = (since.min(len - since) / EASE_MIN).clamp(0.0, 1.0);
    (t * t * (3.0 - 2.0 * t)) as f32
}

fn on_battery() -> bool {
    // Test copies can pretend (DEONYX_BATTERY=1).
    if std::env::var_os("DEONYX_CONFIG_DIR").is_some() && std::env::var_os("DEONYX_BATTERY").is_some() {
        return true;
    }
    let mut s = SYSTEM_POWER_STATUS::default();
    unsafe { GetSystemPowerStatus(&mut s).is_ok() && s.ACLineStatus == 0 }
}

impl Manager {
    fn active(&self) -> bool {
        self.enabled && self.in_time && !self.paused() && !self.battery_paused() && self.paused_by.is_none()
    }

    /// Layers stay out of screen sharing and screenshots.
    fn hidden(&self) -> bool {
        self.share_original || self.sharing
    }

    fn paused(&self) -> bool {
        self.paused_until > now_secs()
    }

    fn battery_paused(&self) -> bool {
        self.on_battery && self.battery_mode == "pause"
    }

    fn schedule_allows(&self) -> bool {
        match self.schedule.0.as_str() {
            "windows" => apps_use_dark_theme(),
            "times" => {
                let (from, to, now) = (self.schedule.1, self.schedule.2, local_minutes());
                if from <= to {
                    now >= from && now < to
                } else {
                    now >= from || now < to
                }
            }
            "sun" => sun::is_night(self.place.0, self.place.1, local_minutes()),
            _ => true,
        }
    }

    /// How dark it should be now: 1, except while "gradual" eases in after
    /// the start of set hours or sunset, and out before their end.
    fn night_level(&self) -> f32 {
        if !self.gradual {
            return 1.0;
        }
        if !self.in_time {
            return 0.0;
        }
        let (start, end) = match self.schedule.0.as_str() {
            "times" => (self.schedule.1, self.schedule.2),
            "sun" => match sun::today(self.place.0, self.place.1) {
                sun::Day::Times { rise, set } => (set, rise),
                _ => return 1.0,
            },
            _ => return 1.0,
        };
        ease(start as f64, end as f64, local_minutes_exact())
    }

    /// Moves every layer towards the strength the time of day asks for, in
    /// steps small enough not to show; `at_once` after a settings change.
    fn follow_level(&mut self, at_once: bool) {
        let level = self.night_level();
        if level == self.level {
            return;
        }
        let ends = level == 0.0 || level == 1.0;
        if !at_once && !ends && (level - self.level).abs() < LEVEL_STEP {
            return;
        }
        self.level = level;
        if self.peek.is_none() && !self.peek_told {
            self.fade_all(level, if at_once { FADE_MS } else { LEVEL_MS });
        }
    }

    /// The other copy of Deonyx (see helper.rs).
    fn other_pid(&self) -> Option<u32> {
        match self.role {
            Role::Helper { parent } => Some(parent),
            Role::Main => self.helper.as_ref().map(|h| h.pid),
        }
    }

    fn exe_key(&mut self, pid: u32) -> Option<String> {
        if pid == std::process::id() || Some(pid) == self.other_pid() {
            return None;
        }
        self.exe_of
            .entry(pid)
            .or_insert_with(|| exe_name(pid).map(|n| n.to_ascii_lowercase()))
            .clone()
    }

    fn screen_rule(&self, h: HWND) -> Option<&Screen> {
        if self.screen_rules.is_empty() {
            return None;
        }
        let id = screens::of_window(h, &self.screens)?;
        self.screen_rules.iter().find(|s| s.id == id)
    }

    fn look(&self, rule: &AppRule, h: HWND) -> Look {
        let style = match self.screen_rule(h) {
            Some(s) if !s.style.is_empty() => s.style.clone(),
            _ => self.style.clone(),
        };
        let (lmin, lmax) = rule.lightness(&style);
        // The paper look is always warm.
        let warm = if style == "paper" { self.warm.max(0.55) } else { self.warm };
        Look {
            lmin,
            lmax,
            warm,
            boost: if self.readable { READABLE } else { 1.0 },
            dim: self.dim,
            split: self.split,
            owner: true,
            photos: rule.photos,
            hidden: self.hidden(),
            strength: if self.peek.is_some() || self.peek_told { 0.0 } else { self.level },
            fade_in: self.fade_new.is_some_and(|t| t > Instant::now()),
            gap: self.gap(),
        }
    }

    fn gap(&self) -> Option<Duration> {
        (self.on_battery && self.battery_mode == "slow").then_some(BATTERY_GAP)
    }

    /// The rule for an app: its own, or the default one in "every light
    /// window" mode.
    fn rule_of(&self, key: &str) -> Option<AppRule> {
        if let Some(r) = self.rules.get(key) {
            return Some(r.clone());
        }
        let own = self.all_rules.contains_key(key);
        if self.everything && !own && !self.exclude.contains(key) && !self.leave.contains(key) && !SHELL_EXES.contains(&key) {
            return Some(AppRule { exe: key.to_string(), ..Default::default() });
        }
        None
    }

    /// The rule for a window, if it's one we should darken.
    fn rule_for(&mut self, h: HWND) -> Option<AppRule> {
        if !self.active() {
            return None;
        }
        // Cheap checks first: most windows belong to programs we don't touch,
        // and those never cost a call to the compositor.
        let pid = window_pid(h);
        let key = self.exe_key(pid)?;
        // Switched one by one with the window hotkey.
        let over = self.window_over.get(&(h.0 as isize)).copied();
        if over == Some(false) {
            return None;
        }
        let rule = match self.rule_of(&key) {
            Some(r) => r,
            None if over == Some(true) => AppRule { exe: key.clone(), ..Default::default() },
            None => return None,
        };
        let implicit = !self.rules.contains_key(&key);
        unsafe {
            if !IsWindowVisible(h).as_bool() || !is_top_level(h) {
                return None;
            }
            let ex = GetWindowLongW(h, GWL_EXSTYLE) as u32;
            // Click-through helper windows (drag images, effects) are left alone.
            if ex & WS_EX_TRANSPARENT.0 != 0 && ex & WS_EX_LAYERED.0 != 0 {
                return None;
            }
        }
        let class = class_name(h);
        if is_cloaked(h) || class == OVERLAY_CLASS || class == "DeonyxSheet" {
            return None;
        }
        if implicit && (SHELL_CLASSES.contains(&class.as_str()) || is_fullscreen(h)) {
            return None;
        }
        // Windows that refuse screen capture (password managers, some
        // meeting and banking apps) would come out as a grey box.
        let mut affinity = 0u32;
        if unsafe { GetWindowDisplayAffinity(h, &mut affinity) }.is_ok() && affinity != 0 {
            return None;
        }
        let r = frame_rect(h);
        if width(&r) < 4 || height(&r) < 4 || !self.mine(pid) {
            return None;
        }
        if self.screen_rule(h).is_some_and(|s| s.off) {
            return None;
        }
        Some(rule)
    }

    /// Whether this copy darkens the program's windows: never above our own
    /// rights, and with a helper only those above a normal program's.
    fn mine(&mut self, pid: u32) -> bool {
        !self.above_us(pid) && (!self.upper || self.level(pid) > LEVEL_MEDIUM)
    }

    fn level(&mut self, pid: u32) -> u32 {
        *self.levels.entry(pid).or_insert_with(|| integrity(pid))
    }

    /// A program running with more rights than we have.
    fn above_us(&mut self, pid: u32) -> bool {
        self.level(pid) > self.my_level
    }

    fn add(&mut self, h: HWND) {
        let key = h.0 as isize;
        if self.layers.contains_key(&key) || self.ignored.contains(&key) {
            return;
        }
        let Some(rule) = self.rule_for(h) else { return };
        match Layer::new(&self.gpu, h, &self.look(&rule, h), self.desk.is_some()) {
            Ok(mut l) => {
                if let Err(e) = l.on_frame_arrived(self.frame_event) {
                    note!("frame notification failed: {e}");
                }
                l.exe = rule.key();
                l.manual = rule.areas.clone();
                if std::env::var_os("DEONYX_DEBUG").is_some() {
                    let r = frame_rect(h);
                    println!("+ {} {} {}x{} @ {},{} owned={}", rule.exe, class_name(h), width(&r), height(&r), r.left, r.top, l.owned);
                }
                if rule.protect {
                    if let Some(p) = self.protect.get(&key) {
                        l.protect = p.clone();
                    }
                }
                if let Some(id) = screens::of_window(h, &self.screens) {
                    self.screen_of.insert(key, id);
                }
                self.layers.insert(key, l);
                self.watch_main_windows();
                self.publish();
            }
            Err(e) => {
                // Window capture hangs on this PC: copy the screen from now on.
                if self.desk.is_none() && crate::store::capture_stuck() {
                    match Desktop::new(&self.gpu) {
                        Ok(d) => {
                            note!("copying the screen instead of capturing windows");
                            self.desk = Some(d);
                            self.ignored.clear();
                            return self.add(h);
                        }
                        Err(e) => note!("screen copy failed: {e}"),
                    }
                }
                note!("capture failed ({}): {e}", class_name(h));
                self.ignored.insert(key);
            }
        }
    }

    /// The window is gone: its layer goes at once.
    fn remove(&mut self, key: isize) {
        if self.layers.remove(&key).is_some() {
            self.watch_main_windows();
            self.publish();
        }
        self.ignored.remove(&key);
        self.screen_of.remove(&key);
        self.window_over.remove(&key);
    }

    /// The window stays but shouldn't be dark any more: its layer fades out
    /// first.
    fn retire(&mut self, key: isize) {
        if let Some(mut l) = self.layers.remove(&key) {
            l.fade_to(0.0, FADE_MS);
            if l.fading() {
                self.leaving.push(l);
            }
        }
        self.screen_of.remove(&key);
    }

    fn protects(&self, exe: &str) -> bool {
        self.rule_of(exe).is_some_and(|r| r.protect)
    }

    /// Only big windows of apps that want it can hold a viewport worth protecting.
    fn watch_main_windows(&self) {
        let big = self
            .layers
            .iter()
            .filter(|(_, l)| self.protects(&l.exe))
            .filter(|(_, l)| {
                let r = frame_rect(l.target);
                width(&r) >= 400 && height(&r) >= 300
            })
            .map(|(k, _)| *k)
            .collect();
        self.finder.watch(big);
    }

    fn rescan(&mut self) {
        let mut admin: HashMap<String, usize> = HashMap::new();
        let mut system: HashMap<String, usize> = HashMap::new();
        if self.active() {
            for h in top_level_windows() {
                self.add(h);
                let pid = window_pid(h);
                let Some(key) = self.exe_key(pid) else { continue };
                if !self.rules.contains_key(&key) {
                    continue;
                }
                let main = unsafe { IsWindowVisible(h).as_bool() && GetWindow(h, GW_OWNER).is_err() } && is_top_level(h);
                if main && self.above_us(pid) {
                    let into = if self.level(pid) >= LEVEL_SYSTEM { &mut system } else { &mut admin };
                    *into.entry(key).or_default() += 1;
                }
            }
        }
        if admin != self.admin || system != self.system {
            self.admin = admin;
            self.system = system;
            self.publish();
        }
    }

    fn apply(&mut self, cfg: Config) {
        let was_active = self.active();
        self.last_cfg = Some(cfg.clone());
        self.enabled = cfg.enabled;
        let before: HashSet<String> = self.rules.keys().cloned().collect();
        self.all_rules = cfg.apps.iter().filter(|r| !r.exe.is_empty()).map(|r| (r.key(), r.clone())).collect();
        self.rules = self.all_rules.iter().filter(|(_, r)| r.enabled).map(|(k, r)| (k.clone(), r.clone())).collect();
        self.everything = cfg.everything;
        self.exclude = cfg.exclude.iter().map(|e| e.exe.to_ascii_lowercase()).collect();
        self.style = cfg.style;
        self.warm = cfg.warm.clamp(0.0, 1.0);
        self.schedule = (cfg.schedule, minutes(&cfg.from), minutes(&cfg.to));
        self.place = (cfg.place.lat, cfg.place.lon);
        self.gradual = cfg.gradual;
        self.dim = cfg.dim.clamp(0.0, DIM_MAX);
        self.paused_until = cfg.paused_until;
        self.battery_mode = cfg.battery;
        self.screen_rules = cfg.screens;
        self.readable = cfg.readable;
        self.share_auto = cfg.share_auto;
        self.pause_apps = cfg.pause_apps.iter().map(|e| e.exe.to_ascii_lowercase()).collect();
        self.paused_by = self.pausing_app();
        self.share_original = cfg.share_original;
        self.update_sharing();
        if self.role == Role::Main {
            self.register_keys(&cfg.keys);
            self.register_app_keys(&cfg.apps);
        }
        self.in_time = self.schedule_allows();
        self.follow_level(true);
        // Switched on, or an app added or switched on: fade in.
        let gained = self.rules.keys().any(|k| !before.contains(k));
        if (!was_active && self.active()) || gained {
            self.fade_new = Some(Instant::now() + Duration::from_millis(1500));
        }
        self.refresh();
        self.tell_helper();
    }

    /// Takes the hotkeys from the settings. Test copies add Shift, so they
    /// never fight the real Deonyx for its keys.
    fn register_keys(&mut self, k: &Keys) {
        let testing = std::env::var_os("DEONYX_CONFIG_DIR").is_some();
        let wanted = [keys::parse(&k.all), keys::parse(&k.front), keys::parse(&k.peek), keys::parse(&k.window)].map(|c| {
            c.map(|mut c| {
                if testing {
                    c.mods |= MOD_SHIFT;
                }
                c
            })
        });
        if wanted == self.keys && self.last_status.is_some() {
            return;
        }
        let ids = [HOTKEY_ALL, HOTKEY_FRONT, HOTKEY_PEEK, HOTKEY_WINDOW];
        let names = ["all", "front", "peek", "window"];
        let mut taken: Vec<String> = self.keys_taken.iter().filter(|t| t.starts_with("app:")).cloned().collect();
        for i in 0..4 {
            unsafe {
                let _ = UnregisterHotKey(None, ids[i]);
                if let Some(c) = wanted[i] {
                    if RegisterHotKey(None, ids[i], c.mods | MOD_NOREPEAT, c.vk).is_err() {
                        taken.push(names[i].to_string());
                    }
                }
            }
        }
        if !taken.is_empty() {
            note!("hotkeys taken by another program: {}", taken.join(", "));
        }
        self.keys = wanted;
        self.keys_taken = taken;
        self.publish();
    }

    /// Apps' own hotkeys. A taken one shows up as "app:<exe>" in keys_taken.
    fn register_app_keys(&mut self, apps: &[AppRule]) {
        let testing = std::env::var_os("DEONYX_CONFIG_DIR").is_some();
        let wanted: Vec<(String, Combo)> = apps
            .iter()
            .filter_map(|a| {
                let mut c = keys::parse(&a.hotkey)?;
                if testing {
                    c.mods |= MOD_SHIFT;
                }
                Some((a.key(), c))
            })
            .collect();
        if wanted == self.app_keys {
            return;
        }
        for i in 0..self.app_keys.len() {
            unsafe {
                let _ = UnregisterHotKey(None, HOTKEY_APP + i as i32);
            }
        }
        self.keys_taken.retain(|t| !t.starts_with("app:"));
        for (i, (exe, c)) in wanted.iter().enumerate() {
            if unsafe { RegisterHotKey(None, HOTKEY_APP + i as i32, c.mods | MOD_NOREPEAT, c.vk) }.is_err() {
                self.keys_taken.push(format!("app:{exe}"));
            }
        }
        self.app_keys = wanted;
        self.publish();
    }

    /// The program on the pause list that is running, if any.
    fn pausing_app(&self) -> Option<String> {
        self.pause_apps.iter().find(|e| self.programs.contains(*e)).cloned()
    }

    /// Hides the layers from screen sharing while that's wanted: always, or
    /// while a sharing app runs.
    fn update_sharing(&mut self) {
        self.sharing = self.share_auto && SHARE_APPS.iter().any(|a| self.programs.contains(*a));
        let hide = self.hidden() || self.desk.is_some();
        for l in self.layers.values() {
            l.hide_from_capture(hide);
        }
    }

    /// Looks at the running programs: game mode and automatic sharing.
    fn watch_programs(&mut self) {
        self.programs = running_programs();
        let was = self.active();
        let pausing = self.pausing_app();
        let sharing = self.share_auto && SHARE_APPS.iter().any(|a| self.programs.contains(*a));
        if sharing != self.sharing {
            self.update_sharing();
        }
        if pausing != self.paused_by {
            self.paused_by = pausing;
            if !was && self.active() {
                self.fade_new = Some(Instant::now() + Duration::from_millis(1500));
            }
            self.refresh();
        }
        self.publish();
    }

    /// The window hotkey: the window under the mouse on or off, whatever its
    /// app does. In admin mode a normal app's window belongs to the helper.
    fn toggle_window_at_mouse(&mut self) {
        // Off or paused, there's nothing to switch.
        if !self.active() {
            return;
        }
        let mut pt = POINT::default();
        let h = unsafe {
            let _ = GetCursorPos(&mut pt);
            GetAncestor(WindowFromPoint(pt), GA_ROOT)
        };
        if h.is_invalid() {
            return;
        }
        let pid = window_pid(h);
        if pid == std::process::id() || Some(pid) == self.other_pid() {
            return;
        }
        if self.role == Role::Main && !self.mine(pid) {
            if let Some(to) = self.helper.as_ref().and_then(|x| x.window) {
                helper::send(to, self.window, helper::WINDOW, &(h.0 as isize));
            }
            return;
        }
        self.toggle_window(h);
    }

    /// Switches one window. Switched back to what its app does, the window
    /// follows its app again.
    fn toggle_window(&mut self, h: HWND) {
        let key = h.0 as isize;
        self.window_over.remove(&key);
        let by_app = self.rule_for(h).is_some();
        if self.layers.contains_key(&key) {
            if by_app {
                self.window_over.insert(key, false);
            }
            self.retire(key);
        } else {
            if !by_app {
                self.window_over.insert(key, true);
            }
            self.ignored.remove(&key);
            self.fade_new = Some(Instant::now() + Duration::from_millis(1500));
            self.add(h);
        }
        self.publish();
    }

    /// Hands the settings to the helper, if there is one.
    fn tell_helper(&self) {
        if let (Some(to), Some(cfg)) = (self.helper.as_ref().and_then(|h| h.window), &self.last_cfg) {
            helper::send(to, self.window, helper::CONFIG, cfg);
        }
    }

    fn start_helper(&mut self) {
        let Some(window) = self.window else { return };
        match Helper::spawn(window) {
            Ok(h) => {
                self.helper = Some(h);
                self.helper_starts.push(Instant::now());
                self.upper = true;
            }
            Err(e) => note!("helper failed to start: {e}"),
        }
    }

    /// Starts the helper again if it died; if it keeps dying, darkens
    /// everything here instead, like without one.
    fn watch_helper(&mut self) {
        if self.helper.as_ref().is_none_or(|h| h.alive()) {
            return;
        }
        note!("the helper stopped");
        self.helper = None;
        self.helper_status = None;
        self.helper_starts.retain(|t| t.elapsed() < Duration::from_secs(300));
        if self.helper_starts.len() < 3 {
            self.start_helper();
        }
        if self.helper.is_none() {
            self.upper = false;
        }
        self.refresh();
    }

    /// A WM_COPYDATA from the other copy, or from the command line.
    fn receive(&mut self, kind: usize, from: u32, bytes: &[u8]) {
        match self.role {
            Role::Main if kind == helper::STATUS && self.helper.as_ref().is_some_and(|h| h.pid == from) => {
                let Ok(st) = serde_json::from_slice::<Status>(bytes) else { return };
                let hello = self.helper.as_ref().is_some_and(|h| h.window.map(|w| w.0 as isize) != Some(st.hwnd));
                if let Some(h) = self.helper.as_mut() {
                    if st.hwnd != 0 {
                        h.window = Some(HWND(st.hwnd as *mut c_void));
                    }
                }
                self.helper_status = Some(st);
                if hello {
                    self.tell_helper();
                }
                self.publish();
            }
            Role::Main if kind == helper::ORDER => {
                if let Ok(o) = serde_json::from_slice::<Order>(bytes) {
                    self.obey(o);
                }
            }
            Role::Helper { parent } if kind == helper::CONFIG && from == parent => {
                if let Ok(cfg) = serde_json::from_slice::<Config>(bytes) {
                    self.apply(cfg);
                }
            }
            Role::Helper { parent } if kind == helper::WINDOW && from == parent => {
                if let Ok(n) = serde_json::from_slice::<isize>(bytes) {
                    self.toggle_window(HWND(n as *mut c_void));
                }
            }
            Role::Helper { parent } if kind == helper::PEEK && from == parent => {
                if let Ok(on) = serde_json::from_slice::<bool>(bytes) {
                    self.peek_told = on;
                    self.fade_all(if on { 0.0 } else { self.level }, PEEK_MS);
                }
            }
            _ => {}
        }
    }

    /// A command from the command line (cli.rs).
    fn obey(&mut self, o: Order) {
        match o.cmd.as_str() {
            "on" => self.set_enabled(true),
            "off" => self.set_enabled(false),
            "toggle" => self.toggle(),
            "front" => self.front_app(),
            "window" => self.toggle_window_at_mouse(),
            "settings" => (self.on_request)(Request::OpenSettings),
            "app" if !o.app.is_empty() => (self.on_request)(Request::SetApp { exe: o.app, on: o.on }),
            "pause" => (self.on_request)(Request::Pause(o.minutes.unwrap_or(60).max(1))),
            "resume" => (self.on_request)(Request::Pause(0)),
            "profile" if !o.profile.is_empty() => (self.on_request)(Request::Profile(o.profile)),
            _ => {}
        }
    }

    /// Re-applies the rules to every layer, and fades out what no longer fits.
    fn refresh(&mut self) {
        let keep: Vec<isize> = self.layers.keys().copied().collect();
        for k in keep {
            let exe = self.layers[&k].exe.clone();
            let target = self.layers[&k].target;
            let pid = window_pid(target);
            let over = self.window_over.get(&k).copied();
            let rule = if self.active() && self.mine(pid) && !self.screen_rule(target).is_some_and(|s| s.off) && over != Some(false) {
                self.rule_of(&exe).or_else(|| (over == Some(true)).then(|| AppRule { exe: exe.clone(), ..Default::default() }))
            } else {
                None
            };
            let Some(rule) = rule else {
                self.retire(k);
                continue;
            };
            let look = self.look(&rule, target);
            let protect = if rule.protect { self.protect.get(&k).cloned().unwrap_or_default() } else { Vec::new() };
            let gap = self.gap();
            let l = self.layers.get_mut(&k).unwrap();
            l.params.lmin = look.lmin;
            l.params.lmax = look.lmax;
            l.params.warm = look.warm;
            l.params.boost = look.boost;
            l.params.dim = look.dim;
            l.manual = rule.areas.clone();
            l.protect = protect;
            l.set_find_photos(rule.photos);
            l.set_gap(gap);
            l.redraw(&self.gpu);
        }
        self.ignored.clear();
        self.watch_main_windows();
        self.rescan();
        self.publish();
    }

    fn set_enabled(&mut self, on: bool) {
        if self.enabled == on {
            return;
        }
        self.toggle();
    }

    fn toggle(&mut self) {
        self.enabled = !self.enabled;
        if let Some(c) = self.last_cfg.as_mut() {
            c.enabled = self.enabled;
        }
        if self.active() {
            self.fade_new = Some(Instant::now() + Duration::from_millis(1500));
        }
        self.refresh();
        self.tell_helper();
    }

    /// The schedule, a pause from the tray, and the battery, every few
    /// seconds.
    fn check_time(&mut self) {
        let was = self.active();
        let battery = on_battery();
        let gap_changed = battery != self.on_battery && self.battery_mode == "slow";
        self.on_battery = battery;
        self.in_time = self.schedule_allows();
        self.follow_level(false);
        if was != self.active() {
            if self.active() {
                self.fade_new = Some(Instant::now() + Duration::from_millis(1500));
            }
            self.refresh();
        } else if gap_changed {
            let gap = self.gap();
            for l in self.layers.values_mut() {
                l.set_gap(gap);
            }
        }
        self.publish();
    }

    /// Every layer (and those leaving) towards one strength.
    fn fade_all(&mut self, to: f32, ms: f32) {
        for l in self.layers.values_mut() {
            l.fade_to(to, ms);
        }
    }

    fn start_peek(&mut self) {
        let Some(c) = self.keys[2] else { return };
        if self.peek.is_some() {
            return;
        }
        self.peek = Some(c.vk);
        self.fade_all(0.0, PEEK_MS);
        self.tell_helper_peek(true);
    }

    /// Ends peeking once its key is let go.
    fn check_peek(&mut self) {
        let Some(vk) = self.peek else { return };
        if keys::held(vk) {
            return;
        }
        self.peek = None;
        self.fade_all(self.level, PEEK_MS);
        self.tell_helper_peek(false);
    }

    fn tell_helper_peek(&self, on: bool) {
        if let Some(to) = self.helper.as_ref().and_then(|h| h.window) {
            helper::send(to, self.window, helper::PEEK, &on);
        }
    }

    fn publish(&mut self) {
        let mut running: HashMap<String, usize> = HashMap::new();
        for l in self.layers.values() {
            // Count main windows only, not menus and tooltips. A window that
            // is dark already counts while it's dimmed.
            if unsafe { GetWindow(l.target, GW_OWNER) }.is_err() && (!l.left_as_is() || self.dim > 0.0) {
                *running.entry(l.exe.clone()).or_default() += 1;
            }
        }
        if let Some(hs) = &self.helper_status {
            for (exe, n) in &hs.running {
                *running.entry(exe.clone()).or_default() += n;
            }
        }
        let is_helper = matches!(self.role, Role::Helper { .. });
        let mut windows: Vec<(isize, bool)> = Vec::new();
        if is_helper {
            windows = self.layers.iter().map(|(k, l)| (*k, l.left_as_is())).collect();
            windows.sort_unstable();
        }
        let s = Status {
            enabled: self.enabled,
            waiting: self.enabled && !self.in_time,
            paused: self.enabled && self.paused(),
            battery: self.enabled && self.battery_paused(),
            paused_by: if self.enabled { self.paused_by.clone().unwrap_or_default() } else { String::new() },
            sharing: self.sharing,
            running,
            admin: self.admin.clone(),
            system: self.system.clone(),
            inspecting: PICK.with(|p| p.borrow().is_some()),
            keys_taken: self.keys_taken.clone(),
            capture: if self.desk.is_some() { "screen" } else { "window" }.into(),
            hwnd: if is_helper { self.window.map_or(0, |h| h.0 as isize) } else { 0 },
            windows,
        };
        let windows: usize = s.running.values().sum();
        NOW.with(|n| {
            n.set(status_bits::make(s.enabled, s.paused, s.waiting, s.battery, !s.paused_by.is_empty(), s.sharing, windows))
        });
        if self.last_status.as_ref() != Some(&s) {
            (self.on_status)(s.clone());
            self.last_status = Some(s);
        }
    }

    fn handle(&mut self, event: u32, key: isize) {
        let h = HWND(key as *mut c_void);
        match event {
            EVENT_OBJECT_SHOW | EVENT_OBJECT_UNCLOAKED => self.add(h),
            EVENT_OBJECT_HIDE | EVENT_OBJECT_DESTROY => self.remove(key),
            EVENT_OBJECT_LOCATIONCHANGE if !self.screen_rules.is_empty() && self.layers.contains_key(&key) => {
                self.moved.insert(key);
            }
            _ => {}
        }
    }

    /// Windows that moved to another screen take that screen's look, or go
    /// light on a screen that's left alone.
    fn check_moved(&mut self) {
        if self.moved.is_empty() {
            return;
        }
        let moved: Vec<isize> = std::mem::take(&mut self.moved).into_iter().collect();
        let mut changed = false;
        for k in moved {
            let Some(l) = self.layers.get(&k) else { continue };
            let id = screens::of_window(l.target, &self.screens);
            if id.as_ref() != self.screen_of.get(&k) {
                match id {
                    Some(id) => self.screen_of.insert(k, id),
                    None => self.screen_of.remove(&k),
                };
                changed = true;
            }
        }
        if changed {
            self.refresh();
        }
    }

    fn sync_all(&mut self) {
        let mut gone = Vec::new();
        for (k, l) in self.layers.iter_mut() {
            if !l.sync(true) {
                gone.push(*k);
            }
        }
        for k in gone {
            self.remove(k);
        }
        self.leaving.retain_mut(|l| l.sync(true));
    }

    fn frames(&mut self) {
        let mut poke = Vec::new();
        let mut changed = false;
        let screen = self.desk.as_mut().map(|d| d.poll(&self.gpu));
        for (k, l) in self.layers.iter_mut() {
            let was = l.left_as_is();
            let got = match (&self.desk, &screen) {
                (Some(d), Some(c)) => l.poll_screen(&self.gpu, d, c, true),
                _ => l.poll(&self.gpu, true),
            };
            match got {
                Ok(true) => {
                    poke.push((*k, l.exe.clone()));
                    changed |= was != l.left_as_is();
                }
                Ok(false) => {}
                Err(e) => note!("frame error: {e}"),
            }
        }
        for l in &mut self.leaving {
            let _ = match (&self.desk, &screen) {
                (Some(d), Some(c)) => l.poll_screen(&self.gpu, d, c, true),
                _ => l.poll(&self.gpu, true),
            };
        }
        // The window changed: if a 3D view moved with it, find out now
        // rather than at the next round.
        for (k, exe) in poke {
            if self.protects(&exe) {
                self.finder.poke(k);
            }
        }
        if changed {
            self.publish();
        }
        if self.since.elapsed().as_secs_f64() >= 5.0 {
            let (f, w): (u32, f64) =
                self.layers.values().map(|l| (l.frames, l.work_ms)).fold((0, 0.0), |a, b| (a.0 + b.0, a.1 + b.1));
            if f > 0 && std::env::var_os("DEONYX_DEBUG").is_some() {
                println!("{} windows, {} frames / 5 s, {:.2} ms per frame", self.layers.len(), f, w / f as f64);
            }
            for l in self.layers.values_mut() {
                l.frames = 0;
                l.work_ms = 0.0;
            }
            self.since = Instant::now();
        }
    }

    /// Between frames: fades, delayed frames, new photos. Returns whether
    /// anything is still moving, so the loop comes back soon.
    fn tick(&mut self) -> bool {
        let mut busy = false;
        for l in self.layers.values_mut() {
            l.tick(&self.gpu);
            busy |= l.fading();
        }
        for l in &mut self.leaving {
            l.tick(&self.gpu);
        }
        // Gone once fully faded out.
        self.leaving.retain(|l| l.fading());
        busy || !self.leaving.is_empty()
    }

    fn take_protect(&mut self) {
        let Some(found) = self.finder.take() else { return };
        self.protect = found;
        let keys: Vec<isize> = self.layers.keys().copied().collect();
        for k in keys {
            if !self.protects(&self.layers[&k].exe) {
                continue;
            }
            let p = self.protect.get(&k).cloned().unwrap_or_default();
            let l = self.layers.get_mut(&k).unwrap();
            if p != l.protect {
                l.protect = p;
                l.redraw(&self.gpu);
            }
        }
    }

    /// Ctrl+Alt+F: the app of the window in front.
    fn front_app(&mut self) {
        unsafe {
            let h = GetAncestor(GetForegroundWindow(), GA_ROOTOWNER);
            let pid = window_pid(h);
            if h.is_invalid() || pid == std::process::id() {
                return;
            }
            if let Some(path) = crate::apps::exe_path(pid) {
                let exe = path.rsplit('\\').next().unwrap_or(&path).to_string();
                (self.on_request)(Request::ToggleApp { exe, path });
            }
        }
    }

    /// Why a window is (or isn't) dark.
    fn diagnose(&mut self, h: HWND) -> Report {
        let h = unsafe { GetAncestor(h, GA_ROOT) };
        let pid = window_pid(h);
        let path = crate::apps::exe_path(pid).unwrap_or_default();
        let exe = path.rsplit('\\').next().unwrap_or("").to_string();
        let key = exe.to_ascii_lowercase();
        let mut title = [0u16; 256];
        let n = unsafe { GetWindowTextW(h, &mut title) };
        let mut r = Report { exe, path, title: String::from_utf16_lossy(&title[..n.max(0) as usize]), reason: String::new() };
        let class = class_name(h);
        let mut affinity = 0u32;
        let ex = unsafe { GetWindowLongW(h, GWL_EXSTYLE) } as u32;
        let by_helper = self
            .helper_status
            .as_ref()
            .and_then(|s| s.windows.iter().find(|(w, _)| *w == h.0 as isize))
            .map(|(_, dark)| *dark);
        r.reason = if pid == std::process::id() || Some(pid) == self.other_pid() {
            "own"
        } else if SHELL_CLASSES.contains(&class.as_str()) || SHELL_EXES.contains(&key.as_str()) {
            "shell"
        } else if let Some(l) = self.layers.get(&(h.0 as isize)) {
            if l.left_as_is() {
                "already_dark"
            } else {
                "darkened"
            }
        } else if let Some(dark) = by_helper {
            if dark {
                "already_dark"
            } else {
                "darkened"
            }
        } else if !self.enabled {
            "off"
        } else if self.paused() {
            "paused"
        } else if self.battery_paused() {
            "battery"
        } else if self.paused_by.is_some() {
            "game"
        } else if self.window_over.get(&(h.0 as isize)) == Some(&false) {
            "window_off"
        } else if !self.in_time {
            "waiting"
        } else if self.all_rules.get(&key).is_some_and(|x| !x.enabled) {
            "app_off"
        } else if self.everything && self.exclude.contains(&key) {
            "excluded"
        } else if self.everything && !self.all_rules.contains_key(&key) && self.leave.contains(&key) {
            "left_alone"
        } else if !self.everything && !self.all_rules.contains_key(&key) {
            "not_listed"
        } else if self.screen_rule(h).is_some_and(|s| s.off) {
            "screen_off"
        } else if unsafe { GetWindowDisplayAffinity(h, &mut affinity) }.is_ok() && affinity != 0 {
            "no_capture"
        } else if self.above_us(pid) {
            if self.level(pid) >= LEVEL_SYSTEM {
                "service"
            } else {
                "admin"
            }
        } else if ex & WS_EX_TRANSPARENT.0 != 0 && ex & WS_EX_LAYERED.0 != 0 {
            "helper"
        } else if is_fullscreen(h) {
            "fullscreen"
        } else if self.ignored.contains(&(h.0 as isize)) {
            "failed"
        } else {
            "unknown"
        }
        .to_string();
        r
    }

    fn start_pick(&mut self, purpose: Purpose) {
        if PICK.with(|p| p.borrow().is_some()) {
            return;
        }
        let hook = unsafe { SetWindowsHookExW(WH_MOUSE_LL, Some(mouse_ll), None, 0) };
        if let Ok(hook) = hook {
            let p = Pick { hook, purpose, at: None, released: false, cancelled: false, since: Instant::now() };
            PICK.with(|s| *s.borrow_mut() = Some(p));
        }
        self.publish();
    }

    fn stop_pick(&mut self) {
        if let Some(p) = PICK.with(|p| p.borrow_mut().take()) {
            unsafe {
                let _ = UnhookWindowsHookEx(p.hook);
            }
        }
        self.publish();
    }

    /// The click was caught (or the wait given up).
    fn check_pick(&mut self) {
        let state = PICK.with(|p| p.borrow().as_ref().map(|s| (s.purpose, s.at, s.released, s.cancelled, s.since.elapsed())));
        let Some((purpose, at, released, cancelled, age)) = state else { return };
        if cancelled || age > Duration::from_secs(30) {
            self.stop_pick();
        } else if let (Some(pt), true) = (at, released) {
            self.stop_pick();
            let h = unsafe { WindowFromPoint(pt) };
            match purpose {
                Purpose::Inspect => {
                    let report = self.diagnose(h);
                    (self.on_request)(Request::Inspected(report));
                }
                Purpose::Area => self.pick_area(h, pt),
            }
        }
    }

    /// The part of the window under a click (a video, a canvas, a preview),
    /// found through UI Automation, becomes an area kept in real colours.
    fn pick_area(&mut self, h: HWND, pt: POINT) {
        let root = unsafe { GetAncestor(h, GA_ROOT) };
        let path = crate::apps::exe_path(window_pid(root)).unwrap_or_default();
        let exe = path.rsplit('\\').next().unwrap_or("").to_string();
        let frame = frame_rect(root);
        let area = part_at(pt, frame)
            .map(|r| Area { left: r.left - frame.left, top: r.top - frame.top, right: frame.right - r.right, bottom: frame.bottom - r.bottom })
            .ok_or_else(|| "no_part".to_string());
        (self.on_request)(Request::Area { exe, path, area });
    }

    fn start_drawing(&mut self, exe: String, hint: String) {
        let key = exe.to_ascii_lowercase();
        // The app's biggest window.
        let mut best: Option<(i64, HWND)> = None;
        for h in top_level_windows() {
            unsafe {
                if !IsWindowVisible(h).as_bool() || GetWindow(h, GW_OWNER).is_ok() || IsIconic(h).as_bool() {
                    continue;
                }
            }
            if self.exe_key(window_pid(h)).as_deref() != Some(key.as_str()) {
                continue;
            }
            let r = frame_rect(h);
            let area = width(&r) as i64 * height(&r) as i64;
            if best.is_none_or(|(a, _)| area > a) {
                best = Some((area, h));
            }
        }
        match best {
            Some((_, h)) if select::begin(h, &hint).is_ok() => self.drawing = Some(exe),
            _ => (self.on_request)(Request::Area { exe, path: String::new(), area: Err("not_open".into()) }),
        }
    }

    fn check_drawing(&mut self) {
        let Some(outcome) = select::take() else { return };
        let exe = self.drawing.take().unwrap_or_default();
        let area = match outcome {
            select::Outcome::Drawn(a) => Ok(a),
            select::Outcome::Cancelled => Err("cancelled".to_string()),
        };
        (self.on_request)(Request::Area { exe, path: String::new(), area });
    }

    fn set_presets(&mut self, list: Vec<Preset>) {
        self.leave = presets::left_alone(&list).into_iter().collect();
        self.finder.set_words(presets::view_words(&list));
        self.refresh();
    }
}

/// The UI part under a point that is worth keeping: big enough to be a
/// picture or a view, smaller than the whole window.
fn part_at(pt: POINT, frame: RECT) -> Option<RECT> {
    unsafe {
        let uia: IUIAutomation = CoCreateInstance(&CUIAutomation, None, CLSCTX_INPROC_SERVER).ok()?;
        let walker = uia.ControlViewWalker().ok()?;
        let mut e = uia.ElementFromPoint(pt).ok()?;
        let frame_area = (width(&frame) as i64 * height(&frame) as i64).max(1);
        for _ in 0..12 {
            if let Ok(r) = e.CurrentBoundingRectangle() {
                let r = RECT {
                    left: r.left.max(frame.left),
                    top: r.top.max(frame.top),
                    right: r.right.min(frame.right),
                    bottom: r.bottom.min(frame.bottom),
                };
                let a = width(&r) as i64 * height(&r) as i64;
                if a * 100 >= frame_area * 97 {
                    return None;
                }
                if width(&r) >= 40 && height(&r) >= 40 {
                    return Some(r);
                }
            }
            e = walker.GetParentElement(&e).ok()?;
        }
        None
    }
}

/// The hidden windows' procedure: keeps what the other copy sends.
unsafe extern "system" fn signal_proc(h: HWND, m: u32, w: WPARAM, l: LPARAM) -> LRESULT {
    if m == WM_COPYDATA {
        if let Some(got) = helper::received(w, l) {
            // `deonyx --status` waits for its answer: give it now.
            if got.0 == helper::ORDER && serde_json::from_slice::<Order>(&got.2).is_ok_and(|o| o.cmd == "status") {
                return LRESULT(NOW.with(|n| n.get()));
            }
            INBOX.with(|i| i.borrow_mut().push(got));
            return LRESULT(1);
        }
        return LRESULT(0);
    }
    DefWindowProcW(h, m, w, l)
}

/// The helper's hidden window, which the main copy sends the settings to.
fn helper_window() -> Option<HWND> {
    unsafe {
        let hinst = GetModuleHandleW(None).ok()?;
        let wc = WNDCLASSW {
            lpfnWndProc: Some(signal_proc),
            hInstance: hinst.into(),
            lpszClassName: w!("DeonyxHelper"),
            ..Default::default()
        };
        RegisterClassW(&wc);
        CreateWindowExW(
            WINDOW_EX_STYLE(0),
            w!("DeonyxHelper"),
            w!("Deonyx helper"),
            WINDOW_STYLE(0),
            0,
            0,
            0,
            0,
            Some(HWND_MESSAGE),
            None,
            Some(hinst.into()),
            None,
        )
        .ok()
    }
}

/// The hidden window a second launch looks for (see admin.rs and cli.rs).
fn signal_window(msg: u32) -> Option<HWND> {
    unsafe {
        let hinst = GetModuleHandleW(None).ok()?;
        let wc = WNDCLASSW {
            lpfnWndProc: Some(signal_proc),
            hInstance: hinst.into(),
            lpszClassName: crate::admin::SIGNAL_CLASS,
            ..Default::default()
        };
        RegisterClassW(&wc);
        let h = CreateWindowExW(
            WINDOW_EX_STYLE(0),
            crate::admin::SIGNAL_CLASS,
            w!("Deonyx"),
            WS_POPUP,
            0,
            0,
            0,
            0,
            None,
            None,
            Some(hinst.into()),
            None,
        )
        .ok()?;
        // Lets a Deonyx without administrator rights reach us when we have
        // them: a second launch, the command line, and our helper.
        let _ = ChangeWindowMessageFilterEx(h, msg, MSGFLT_ALLOW, None);
        let _ = ChangeWindowMessageFilterEx(h, WM_COPYDATA, MSGFLT_ALLOW, None);
        Some(h)
    }
}

/// After a failed run: hooks, the hidden window, hotkeys and a waiting click
/// it left behind.
fn clean_up_after_failure() {
    let (hooks, window) = LEFT.with(|l| std::mem::take(&mut *l.borrow_mut()));
    unsafe {
        for h in hooks {
            let _ = UnhookWinEvent(h);
        }
        if let Some(w) = window {
            let _ = DestroyWindow(w);
        }
        for id in [HOTKEY_ALL, HOTKEY_FRONT, HOTKEY_PEEK, HOTKEY_WINDOW].into_iter().chain(HOTKEY_APP..HOTKEY_APP + 64) {
            let _ = UnregisterHotKey(None, id);
        }
    }
    if let Some(p) = PICK.with(|p| p.borrow_mut().take()) {
        unsafe {
            let _ = UnhookWindowsHookEx(p.hook);
        }
    }
}

/// For testing the recovery: DEONYX_CRASH_TEST=engine makes a test copy's
/// engine fail once, a few seconds in.
static CRASHED_ONCE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

fn run(cfg: Config, role: Role, cmds: Arc<Mutex<Vec<Cmd>>>, on_status: OnStatus, on_request: OnRequest) -> Result<()> {
    clean_up_after_failure();
    let started = Instant::now();
    let crash_test = std::env::var_os("DEONYX_CONFIG_DIR").is_some() && std::env::var("DEONYX_CRASH_TEST").as_deref() == Ok("engine");
    crate::layer::register_class()?;
    let open_msg = unsafe { RegisterWindowMessageW(crate::admin::OPEN_MSG) };
    let signal = match role {
        Role::Main => signal_window(open_msg),
        Role::Helper { .. } => helper_window(),
    };
    let frame_event = unsafe { CreateEventW(None, false, false, None)? };
    let gpu = Gpu::new()?;
    let desk = if crate::desk::needed() {
        match Desktop::new(&gpu) {
            Ok(d) => Some(d),
            Err(e) => {
                note!("screen copy failed, capturing windows instead: {e}");
                None
            }
        }
    } else {
        None
    };
    let mut m = Manager {
        gpu,
        rules: HashMap::new(),
        all_rules: HashMap::new(),
        everything: false,
        exclude: HashSet::new(),
        leave: HashSet::new(),
        style: "gray".into(),
        warm: 0.0,
        schedule: ("always".into(), 0, 0),
        place: (0.0, 0.0),
        gradual: false,
        level: 1.0,
        dim: 0.0,
        enabled: true,
        in_time: true,
        paused_until: 0,
        battery_mode: "keep".into(),
        on_battery: on_battery(),
        share_original: false,
        split: std::env::var_os("DEONYX_SPLIT").is_some(),
        layers: HashMap::new(),
        leaving: Vec::new(),
        fade_new: None,
        peek: None,
        peek_told: false,
        desk,
        keys: [None; 4],
        keys_taken: Vec::new(),
        app_keys: Vec::new(),
        readable: true,
        programs: running_programs(),
        pause_apps: HashSet::new(),
        paused_by: None,
        share_auto: false,
        sharing: false,
        window_over: HashMap::new(),
        screens: screens::list(),
        screen_rules: Vec::new(),
        screen_of: HashMap::new(),
        moved: HashSet::new(),
        exe_of: HashMap::new(),
        levels: HashMap::new(),
        my_level: integrity(std::process::id()),
        admin: HashMap::new(),
        system: HashMap::new(),
        ignored: HashSet::new(),
        finder: Finder::start(),
        protect: HashMap::new(),
        frame_event,
        since: Instant::now(),
        on_status,
        on_request,
        last_status: None,
        drawing: None,
        role,
        upper: false,
        helper: None,
        helper_status: None,
        helper_starts: Vec::new(),
        last_cfg: None,
        window: signal,
    };
    if m.desk.is_some() {
        note!("Windows build {}: copying the screen instead of capturing windows", crate::desk::windows_build());
    }
    // As administrator, normal apps go to a helper without those rights.
    // DEONYX_HELPER_TEST tries the same without them.
    let wants_helper = crate::admin::elevated() || std::env::var_os("DEONYX_HELPER_TEST").is_some();
    if role == Role::Main && wants_helper && std::env::var_os("DEONYX_NO_HELPER").is_none() {
        m.start_helper();
    }
    let list = presets::load();
    m.leave = presets::left_alone(&list).into_iter().collect();
    m.finder.set_words(presets::view_words(&list));
    m.apply(cfg);

    unsafe {
        let hook = |a: u32, b: u32| SetWinEventHook(a, b, None, Some(win_event), 0, 0, WINEVENT_OUTOFCONTEXT);
        let hooks = [
            hook(EVENT_SYSTEM_FOREGROUND, EVENT_SYSTEM_MINIMIZEEND),
            hook(EVENT_OBJECT_DESTROY, EVENT_OBJECT_REORDER),
            hook(EVENT_OBJECT_LOCATIONCHANGE, EVENT_OBJECT_LOCATIONCHANGE),
            hook(EVENT_OBJECT_CLOAKED, EVENT_OBJECT_UNCLOAKED),
        ];
        LEFT.with(|l| *l.borrow_mut() = (hooks.to_vec(), signal));

        let mut msg = MSG::default();
        let mut last_rescan = Instant::now();
        let mut last_purge = Instant::now();
        let mut last_time = Instant::now();
        let mut busy = false;
        'outer: loop {
            // Quick rounds while something fades, a key is held for peeking,
            // or the screen copy needs looking at; otherwise wait for news.
            let screen_copy = m.desk.is_some() && !m.layers.is_empty();
            let wait = if busy || m.peek.is_some() || screen_copy { 16 } else { 250 };
            MsgWaitForMultipleObjects(Some(&[frame_event]), false, wait, QS_ALLINPUT);
            while PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE).as_bool() {
                if msg.message == open_msg {
                    (m.on_request)(Request::OpenSettings);
                    continue;
                }
                match msg.message {
                    WM_HOTKEY if msg.wParam.0 as i32 == HOTKEY_ALL => m.toggle(),
                    WM_HOTKEY if msg.wParam.0 as i32 == HOTKEY_FRONT => m.front_app(),
                    WM_HOTKEY if msg.wParam.0 as i32 == HOTKEY_PEEK => m.start_peek(),
                    WM_HOTKEY if msg.wParam.0 as i32 == HOTKEY_WINDOW => m.toggle_window_at_mouse(),
                    WM_HOTKEY if msg.wParam.0 as i32 >= HOTKEY_APP => {
                        let i = (msg.wParam.0 as i32 - HOTKEY_APP) as usize;
                        if let Some((exe, _)) = m.app_keys.get(i).cloned() {
                            (m.on_request)(Request::SetApp { exe, on: None });
                        }
                    }
                    WM_ENGINE => {
                        let list: Vec<Cmd> = std::mem::take(&mut *cmds.lock().unwrap());
                        for c in list {
                            match c {
                                Cmd::Config(cfg) => m.apply(cfg),
                                Cmd::Presets(p) => m.set_presets(p),
                                Cmd::Inspect => m.start_pick(Purpose::Inspect),
                                Cmd::PickArea => m.start_pick(Purpose::Area),
                                Cmd::StopInspect => m.stop_pick(),
                                Cmd::DrawArea { exe, hint } => m.start_drawing(exe, hint),
                                Cmd::Quit => break 'outer,
                            }
                        }
                    }
                    _ => {
                        let _ = TranslateMessage(&msg);
                        DispatchMessageW(&msg);
                    }
                }
            }
            let inbox: Vec<(usize, u32, Vec<u8>)> = INBOX.with(|i| std::mem::take(&mut *i.borrow_mut()));
            for (kind, from, bytes) in &inbox {
                m.receive(*kind, *from, bytes);
            }
            let events: Vec<(u32, isize)> = EVENTS.with(|e| std::mem::take(&mut *e.borrow_mut()));
            for (ev, key) in &events {
                m.handle(*ev, *key);
            }
            m.check_peek();
            m.frames();
            busy = m.tick();
            m.take_protect();
            m.sync_all();
            m.check_moved();
            m.check_pick();
            if select::active() {
                m.check_drawing();
            }
            if last_rescan.elapsed().as_secs() >= 1 {
                m.rescan();
                m.watch_helper();
                last_rescan = Instant::now();
            }
            if crash_test && started.elapsed().as_secs() >= 3 && !CRASHED_ONCE.swap(true, std::sync::atomic::Ordering::Relaxed) {
                panic!("crash test");
            }
            if last_time.elapsed().as_secs() >= 5 {
                m.screens = screens::list();
                m.watch_programs();
                m.check_time();
                last_time = Instant::now();
            }
            // Process ids get reused; forget old ones now and then.
            if last_purge.elapsed().as_secs() >= 10 {
                m.exe_of.clear();
                m.levels.clear();
                last_purge = Instant::now();
            }
        }
        m.stop_pick();
        m.layers.clear();
        m.leaving.clear();
        // Ends the helper too.
        m.helper = None;
        if let Some(h) = signal {
            let _ = DestroyWindow(h);
        }
        for h in hooks {
            let _ = UnhookWinEvent(h);
        }
        LEFT.with(|l| *l.borrow_mut() = (Vec::new(), None));
        for id in [HOTKEY_ALL, HOTKEY_FRONT, HOTKEY_PEEK, HOTKEY_WINDOW] {
            let _ = UnregisterHotKey(None, id);
        }
        for i in 0..m.app_keys.len() {
            let _ = UnregisterHotKey(None, HOTKEY_APP + i as i32);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::ease;

    #[test]
    fn eases_in_after_the_start_and_out_before_the_end() {
        // 20:00 to 07:00, past midnight.
        let (from, to) = (20.0 * 60.0, 7.0 * 60.0);
        assert_eq!(ease(from, to, from), 0.0);
        assert!((ease(from, to, from + 15.0) - 0.5).abs() < 1e-6);
        assert_eq!(ease(from, to, from + 30.0), 1.0);
        assert_eq!(ease(from, to, 30.0), 1.0);
        assert!((ease(from, to, to - 15.0) - 0.5).abs() < 1e-6);
        assert_eq!(ease(from, to, to), 0.0);
        // Outside the stretch the schedule is off anyway.
        assert_eq!(ease(from, to, 12.0 * 60.0), 1.0);
    }

    #[test]
    fn a_short_stretch_never_gets_fully_dark() {
        // 40 minutes: the middle is 20 minutes from either end.
        let v = ease(600.0, 640.0, 620.0);
        assert!(v > 0.6 && v < 1.0, "{v}");
    }
}
