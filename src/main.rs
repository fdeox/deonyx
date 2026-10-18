// Deonyx: dark mode for any app.
//
// Every visible window of the chosen programs (main windows, menus, lists,
// dialogs, tooltips) is captured with Windows.Graphics.Capture, its lightness
// is flipped on the GPU, and the result is shown in a click-through window
// right above it. The program itself is never touched.

mod gpu;
mod layer;
mod protect;
mod win;

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::ffi::c_void;
use std::time::Instant;

use windows::core::Result;
use windows::Win32::Foundation::*;
use windows::Win32::System::Threading::CreateEventW;
use windows::Win32::UI::Accessibility::{SetWinEventHook, HWINEVENTHOOK};
use windows::Win32::UI::HiDpi::{SetProcessDpiAwarenessContext, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2};
use windows::Win32::UI::Input::KeyboardAndMouse::{RegisterHotKey, MOD_ALT, MOD_CONTROL, MOD_NOREPEAT};
use windows::Win32::UI::WindowsAndMessaging::*;

use gpu::Gpu;
use layer::{Layer, Look, OVERLAY_CLASS};
use protect::Finder;
use win::*;

const OBJID_WINDOW: i32 = 0;

thread_local! {
    static EVENTS: RefCell<Vec<(u32, isize)>> = const { RefCell::new(Vec::new()) };
}

unsafe extern "system" fn win_event(_: HWINEVENTHOOK, event: u32, hwnd: HWND, obj: i32, child: i32, _: u32, _: u32) {
    if obj != OBJID_WINDOW || child != 0 {
        return;
    }
    EVENTS.with(|e| e.borrow_mut().push((event, hwnd.0 as isize)));
}

struct Opts {
    processes: Vec<String>,
    split: bool,
    owner: bool,
    lmin: f32,
    lmax: f32,
    auto_protect: bool,
}

fn parse_opts() -> Opts {
    let mut o = Opts { processes: Vec::new(), split: false, owner: true, lmin: 0.24, lmax: 0.90, auto_protect: true };
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--process" => o.processes.extend(args.next().map(|s| s.to_ascii_lowercase())),
            "--split" => o.split = true,
            "--no-owner" => o.owner = false,
            "--no-protect" => o.auto_protect = false,
            "--lmin" => o.lmin = args.next().and_then(|v| v.parse().ok()).unwrap_or(o.lmin),
            "--lmax" => o.lmax = args.next().and_then(|v| v.parse().ok()).unwrap_or(o.lmax),
            _ => eprintln!("bilinmeyen secenek {a}"),
        }
    }
    o
}

struct Manager {
    o: Opts,
    gpu: Gpu,
    look: Look,
    layers: HashMap<isize, Layer>,
    exe_of: HashMap<u32, bool>,
    ignored: HashSet<isize>,
    finder: Option<Finder>,
    protect: HashMap<isize, Vec<RECT>>,
    enabled: bool,
    frame_event: HANDLE,
    since: Instant,
}

impl Manager {
    fn is_target_process(&mut self, pid: u32) -> bool {
        if pid == std::process::id() {
            return false;
        }
        let names = &self.o.processes;
        *self.exe_of.entry(pid).or_insert_with(|| {
            exe_name(pid).map(|n| names.contains(&n.to_ascii_lowercase())).unwrap_or(false)
        })
    }

    fn wants(&mut self, h: HWND) -> bool {
        unsafe {
            if !IsWindowVisible(h).as_bool() || !is_top_level(h) || is_cloaked(h) {
                return false;
            }
            let ex = GetWindowLongW(h, GWL_EXSTYLE) as u32;
            // Click-through helper windows (drag images, effects) are left alone.
            if ex & WS_EX_TRANSPARENT.0 != 0 && ex & WS_EX_LAYERED.0 != 0 {
                return false;
            }
        }
        let r = frame_rect(h);
        if width(&r) < 4 || height(&r) < 4 {
            return false;
        }
        self.is_target_process(window_pid(h)) && class_name(h) != OVERLAY_CLASS
    }

    fn add(&mut self, h: HWND) {
        let key = h.0 as isize;
        if self.layers.contains_key(&key) || self.ignored.contains(&key) || !self.wants(h) {
            return;
        }
        match Layer::new(&self.gpu, h, &self.look) {
            Ok(mut l) => {
                if let Err(e) = l.on_frame_arrived(self.frame_event) {
                    eprintln!("kare bildirimi kurulamadi: {e}");
                }
                if let Some(p) = self.protect.get(&key) {
                    l.protect = p.clone();
                }
                let r = frame_rect(h);
                println!("+ {} {}x{}", class_name(h), width(&r), height(&r));
                self.layers.insert(key, l);
                self.watch_main_windows();
            }
            Err(e) => {
                println!("yakalanamadi ({}): {e}", class_name(h));
                self.ignored.insert(key);
            }
        }
    }

