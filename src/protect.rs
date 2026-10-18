// Finds the parts of a window that must keep their real colours (3D
// viewports, canvases, pictures) through UI Automation, the same tree screen
// readers use. Runs on its own thread: UIA calls into other processes can be
// slow and must never stall drawing.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
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

#[derive(Clone, Default)]
struct Shared {
    want: Vec<isize>,
    found: HashMap<isize, Vec<RECT>>,
}

pub struct Finder {
    shared: Arc<Mutex<Shared>>,
    dirty: Arc<AtomicBool>,
}

impl Finder {
    pub fn start() -> Finder {
        let shared = Arc::new(Mutex::new(Shared::default()));
        let dirty = Arc::new(AtomicBool::new(false));
        let (s, d) = (shared.clone(), dirty.clone());
        std::thread::spawn(move || {
            if let Err(e) = run(s, d) {
                eprintln!("3D alan arayici durdu: {e}");
            }
        });
        Finder { shared, dirty }
    }

    pub fn watch(&self, windows: Vec<isize>) {
        self.shared.lock().unwrap().want = windows;
    }

    /// New results since the last call, if any.
    pub fn take(&self) -> Option<HashMap<isize, Vec<RECT>>> {
        if self.dirty.swap(false, Ordering::Relaxed) {
            Some(self.shared.lock().unwrap().found.clone())
        } else {
            None
        }
    }
}

fn run(shared: Arc<Mutex<Shared>>, dirty: Arc<AtomicBool>) -> Result<()> {
    unsafe {
        CoInitializeEx(None, COINIT_MULTITHREADED).ok()?;
        let uia: IUIAutomation = CoCreateInstance(&CUIAutomation, None, CLSCTX_INPROC_SERVER)?;
        let cache = uia.CreateCacheRequest()?;
        cache.AddProperty(UIA_ClassNamePropertyId)?;
        cache.AddProperty(UIA_AutomationIdPropertyId)?;
        cache.AddProperty(UIA_ControlTypePropertyId)?;
        cache.AddProperty(UIA_BoundingRectanglePropertyId)?;
        let all = uia.CreateTrueCondition()?;
        let mut reported = std::collections::HashSet::new();
        loop {
            let want = shared.lock().unwrap().want.clone();
            for h in want {
                let t = Instant::now();
                let rects = scan(&uia, &cache, &all, HWND(h as *mut _)).unwrap_or_default();
                if reported.insert(h) {
                    println!(
                        "3D alan taramasi {:.0} ms, korunacak alan {}: {:?}",
                        t.elapsed().as_secs_f64() * 1000.0,
                        rects.len(),
                        rects.iter().map(|r| (r.left, r.top, width(r), height(r))).collect::<Vec<_>>()
                    );
                }
                let mut s = shared.lock().unwrap();
                if s.found.get(&h) != Some(&rects) {
                    s.found.insert(h, rects);
                    dirty.store(true, Ordering::Relaxed);
                }
            }
            std::thread::sleep(Duration::from_millis(700));
        }
    }
}

fn is_view(class: &str, id: &str) -> bool {
    let c = class.to_ascii_lowercase();
    let i = id.to_ascii_lowercase();
    let i = i.rsplit('.').next().unwrap_or("");
    // Scroll areas call their content a "viewport" too; that's plain UI.
    if c.contains("scroll") || i.contains("scroll") {
        return false;
    }
    VIEW_WORDS.iter().any(|w| c.contains(w) || i.contains(w))
}

fn scan(uia: &IUIAutomation, cache: &IUIAutomationCacheRequest, all: &IUIAutomationCondition, h: HWND) -> Result<Vec<RECT>> {
    let frame = frame_rect(h);
    let frame_area = (width(&frame) as i64 * height(&frame) as i64).max(1);
    let mut out: Vec<RECT> = Vec::new();
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
            let r = e.CachedBoundingRectangle().unwrap_or_default();
            let r = RECT {
                left: r.left.max(frame.left),
                top: r.top.max(frame.top),
                right: r.right.min(frame.right),
                bottom: r.bottom.min(frame.bottom),
            };
            if width(&r) <= 0 || height(&r) <= 0 {
                continue;
            }
            let share = (width(&r) as i64 * height(&r) as i64) as f64 / frame_area as f64;
            let keep = (is_view(&class, &id) && share > 0.08 && share < 0.97)
                || (kind == UIA_ImageControlTypeId && share > 0.01);
            if keep {
                if std::env::var_os("DEONYX_DEBUG").is_some() {
                    println!("  koru: sinif {class:?} kimlik {id:?} tur {} pay {share:.2}", kind.0);
                }
                out.push(RECT {
                    left: r.left - frame.left,
                    top: r.top - frame.top,
                    right: r.right - frame.left,
                    bottom: r.bottom - frame.top,
                });
            }
        }
    }
    // Drop areas already inside a bigger one.
    out.sort_by_key(|r| -(width(r) as i64 * height(r) as i64));
    let mut kept: Vec<RECT> = Vec::new();
    for r in out {
        let inside = kept
            .iter()
            .any(|k| r.left >= k.left && r.top >= k.top && r.right <= k.right && r.bottom <= k.bottom);
        if !inside {
            kept.push(r);
        }
    }
    Ok(kept)
}
