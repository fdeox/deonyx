// Finds the parts of a window that must keep their real colours (3D
// viewports, canvases, pictures) through UI Automation, the same tree screen
// readers use. Runs on its own thread: UIA calls into other processes can be
// slow and must never stall drawing.

use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use windows::core::Result;
use windows::Win32::Foundation::{HWND, RECT};
use windows::Win32::System::Com::{CoCreateInstance, CoInitializeEx, CLSCTX_INPROC_SERVER, COINIT_MULTITHREADED};
use windows::Win32::UI::Accessibility::*;

use crate::layer::OVERLAY_CLASS;
use crate::win::{frame_rect, height, width};

/// Class names or automation ids of surfaces drawn by the GPU or showing
/// pictures. Matched case-insensitively as substrings.
const VIEW_WORDS: &[&str] = &[
    "viewerwidget", // SketchUp
    "viewport",
    "glwidget",
    "openglwidget",
    "opengl",
    "d3dview",
    "renderview",
    "renderwidget",
    "renderwindow",
    "view3d",
    "3dview",
    "sceneview",
    "canvas",
    "graphicsview",
    "imageview",
    "videowidget",
];

/// A full scan walks the whole UI tree, which costs the app real time (about
/// 100 ms for SketchUp), so it only runs at first, when the window changes
/// size, and now and then. In between we just ask the found elements where
/// they are, one cheap call each.
const FULL_EVERY: Duration = Duration::from_secs(10);

struct Tracked {
    elems: Vec<IUIAutomationElement>,
    size: (i32, i32),
    last_full: Instant,
}

#[derive(Clone, Default)]
struct Shared {
    want: Vec<isize>,
    /// More words, from the app presets (presets.rs).
    words: Vec<String>,
    found: HashMap<isize, Vec<RECT>>,
    /// Windows that just drew something new.
    poked: HashSet<isize>,
}

type Guarded = Arc<(Mutex<Shared>, Condvar)>;

pub struct Finder {
    shared: Guarded,
    dirty: Arc<AtomicBool>,
}

impl Finder {
    pub fn start() -> Finder {
        let shared: Guarded = Arc::new((Mutex::new(Shared::default()), Condvar::new()));
        let dirty = Arc::new(AtomicBool::new(false));
        let (s, d) = (shared.clone(), dirty.clone());
        std::thread::spawn(move || {
            if let Err(e) = run(s, d) {
                crate::note!("3D view finder stopped: {e}");
            }
        });
        Finder { shared, dirty }
    }

    pub fn watch(&self, windows: Vec<isize>) {
        self.shared.0.lock().unwrap().want = windows;
    }

    /// Words of the presets, on top of VIEW_WORDS.
    pub fn set_words(&self, words: Vec<String>) {
        self.shared.0.lock().unwrap().words = words;
    }

    /// The window drew a new frame: check its 3D view soon.
    pub fn poke(&self, h: isize) {
        let (m, cv) = &*self.shared;
        let mut s = m.lock().unwrap();
        if s.want.contains(&h) && s.poked.insert(h) {
            cv.notify_one();
        }
    }

    /// New results since the last call, if any.
    pub fn take(&self) -> Option<HashMap<isize, Vec<RECT>>> {
        if self.dirty.swap(false, Ordering::Relaxed) {
            Some(self.shared.0.lock().unwrap().found.clone())
        } else {
            None
        }
    }
}

fn run(shared: Guarded, dirty: Arc<AtomicBool>) -> Result<()> {
    unsafe {
        CoInitializeEx(None, COINIT_MULTITHREADED).ok()?;
        let uia: IUIAutomation = CoCreateInstance(&CUIAutomation, None, CLSCTX_INPROC_SERVER)?;
        let cache = uia.CreateCacheRequest()?;
        cache.AddProperty(UIA_ClassNamePropertyId)?;
        cache.AddProperty(UIA_AutomationIdPropertyId)?;
        cache.AddProperty(UIA_ControlTypePropertyId)?;
        cache.AddProperty(UIA_BoundingRectanglePropertyId)?;
        let all = uia.CreateTrueCondition()?;
        let mut reported = HashSet::new();
        let mut tracked: HashMap<isize, Tracked> = HashMap::new();
        loop {
            // Wait for a window to draw something new, or look at all of them
            // every 700 ms anyway.
            let (want, poked, words) = {
                let (m, cv) = &*shared;
                let mut s = m.lock().unwrap();
                if s.poked.is_empty() {
                    s = cv.wait_timeout(s, Duration::from_millis(700)).unwrap().0;
                }
                (s.want.clone(), std::mem::take(&mut s.poked), s.words.clone())
            };
            tracked.retain(|h, _| want.contains(h));
            let list: Vec<isize> = if poked.is_empty() { want } else { want.into_iter().filter(|h| poked.contains(h)).collect() };
            for h in list {
                let hwnd = HWND(h as *mut _);
                let frame = frame_rect(hwnd);
                let size = (width(&frame), height(&frame));
                let full = tracked.get(&h).is_none_or(|t| t.size != size || t.last_full.elapsed() >= FULL_EVERY);
                let rects = if full {
                    let t = Instant::now();
                    let found = scan(&uia, &cache, &all, hwnd, frame, &words).unwrap_or_default();
                    let rects: Vec<RECT> = found.iter().map(|(r, _)| *r).collect();
                    if reported.insert(h) {
                        crate::note!(
                            "3D view scan {:.0} ms, protected areas {}: {:?}",
                            t.elapsed().as_secs_f64() * 1000.0,
                            rects.len(),
                            rects.iter().map(|r| (r.left, r.top, width(r), height(r))).collect::<Vec<_>>()
                        );
                    }
                    let elems = found.into_iter().map(|(_, e)| e).collect();
                    tracked.insert(h, Tracked { elems, size, last_full: Instant::now() });
                    rects
                } else {
                    match follow(&tracked[&h].elems, frame) {
                        Some(r) => r,
                        None => {
                            // An element went away: look again next round.
                            tracked.remove(&h);
                            continue;
                        }
                    }
                };
                let mut s = shared.0.lock().unwrap();
                if s.found.get(&h) != Some(&rects) {
                    s.found.insert(h, rects);
                    dirty.store(true, Ordering::Relaxed);
                }
            }
            // At most about 30 looks a second while a window keeps drawing.
            if !poked.is_empty() {
                std::thread::sleep(Duration::from_millis(30));
            }
        }
    }
}

