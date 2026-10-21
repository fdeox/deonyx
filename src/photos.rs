// Photos and videos inside a window, kept in their real colours. The GPU
// rates every 4x4 block of the window (shader.hlsl, photo): photo-like,
// calm but not a flat fill, or flat. Here the photo-like blocks become
// rectangles, because pictures in an interface are rectangles, and each
// rectangle then grows over the calm blocks around it (a smooth sky is part
// of the photo) until it meets flat interface. A lone photo-like block (an
// icon, a coloured letter) is too small to count, and text is never dense
// enough.

use windows::Win32::Foundation::RECT;

/// Pixels per block.
pub const BLOCK: i32 = 4;
/// Smallest picture kept, in blocks (48 pixels).
const MIN_SIDE: usize = 12;
/// At least this share of a picture's blocks must look like a photo: smooth
/// skies and plain backgrounds inside a photo don't, its busy parts do.
const MIN_DENSITY: f32 = 0.3;
/// At most this many pictures per window.
pub const MAX: usize = 16;

/// `cells` holds one byte per block, `pitch` bytes per row: above 191 means
/// photo-like, above 63 calm. Returns the pictures in window pixels.
pub fn find(cells: &[u8], w: usize, h: usize, pitch: usize) -> Vec<RECT> {
    if w == 0 || h == 0 {
        return Vec::new();
    }
    let cell = |i: usize| cells[(i / w) * pitch + i % w];
    let on: Vec<bool> = (0..w * h).map(|i| cell(i) > 191).collect();
    let calm: Vec<bool> = (0..w * h).map(|i| cell(i) > 63).collect();
    // Close small gaps (2 blocks) so one photo is one shape, even across its
    // calm parts.
    let grown = spread(&on, w, h, 2, true);
    let closed = spread(&grown, w, h, 2, false);

    let mut seen = vec![false; w * h];
    let mut found: Vec<(RECT, f32)> = Vec::new();
    let mut stack = Vec::new();
    for start in 0..w * h {
        if !closed[start] || seen[start] {
            continue;
        }
        // One shape: its bounding box.
        let (mut x0, mut y0, mut x1, mut y1) = (w, h, 0, 0);
        seen[start] = true;
        stack.push(start);
        while let Some(i) = stack.pop() {
            let (x, y) = (i % w, i / w);
            x0 = x0.min(x);
            y0 = y0.min(y);
            x1 = x1.max(x);
            y1 = y1.max(y);
            let mut visit = |j: usize| {
                if closed[j] && !seen[j] {
                    seen[j] = true;
                    stack.push(j);
                }
            };
            if x > 0 {
                visit(i - 1);
            }
            if x + 1 < w {
                visit(i + 1);
            }
            if y > 0 {
                visit(i - w);
            }
            if y + 1 < h {
                visit(i + w);
            }
        }
        // Growing made it a little bigger than the picture: trim edges where
        // hardly anything looks like a photo.
        let share = |xa: usize, ya: usize, xb: usize, yb: usize| {
            let mut n = 0;
            for y in ya..=yb {
                for x in xa..=xb {
                    n += on[y * w + x] as usize;
                }
            }
            n as f32 / ((xb - xa + 1) * (yb - ya + 1)) as f32
        };
        while y1 > y0 && share(x0, y0, x1, y0) < 0.25 {
            y0 += 1;
        }
        while y1 > y0 && share(x0, y1, x1, y1) < 0.25 {
            y1 -= 1;
        }
        while x1 > x0 && share(x0, y0, x0, y1) < 0.25 {
            x0 += 1;
        }
        while x1 > x0 && share(x1, y0, x1, y1) < 0.25 {
            x1 -= 1;
        }
        if x1 + 1 - x0 < MIN_SIDE || y1 + 1 - y0 < MIN_SIDE {
            continue;
        }
        let density = share(x0, y0, x1, y1);
        if density < MIN_DENSITY {
            continue;
        }
        // Grow over the calm parts of the picture: far while most of an edge
        // row is calm, then a few blocks more while some of it is, so the
        // picture's own edge is reached but flat interface stops it.
        let calm_share = |xa: usize, ya: usize, xb: usize, yb: usize| {
            let mut n = 0;
            for y in ya..=yb {
                for x in xa..=xb {
                    n += calm[y * w + x] as usize;
                }
            }
            n as f32 / ((xb - xa + 1) * (yb - ya + 1)) as f32
        };
        while y0 > 0 && calm_share(x0, y0 - 1, x1, y0 - 1) >= 0.6 {
            y0 -= 1;
        }
        while y1 + 1 < h && calm_share(x0, y1 + 1, x1, y1 + 1) >= 0.6 {
            y1 += 1;
        }
        while x0 > 0 && calm_share(x0 - 1, y0, x0 - 1, y1) >= 0.6 {
            x0 -= 1;
        }
        while x1 + 1 < w && calm_share(x1 + 1, y0, x1 + 1, y1) >= 0.6 {
            x1 += 1;
        }
        for _ in 0..6 {
            if y0 > 0 && calm_share(x0, y0 - 1, x1, y0 - 1) >= 0.3 {
                y0 -= 1;
            }
            if y1 + 1 < h && calm_share(x0, y1 + 1, x1, y1 + 1) >= 0.3 {
                y1 += 1;
            }
            if x0 > 0 && calm_share(x0 - 1, y0, x0 - 1, y1) >= 0.3 {
                x0 -= 1;
            }
            if x1 + 1 < w && calm_share(x1 + 1, y0, x1 + 1, y1) >= 0.3 {
                x1 += 1;
            }
        }
        let r = RECT {
            left: x0 as i32 * BLOCK,
            top: y0 as i32 * BLOCK,
            right: (x1 as i32 + 1) * BLOCK,
            bottom: (y1 as i32 + 1) * BLOCK,
        };
        found.push((r, density));
    }
    // Biggest first, when there are more than we can keep.
    found.sort_by_key(|(r, _)| -((r.right - r.left) as i64 * (r.bottom - r.top) as i64));
    found.into_iter().take(MAX).map(|(r, _)| r).collect()
}

