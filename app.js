// The before/after slider, the newest version number, and "Try it": a
// screenshot darkened in the browser with the same math as Deonyx,
// photos in it kept in their real colors the way the app finds them.

// ---------- before / after
const compare = document.getElementById("compare");
const clip = compare.querySelector(".before-clip");
const handle = compare.querySelector(".handle");
const beforeImg = clip.querySelector("img");

function place(frac) {
  const f = Math.min(1, Math.max(0, frac));
  clip.style.width = `${f * 100}%`;
  handle.style.left = `${f * 100}%`;
}
function fit() {
  beforeImg.style.width = `${compare.clientWidth}px`;
}
let dragging = false;
const at = (e) => (e.clientX - compare.getBoundingClientRect().left) / compare.clientWidth;
compare.addEventListener("pointerdown", (e) => { dragging = true; compare.setPointerCapture(e.pointerId); place(at(e)); });
compare.addEventListener("pointermove", (e) => dragging && place(at(e)));
compare.addEventListener("pointerup", () => (dragging = false));
handle.addEventListener("keydown", (e) => {
  const now = parseFloat(handle.style.left || "50") / 100;
  if (e.key === "ArrowLeft") place(now - 0.05);
  if (e.key === "ArrowRight") place(now + 0.05);
});
window.addEventListener("resize", fit);
window.addEventListener("load", fit);
fit();

// A slow sweep once, so people see it moves.
(() => {
  const t0 = performance.now();
  const step = (t) => {
    const k = (t - t0) / 1600;
    if (dragging || k > 1) return;
    place(0.5 + 0.22 * Math.sin(k * Math.PI * 2) * (1 - k));
    requestAnimationFrame(step);
  };
  setTimeout(() => requestAnimationFrame(step), 900);
})();

// ---------- newest version
fetch("https://api.github.com/repos/fdeox/deonyx/releases/latest")
  .then((r) => (r.ok ? r.json() : null))
  .then((r) => {
    if (r?.tag_name) document.getElementById("version").textContent = `${r.tag_name} · Windows 10 and 11 · free · GPL-3.0`;
  })
  .catch(() => {});

// ---------- try it
// The same numbers as Deonyx's shader: OKLab lightness flipped between the
// look's floor and ceiling, hue kept, saturation a little calmer.
const LOOKS = { gray: [0.232, 0.894], black: [0.04, 0.894], soft: [0.34, 0.812] };
let look = "gray";
let source = null;
let showOriginal = false;
// Photos found in the source, in its pixels, and how much of it they cover.
let kept = [];
let keptShare = 0;

const toLin = (c) => (c <= 0.04045 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4);
const toSrgb = (c) => (c <= 0.0031308 ? c * 12.92 : 1.055 * c ** (1 / 2.4) - 0.055);
// sRGB byte to linear, worked out once.
const LIN = Float32Array.from({ length: 256 }, (_, i) => toLin(i / 255));

function darken(img, mask) {
  const [lmin, lmax] = LOOKS[look];
  const d = img.data;
  // Screenshots repeat a few thousand colors: each is worked out once.
  const seen = new Map();
  for (let i = 0; i < d.length; i += 4) {
    if (mask && mask[i >> 2]) continue;
    const key = (d[i] << 16) | (d[i + 1] << 8) | d[i + 2];
    const done = seen.get(key);
    if (done !== undefined) {
      d[i] = done >> 16;
      d[i + 1] = (done >> 8) & 255;
      d[i + 2] = done & 255;
      continue;
    }
    const r = LIN[d[i]], g = LIN[d[i + 1]], b = LIN[d[i + 2]];
    let l = Math.cbrt(0.4122214708 * r + 0.5363325363 * g + 0.0514459929 * b);
    let m = Math.cbrt(0.2119034982 * r + 0.6806995451 * g + 0.1073969566 * b);
    let s = Math.cbrt(0.0883024619 * r + 0.2817188376 * g + 0.6299787005 * b);
    const L = 0.2104542553 * l + 0.793617785 * m - 0.0040720468 * s;
    let A = 1.9779984951 * l - 2.428592205 * m + 0.4505937099 * s;
    let B = 0.0259040371 * l + 0.7827717662 * m - 0.808675766 * s;
    const L2 = lmin + (1 - L) * (lmax - lmin);
    const keep = 0.95 * Math.min(1, L2 / Math.max(L, 0.001)) ** 0.8;
    A *= keep;
    B *= keep;
    l = (L2 + 0.3963377774 * A + 0.2158037573 * B) ** 3;
    m = (L2 - 0.1055613458 * A - 0.0638541728 * B) ** 3;
    s = (L2 - 0.0894841775 * A - 1.291485548 * B) ** 3;
    const out = [
      4.0767416621 * l - 3.3077115913 * m + 0.2309699292 * s,
      -1.2684380046 * l + 2.6097574011 * m - 0.3413193965 * s,
      -0.0041960863 * l - 0.7034186147 * m + 1.707614701 * s,
    ];
    for (let k = 0; k < 3; k++) d[i + k] = Math.round(255 * toSrgb(Math.min(1, Math.max(0, out[k]))));
    seen.set(key, (d[i] << 16) | (d[i + 1] << 8) | d[i + 2]);
  }
  return img;
}

