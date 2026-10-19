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
use windows::core::{w, Result};
use windows::Win32::Foundation::*;
use windows::Win32::System::Com::{CoInitializeEx, COINIT_MULTITHREADED};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
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
    /// Lower-case exe name -> main windows left alone because the program
    /// runs as administrator and we don't: Windows won't let a layer sit
    /// on them.
    pub admin: HashMap<String, usize>,
}

pub type OnStatus = Box<dyn Fn(Status) + Send>;
pub type OnOpen = Box<dyn Fn() + Send>;

pub struct Engine {
    tid: u32,
    cmds: Arc<Mutex<Vec<Cmd>>>,
    _thread: JoinHandle<()>,
}

impl Engine {
    pub fn start(cfg: Config, on_status: OnStatus, on_open: OnOpen) -> Engine {
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
                if let Err(e) = run(cfg, queue, on_status, on_open) {
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
    elevated: HashMap<u32, bool>,
    admin_ok: bool,
    admin: HashMap<String, usize>,
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
        // Cheap checks first: most windows belong to programs we don't touch,
        // and those never cost a call to the compositor.
        let pid = window_pid(h);
        let key = self.exe_key(pid)?;
        let rule = self.rules.get(&key)?.clone();
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
        if is_cloaked(h) || class_name(h) == OVERLAY_CLASS {
            return None;
        }
        let r = frame_rect(h);
        if width(&r) < 4 || height(&r) < 4 || self.above_us(pid) {
            return None;
        }
        Some(rule)
    }

    /// A program running as administrator while we don't.
    fn above_us(&mut self, pid: u32) -> bool {
        !self.admin_ok && *self.elevated.entry(pid).or_insert_with(|| is_elevated(pid))
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
                if std::env::var_os("DEONYX_DEBUG").is_some() {
                    let r = frame_rect(h);
                    println!("+ {} {} {}x{} @ {},{} sahip={}", rule.exe, class_name(h), width(&r), height(&r), r.left, r.top, l.owned);
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
        let mut admin: HashMap<String, usize> = HashMap::new();
        if self.enabled {
            for h in top_level_windows() {
                self.add(h);
                if self.admin_ok {
                    continue;
                }
                let pid = window_pid(h);
                let Some(key) = self.exe_key(pid) else { continue };
                if !self.rules.contains_key(&key) {
                    continue;
                }
                let main = unsafe { IsWindowVisible(h).as_bool() && GetWindow(h, GW_OWNER).is_err() } && is_top_level(h);
                if main && self.above_us(pid) {
                    *admin.entry(key).or_default() += 1;
                }
            }
        }
        if admin != self.admin {
            self.admin = admin;
            self.publish();
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
        let s = Status { enabled: self.enabled, running, admin: self.admin.clone() };
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

unsafe extern "system" fn signal_proc(h: HWND, m: u32, w: WPARAM, l: LPARAM) -> LRESULT {
    DefWindowProcW(h, m, w, l)
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
        // Lets a Deonyx without administrator rights reach us when we have them.
        let _ = ChangeWindowMessageFilterEx(h, msg, MSGFLT_ALLOW, None);
        Some(h)
    }
}

fn run(cfg: Config, cmds: Arc<Mutex<Vec<Cmd>>>, on_status: OnStatus, on_open: OnOpen) -> Result<()> {
    crate::layer::register_class()?;
    let open_msg = unsafe { RegisterWindowMessageW(crate::admin::OPEN_MSG) };
    let signal = signal_window(open_msg);
    let frame_event = unsafe { CreateEventW(None, false, false, None)? };
    let mut m = Manager {
        gpu: Gpu::new()?,
        rules: HashMap::new(),
        enabled: true,
        split: std::env::var_os("DEONYX_SPLIT").is_some(),
        layers: HashMap::new(),
        exe_of: HashMap::new(),
        elevated: HashMap::new(),
        admin_ok: is_elevated(std::process::id()),
        admin: HashMap::new(),
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
                if msg.message == open_msg {
                    on_open();
                    continue;
                }
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
                m.elevated.clear();
                last_purge = Instant::now();
            }
        }
        m.layers.clear();
        if let Some(h) = signal {
            let _ = DestroyWindow(h);
        }
        for h in hooks {
            let _ = UnhookWinEvent(h);
        }
        if hotkey {
            let _ = UnregisterHotKey(None, HOTKEY_ID);
        }
    }
    Ok(())
}