    fn remove(&mut self, key: isize) {
        if let Some(l) = self.layers.remove(&key) {
            println!(
                "- {} (ilk kare {:.0} ms)",
                class_name(l.target),
                l.first_frame_ms.unwrap_or(-1.0)
            );
            self.watch_main_windows();
        }
        self.ignored.remove(&key);
    }

    /// Only big windows can hold a viewport worth protecting.
    fn watch_main_windows(&self) {
        if let Some(f) = &self.finder {
            let big = self
                .layers
                .iter()
                .filter(|(_, l)| {
                    let r = frame_rect(l.target);
                    width(&r) >= 400 && height(&r) >= 300
                })
                .map(|(k, _)| *k)
                .collect();
            f.watch(big);
        }
    }

    fn rescan(&mut self) {
        for h in top_level_windows() {
            self.add(h);
        }
    }

    fn handle(&mut self, event: u32, key: isize) {
        let h = HWND(key as *mut c_void);
        match event {
            EVENT_OBJECT_SHOW | EVENT_OBJECT_UNCLOAKED => self.add(h),
            EVENT_OBJECT_HIDE | EVENT_OBJECT_DESTROY => self.remove(key),
            EVENT_OBJECT_CLOAKED => {
                if let Some(l) = self.layers.get_mut(&key) {
                    l.sync(self.enabled);
                }
            }
            _ => {}
        }
    }

    fn sync_all(&mut self) {
        let enabled = self.enabled;
        let mut gone = Vec::new();
        for (k, l) in self.layers.iter_mut() {
            if !l.sync(enabled) {
                gone.push(*k);
            }
        }
        for k in gone {
            self.remove(k);
        }
    }

    fn frames(&mut self) {
        let enabled = self.enabled;
        for l in self.layers.values_mut() {
            if let Err(e) = l.poll(&self.gpu, enabled) {
                eprintln!("kare hatasi: {e}");
            }
        }
        if self.since.elapsed().as_secs_f64() >= 5.0 {
            let (f, w): (u32, f64) = self.layers.values().map(|l| (l.frames, l.work_ms)).fold((0, 0.0), |a, b| (a.0 + b.0, a.1 + b.1));
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
        let Some(found) = self.finder.as_ref().and_then(|f| f.take()) else { return };
        self.protect = found;
        for (k, l) in self.layers.iter_mut() {
            let p = self.protect.get(k).cloned().unwrap_or_default();
            if p != l.protect {
                l.protect = p;
                l.redraw(&self.gpu);
            }
        }
    }
}

fn main() -> Result<()> {
    unsafe {
        let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
    }
    let o = parse_opts();
    if o.processes.is_empty() {
        eprintln!("kullanim: deonyx --process SketchUp.exe [--process baska.exe] [--split] [--no-protect]");
        std::process::exit(2);
    }
    layer::register_class()?;
    let look = Look { lmin: o.lmin, lmax: o.lmax, split: o.split, owner: o.owner };
    let finder = o.auto_protect.then(Finder::start);
    let frame_event = unsafe { CreateEventW(None, false, false, None)? };
    let mut m = Manager {
        o,
        gpu: Gpu::new()?,
        look,
        layers: HashMap::new(),
        exe_of: HashMap::new(),
        ignored: HashSet::new(),
        finder,
        protect: HashMap::new(),
        enabled: true,
        frame_event,
        since: Instant::now(),
    };
    m.rescan();

    unsafe {
        let hook = |a: u32, b: u32| SetWinEventHook(a, b, None, Some(win_event), 0, 0, WINEVENT_OUTOFCONTEXT);
        let _ = hook(EVENT_SYSTEM_FOREGROUND, EVENT_SYSTEM_MINIMIZEEND);
        let _ = hook(EVENT_OBJECT_DESTROY, EVENT_OBJECT_REORDER);
        let _ = hook(EVENT_OBJECT_LOCATIONCHANGE, EVENT_OBJECT_LOCATIONCHANGE);
        let _ = hook(EVENT_OBJECT_CLOAKED, EVENT_OBJECT_UNCLOAKED);
        if RegisterHotKey(None, 1, MOD_CONTROL | MOD_ALT | MOD_NOREPEAT, b'D' as u32).is_ok() {
            println!("Ctrl+Alt+D: karartmayi ac/kapat");
        }

        let mut msg = MSG::default();
        let mut last_rescan = Instant::now();
        loop {
            MsgWaitForMultipleObjects(Some(&[frame_event]), false, 250, QS_ALLINPUT);
            while PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE).as_bool() {
                if msg.message == WM_QUIT {
                    return Ok(());
                }
                if msg.message == WM_HOTKEY {
                    m.enabled = !m.enabled;
                    println!("{}", if m.enabled { "karartma acik" } else { "karartma kapali" });
                }
                let _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
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
        }
    }
}
