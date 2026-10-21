// The tray window: darkening on and off, a pause, the look and the apps.

const { invoke } = window.__TAURI__.core;
const { listen } = window.__TAURI__.event;

let config = null;
let status = {};
const $ = (id) => document.getElementById(id);
const esc = (s) => String(s ?? "").replace(/[&<>"']/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" })[c]);
const hhmm = (secs) => new Date(secs * 1000).toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" });
const appName = (exe) => {
  const k = String(exe || "").toLowerCase();
  const all = [...(config?.pause_apps || []), ...(config?.apps || [])];
  return all.find((a) => String(a.exe).toLowerCase() === k)?.name || String(exe || "").replace(/\.exe$/i, "");
};

function save() {
  invoke("set_config", { config }).catch((e) => console.error(e));
}

function render() {
  if (!config) return;
  LANG = STRINGS[config.language] ? config.language : LANG;
  applyI18n();
  $("master").checked = config.enabled;
  const total = Object.values(status.running || {}).reduce((a, b) => a + b, 0);
  let sub = t("master.paused");
  if (config.enabled) {
    if (status.paused) sub = t("master.paused.until", { time: hhmm(config.paused_until) });
    else if (status.paused_by) sub = t("master.game", { app: appName(status.paused_by) });
    else if (status.battery) sub = t("master.battery");
    else if (status.waiting) sub = t("master.waiting");
    else if (total === 1) sub = t("master.count.one");
    else if (total > 1) sub = t("master.count.many", { n: total });
    else sub = t("master.none");
  }
  $("sub").textContent = sub;
  const paused = status.paused;
  $("pause").textContent = paused ? t("pause.resume") : t("flyout.pause");
  $("pause").disabled = !config.enabled;
  $("pause").classList.toggle("primary", !!paused);
  document.querySelectorAll("#style button").forEach((b) => b.classList.toggle("on", b.dataset.style === (config.style || "gray")));
  const profiles = config.profiles || [];
  $("profiles").hidden = !profiles.length;
  $("profiles").innerHTML = profiles.map((p, i) =>
    `<button data-profile="${i}" class="${p.name === config.profile ? "on" : ""}" title="${esc(p.name)}">${esc(p.name)}</button>`).join("");
  const list = $("apps");
  if (!config.apps.length) {
    list.innerHTML = `<div class="empty">${esc(config.everything ? t("apps.empty.everything") : t("apps.empty"))}</div>`;
    return;
  }
  list.innerHTML = config.apps.map((a, i) => `
    <label class="item">
      <div class="icon">${a.icon ? `<img src="${a.icon}" alt="">` : `<span>${esc((a.name || a.exe).charAt(0).toUpperCase())}</span>`}</div>
      <span class="name">${esc(a.name || a.exe)}</span>
      <input type="checkbox" class="switch" data-i="${i}" ${a.enabled ? "checked" : ""}>
    </label>`).join("");
}

$("master").addEventListener("change", (e) => {
  config.enabled = e.target.checked;
  save();
  render();
});
$("pause").addEventListener("click", () => invoke("pause", { minutes: status.paused ? 0 : 60 }));
$("style").addEventListener("click", (e) => {
  if (!e.target.dataset.style) return;
  config.style = e.target.dataset.style;
  save();
  render();
});
$("profiles").addEventListener("click", (e) => {
  const i = e.target.dataset.profile;
  if (i === undefined) return;
  invoke("use_profile", { name: config.profiles[+i].name });
});
$("apps").addEventListener("change", (e) => {
  const i = e.target.dataset.i;
  if (i === undefined) return;
  config.apps[+i].enabled = e.target.checked;
  save();
});
$("settings").addEventListener("click", () => invoke("open_settings_cmd"));

// Gone as soon as the user looks elsewhere, like Windows' own tray windows.
// Windows may hand the focus over and take it back at once while showing
// it; that blink isn't the user leaving.
let focusedAt = 0;
window.addEventListener("focus", () => (focusedAt = Date.now()));
window.addEventListener("blur", () => {
  if (focusedAt && Date.now() - focusedAt > 250) invoke("close_flyout");
  focusedAt = 0;
});
document.addEventListener("keydown", (e) => {
  if (e.key === "Escape") invoke("close_flyout");
});

async function load() {
  const s = await invoke("get_state");
  config = s.config;
  status = s.status;
  LANG = STRINGS[s.lang] ? s.lang : "en";
  render();
}

load();
listen("flyout-open", () => {
  focusedAt = 0;
  load();
});
listen("status", (e) => {
  status = e.payload;
  if (config) config.enabled = status.enabled;
  render();
});
listen("config", (e) => {
  config = e.payload;
  render();
});