/// Where the elements of the last full scan are now.
fn follow(elems: &[IUIAutomationElement], frame: RECT) -> Option<Vec<RECT>> {
    let mut out = Vec::new();
    for e in elems {
        let r = unsafe { e.CurrentBoundingRectangle() }.ok()?;
        out.extend(relative(r, frame));
    }
    Some(out)
}

/// A screen rectangle clipped to the window and made relative to it.
fn relative(r: RECT, frame: RECT) -> Option<RECT> {
    let r = RECT {
        left: r.left.max(frame.left),
        top: r.top.max(frame.top),
        right: r.right.min(frame.right),
        bottom: r.bottom.min(frame.bottom),
    };
    if width(&r) <= 0 || height(&r) <= 0 {
        return None;
    }
    Some(RECT { left: r.left - frame.left, top: r.top - frame.top, right: r.right - frame.left, bottom: r.bottom - frame.top })
}

fn is_view(class: &str, id: &str, words: &[String]) -> bool {
    let c = class.to_ascii_lowercase();
    let i = id.to_ascii_lowercase();
    let i = i.rsplit('.').next().unwrap_or("");
    // Scroll areas call their content a "viewport" too; that's plain UI.
    if c.contains("scroll") || i.contains("scroll") {
        return false;
    }
    VIEW_WORDS.iter().any(|w| c.contains(w) || i.contains(w)) || words.iter().any(|w| c.contains(w.as_str()) || i.contains(w.as_str()))
}

fn scan(
    uia: &IUIAutomation,
    cache: &IUIAutomationCacheRequest,
    all: &IUIAutomationCondition,
    h: HWND,
    frame: RECT,
    words: &[String],
) -> Result<Vec<(RECT, IUIAutomationElement)>> {
    let frame_area = (width(&frame) as i64 * height(&frame) as i64).max(1);
    let mut out: Vec<(RECT, IUIAutomationElement)> = Vec::new();
    unsafe {
        let root = uia.ElementFromHandle(h)?;
        let list = root.FindAllBuildCache(TreeScope_Descendants, all, cache)?;
        for i in 0..list.Length()? {
            let e = list.GetElement(i)?;
            let class = e.CachedClassName().map(|b| b.to_string()).unwrap_or_default();
            if class == OVERLAY_CLASS {
                continue;
            }
            let id = e.CachedAutomationId().map(|b| b.to_string()).unwrap_or_default();
            let kind = e.CachedControlType().unwrap_or_default();
            let Some(r) = relative(e.CachedBoundingRectangle().unwrap_or_default(), frame) else { continue };
            let share = (width(&r) as i64 * height(&r) as i64) as f64 / frame_area as f64;
            let keep = (is_view(&class, &id, words) && share > 0.08 && share < 0.97)
                || (kind == UIA_ImageControlTypeId && share > 0.01);
            if keep {
                if std::env::var_os("DEONYX_DEBUG").is_some() {
                    println!("  protect: class {class:?} id {id:?} type {} share {share:.2}", kind.0);
                }
                out.push((r, e));
            }
        }
    }
    // Drop areas already inside a bigger one.
    out.sort_by_key(|(r, _)| -(width(r) as i64 * height(r) as i64));
    let mut kept: Vec<(RECT, IUIAutomationElement)> = Vec::new();
    for (r, e) in out {
        let inside = kept
            .iter()
            .any(|(k, _)| r.left >= k.left && r.top >= k.top && r.right <= k.right && r.bottom <= k.bottom);
        if !inside {
            kept.push((r, e));
        }
    }
    Ok(kept)
}
