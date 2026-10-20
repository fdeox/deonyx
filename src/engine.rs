// The darkening engine. Runs on its own thread with its own message loop:
// WinEvent hooks, the hotkeys and capture frames all arrive there, so the
// settings window can never slow it down.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::ffi::c_void;
use std::sync::{mpsc, Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use windows::core::{w, Result};
use windows::Win32::Foundation::*;
use windows::Win32::System::Com::{CoInitializeEx, COINIT_MULTITHREADED};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Threading::{CreateEventW, GetCurrentThreadId};
use windows::Win32::UI::Accessibility::{SetWinEventHook, UnhookWinEvent, HWINEVENTHOOK};
use windows::Win32::UI::Input::KeyboardAndMouse::{RegisterHotKey, UnregisterHotKey, MOD_ALT, MOD_CONTROL, MOD_NOREPEAT, MOD_SHIFT};
use windows::Win32::UI::WindowsAndMessaging::*;

use crate::config::{AppRule, Area, Config};
use crate::gpu::Gpu;
use crate::helper::{self, Helper};
use crate::layer::{Layer, Look, OVERLAY_CLASS};
use crate::protect::Finder;
use crate::select;
use crate::win::*;

const OBJID_WINDOW: i32 = 0;
const WM_ENGINE: u32 = WM_APP + 1;
const HOTKEY_ALL: i32 = 1;
const HOTKEY_FRONT: i32 = 2;

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
    /// Wait for a click on a window, then explain what Deonyx does with it.
    Inspect,
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
    Inspected(Report),
    /// The outcome of DrawArea: the area, or why there is none.
    Area { exe: String, area: std::result::Result<Area, String> },
}