// ---------- photos, as the app finds them (shader.hlsl photo, photos.rs)
// Every 4x4 block is rated: photo-like (neighbours differ, many in-between
// tones), calm (not a flat fill) or flat. Photo-like blocks become
// rectangles that grow over the calm blocks around them, and those
// rectangles keep their real colors.
const BLOCK = 4;

function rateBlocks(img) {
  const { width: W, height: H, data: d } = img;
  const w = Math.floor(W / BLOCK), h = Math.floor(H / BLOCK);
  const cells = new Uint8Array(w * h);
  const light = new Float32Array(16);
  const same = (i, j) => d[i] === d[j] && d[i + 1] === d[j + 1] && d[i + 2] === d[j + 2];
  for (let by = 0; by < h; by++) {
    for (let bx = 0; bx < w; bx++) {
      let lo = 1, hi = 0, flat = 0, solid = 0;
      for (let y = 0; y < 4; y++) {
        for (let x = 0; x < 4; x++) {
          const px = bx * 4 + x, py = by * 4 + y;
          const i = (py * W + px) * 4;
          const r = (py * W + Math.min(px + 1, W - 1)) * 4;
          const dn = (Math.min(py + 1, H - 1) * W + px) * 4;
          const sr = same(i, r), sd = same(i, dn);
          flat += sr || sd ? 1 : 0;
          solid += sr && sd ? 1 : 0;
          const v = Math.cbrt(0.2126 * LIN[d[i]] + 0.7152 * LIN[d[i + 1]] + 0.0722 * LIN[d[i + 2]]);
          light[y * 4 + x] = v;
          lo = Math.min(lo, v);
          hi = Math.max(hi, v);
        }
      }
      const range = hi - lo;
      let mid = 0;
      for (let k = 0; k < 16; k++) mid += light[k] > lo + 0.2 * range && light[k] < hi - 0.2 * range ? 1 : 0;
      cells[by * w + bx] = flat / 16 < 0.35 && mid / 16 >= 0.3 && range >= 0.05 ? 2 : solid / 16 < 0.5 ? 1 : 0;
    }
  }
  return { cells, w, h };
}

function spread(src, w, h, n, grow) {
  const pick = (a, b) => (grow ? a || b : a && b);
  const rows = new Uint8Array(w * h);
  for (let y = 0; y < h; y++) {
    for (let x = 0; x < w; x++) {
      let v = !grow;
      for (let k = Math.max(0, x - n); k <= Math.min(w - 1, x + n); k++) v = pick(v, !!src[y * w + k]);
      if (!grow && (x < n || x + n >= w)) v = false;
      rows[y * w + x] = v ? 1 : 0;
    }
  }
  const out = new Uint8Array(w * h);
  for (let y = 0; y < h; y++) {
    for (let x = 0; x < w; x++) {
      let v = !grow;
      for (let k = Math.max(0, y - n); k <= Math.min(h - 1, y + n); k++) v = pick(v, !!rows[k * w + x]);
      if (!grow && (y < n || y + n >= h)) v = false;
      out[y * w + x] = v ? 1 : 0;
    }
  }
  return out;
}