/// Grows (`grow`) or shrinks the "on" blocks by `n` in every direction.
fn spread(src: &[bool], w: usize, h: usize, n: usize, grow: bool) -> Vec<bool> {
    let pick = |a: bool, b: bool| if grow { a || b } else { a && b };
    let mut rows = vec![false; w * h];
    for y in 0..h {
        for x in 0..w {
            let (a, b) = (x.saturating_sub(n), (x + n).min(w - 1));
            let mut v = !grow;
            for k in a..=b {
                v = pick(v, src[y * w + k]);
            }
            // Shrinking also wears away at the window's edges.
            if !grow && (x < n || x + n >= w) {
                v = false;
            }
            rows[y * w + x] = v;
        }
    }
    let mut out = vec![false; w * h];
    for y in 0..h {
        for x in 0..w {
            let (a, b) = (y.saturating_sub(n), (y + n).min(h - 1));
            let mut v = !grow;
            for k in a..=b {
                v = pick(v, rows[k * w + x]);
            }
            if !grow && (y < n || y + n >= h) {
                v = false;
            }
            out[y * w + x] = v;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn grid(w: usize, h: usize, f: impl Fn(usize, usize) -> bool) -> Vec<u8> {
        (0..w * h).map(|i| if f(i % w, i / w) { 255 } else { 0 }).collect()
    }

    #[test]
    fn a_photo_is_one_rectangle() {
        // A 30x20 block photo with a calm middle (sky) at (10, 5).
        let cells = grid(60, 40, |x, y| (10..40).contains(&x) && (5..25).contains(&y) && !((18..32).contains(&x) && (9..15).contains(&y)));
        let r = find(&cells, 60, 40, 60);
        assert_eq!(r.len(), 1);
        assert_eq!((r[0].left, r[0].top, r[0].right, r[0].bottom), (40, 20, 160, 100));
    }

    #[test]
    fn a_calm_sky_belongs_to_the_photo() {
        // Busy ground (photo-like) under a calm sky (128), on flat paper.
        let cells: Vec<u8> = (0..60 * 40)
            .map(|i| {
                let (x, y) = (i % 60, i / 60);
                if !(10..50).contains(&x) || !(5..35).contains(&y) {
                    0
                } else if y >= 20 {
                    255
                } else {
                    128
                }
            })
            .collect();
        let r = find(&cells, 60, 40, 60);
        assert_eq!(r.len(), 1);
        assert_eq!((r[0].left, r[0].top, r[0].right, r[0].bottom), (40, 20, 200, 140));
    }

    #[test]
    fn icons_and_text_are_not_photos() {
        // Small 6x6 icons in a row, and scattered text-like blocks.
        let cells = grid(80, 40, |x, y| ((x % 10) < 6 && y < 6) || ((x * 7 + y * 13) % 11 == 0 && y > 15));
        assert!(find(&cells, 80, 40, 80).is_empty());
    }
}