/// What Deonyx does with a window, for "Why isn't this window dark?".
#[derive(Clone, Debug, Default, Serialize)]
pub struct Report {
    pub exe: String,
    pub path: String,
    pub title: String,
    /// darkened, already_dark, not_listed, excluded, app_off, off, waiting,
    /// admin, service, no_capture, helper, fullscreen, shell, own, failed or
    /// unknown.
    pub reason: String,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Status {
    pub enabled: bool,
    /// On, but the schedule says not now.
    pub waiting: bool,
    /// Lower-case exe name -> number of its main windows being darkened.
    pub running: HashMap<String, usize>,
    /// Lower-case exe name -> main windows left alone because the program
    /// runs as administrator and we don't: Windows won't let a layer sit
    /// on them.
    pub admin: HashMap<String, usize>,
    /// Same, for windows of a Windows service (system level), which nothing
    /// can draw over.
    pub system: HashMap<String, usize>,
    /// Waiting for a click, for "Why isn't this window dark?".
    pub inspecting: bool,
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

pub type OnStatus = Box<dyn Fn(Status) + Send>;
pub type OnRequest = Box<dyn Fn(Request) + Send>;

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
                if let Err(e) = run(cfg, role, queue, on_status, on_request) {
                    eprintln!("engine stopped: {e}");
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
}

unsafe extern "system" fn win_event(_: HWINEVENTHOOK, event: u32, hwnd: HWND, obj: i32, child: i32, _: u32, _: u32) {
    if obj != OBJID_WINDOW || child != 0 {
        return;
    }
    EVENTS.with(|e| e.borrow_mut().push((event, hwnd.0 as isize)));
}

/// "Why isn't this window dark?": the next click anywhere is caught, not
/// passed on.
struct Pick {
    hook: HHOOK,
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
    style: String,
    warm: f32,
    schedule: (String, u32, u32),
    enabled: bool,
    /// The schedule allows darkening right now.
    in_time: bool,
    split: bool,
    layers: HashMap<isize, Layer>,
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

impl Manager {
    fn active(&self) -> bool {
        self.enabled && self.in_time
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
            _ => true,
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

    fn look(&self, rule: &AppRule) -> Look {
        let (lmin, lmax) = rule.lightness(&self.style);
        Look { lmin, lmax, warm: self.warm, split: self.split, owner: true }
    }

    /// The rule for an app: its own, or the default one in "every light
    /// window" mode.
    fn rule_of(&self, key: &str) -> Option<AppRule> {
        if let Some(r) = self.rules.get(key) {
            return Some(r.clone());
        }
        let own = self.all_rules.contains_key(key);
        if self.everything && !own && !self.exclude.contains(key) && !SHELL_EXES.contains(&key) {
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
        let rule = self.rule_of(&key)?;
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
        match Layer::new(&self.gpu, h, &self.look(&rule)) {
            Ok(mut l) => {
                if let Err(e) = l.on_frame_arrived(self.frame_event) {
                    eprintln!("frame notification failed: {e}");
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
                self.layers.insert(key, l);
                self.watch_main_windows();
                self.publish();
            }
            Err(e) => {
                eprintln!("capture failed ({}): {e}", class_name(h));
                self.ignored.insert(key);
            }
        }
    }

    fn remove(&mut self, key: isize) {
        if self.layers.remove(&key).is_some() {
            self.watch_main_windows();
            self.publish();
        }
        self.ignored.remove(&key);
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
        self.last_cfg = Some(cfg.clone());
        self.enabled = cfg.enabled;
        self.all_rules = cfg.apps.iter().filter(|r| !r.exe.is_empty()).map(|r| (r.key(), r.clone())).collect();
        self.rules = self.all_rules.iter().filter(|(_, r)| r.enabled).map(|(k, r)| (k.clone(), r.clone())).collect();
        self.everything = cfg.everything;
        self.exclude = cfg.exclude.iter().map(|e| e.exe.to_ascii_lowercase()).collect();
        self.style = cfg.style;
        self.warm = cfg.warm.clamp(0.0, 1.0);
        self.schedule = (cfg.schedule, minutes(&cfg.from), minutes(&cfg.to));
        self.in_time = self.schedule_allows();
        self.refresh();
        self.tell_helper();
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
            Err(e) => eprintln!("helper failed to start: {e}"),
        }
    }

    /// Starts the helper again if it died; if it keeps dying, darkens
    /// everything here instead, like without one.
    fn watch_helper(&mut self) {
        if self.helper.as_ref().is_none_or(|h| h.alive()) {
            return;
        }
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

    /// A WM_COPYDATA from the other copy.
    fn receive(&mut self, kind: usize, from: u32, bytes: &[u8]) {
        match self.role {
            Role::Main if kind == helper::STATUS && self.helper.as_ref().is_some_and(|h| h.pid == from) => {
                let Ok(st) = serde_json::from_slice::<Status>(bytes) else { return };
                let hello = self.helper.as_ref().is_some_and(|h| h.window.is_none());
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
            Role::Helper { parent } if kind == helper::CONFIG && from == parent => {
                if let Ok(cfg) = serde_json::from_slice::<Config>(bytes) {
                    self.apply(cfg);
                }
            }
            _ => {}
        }
    }

    /// Re-applies the rules to every layer, and drops what no longer fits.
    fn refresh(&mut self) {
        if !self.active() {
            // Nothing to capture while switched off.
            self.layers.clear();
        }
        let keep: Vec<isize> = self.layers.keys().copied().collect();
        for k in keep {
            let exe = self.layers[&k].exe.clone();
            let pid = window_pid(self.layers[&k].target);
            let rule = if self.mine(pid) { self.rule_of(&exe) } else { None };
            let Some(rule) = rule else {
                self.layers.remove(&k);
                continue;
            };
            let look = self.look(&rule);
            let protect = if rule.protect { self.protect.get(&k).cloned().unwrap_or_default() } else { Vec::new() };
            let l = self.layers.get_mut(&k).unwrap();
            l.params.lmin = look.lmin;
            l.params.lmax = look.lmax;
            l.params.warm = look.warm;
            l.manual = rule.areas.clone();
            l.protect = protect;
            l.redraw(&self.gpu);
        }
        self.ignored.clear();
        self.watch_main_windows();
        self.rescan();
        self.publish();
    }

    fn toggle(&mut self) {
        self.enabled = !self.enabled;
        if let Some(c) = self.last_cfg.as_mut() {
            c.enabled = self.enabled;
        }
        self.refresh();
        self.tell_helper();
    }

    fn check_schedule(&mut self) {
        let now = self.schedule_allows();
        if now != self.in_time {
            self.in_time = now;
            self.refresh();
        }
    }

    fn publish(&mut self) {
        let mut running: HashMap<String, usize> = HashMap::new();
        for l in self.layers.values() {
            // Count main windows only, not menus and tooltips.
            if unsafe { GetWindow(l.target, GW_OWNER) }.is_err() && !l.left_as_is() {
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
            running,
            admin: self.admin.clone(),
            system: self.system.clone(),
            inspecting: PICK.with(|p| p.borrow().is_some()),
            hwnd: if is_helper { self.window.map_or(0, |h| h.0 as isize) } else { 0 },
            windows,
        };
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
            _ => {}
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
    }

    fn frames(&mut self) {
        let mut poke = Vec::new();
        let mut changed = false;
        for (k, l) in self.layers.iter_mut() {
            let was = l.left_as_is();
            match l.poll(&self.gpu, true) {
                Ok(true) => {
                    poke.push((*k, l.exe.clone()));
                    changed |= was != l.left_as_is();
                }
                Ok(false) => {}
                Err(e) => eprintln!("frame error: {e}"),
            }
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
        } else if !self.in_time {
            "waiting"
        } else if self.all_rules.get(&key).is_some_and(|x| !x.enabled) {
            "app_off"
        } else if self.everything && self.exclude.contains(&key) {
            "excluded"
        } else if !self.everything && !self.all_rules.contains_key(&key) {
            "not_listed"
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

    fn start_inspect(&mut self) {
        if PICK.with(|p| p.borrow().is_some()) {
            return;
        }
        let hook = unsafe { SetWindowsHookExW(WH_MOUSE_LL, Some(mouse_ll), None, 0) };
        if let Ok(hook) = hook {
            PICK.with(|p| *p.borrow_mut() = Some(Pick { hook, at: None, released: false, cancelled: false, since: Instant::now() }));
        }
        self.publish();
    }

    fn stop_inspect(&mut self) {
        if let Some(p) = PICK.with(|p| p.borrow_mut().take()) {
            unsafe {
                let _ = UnhookWindowsHookEx(p.hook);
            }
        }
        self.publish();
    }

    /// The inspector caught its click (or gave up).
    fn check_inspect(&mut self) {
        let state = PICK.with(|p| p.borrow().as_ref().map(|s| (s.at, s.released, s.cancelled, s.since.elapsed())));
        let Some((at, released, cancelled, age)) = state else { return };
        if cancelled || age > Duration::from_secs(30) {
            self.stop_inspect();
        } else if let (Some(pt), true) = (at, released) {
            self.stop_inspect();
            let h = unsafe { WindowFromPoint(pt) };
            let report = self.diagnose(h);
            (self.on_request)(Request::Inspected(report));
        }
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
            _ => (self.on_request)(Request::Area { exe, area: Err("not_open".into()) }),
        }
    }

    fn check_drawing(&mut self) {
        let Some(outcome) = select::take() else { return };
        let exe = self.drawing.take().unwrap_or_default();
        let area = match outcome {
            select::Outcome::Drawn(a) => Ok(a),
            select::Outcome::Cancelled => Err("cancelled".to_string()),
        };
        (self.on_request)(Request::Area { exe, area });
    }
}

/// The hidden windows' procedure: keeps what the other copy sends.
unsafe extern "system" fn signal_proc(h: HWND, m: u32, w: WPARAM, l: LPARAM) -> LRESULT {
    if m == WM_COPYDATA {
        if let Some(got) = helper::received(w, l) {
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

/// The hidden window a second launch looks for (see admin.rs).
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
        // them: a second launch, and our helper.
        let _ = ChangeWindowMessageFilterEx(h, msg, MSGFLT_ALLOW, None);
        let _ = ChangeWindowMessageFilterEx(h, WM_COPYDATA, MSGFLT_ALLOW, None);
        Some(h)
    }
}

fn run(cfg: Config, role: Role, cmds: Arc<Mutex<Vec<Cmd>>>, on_status: OnStatus, on_request: OnRequest) -> Result<()> {
    crate::layer::register_class()?;
    let open_msg = unsafe { RegisterWindowMessageW(crate::admin::OPEN_MSG) };
    let signal = match role {
        Role::Main => signal_window(open_msg),
        Role::Helper { .. } => helper_window(),
    };
    let frame_event = unsafe { CreateEventW(None, false, false, None)? };
    let mut m = Manager {
        gpu: Gpu::new()?,
        rules: HashMap::new(),
        all_rules: HashMap::new(),
        everything: false,
        exclude: HashSet::new(),
        style: "gray".into(),
        warm: 0.0,
        schedule: ("always".into(), 0, 0),
        enabled: true,
        in_time: true,
        split: std::env::var_os("DEONYX_SPLIT").is_some(),
        layers: HashMap::new(),
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
    // As administrator, normal apps go to a helper without those rights.
    // DEONYX_HELPER_TEST tries the same without them.
    let wants_helper = crate::admin::elevated() || std::env::var_os("DEONYX_HELPER_TEST").is_some();
    if role == Role::Main && wants_helper && std::env::var_os("DEONYX_NO_HELPER").is_none() {
        m.start_helper();
    }
    m.apply(cfg);

    unsafe {
        let hook = |a: u32, b: u32| SetWinEventHook(a, b, None, Some(win_event), 0, 0, WINEVENT_OUTOFCONTEXT);
        let hooks = [
            hook(EVENT_SYSTEM_FOREGROUND, EVENT_SYSTEM_MINIMIZEEND),
            hook(EVENT_OBJECT_DESTROY, EVENT_OBJECT_REORDER),
            hook(EVENT_OBJECT_LOCATIONCHANGE, EVENT_OBJECT_LOCATIONCHANGE),
            hook(EVENT_OBJECT_CLOAKED, EVENT_OBJECT_UNCLOAKED),
        ];
        // Test copies add Shift, so they never fight the real Deonyx for its keys.
        let mut mods = MOD_CONTROL | MOD_ALT | MOD_NOREPEAT;
        if std::env::var_os("DEONYX_CONFIG_DIR").is_some() {
            mods |= MOD_SHIFT;
        }
        if role == Role::Main {
            let all = RegisterHotKey(None, HOTKEY_ALL, mods, b'D' as u32).is_ok();
            let front = RegisterHotKey(None, HOTKEY_FRONT, mods, b'F' as u32).is_ok();
            if !all || !front {
                eprintln!("a hotkey is taken by another program");
            }
        }

        let mut msg = MSG::default();
        let mut last_rescan = Instant::now();
        let mut last_purge = Instant::now();
        let mut last_schedule = Instant::now();
        'outer: loop {
            MsgWaitForMultipleObjects(Some(&[frame_event]), false, 250, QS_ALLINPUT);
            while PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE).as_bool() {
                if msg.message == open_msg {
                    (m.on_request)(Request::OpenSettings);
                    continue;
                }
                match msg.message {
                    WM_HOTKEY if msg.wParam.0 as i32 == HOTKEY_ALL => m.toggle(),
                    WM_HOTKEY if msg.wParam.0 as i32 == HOTKEY_FRONT => m.front_app(),
                    WM_ENGINE => {
                        let list: Vec<Cmd> = std::mem::take(&mut *cmds.lock().unwrap());
                        for c in list {
                            match c {
                                Cmd::Config(cfg) => m.apply(cfg),
                                Cmd::Inspect => m.start_inspect(),
                                Cmd::StopInspect => m.stop_inspect(),
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
            m.frames();
            m.take_protect();
            m.sync_all();
            m.check_inspect();
            if select::active() {
                m.check_drawing();
            }
            if last_rescan.elapsed().as_secs() >= 1 {
                m.rescan();
                m.watch_helper();
                last_rescan = Instant::now();
            }
            if last_schedule.elapsed().as_secs() >= 5 {
                m.check_schedule();
                last_schedule = Instant::now();
            }
            // Process ids get reused; forget old ones now and then.
            if last_purge.elapsed().as_secs() >= 10 {
                m.exe_of.clear();
                m.levels.clear();
                last_purge = Instant::now();
            }
        }
        m.stop_inspect();
        m.layers.clear();
        // Ends the helper too.
        m.helper = None;
        if let Some(h) = signal {
            let _ = DestroyWindow(h);
        }
        for h in hooks {
            let _ = UnhookWinEvent(h);
        }
        let _ = UnregisterHotKey(None, HOTKEY_ALL);
        let _ = UnregisterHotKey(None, HOTKEY_FRONT);
    }
    Ok(())
}