// Rectangles in pixels of the rated image.
function findPhotos({ cells, w, h }) {
  const on = cells.map((c) => (c === 2 ? 1 : 0));
  const calm = cells.map((c) => (c >= 1 ? 1 : 0));
  const closed = spread(spread(on, w, h, 2, true), w, h, 2, false);
  const seen = new Uint8Array(w * h);
  const share = (grid, xa, ya, xb, yb) => {
    let n = 0;
    for (let y = ya; y <= yb; y++) for (let x = xa; x <= xb; x++) n += grid[y * w + x];
    return n / ((xb - xa + 1) * (yb - ya + 1));
  };
  const found = [];
  for (let start = 0; start < w * h; start++) {
    if (!closed[start] || seen[start]) continue;
    let x0 = w, y0 = h, x1 = 0, y1 = 0;
    const stack = [start];
    seen[start] = 1;
    while (stack.length) {
      const i = stack.pop();
      const x = i % w, y = (i - x) / w;
      x0 = Math.min(x0, x); y0 = Math.min(y0, y); x1 = Math.max(x1, x); y1 = Math.max(y1, y);
      for (const j of [x > 0 ? i - 1 : -1, x + 1 < w ? i + 1 : -1, y > 0 ? i - w : -1, y + 1 < h ? i + w : -1]) {
        if (j >= 0 && closed[j] && !seen[j]) { seen[j] = 1; stack.push(j); }
      }
    }
    while (y1 > y0 && share(on, x0, y0, x1, y0) < 0.25) y0++;
    while (y1 > y0 && share(on, x0, y1, x1, y1) < 0.25) y1--;
    while (x1 > x0 && share(on, x0, y0, x0, y1) < 0.25) x0++;
    while (x1 > x0 && share(on, x1, y0, x1, y1) < 0.25) x1--;
    if (x1 + 1 - x0 < 12 || y1 + 1 - y0 < 12 || share(on, x0, y0, x1, y1) < 0.3) continue;
    while (y0 > 0 && share(calm, x0, y0 - 1, x1, y0 - 1) >= 0.6) y0--;
    while (y1 + 1 < h && share(calm, x0, y1 + 1, x1, y1 + 1) >= 0.6) y1++;
    while (x0 > 0 && share(calm, x0 - 1, y0, x0 - 1, y1) >= 0.6) x0--;
    while (x1 + 1 < w && share(calm, x1 + 1, y0, x1 + 1, y1) >= 0.6) x1++;
    for (let k = 0; k < 6; k++) {
      if (y0 > 0 && share(calm, x0, y0 - 1, x1, y0 - 1) >= 0.3) y0--;
      if (y1 + 1 < h && share(calm, x0, y1 + 1, x1, y1 + 1) >= 0.3) y1++;
      if (x0 > 0 && share(calm, x0 - 1, y0, x0 - 1, y1) >= 0.3) x0--;
      if (x1 + 1 < w && share(calm, x1 + 1, y0, x1 + 1, y1) >= 0.3) x1++;
    }
    found.push({ x: x0 * BLOCK, y: y0 * BLOCK, w: (x1 + 1 - x0) * BLOCK, h: (y1 + 1 - y0) * BLOCK });
  }
  return found;
}

// A whole picture that's a photo. Interfaces are made of a few flat colors:
// their 32 most common colors cover most of the picture, even in a JPEG.
// A photo spreads over thousands of them.
function looksLikePhoto({ width: W, height: H, data: d }) {
  const step = W * H > 2e6 ? 2 : 1;
  const count = new Map();
  let total = 0;
  for (let y = 0; y < H; y += step) {
    for (let x = 0; x < W; x += step) {
      const i = (y * W + x) * 4;
      const c = (d[i] << 16) | (d[i + 1] << 8) | d[i + 2];
      count.set(c, (count.get(c) || 0) + 1);
      total++;
    }
  }
  const top = [...count.values()].sort((a, b) => b - a).slice(0, 32).reduce((a, b) => a + b, 0);
  return top / total < 0.5;
}

