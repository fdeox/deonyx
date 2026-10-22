// The before/after slider, the newest version number, and "Try it": a
// screenshot darkened in the browser with the same math as Deonyx.

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

const toLin = (c) => (c <= 0.04045 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4);
const toSrgb = (c) => (c <= 0.0031308 ? c * 12.92 : 1.055 * c ** (1 / 2.4) - 0.055);

function darken(img) {
  const [lmin, lmax] = LOOKS[look];
  const d = img.data;
  for (let i = 0; i < d.length; i += 4) {
    const r = toLin(d[i] / 255), g = toLin(d[i + 1] / 255), b = toLin(d[i + 2] / 255);
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
  }
  return img;
}

const canvas = document.getElementById("out");
const ctx = canvas.getContext("2d", { willReadFrequently: true });

function draw() {
  if (!source) return;
  const scale = Math.min(1, 1600 / source.width);
  canvas.width = Math.round(source.width * scale);
  canvas.height = Math.round(source.height * scale);
  ctx.drawImage(source, 0, 0, canvas.width, canvas.height);
  if (!showOriginal) ctx.putImageData(darken(ctx.getImageData(0, 0, canvas.width, canvas.height)), 0, 0);
}

function load(file) {
  if (!file || !file.type.startsWith("image/")) return;
  const img = new Image();
  img.onload = () => {
    source = img;
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
