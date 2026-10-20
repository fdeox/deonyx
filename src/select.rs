// The "draw an area" sheet: a dimmed cover over a window, where the user
// drags out the part that should keep its real colours. Esc or a right
// click cancels.

use std::cell::RefCell;

use windows::core::{w, Result, HSTRING};
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Input::KeyboardAndMouse::{ReleaseCapture, SetCapture, SetFocus, VK_ESCAPE};
use windows::Win32::UI::WindowsAndMessaging::*;

use crate::config::Area;
use crate::win::{frame_rect, height, width};

pub enum Outcome {
    Drawn(Area),
    Cancelled,
}

struct Sheet {
    hwnd: HWND,
    size: (i32, i32),
    start: Option<POINT>,
    cur: POINT,
    hint: Vec<u16>,
    done: Option<Outcome>,
}

thread_local! {
    static SHEET: RefCell<Option<Sheet>> = const { RefCell::new(None) };
}

fn point(l: LPARAM) -> POINT {
    POINT { x: (l.0 & 0xffff) as i16 as i32, y: ((l.0 >> 16) & 0xffff) as i16 as i32 }
}

fn with_sheet(f: impl FnOnce(&mut Sheet)) {
    SHEET.with(|s| {
        if let Some(x) = s.borrow_mut().as_mut() {
            f(x);
        }
    });
}

fn finish(o: Outcome) {
    with_sheet(|x| {
        if x.done.is_none() {
            x.done = Some(o);
        }
    });
}

unsafe extern "system" fn sheet_proc(h: HWND, m: u32, w: WPARAM, l: LPARAM) -> LRESULT {
    match m {
        WM_LBUTTONDOWN => {
            SetCapture(h);
            let p = point(l);
            with_sheet(|x| {
                x.start = Some(p);
                x.cur = p;
            });
            let _ = InvalidateRect(Some(h), None, false);
            LRESULT(0)
        }
        WM_MOUSEMOVE => {
            let p = point(l);
            with_sheet(|x| {
                if x.start.is_some() {
                    x.cur = p;
                }
            });
            let _ = InvalidateRect(Some(h), None, false);
            LRESULT(0)
        }
        WM_LBUTTONUP => {
            let _ = ReleaseCapture();
            let mut area = None;
            with_sheet(|x| {
                if let Some(s) = x.start.take() {
                    let (l, r) = (s.x.min(x.cur.x).max(0), s.x.max(x.cur.x).min(x.size.0));
                    let (t, b) = (s.y.min(x.cur.y).max(0), s.y.max(x.cur.y).min(x.size.1));
                    if r - l >= 8 && b - t >= 8 {
                        area = Some(Area { left: l, top: t, right: x.size.0 - r, bottom: x.size.1 - b });
                    }
                }
            });
            match area {
                Some(a) => finish(Outcome::Drawn(a)),
                None => {
                    let _ = InvalidateRect(Some(h), None, false);
                }
            }
            LRESULT(0)
        }
        WM_RBUTTONDOWN => {
            finish(Outcome::Cancelled);
            LRESULT(0)
        }
        WM_KEYDOWN if w.0 as u16 == VK_ESCAPE.0 => {
            finish(Outcome::Cancelled);
            LRESULT(0)
        }
        WM_SETCURSOR => {
            if let Ok(c) = LoadCursorW(None, IDC_CROSS) {
                SetCursor(Some(c));
            }
            LRESULT(1)
        }
        WM_PAINT => {
            let mut ps = PAINTSTRUCT::default();
            let dc = BeginPaint(h, &mut ps);
            let mut client = RECT::default();
            let _ = GetClientRect(h, &mut client);
            let dim = CreateSolidBrush(COLORREF(0x0e0a0a));
            FillRect(dc, &client, dim);
            let _ = DeleteObject(dim.into());
            SHEET.with(|s| {
                let s = s.borrow();
                let Some(x) = s.as_ref() else { return };
                if let Some(st) = x.start {
                    let r = RECT {
                        left: st.x.min(x.cur.x),
                        top: st.y.min(x.cur.y),
                        right: st.x.max(x.cur.x),
                        bottom: st.y.max(x.cur.y),
                    };
                    // Accent violet (#8b7cf6), stored as 0x00BBGGRR.
                    let fill = CreateSolidBrush(COLORREF(0x00f67c8b));
                    FillRect(dc, &r, fill);
                    let _ = DeleteObject(fill.into());
                }
                let font = CreateFontW(22, 0, 0, 0, 600, 0, 0, 0, DEFAULT_CHARSET, OUT_DEFAULT_PRECIS, CLIP_DEFAULT_PRECIS, CLEARTYPE_QUALITY, 0, w!("Segoe UI"));
                let old = SelectObject(dc, font.into());
                SetBkMode(dc, TRANSPARENT);
                SetTextColor(dc, COLORREF(0x00f3eeee));
                let mut text = x.hint.clone();
                let mut box_ = RECT { left: 0, top: 24, right: client.right, bottom: 70 };
                DrawTextW(dc, &mut text, &mut box_, DT_CENTER | DT_SINGLELINE | DT_VCENTER);
                SelectObject(dc, old);
                let _ = DeleteObject(font.into());
            });
            let _ = EndPaint(h, &ps);
            LRESULT(0)
        }
        _ => DefWindowProcW(h, m, w, l),
    }
}

/// Covers `target` with the sheet. The result comes through take().
pub fn begin(target: HWND, hint: &str) -> Result<()> {
    unsafe {
        let hinst = GetModuleHandleW(None)?;
        let wc = WNDCLASSW {
            lpfnWndProc: Some(sheet_proc),
            hInstance: hinst.into(),
            lpszClassName: w!("DeonyxSheet"),
            ..Default::default()
        };
        RegisterClassW(&wc);
        let _ = SetForegroundWindow(target);
        let r = frame_rect(target);
        let h = CreateWindowExW(
            WS_EX_TOPMOST | WS_EX_LAYERED | WS_EX_TOOLWINDOW,
            w!("DeonyxSheet"),
            &HSTRING::from("Deonyx"),
            WS_POPUP,
            r.left,
            r.top,
            width(&r),
            height(&r),
            None,
            None,
            Some(hinst.into()),
            None,
        )?;
        // The window stays faintly visible underneath.
        SetLayeredWindowAttributes(h, COLORREF(0), 175, LWA_ALPHA)?;
        SHEET.with(|s| {
            *s.borrow_mut() = Some(Sheet {
                hwnd: h,
                size: (width(&r), height(&r)),
                start: None,
                cur: POINT::default(),
                hint: hint.encode_utf16().collect(),
                done: None,
            })
        });
        let _ = ShowWindow(h, SW_SHOW);
        let _ = SetForegroundWindow(h);
        let _ = SetFocus(Some(h));
    }
    Ok(())
}

pub fn active() -> bool {
    SHEET.with(|s| s.borrow().is_some())
}

/// The outcome once the user is done; closes the sheet.
pub fn take() -> Option<Outcome> {
    SHEET.with(|s| {
        let mut b = s.borrow_mut();
        if b.as_ref().is_some_and(|x| x.done.is_some()) {
            let sheet = b.take()?;
            unsafe {
                let _ = DestroyWindow(sheet.hwnd);
            }
            sheet.done
        } else {
            None
        }
    })
}