// Rated on the picture's own pixels: scaling would blur the flat interface.
function findKept() {
  const k = Math.min(1, 4096 / Math.max(source.width, source.height));
  const c = document.createElement("canvas");
  c.width = Math.round(source.width * k);
  c.height = Math.round(source.height * k);
  const x = c.getContext("2d", { willReadFrequently: true });
  x.drawImage(source, 0, 0, c.width, c.height);
  const pixels = x.getImageData(0, 0, c.width, c.height);
  const rects = looksLikePhoto(pixels) ? [{ x: 0, y: 0, w: c.width, h: c.height }] : findPhotos(rateBlocks(pixels));
  kept = rects.map((r) => ({ x: r.x / c.width, y: r.y / c.height, w: r.w / c.width, h: r.h / c.height }));
  keptShare = Math.min(1, kept.reduce((a, r) => a + r.w * r.h, 0));
  const note = document.getElementById("try-note");
  if (keptShare > 0.6) note.textContent = "This looks like a photo, so Deonyx leaves it as it is. Try a screenshot of a light app.";
  else if (kept.length) note.textContent = `Kept ${kept.length === 1 ? "1 photo" : `${kept.length} photos`} in real colors, like the app does.`;
  else note.textContent = NOTE;
}

const NOTE = "Photos keep their real colors here too, like in the app. 3D views are found by the app itself.";
const canvas = document.getElementById("out");
const ctx = canvas.getContext("2d", { willReadFrequently: true });

function draw() {
  if (!source) return;
  const scale = Math.min(1, 1600 / source.width);
  canvas.width = Math.round(source.width * scale);
  canvas.height = Math.round(source.height * scale);
  ctx.drawImage(source, 0, 0, canvas.width, canvas.height);
  if (showOriginal) return;
  const W = canvas.width, H = canvas.height;
  const mask = new Uint8Array(W * H);
  for (const r of kept) {
    const x0 = Math.floor(r.x * W), y0 = Math.floor(r.y * H), x1 = Math.ceil((r.x + r.w) * W), y1 = Math.ceil((r.y + r.h) * H);
    for (let y = y0; y < y1; y++) mask.fill(1, y * W + x0, y * W + x1);
  }
  ctx.putImageData(darken(ctx.getImageData(0, 0, W, H), mask), 0, 0);
}

function load(file) {
  if (!file || !file.type.startsWith("image/")) return;
  const img = new Image();
  img.onload = () => {
    source = img;
    findKept();
    document.getElementById("drop-text").hidden = true;
    canvas.hidden = false;
    document.getElementById("try-tools").hidden = false;
    draw();
  };
  img.src = URL.createObjectURL(file);
}

const drop = document.getElementById("drop");
document.getElementById("file").addEventListener("change", (e) => load(e.target.files[0]));
drop.addEventListener("dragover", (e) => { e.preventDefault(); drop.classList.add("over"); });
drop.addEventListener("dragleave", () => drop.classList.remove("over"));
drop.addEventListener("drop", (e) => { e.preventDefault(); drop.classList.remove("over"); load(e.dataTransfer.files[0]); });
window.addEventListener("paste", (e) => load([...e.clipboardData.files][0]));
document.getElementById("style").addEventListener("click", (e) => {
  if (!e.target.dataset.style) return;
  look = e.target.dataset.style;
  document.querySelectorAll("#style button").forEach((b) => b.classList.toggle("on", b === e.target));
  draw();
});
document.getElementById("toggle").addEventListener("click", (e) => {
  showOriginal = !showOriginal;
  e.target.textContent = showOriginal ? "Show with Deonyx" : "Show original";
  draw();
});

// The name card loads, and so starts its animation, when it comes into view.
const nameCard = document.querySelector(".name-card");
const startName = () => {
  nameCard.parentElement.querySelectorAll("source").forEach((s) => (s.srcset = s.dataset.srcset));
  nameCard.src = nameCard.dataset.src;
};
if ("IntersectionObserver" in window) {
  const seen = new IntersectionObserver((entries) => {
    if (!entries.some((e) => e.isIntersecting)) return;
    seen.disconnect();
    startName();
  }, { threshold: 0.35 });
  seen.observe(nameCard);
} else {
  startName();
}
