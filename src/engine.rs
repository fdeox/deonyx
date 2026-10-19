// The darkening engine. Runs on its own thread with its own message loop:
// WinEvent hooks, the hotkey and capture frames all arrive there, so the
// settings window can never slow it down.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::ffi::c_void;
use std::sync::{mpsc, Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Instant;

use serde::Serialize;
use windows::core::Result;
use windows::Win32::Foundation::*;
use windows::Win32::System::Com::{CoInitializeEx, COINIT_MULTITHREADED};
use windows::Win32::System::Threading::{CreateEventW, GetCurrentThreadId};
use windows::Win32::UI::Accessibility::{SetWinEventHook, UnhookWinEvent, HWINEVENTHOOK};
use windows::Win32::UI::Input::KeyboardAndMouse::{RegisterHotKey, UnregisterHotKey, MOD_ALT, MOD_CONTROL, MOD_NOREPEAT};
use windows::Win32::UI::WindowsAndMessaging::*;

use crate::config::{AppRule, Config};
use crate::gpu::Gpu;
use crate::layer::{Layer, Look, OVERLAY_CLASS};
use crate::protect::Finder;
use crate::win::*;

const OBJID_WINDOW: i32 = 0;
const WM_ENGINE: u32 = WM_APP + 1;
const HOTKEY_ID: i32 = 1;

pub enum Cmd {
    Config(Config),
    Quit,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct Status {
    pub enabled: bool,
    /// Lower-case exe name -> number of its main windows being darkened.
    pub running: HashMap<String, usize>,
}

pub type OnStatus = Box<dyn Fn(Status) + Send>;

pub struct Engine {
    tid: u32,
    cmds: Arc<Mutex<Vec<Cmd>>>,
    _thread: JoinHandle<()>,
}

impl Engine {
    pub fn start(cfg: Config, on_status: OnStatus) -> Engine {
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
                if let Err(e) = run(cfg, queue, on_status) {
                    eprintln!("motor durdu: {e}");
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
}

unsafe extern "system" fn win_event(_: HWINEVENTHOOK, event: u32, hwnd: HWND, obj: i32, child: i32, _: u32, _: u32) {
    if obj != OBJID_WINDOW || child != 0 {
        return;
    }
    EVENTS.with(|e| e.borrow_mut().push((event, hwnd.0 as isize)));
}

struct Manager {
    gpu: Gpu,
    rules: HashMap<String, AppRule>,
    enabled: bool,
    split: bool,
    layers: HashMap<isize, Layer>,
    exe_of: HashMap<u32, Option<String>>,
    ignored: HashSet<isize>,
    finder: Finder,
    protect: HashMap<isize, Vec<RECT>>,
    frame_event: HANDLE,
    since: Instant,
    on_status: OnStatus,
    last_status: Option<Status>,
}

impl Manager {
    fn exe_key(&mut self, pid: u32) -> Option<String> {
        if pid == std::process::id() {
            return None;
        }
        self.exe_of
            .entry(pid)
            .or_insert_with(|| exe_name(pid).map(|n| n.to_ascii_lowercase()))
            .clone()
    }

    /// The rule for a window, if it's one we should darken.
    fn rule_for(&mut self, h: HWND) -> Option<AppRule> {
        if !self.enabled {
            return None;
        }
        unsafe {
            if !IsWindowVisible(h).as_bool() || !is_top_level(h) || is_cloaked(h) {
                return None;
            }
            let ex = GetWindowLongW(h, GWL_EXSTYLE) as u32;
            // Click-through helper windows (drag images, effects) are left alone.
            if ex & WS_EX_TRANSPARENT.0 != 0 && ex & WS_EX_LAYERED.0 != 0 {
                return None;
            }
        }
        let r = frame_rect(h);
        if width(&r) < 4 || height(&r) < 4 {
            return None;
        }
        let key = self.exe_key(window_pid(h))?;
        let rule = self.rules.get(&key)?.clone();
        (class_name(h) != OVERLAY_CLASS).then_some(rule)
    }

    fn add(&mut self, h: HWND) {
        let key = h.0 as isize;
        if self.layers.contains_key(&key) || self.ignored.contains(&key) {
            return;
        }
        let Some(rule) = self.rule_for(h) else { return };
        let look = Look { lmin: rule.lmin(), lmax: rule.lmax(), split: self.split, owner: true };
        match Layer::new(&self.gpu, h, &look) {
            Ok(mut l) => {
                if let Err(e) = l.on_frame_arrived(self.frame_event) {
                    eprintln!("kare bildirimi kurulamadi: {e}");
                }
                l.exe = rule.key();
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
                eprintln!("yakalanamadi ({}): {e}", class_name(h));
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

    /// Only big windows of apps that want it can hold a viewport worth protecting.
    fn watch_main_windows(&self) {
        let big = self
            .layers
            .iter()
            .filter(|(_, l)| self.rules.get(&l.exe).map(|r| r.protect).unwrap_or(false))
            .filter(|(_, l)| {
                let r = frame_rect(l.target);
                width(&r) >= 400 && height(&r) >= 300
            })
            .map(|(k, _)| *k)
            .collect();
        self.finder.watch(big);
    }

    fn rescan(&mut self) {
        if !self.enabled {
            return;
        }
        for h in top_level_windows() {
            self.add(h);
        }
    }

    fn apply(&mut self, cfg: Config) {
        self.enabled = cfg.enabled;
        self.rules = cfg.apps.into_iter().filter(|r| r.enabled && !r.exe.is_empty()).map(|r| (r.key(), r)).collect();
        if !self.enabled {
            // Nothing to capture while switched off.
            self.layers.clear();
        }
        let rules = &self.rules;
        self.layers.retain(|_, l| rules.contains_key(&l.exe));
        for (k, l) in self.layers.iter_mut() {
            let rule = &self.rules[&l.exe];
            l.params.lmin = rule.lmin();
            l.params.lmax = rule.lmax();
            l.protect = if rule.protect { self.protect.get(k).cloned().unwrap_or_default() } else { Vec::new() };
            l.redraw(&self.gpu);
        }
        self.ignored.clear();
        self.watch_main_windows();
        self.rescan();
        self.publish();
    }

    fn toggle(&mut self) {
        self.enabled = !self.enabled;
        if self.enabled {
            self.rescan();
        } else {
            self.layers.clear();
        }
        self.publish();
    }

    fn publish(&mut self) {
        let mut running: HashMap<String, usize> = HashMap::new();
        for l in self.layers.values() {
            // Count main windows only, not menus and tooltips.
            if unsafe { GetWindow(l.target, GW_OWNER) }.is_err() {
                *running.entry(l.exe.clone()).or_default() += 1;
            }
        }
        let s = Status { enabled: self.enabled, running };
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
        for l in self.layers.values_mut() {
            if let Err(e) = l.poll(&self.gpu, true) {
                eprintln!("kare hatasi: {e}");
            }
        }
        if self.since.elapsed().as_secs_f64() >= 5.0 {
            let (f, w): (u32, f64) =
                self.layers.values().map(|l| (l.frames, l.work_ms)).fold((0, 0.0), |a, b| (a.0 + b.0, a.1 + b.1));
            if f > 0 {
                println!("{} pencere, {} kare / 5 sn, kare basina {:.2} ms", self.layers.len(), f, w / f as f64);
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
        for (k, l) in self.layers.iter_mut() {
            if !self.rules.get(&l.exe).map(|r| r.protect).unwrap_or(false) {
                continue;
            }
            let p = self.protect.get(k).cloned().unwrap_or_default();
            if p != l.protect {
                l.protect = p;
                l.redraw(&self.gpu);
            }
        }
    }
}

fn run(cfg: Config, cmds: Arc<Mutex<Vec<Cmd>>>, on_status: OnStatus) -> Result<()> {
    crate::layer::register_class()?;
    let frame_event = unsafe { CreateEventW(None, false, false, None)? };
    let mut m = Manager {
        gpu: Gpu::new()?,
        rules: HashMap::new(),
        enabled: true,
        split: std::env::var_os("DEONYX_SPLIT").is_some(),
        layers: HashMap::new(),
        exe_of: HashMap::new(),
        ignored: HashSet::new(),
        finder: Finder::start(),
        protect: HashMap::new(),
        frame_event,
        since: Instant::now(),
        on_status,
        last_status: None,
    };
    m.apply(cfg);

    unsafe {
        let hook = |a: u32, b: u32| SetWinEventHook(a, b, None, Some(win_event), 0, 0, WINEVENT_OUTOFCONTEXT);
        let hooks = [
            hook(EVENT_SYSTEM_FOREGROUND, EVENT_SYSTEM_MINIMIZEEND),
            hook(EVENT_OBJECT_DESTROY, EVENT_OBJECT_REORDER),
            hook(EVENT_OBJECT_LOCATIONCHANGE, EVENT_OBJECT_LOCATIONCHANGE),
            hook(EVENT_OBJECT_CLOAKED, EVENT_OBJECT_UNCLOAKED),
        ];
        let hotkey = RegisterHotKey(None, HOTKEY_ID, MOD_CONTROL | MOD_ALT | MOD_NOREPEAT, b'D' as u32).is_ok();
        if !hotkey {
            eprintln!("Ctrl+Alt+D baska bir program tarafindan kullaniliyor");
        }

        let mut msg = MSG::default();
        let mut last_rescan = Instant::now();
        let mut last_purge = Instant::now();
        'outer: loop {
            MsgWaitForMultipleObjects(Some(&[frame_event]), false, 250, QS_ALLINPUT);
            while PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE).as_bool() {
                match msg.message {
                    WM_HOTKEY => m.toggle(),
                    WM_ENGINE => {
                        let list: Vec<Cmd> = std::mem::take(&mut *cmds.lock().unwrap());
                        for c in list {
                            match c {
                                Cmd::Config(cfg) => m.apply(cfg),
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
            let events: Vec<(u32, isize)> = EVENTS.with(|e| std::mem::take(&mut *e.borrow_mut()));
            for (ev, key) in &events {
                m.handle(*ev, *key);
            }
            m.frames();
            m.take_protect();
            m.sync_all();
            if last_rescan.elapsed().as_secs() >= 1 {
                m.rescan();
                last_rescan = Instant::now();
            }
            // Process ids get reused; forget old ones now and then.
            if last_purge.elapsed().as_secs() >= 10 {
                m.exe_of.clear();
                last_purge = Instant::now();
            }
        }
        m.layers.clear();
        for h in hooks {
            let _ = UnhookWinEvent(h);
        }
        if hotkey {
            let _ = UnregisterHotKey(None, HOTKEY_ID);
        }
    }
    Ok(())
}
