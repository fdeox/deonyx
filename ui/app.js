// Settings window. Talks to the Rust side through Tauri commands; every
// change is saved and applied right away.

const { invoke } = window.__TAURI__.core;
const { listen } = window.__TAURI__.event;

let config = {
  enabled: true, apps: [], exclude: [], everything: false, style: "gray", warm: 0, dim: 0, schedule: "always", from: "20:00", to: "07:00", gradual: false,
  language: "auto", updates: true, keys: { all: "Ctrl+Alt+D", front: "Ctrl+Alt+F", peek: "Ctrl+Alt+P", window: "Ctrl+Alt+W" }, share_original: false,
  battery: "keep", place: { name: "", lat: 0, lon: 0 }, screens: [], paused_until: 0, welcomed: false, seen_version: "",
  readable: true, share_auto: true, pause_apps: [], profiles: [], profile: "",
};
let status = {
  enabled: true, waiting: false, paused: false, battery: false, paused_by: "", sharing: false, running: {}, admin: {}, system: {},
  inspecting: false, keys_taken: [],
};
let crash = null;
let version = "";
let elevated = false;
let update = null;
let presets = [];
let screens = [];
const open = new Set();

const $ = (id) => document.getElementById(id);
const key = (exe) => String(exe || "").toLowerCase();
const esc = (s) => String(s ?? "").replace(/[&<>"']/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" })[c]);
const stem = (exe) => String(exe || "").replace(/\.exe$/i, "");
const presetOf = (exe) => presets.find((p) => p.exe === key(exe));

let saveTimer = 0;
function save() {
  clearTimeout(saveTimer);
  saveTimer = setTimeout(() => invoke("set_config", { config }).then(checkProfile).catch((e) => console.error(e)), 120);
}

// The settings differ from the profile in use (asked from Deonyx after each
// change): offer to save them into it, or to go back.
let profileChanged = false;
async function checkProfile() {
  profileChanged = !!config.profile && (await invoke("profile_changed").catch(() => false));
  renderProfiles();
}

let toastTimer = 0;
function toast(msg) {
  const el = $("toast");
  el.textContent = msg;
  el.hidden = false;
  clearTimeout(toastTimer);
  toastTimer = setTimeout(() => (el.hidden = true), 3200);
}

// The language Windows shows itself in, as Deonyx names it.
let systemLang = "";

function chooseLang() {
  const nav = (navigator.language || "en").toLowerCase();
  const sys = STRINGS[systemLang] ? systemLang : Object.keys(STRINGS).find((c) => nav === c || nav.startsWith(c + "-")) || "en";
  LANG = STRINGS[config.language] ? config.language : sys;
}

function iconHtml(app) {
  if (app.icon) return `<div class="icon"><img src="${app.icon}" alt=""></div>`;
  return `<div class="icon"><span>${esc((app.name || app.exe || "?").charAt(0).toUpperCase())}</span></div>`;
}

function nameOf(exe) {
  const k = key(exe);
  return [...config.apps, ...config.exclude, ...(config.pause_apps || [])].find((a) => key(a.exe) === k)?.name || stem(exe);
}

const hhmm = (secs) => new Date(secs * 1000).toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" });

// ---------- status lines

function stateText(app) {
  if (!config.enabled) return { live: false, text: t("state.paused") };
  if (status.paused) return { live: false, text: t("state.paused") };
  if (status.battery) return { live: false, text: t("state.battery") };
  if (status.paused_by) return { live: false, text: t("master.game", { app: nameOf(status.paused_by) }) };
  if (status.waiting) return { live: false, text: t("state.waiting") };
  if (!app.enabled) return { live: false, text: t("state.off") };
  const k = key(app.exe);
  const n = status.running?.[k] ?? 0;
  const sys = status.system?.[k] ?? 0;
  const service = t("state.service");
  if (n > 0) {
    const text = n === 1 ? t("state.one") : t("state.many", { n });
    return { live: true, text: sys > 0 ? `${text}. ${t("state.others", { s: service })}` : text };
  }
  if (sys > 0) return { live: false, text: t("state.all", { s: service }) };
  if ((status.admin?.[k] ?? 0) > 0) return { live: false, text: t("state.admin") };
  return { live: false, text: t("state.idle") };
}

function renderMaster() {
  $("master").checked = config.enabled;
  $("master-label").textContent = config.enabled ? t("on") : t("off");
  const total = Object.values(status.running || {}).reduce((a, b) => a + b, 0);
  let sub = t("master.paused");
  if (config.enabled) {
    if (status.paused) sub = t("master.paused.until", { time: hhmm(config.paused_until) });
    else if (status.battery) sub = t("master.battery");
    else if (status.paused_by) sub = t("master.game", { app: nameOf(status.paused_by) });
    else if (status.waiting) sub = t("master.waiting");
    else if (total === 1) sub = t("master.count.one");
    else if (total > 1) sub = t("master.count.many", { n: total });
    else sub = t("master.none");
    if (status.sharing && !status.paused && !status.paused_by) sub += ` · ${t("master.sharing")}`;
  }
  $("master-sub").textContent = sub;
  $("crash-bar").hidden = !crash;
  $("pause-bar").hidden = !(config.enabled && status.paused);
  $("pause-text").textContent = t("pause.until", { time: hhmm(config.paused_until) });
}

$("resume").addEventListener("click", () => invoke("pause", { minutes: 0 }));

// After a crash: the report for a bug report, until dismissed.
$("crash-copy").addEventListener("click", async () => {
  await navigator.clipboard.writeText(await invoke("diagnostics"));
  toast(t("crash.copied"));
});
$("crash-dismiss").addEventListener("click", () => {
  crash = null;
  invoke("dismiss_crash");
  renderMaster();
});

// ---------- mode

function renderMode() {
  document.querySelectorAll("#mode button").forEach((b) => b.classList.toggle("on", (b.dataset.mode === "everything") === !!config.everything));
  $("mode-hint").textContent = config.everything ? t("mode.everything.hint") : t("mode.list.hint");
  $("apps-title").textContent = config.everything ? t("apps.title.everything") : t("apps.title");
  $("apps-hint").textContent = config.everything ? t("apps.hint.everything") : t("apps.hint");
  $("exclude-section").hidden = !config.everything;
}

$("mode").addEventListener("click", (e) => {
  const m = e.target.dataset.mode;
  if (!m) return;
  config.everything = m === "everything";
  save();
  renderMode();
  renderApps();
});

// ---------- apps

function renderApps() {
  const list = $("apps");
  if (!config.apps.length) {
    list.innerHTML = `<div class="empty"><strong>${esc(t("apps.empty.title"))}</strong>${esc(config.everything ? t("apps.empty.everything") : t("apps.empty"))}</div>`;
    return;
  }
  list.innerHTML = config.apps.map((app, i) => {
    const s = stateText(app);
    const isOpen = open.has(key(app.exe));
    const areas = app.areas?.length ?? 0;
    const native = presetOf(app.exe)?.native;
    return `
    <div class="app ${isOpen ? "open" : ""}" data-i="${i}">
      <div class="app-head" data-act="expand">
        ${iconHtml(app)}
        <div class="app-name">
          <strong>${esc(app.name || app.exe)}</strong>
          <small class="state"><span class="dot ${s.live ? "live" : ""}"></span><span data-state>${esc(s.text)}</span></small>
        </div>
        <span class="chev" aria-hidden="true">&#9654;</span>
        <input type="checkbox" class="switch" data-act="enabled" ${app.enabled ? "checked" : ""} aria-label="${esc(app.name)}">
      </div>
      ${isOpen ? `
      <div class="app-body">
        ${native ? `<div class="note">${esc(t("app.native", { app: app.name || stem(app.exe) }))} <b>${esc(native)}</b></div>` : ""}
        <label class="slider"><span>${esc(t("app.darkness"))}</span>
          <input type="range" min="0" max="100" step="1" value="${Math.round(app.darkness * 100)}" data-act="darkness">
          <span data-val="darkness">${Math.round(app.darkness * 100)}%</span></label>
        <label class="slider"><span>${esc(t("app.text"))}</span>
          <input type="range" min="0" max="100" step="1" value="${Math.round(app.text * 100)}" data-act="text">
          <span data-val="text">${Math.round(app.text * 100)}%</span></label>
        <label class="check">
          <input type="checkbox" class="switch" data-act="protect" ${app.protect ? "checked" : ""}>
          <span><strong>${esc(t("app.protect"))}</strong>
          <small>${esc(t("app.protect.hint"))}</small></span>
        </label>
        <label class="check">
          <input type="checkbox" class="switch" data-act="photos" ${app.photos !== false ? "checked" : ""}>
          <span><strong>${esc(t("app.photos"))}</strong>
          <small>${esc(t("app.photos.hint"))}</small></span>
        </label>
        <div class="check areas">
          <span><strong>${esc(t("app.hotkey"))}</strong>
          <small>${esc(appKeyTaken(app) ? t("keys.taken") : t("app.hotkey.hint"))}</small></span>
          <span class="actions">
            <kbd class="${appKeyTaken(app) ? "taken" : ""} ${recording === appKeyId(app) ? "rec" : ""}">${esc(recording === appKeyId(app) ? t("keys.press") : app.hotkey || t("keys.none"))}</kbd>
            <button class="quiet" data-act="hotkey">${esc(recording === appKeyId(app) ? t("tools.cancel") : t("keys.change"))}</button>
          </span>
        </div>
        <div class="check areas">
          <span><strong>${esc(t("app.areas"))}</strong>
          <small>${esc(areas ? t("app.areas.some", { n: areas }) : t("app.areas.none"))}</small></span>
          <span class="actions">
            ${areas ? `<button class="quiet" data-act="clear">${esc(t("app.clear"))}</button>` : ""}
            <button class="quiet" data-act="pick">${esc(t("app.pick"))}</button>
            <button data-act="draw">${esc(t("app.draw"))}</button>
          </span>
        </div>
        <div class="app-foot">
          <small>${esc(app.exe)}</small>
          <button class="danger" data-act="remove">${esc(t("app.remove"))}</button>
        </div>
      </div>` : ""}
    </div>`;
  }).join("");
}

function renderStates() {
  document.querySelectorAll("#apps .app").forEach((el) => {
    const app = config.apps[+el.dataset.i];
    if (!app) return;
    const s = stateText(app);
    el.querySelector("[data-state]").textContent = s.text;
    el.querySelector(".dot").classList.toggle("live", s.live);
  });
  renderMaster();
  renderKeys();
}

$("apps").addEventListener("click", (e) => {
  const card = e.target.closest(".app");
  if (!card) return;
  const app = config.apps[+card.dataset.i];
  const act = e.target.dataset.act;
  if (act === "remove") {
    config.apps.splice(+card.dataset.i, 1);
    open.delete(key(app.exe));
    save();
    renderApps();
  } else if (act === "draw") {
    invoke("draw_area", { exe: app.exe, hint: t("draw.hint") });
  } else if (act === "pick") {
    startPick();
  } else if (act === "clear") {
    app.areas = [];
    save();
    renderApps();
  } else if (act === "hotkey") {
    recording = recording === appKeyId(app) ? "" : appKeyId(app);
    renderKeys();
    renderApps();
  } else if (act === "expand" || (e.target.closest(".app-head") && !act)) {
    const k = key(app.exe);
    open.has(k) ? open.delete(k) : open.add(k);
    renderApps();
  }
});

$("apps").addEventListener("input", (e) => {
  const card = e.target.closest(".app");
  if (!card) return;
  const app = config.apps[+card.dataset.i];
  const act = e.target.dataset.act;
  if (act === "darkness" || act === "text") {
    app[act] = e.target.value / 100;
    card.querySelector(`[data-val="${act}"]`).textContent = `${e.target.value}%`;
    save();
  } else if (act === "enabled" || act === "protect" || act === "photos") {
    app[act] = e.target.checked;
    save();
    renderStates();
  }
});

// Each app's own hotkey goes by "app:<exe>", like Deonyx's own by name.
const appKeyId = (app) => `app:${key(app.exe)}`;
const appKeyTaken = (app) => !!app.hotkey && (status.keys_taken || []).includes(appKeyId(app));

// A new app, with the settings its preset suggests.
function newApp(r) {
  const p = presetOf(r.exe) || {};
  return {
    exe: r.exe, name: r.name, enabled: true, darkness: p.darkness ?? 0.6, text: p.text ?? 0.7,
    protect: p.protect ?? true, photos: p.photos ?? true, areas: [], icon: r.icon || undefined,
  };
}

function addApp(r) {
  const k = key(r.exe);
  if (!config.apps.some((a) => key(a.exe) === k)) {
    config.apps.push(newApp(r));
    // An app of its own isn't left alone any more.
    config.exclude = config.exclude.filter((x) => key(x.exe) !== k);
    save();
  }
  open.add(k);
  closePicker("apps");
  renderApps();
  renderExclude();
}

// ---------- left alone

function renderExclude() {
  const list = $("exclude");
  if (!config.exclude.length) {
    list.innerHTML = `<div class="empty small">${esc(t("exclude.empty"))}</div>`;
    return;
  }
  list.innerHTML = config.exclude.map((x, i) => `
    <div class="app" data-x="${i}"><div class="app-head flat">
      ${iconHtml(x)}
      <div class="app-name"><strong>${esc(x.name || x.exe)}</strong><small>${esc(x.exe)}</small></div>
      <button class="danger" data-act="unexclude">${esc(t("app.remove"))}</button>
    </div></div>`).join("");
}

$("exclude").addEventListener("click", (e) => {
  if (e.target.dataset.act !== "unexclude") return;
  config.exclude.splice(+e.target.closest("[data-x]").dataset.x, 1);
  save();
  renderExclude();
});

function addExclude(r) {
  const k = key(r.exe);
  if (!config.exclude.some((a) => key(a.exe) === k)) {
    config.exclude.push({ exe: r.exe, name: r.name, icon: r.icon || undefined });
    save();
  }
  closePicker("exclude");
  renderExclude();
}

// ---------- paused while these run

function renderPause() {
  const list = $("pause-apps");
  const apps = config.pause_apps || [];
  if (!apps.length) {
    list.innerHTML = `<div class="empty small">${esc(t("game.empty"))}</div>`;
    return;
  }
  list.innerHTML = apps.map((x, i) => {
    const now = key(status.paused_by) === key(x.exe);
    return `
    <div class="app" data-p="${i}"><div class="app-head flat">
      ${iconHtml(x)}
      <div class="app-name"><strong>${esc(x.name || x.exe)}</strong>
        <small class="state"><span class="dot ${now ? "live" : ""}"></span><span>${esc(now ? t("game.now") : x.exe)}</span></small></div>
      <button class="danger" data-act="unpause">${esc(t("app.remove"))}</button>
    </div></div>`;
  }).join("");
}

$("pause-apps").addEventListener("click", (e) => {
  if (e.target.dataset.act !== "unpause") return;
  config.pause_apps.splice(+e.target.closest("[data-p]").dataset.p, 1);
  save();
  renderPause();
});

function addPause(r) {
  const k = key(r.exe);
  config.pause_apps ||= [];
  if (!config.pause_apps.some((a) => key(a.exe) === k)) {
    config.pause_apps.push({ exe: r.exe, name: r.name, icon: r.icon || undefined });
    save();
  }
  closePicker("pause");
  renderPause();
}

// ---------- program picker, for every list

const PICKERS = { apps: "picker", exclude: "exclude-picker", pause: "pause-picker" };
const PICKED = { apps: () => config.apps, exclude: () => config.exclude, pause: () => config.pause_apps || [] };

function closePicker(which) {
  const p = $(PICKERS[which]);
  p.hidden = true;
  p.innerHTML = "";
}

const fold = (s) => String(s ?? "").toLocaleLowerCase("tr").normalize("NFD").replace(/[̀-ͯ]/g, "").replace(/ı/g, "i");

function pickRow(r, i, group, added) {
  return `
    <div class="pick" data-search="${esc(fold(`${r.name} ${r.exe}`))}" data-row="${group}:${i}">
      ${iconHtml(r)}
      <div class="app-name"><strong>${esc(r.name)}</strong><small>${esc(r.title || r.exe)}</small></div>
      ${added.has(key(r.exe)) ? `<small>${esc(t("picker.added"))}</small>` : `<button data-pick="${group}:${i}">${esc(t("picker.add"))}</button>`}
    </div>`;
}

// Icons of installed programs arrive in small batches, so the list is usable
// at once even when Windows is slow to open the files.
async function fillIcons(p, list) {
  for (let start = 0; start < list.length; start += 16) {
    if (p.hidden) return;
    const batch = list.slice(start, start + 16);
    const icons = await invoke("app_icons", { paths: batch.map((r) => r.title) }).catch(() => []);
    icons.forEach((icon, j) => {
      const i = start + j;
      list[i].icon = icon || undefined;
      const box = p.querySelector(`[data-row="installed:${i}"] .icon`);
      if (icon && box) box.innerHTML = `<img src="${icon}" alt="">`;
    });
  }
}

function openPicker(which) {
  const p = $(PICKERS[which]);
  const onPick = { apps: addApp, exclude: addExclude, pause: addPause }[which];
  const added = new Set(PICKED[which]().map((a) => key(a.exe)));
  p.hidden = false;
  p.innerHTML = `
    <div class="picker-head">
      <input type="search" class="pick-search" placeholder="${esc(t("picker.search"))}" autocomplete="off" spellcheck="false">
      <button class="quiet" data-pick="close">${esc(t("picker.close"))}</button>
    </div>
    <div class="picker-list">
      <div class="group" data-group="open">${esc(t("picker.open"))}</div><div class="pick-open"><small class="pad">${esc(t("picker.looking.open"))}</small></div>
      <div class="group" data-group="installed">${esc(t("picker.installed"))}</div><div class="pick-installed"><small class="pad">${esc(t("picker.looking.installed"))}</small></div>
      <small class="pad pick-none" hidden>${esc(t("picker.none"))}</small>
    </div>
    <div class="picker-foot"><small>${esc(t("picker.notlisted"))}</small><button class="quiet" data-pick="browse">${esc(t("picker.browse"))}</button></div>`;
  const search = p.querySelector(".pick-search");
  search.focus();
  const lists = { open: [], installed: [] };

  const filter = () => {
    const q = fold(search.value.trim());
    let any = false;
    for (const g of ["open", "installed"]) {
      let shown = 0;
      p.querySelectorAll(`.pick-${g} .pick`).forEach((row) => {
        const hit = !q || row.dataset.search.includes(q);
        row.hidden = !hit;
        if (hit) shown++;
      });
      const loading = !!p.querySelector(`.pick-${g} .pad`);
      p.querySelector(`[data-group="${g}"]`).hidden = !shown && !loading;
      any ||= shown > 0 || loading;
    }
    p.querySelector(".pick-none").hidden = any;
  };
  search.addEventListener("input", filter);
  search.addEventListener("keydown", (e) => {
    if (e.key === "Enter") p.querySelector(".pick:not([hidden]) button")?.click();
    if (e.key === "Escape") closePicker(which);
  });

  const openReady = invoke("running_apps").then((list) => {
    lists.open = list;
    p.querySelector(".pick-open").innerHTML = list.map((r, i) => pickRow(r, i, "open", added)).join("");
    filter();
    return list;
  });
  // Wait for the open ones: a program shows up under one heading only.
  Promise.all([openReady, invoke("installed_apps")]).then(([opened, list]) => {
    const running = new Set(opened.map((r) => key(r.exe)));
    lists.installed = list.filter((r) => !running.has(key(r.exe)));
    p.querySelector(".pick-installed").innerHTML = lists.installed.map((r, i) => pickRow(r, i, "installed", added)).join("");
    filter();
    fillIcons(p, lists.installed);
  });

  p.onclick = async (e) => {
    const what = e.target.dataset.pick;
    if (what === undefined) return;
    if (what === "close") return closePicker(which);
    if (what === "browse") {
      const r = await invoke("pick_exe");
      if (r) onPick(r);
      return;
    }
    const [g, i] = what.split(":");
    onPick(lists[g][+i]);
  };
}

$("add").addEventListener("click", () => ($("picker").hidden ? openPicker("apps") : closePicker("apps")));
$("add-exclude").addEventListener("click", () => ($("exclude-picker").hidden ? openPicker("exclude") : closePicker("exclude")));
$("add-pause").addEventListener("click", () => ($("pause-picker").hidden ? openPicker("pause") : closePicker("pause")));

// ---------- profiles

// Two clicks to delete: the first one asks.
let deleting = "";

function renderProfiles() {
  const list = config.profiles || [];
  const changed = profileChanged && list.some((p) => p.name === config.profile);
  $("profile-changed").hidden = !changed;
  if (changed) $("profile-changed-text").textContent = t("profiles.changed", { name: config.profile });
  const nav = document.querySelector('#side button[data-page="profiles"]');
  nav.querySelector(".mark")?.remove();
  if (changed) nav.insertAdjacentHTML("beforeend", '<i class="mark"></i>');
  if (!list.length) {
    $("profiles").innerHTML = `<div class="empty small">${esc(t("profiles.empty"))}</div>`;
    return;
  }
  $("profiles").innerHTML = list.map((p, i) => `
    <span class="chip ${p.name === config.profile ? "on" : ""}" data-i="${i}">
      <button data-act="use" title="${esc(t("profiles.use"))}">${esc(p.name)}${changed && p.name === config.profile ? " •" : ""}</button>
      <button data-act="delete" class="${deleting === p.name ? "sure" : ""}" aria-label="${esc(t("profiles.delete"))}">${deleting === p.name ? esc(t("profiles.delete")) : "&#215;"}</button>
    </span>`).join("");
}

$("profiles").addEventListener("click", async (e) => {
  const chip = e.target.closest(".chip");
  if (!chip) return;
  const p = config.profiles[+chip.dataset.i];
  if (e.target.dataset.act === "use") {
    deleting = "";
    if (p.name !== config.profile) await invoke("use_profile", { name: p.name });
  } else if (e.target.dataset.act === "delete") {
    if (deleting !== p.name) {
      deleting = p.name;
      setTimeout(() => {
        if (deleting === p.name) {
          deleting = "";
          renderProfiles();
        }
      }, 3000);
      return renderProfiles();
    }
    deleting = "";
    await invoke("delete_profile", { name: p.name });
  }
});

$("profile-new").addEventListener("click", () => {
  const form = $("profile-form");
  form.hidden = !form.hidden;
  if (!form.hidden) {
    $("profile-name").value = "";
    $("profile-name").focus();
  }
});
$("profile-cancel").addEventListener("click", () => ($("profile-form").hidden = true));
$("profile-form").addEventListener("submit", async (e) => {
  e.preventDefault();
  const name = $("profile-name").value.trim();
  if (!name) return $("profile-name").focus();
  await invoke("save_profile", { name });
  $("profile-form").hidden = true;
  toast(t("profiles.saved", { name }));
});
$("profile-update").addEventListener("click", async () => {
  await invoke("save_profile", { name: config.profile });
  toast(t("profiles.saved", { name: config.profile }));
  checkProfile();
});
$("profile-undo").addEventListener("click", () => invoke("use_profile", { name: config.profile }));
$("profile-name").addEventListener("keydown", (e) => {
  if (e.key === "Escape") $("profile-form").hidden = true;
});

// ---------- the look, on a sample window
// A light app drawn here, its right half shown the way Deonyx would show it:
// the same steps as shader.hlsl, with an app's default darkness and text.

const LIN = Float32Array.from({ length: 256 }, (_, i) => {
  const c = i / 255;
  return c <= 0.04045 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4;
});
const toSrgb = (c) => (c <= 0.0031308 ? c * 12.92 : 1.055 * c ** (1 / 2.4) - 0.055);

// config.rs AppRule::lightness, darkness 0.6 and text 0.7.
function lightness(style) {
  const d = 0.6, t = 0.7;
  if (style === "black") return [0.1 * (1 - d), 0.74 + 0.22 * t];
  if (style === "soft") return [0.46 - 0.2 * d, 0.7 + 0.16 * t];
  if (style === "paper") return [0.4 - 0.18 * d, 0.66 + 0.14 * t];
  return [0.4 - 0.28 * d, 0.74 + 0.22 * t];
}

function flip(img, skip) {
  const style = config.style || "gray";
  const [lmin, lmax] = lightness(style);
  const warm = style === "paper" ? Math.max(config.warm || 0, 0.55) : config.warm || 0;
  const boost = config.readable !== false ? 0.78 : 1;
  const tint = [1, 1 - 0.14 * warm, 1 - 0.32 * warm];
  const d = img.data;
  const seen = new Map();
  for (let i = 0; i < d.length; i += 4) {
    const p = i >> 2;
    if (skip(p % img.width, (p / img.width) | 0)) continue;
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
    let t = Math.min(1, Math.max(0, 1 - L));
    if (boost < 1) t = t ** boost;
    const L2 = lmin + t * (lmax - lmin);
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
    for (let k = 0; k < 3; k++) d[i + k] = Math.round(255 * toSrgb(Math.min(1, Math.max(0, out[k]))) * tint[k]);
    seen.set(key, (d[i] << 16) | (d[i + 1] << 8) | d[i + 2]);
  }
}

// The sample: title bar, toolbar, a list, text (one line faint), a button
// and a photo. Returns the photo's box, which Deonyx keeps as it is.
function drawSample(ctx, W, H) {
  const box = (x, y, w, h, c, r = 0) => {
    ctx.fillStyle = c;
    ctx.beginPath();
    ctx.roundRect(x, y, w, h, r);
    ctx.fill();
  };
  box(0, 0, W, H, "#ffffff");
  box(0, 0, W, 30, "#f1f2f4");
  box(14, 11, 70, 8, "#5f6368", 4);
  for (const x of [W - 70, W - 46, W - 22]) box(x, 14, 10, 2, "#5f6368");
  box(0, 30, W, 38, "#fafafa");
  box(0, 67, W, 1, "#e3e5e8");
  ["#1a73e8", "#e8453c", "#34a853", "#f9ab00", "#9aa0a6", "#9aa0a6"].forEach((c, i) => box(14 + i * 30, 39, 20, 20, c, 5));
  box(0, 68, 130, H - 68, "#f4f5f7");
  box(130, 68, 1, H - 68, "#e3e5e8");
  box(8, 84, 114, 22, "#dbe7fd", 6);
  box(18, 92, 70, 6, "#1a73e8", 3);
  [118, 142, 166, 190, 214].forEach((y, i) => box(18, y, 60 + ((i * 23) % 40), 6, "#80868b", 3));
  box(152, 86, 190, 12, "#202124", 6);
  [112, 128, 144, 160].forEach((y, i) => box(152, y, [250, 230, 244, 190][i], 7, "#3c4043", 3.5));
  box(152, 182, 210, 7, "#b4b8bd", 3.5);
  box(152, 206, 220, 7, "#3c4043", 3.5);
  box(152, 238, 96, 30, "#1a73e8", 8);
  box(170, 250, 60, 6, "#ffffff", 3);
  // The photo: a sky, a sun and hills.
  const P = { x: W - 190, y: 92, w: 168, h: 116 };
  ctx.save();
  ctx.beginPath();
  ctx.roundRect(P.x, P.y, P.w, P.h, 6);
  ctx.clip();
  const sky = ctx.createLinearGradient(0, P.y, 0, P.y + P.h);
  sky.addColorStop(0, "#5aa7ff");
  sky.addColorStop(1, "#d6ecff");
  ctx.fillStyle = sky;
  ctx.fillRect(P.x, P.y, P.w, P.h);
  ctx.fillStyle = "#ffd25e";
  ctx.beginPath();
  ctx.arc(P.x + 122, P.y + 34, 15, 0, Math.PI * 2);
  ctx.fill();
  ctx.fillStyle = "#5daa4f";
  ctx.beginPath();
  ctx.ellipse(P.x + 40, P.y + P.h + 6, 90, 52, 0, 0, Math.PI * 2);
  ctx.fill();
  ctx.fillStyle = "#3b8a3d";
  ctx.beginPath();
  ctx.ellipse(P.x + 140, P.y + P.h + 14, 80, 48, 0, 0, Math.PI * 2);
  ctx.fill();
  ctx.restore();
  return P;
}

function drawPreview() {
  const c = $("preview");
  if (!c || !c.offsetParent) return;
  const W = c.width, H = c.height;
  const ctx = c.getContext("2d", { willReadFrequently: true });
  const P = drawSample(ctx, W, H);
  const half = Math.round(W * 0.42);
  const img = ctx.getImageData(half, 0, W - half, H);
  flip(img, (x, y) => x + half >= P.x && x + half < P.x + P.w && y >= P.y && y < P.y + P.h);
  ctx.putImageData(img, half, 0);
  ctx.fillStyle = "#a99cff";
  ctx.fillRect(half - 1, 0, 3, H);
}

// ---------- pages

let page = "apps";
try {
  page = localStorage.getItem("deonyx.page") || "apps";
} catch {}

function showPage(p) {
  if (!document.querySelector(`.page[data-page="${p}"]`)) p = "apps";
  // A hotkey waiting for its keys would swallow the typing on the next page.
  if (recording) {
    recording = "";
    renderKeys();
    renderApps();
  }
  page = p;
  document.querySelectorAll(".page").forEach((el) => el.classList.toggle("on", el.dataset.page === p));
  document.querySelectorAll("#side button").forEach((b) => b.classList.toggle("on", b.dataset.page === p));
  try {
    localStorage.setItem("deonyx.page", p);
  } catch {}
  window.scrollTo(0, 0);
  drawPreview();
}

$("side").addEventListener("click", (e) => {
  const b = e.target.closest("button[data-page]");
  if (b) showPage(b.dataset.page);
});

// ---------- look

function renderLook() {
  document.querySelectorAll("#style button").forEach((b) => b.classList.toggle("on", b.dataset.style === (config.style || "gray")));
  const w = Math.round((config.warm || 0) * 100);
  $("warm").value = w;
  $("warm-val").textContent = w ? `${w}%` : t("off");
  $("share").checked = !!config.share_original;
  $("share-auto").checked = config.share_auto !== false;
  $("share-auto-row").hidden = !!config.share_original;
  $("readable").checked = config.readable !== false;
  const d = Math.round((config.dim || 0) * 100);
  $("dim").value = d;
  $("dim-val").textContent = d ? `${d}%` : t("off");
  renderScreens();
  drawPreview();
}

$("style").addEventListener("click", (e) => {
  if (!e.target.dataset.style) return;
  config.style = e.target.dataset.style;
  save();
  renderLook();
});

$("warm").addEventListener("input", (e) => {
  config.warm = e.target.value / 100;
  $("warm-val").textContent = +e.target.value ? `${e.target.value}%` : t("off");
  save();
  drawPreview();
});

$("share").addEventListener("change", (e) => {
  config.share_original = e.target.checked;
  save();
  renderLook();
});
$("share-auto").addEventListener("change", (e) => {
  config.share_auto = e.target.checked;
  save();
});
$("readable").addEventListener("change", (e) => {
  config.readable = e.target.checked;
  save();
  drawPreview();
});
$("dim").addEventListener("input", (e) => {
  config.dim = e.target.value / 100;
  $("dim-val").textContent = +e.target.value ? `${e.target.value}%` : t("off");
  save();
});

// Laptop panels often have no name of their own.
const screenName = (s, i) => s.name || (s.builtin ? t("screens.builtin") : t("screens.unnamed", { n: i + 1 }));

// Each screen: the same look as everywhere, its own, or not darkened.
function renderScreens() {
  const box = $("screens");
  if (screens.length < 2) {
    box.innerHTML = "";
    return;
  }
  box.innerHTML = screens.map((s, i) => {
    const rule = config.screens.find((r) => r.id === s.id) || {};
    const name = screenName(s, i);
    const value = rule.off ? "off" : rule.style || "";
    const opt = (v, label) => `<option value="${v}" ${v === value ? "selected" : ""}>${esc(label)}</option>`;
    return `
      <div class="row" data-screen="${i}">
        <span><strong>${esc(name)}</strong><small>${esc(s.primary ? t("screens.main") : t("screens.other"))}</small></span>
        <select data-act="screen">
          ${opt("", t("screens.same"))}${opt("gray", t("look.gray"))}${opt("black", t("look.black"))}${opt("soft", t("look.soft"))}${opt("paper", t("look.paper"))}${opt("off", t("screens.off"))}
        </select>
      </div>`;
  }).join("");
}

$("screens").addEventListener("change", (e) => {
  const row = e.target.closest("[data-screen]");
  if (!row) return;
  const s = screens[+row.dataset.screen];
  const v = e.target.value;
  config.screens = config.screens.filter((r) => r.id !== s.id);
  if (v) config.screens.push({ id: s.id, name: screenName(s, +row.dataset.screen), style: v === "off" ? "" : v, off: v === "off" });
  save();
});

// ---------- when

function renderWhen() {
  $("schedule").value = config.schedule || "always";
  $("times-row").hidden = config.schedule !== "times";
  $("sun-row").hidden = config.schedule !== "sun";
  // Easing in and out only makes sense where the schedule has a start and an end.
  $("gradual-row").hidden = config.schedule !== "times" && config.schedule !== "sun";
  $("gradual").checked = !!config.gradual;
  $("from").value = config.from || "20:00";
  $("to").value = config.to || "07:00";
  $("when-hint").textContent = t(`when.hint.${config.schedule || "always"}`);
  $("battery").value = config.battery || "keep";
  $("place").value = config.place?.name || "";
  renderSun();
}

async function renderSun() {
  const el = $("sun-times");
  if (config.schedule !== "sun") return;
  if (!config.place?.name) {
    el.textContent = t("sun.pick");
    return;
  }
  const s = await invoke("sun_times", { lat: config.place.lat, lon: config.place.lon });
  el.textContent = s.kind === "times" ? t("sun.today", { set: s.set, rise: s.rise }) : t(`sun.${s.kind}`);
}

$("schedule").addEventListener("change", (e) => {
  config.schedule = e.target.value;
  save();
  renderWhen();
});
for (const id of ["from", "to"]) {
  $(id).addEventListener("change", (e) => {
    if (!e.target.value) return;
    config[id] = e.target.value;
    save();
  });
}
$("battery").addEventListener("change", (e) => {
  config.battery = e.target.value;
  save();
});
$("gradual").addEventListener("change", (e) => {
  config.gradual = e.target.checked;
  save();
});

// Where the sun is watched: a city from the list, or typed coordinates.
function placeMatches(q) {
  const coords = q.match(/^\s*(-?\d+(?:[.,]\d+)?)\s*[,; ]\s*(-?\d+(?:[.,]\d+)?)\s*$/);
  if (coords) {
    const lat = parseFloat(coords[1].replace(",", "."));
    const lon = parseFloat(coords[2].replace(",", "."));
    if (Math.abs(lat) <= 90 && Math.abs(lon) <= 180) return [{ name: `${lat.toFixed(2)}, ${lon.toFixed(2)}`, lat, lon }];
  }
  const f = fold(q.trim());
  if (!f) return [];
  return CITIES.filter((c) => fold(`${c[0]} ${c[4] || ""}`).includes(f))
    .slice(0, 8)
    .map((c) => ({ name: `${c[0]}, ${c[1]}`, lat: c[2], lon: c[3] }));
}

$("place").addEventListener("input", (e) => {
  const list = $("place-list");
  const found = placeMatches(e.target.value);
  list.hidden = !found.length;
  list.innerHTML = found.map((p, i) => `<button class="quiet" data-place="${i}">${esc(p.name)}</button>`).join("");
  list.onclick = (ev) => {
    const i = ev.target.dataset.place;
    if (i === undefined) return;
    config.place = found[+i];
    $("place").value = config.place.name;
    list.hidden = true;
    save();
    renderSun();
  };
});
$("place").addEventListener("keydown", (e) => {
  if (e.key === "Enter") $("place-list").querySelector("button")?.click();
  if (e.key === "Escape") $("place-list").hidden = true;
});

// ---------- tools

function setInspecting(on) {
  $("inspect").textContent = on && picking !== "part" ? t("tools.cancel") : t("tools.check");
  $("inspect-hint").textContent = on && picking !== "part" ? t("tools.waiting") : t("tools.inspect.hint");
  $("inspect").classList.toggle("primary", on && picking !== "part");
  $("pick-part").textContent = on && picking === "part" ? t("tools.cancel") : t("tools.part.pick");
  $("part-hint").textContent = on && picking === "part" ? t("tools.part.waiting") : t("tools.part.hint");
  $("pick-part").classList.toggle("primary", on && picking === "part");
}

// What the next click is for: "inspect" or "part".
let picking = "";

$("inspect").addEventListener("click", () => {
  const on = !status.inspecting;
  picking = on ? "inspect" : "";
  status.inspecting = on;
  setInspecting(on);
  if (on) $("inspect-result").hidden = true;
  invoke("inspect", { on });
});

function startPick() {
  const on = !status.inspecting;
  picking = on ? "part" : "";
  status.inspecting = on;
  setInspecting(on);
  invoke("pick_area", { on });
  if (on) toast(t("tools.part.waiting"));
}
$("pick-part").addEventListener("click", startPick);

function showReport(r) {
  const name = r.exe ? nameOf(r.exe) : "?";
  const game = status.paused_by ? nameOf(status.paused_by) : "";
  const actions = {
    not_listed: ["why.add", () => invoke("describe_app", { path: r.path }).then(addApp)],
    fullscreen: ["why.add", () => invoke("describe_app", { path: r.path }).then(addApp)],
    left_alone: ["why.add", () => invoke("describe_app", { path: r.path }).then(addApp)],
    excluded: ["why.include", () => { config.exclude = config.exclude.filter((x) => key(x.exe) !== key(r.exe)); save(); renderExclude(); }],
    app_off: ["why.turn_on", () => { const a = config.apps.find((x) => key(x.exe) === key(r.exe)); if (a) { a.enabled = true; save(); renderApps(); } }],
    off: ["why.turn_on", () => { config.enabled = true; save(); renderMaster(); renderStates(); }],
    paused: ["pause.resume", () => invoke("pause", { minutes: 0 })],
    game: ["why.game.remove", () => { config.pause_apps = config.pause_apps.filter((x) => key(x.exe) !== key(status.paused_by)); save(); renderPause(); }],
    admin: ["why.run_admin", () => $("admin").click()],
  };
  const act = actions[r.reason];
  const native = presetOf(r.exe)?.native;
  const box = $("inspect-result");
  box.innerHTML = `
    <div class="result-text">
      <strong>${esc(t(`why.${r.reason}`, { app: name, game, key: config.keys?.window || "" }))}</strong>
      <small>${esc(r.title || r.exe || "")}</small>
      ${native ? `<small class="native">${esc(t("app.native", { app: name }))} <b>${esc(native)}</b></small>` : ""}
    </div>
    ${act ? `<button class="primary" id="why-act">${esc(t(act[0]))}</button>` : ""}`;
  box.hidden = false;
  if (act) $("why-act").onclick = () => { act[1](); box.hidden = true; };
}

// Hotkeys: shown as they are set, changed by pressing the new ones.
const KEY_ROWS = [
  ["all", "tools.all", "tools.all.hint"],
  ["front", "tools.front", "tools.front.hint"],
  ["peek", "tools.peek", "tools.peek.hint"],
  ["window", "tools.window", "tools.window.hint"],
];
let recording = "";

function renderKeys() {
  const keys = config.keys || {};
  $("keys").innerHTML = KEY_ROWS.map(([k, title, hint]) => {
    const taken = (status.keys_taken || []).includes(k);
    const rec = recording === k;
    const shown = rec ? t("keys.press") : keys[k] || t("keys.none");
    return `
      <div class="row" data-key="${k}">
        <span>
          <strong>${esc(t(title))}</strong>
          <small>${esc(taken ? t("keys.taken") : t(hint))}</small>
        </span>
        <span class="actions">
          <kbd class="${taken ? "taken" : ""} ${rec ? "rec" : ""}">${esc(shown)}</kbd>
          <button class="quiet" data-act="change">${esc(rec ? t("tools.cancel") : t("keys.change"))}</button>
        </span>
      </div>`;
  }).join("");
}

$("keys").addEventListener("click", (e) => {
  if (e.target.dataset.act !== "change") return;
  const k = e.target.closest("[data-key]").dataset.key;
  recording = recording === k ? "" : k;
  renderKeys();
});

// The pressed combination as Deonyx writes it, like "Ctrl+Alt+D".
function comboOf(e) {
  let k = "";
  if (/^Key[A-Z]$/.test(e.code)) k = e.key.length === 1 && /[a-z]/i.test(e.key) ? e.key.toUpperCase() : e.code.slice(3);
  else if (/^Digit\d$/.test(e.code)) k = e.code.slice(5);
  else if (/^F\d{1,2}$/.test(e.code)) k = e.code;
  else if (e.code === "Space") k = "Space";
  else if (["Pause", "Insert", "Delete", "Home", "End", "PageUp", "PageDown"].includes(e.code)) k = e.code;
  else if (e.code.startsWith("Arrow")) k = e.code.slice(5);
  if (!k) return null;
  const mods = [e.ctrlKey && "Ctrl", e.altKey && "Alt", e.shiftKey && "Shift", e.metaKey && "Win"].filter(Boolean);
  if (!mods.some((m) => m !== "Shift") && !/^F\d/.test(k)) return null;
  return [...mods, k].join("+");
}

document.addEventListener("keydown", (e) => {
  if (!recording) return;
  e.preventDefault();
  const forApp = recording.startsWith("app:");
  const done = () => {
    recording = "";
    renderKeys();
    if (forApp) renderApps();
  };
  if (e.key === "Escape") return done();
  if (e.key === "Backspace" || e.key === "Delete") {
    setHotkey(recording, "");
  } else {
    const c = comboOf(e);
    if (!c) return;
    const other = hotkeys().find(([k, v]) => k !== recording && v === c);
    if (other) return toast(t("keys.used"));
    setHotkey(recording, c);
  }
  save();
  done();
}, true);

// Every hotkey in use: Deonyx's own and the apps'.
function hotkeys() {
  return [...Object.entries(config.keys || {}), ...config.apps.map((a) => [appKeyId(a), a.hotkey || ""])];
}

function setHotkey(id, combo) {
  if (id.startsWith("app:")) {
    const app = config.apps.find((a) => appKeyId(a) === id);
    if (app) app.hotkey = combo;
  } else {
    config.keys[id] = combo;
  }
}

$("copy-cli").addEventListener("click", () => {
  navigator.clipboard.writeText("deonyx --toggle").then(() => toast(t("tools.copied")));
});

$("copy-diag").addEventListener("click", async () => {
  const text = await invoke("diagnostics");
  await navigator.clipboard.writeText(text);
  toast(t("tools.diag.copied"));
});

// ---------- general

// From the Microsoft Store: the Store updates Deonyx, and there's no
// administrator mode.
let packaged = false;

function renderGeneral() {
  $("admin").closest(".row").hidden = packaged;
  $("updates").closest(".row").hidden = packaged;
  $("autostart").checked = !!config.autostart;
  $("admin").checked = !!config.admin;
  $("language").value = config.language || "auto";
  $("updates").checked = config.updates !== false;
}

$("autostart").addEventListener("change", (e) => {
  const on = e.target.checked;
  invoke("set_autostart", { on }).then(() => { config.autostart = on; }).catch((err) => {
    e.target.checked = !on;
    if (packaged) toast(String(err));
  });
});

$("admin").addEventListener("change", async (e) => {
  const sw = e.target;
  const on = sw.checked;
  sw.disabled = true;
  $("admin-note").textContent = t("general.admin.waiting");
  try {
    await invoke("set_admin", { on });
    config.admin = on;
    $("admin-note").textContent = on === elevated ? t("general.admin.hint") : t("general.admin.restarting");
  } catch (err) {
    sw.checked = !on;
    $("admin-note").textContent = String(err);
  }
  sw.disabled = false;
});

$("language").addEventListener("change", (e) => {
  config.language = e.target.value;
  save();
  render();
});

$("updates").addEventListener("change", (e) => {
  config.updates = e.target.checked;
  save();
});

$("check-update").addEventListener("click", async () => {
  $("update-status").textContent = t("general.updates.checking");
  try {
    update = await invoke("check_update");
    $("update-status").textContent = update ? t("update.ready", { v: update }) : t("general.updates.latest");
  } catch {
    $("update-status").textContent = t("general.updates.failed");
  }
  renderUpdate();
});

function renderUpdate() {
  $("update-bar").hidden = !update;
  if (update) $("update-text").textContent = t("update.ready", { v: update });
}

$("update-now").addEventListener("click", () => {
  $("update-now").disabled = true;
  $("update-now").textContent = t("update.installing");
  invoke("install_update").catch(() => {
    $("update-now").disabled = false;
    $("update-now").textContent = t("update.install");
    toast(t("general.updates.failed"));
  });
});

$("export").addEventListener("click", async () => {
  if (await invoke("export_config").catch(() => false)) toast(t("general.exported"));
});

$("import").addEventListener("click", async () => {
  try {
    const c = await invoke("import_config");
    if (c) {
      config = c;
      render();
      toast(t("general.imported"));
    }
  } catch {
    toast(t("general.import.failed"));
  }
});

// ---------- first start and updates

const WELCOME = ["welcome.1", "welcome.2", "welcome.3"];
let step = 0;

function renderWelcome() {
  const k = WELCOME[step];
  const keys = config.keys || {};
  let body = `<h2>${esc(t(`${k}.title`))}</h2><p class="hint">${esc(t(`${k}.text`))}</p>`;
  if (step === 0) {
    body += `<div class="seg" id="w-mode">
      <button data-mode="list" class="${config.everything ? "" : "on"}">${esc(t("mode.list"))}</button>
      <button data-mode="everything" class="${config.everything ? "on" : ""}">${esc(t("mode.everything"))}</button></div>`;
  } else if (step === 1) {
    body += `<div class="seg" id="w-style">${["gray", "black", "soft", "paper"].map((s) =>
      `<button data-style="${s}" class="${(config.style || "gray") === s ? "on" : ""}">${esc(t(`look.${s}`))}</button>`).join("")}</div>`;
  } else {
    body += `<div class="keys-list">
      <span><kbd>${esc(keys.all)}</kbd> ${esc(t("tools.all"))}</span>
      <span><kbd>${esc(keys.front)}</kbd> ${esc(t("tools.front"))}</span>
      <span><kbd>${esc(keys.peek)}</kbd> ${esc(t("tools.peek"))}</span>
      <span><kbd>${esc(keys.window)}</kbd> ${esc(t("tools.window"))}</span></div>`;
  }
  $("welcome-steps").innerHTML = body;
  $("welcome-dots").innerHTML = WELCOME.map((_, i) => `<i class="${i === step ? "on" : ""}"></i>`).join("");
  $("welcome-next").textContent = step === WELCOME.length - 1 ? t("welcome.done") : t("welcome.next");
}

$("welcome-steps").addEventListener("click", (e) => {
  if (e.target.dataset.mode) {
    config.everything = e.target.dataset.mode === "everything";
    save();
    renderMode();
    renderApps();
  }
  if (e.target.dataset.style) {
    config.style = e.target.dataset.style;
    save();
    renderLook();
  }
  renderWelcome();
});

function closeWelcome() {
  $("welcome").hidden = true;
  config.welcomed = true;
  config.seen_version = version;
  save();
  // In "only the apps I choose" mode, the next thing to do is add one.
  if (!config.everything && !config.apps.length) {
    showPage("apps");
    openPicker("apps");
  }
}

$("welcome-next").addEventListener("click", () => {
  if (step < WELCOME.length - 1) {
    step++;
    renderWelcome();
  } else {
    closeWelcome();
  }
});
$("welcome-skip").addEventListener("click", closeWelcome);

function showNews() {
  const notes = NEWS[version];
  if (!notes) return;
  $("news-title").textContent = t("news.title", { v: version });
  $("news-list").innerHTML = (notes[LANG] || notes.en).map((n) => `<li>${esc(n)}</li>`).join("");
  $("news").hidden = false;
}

$("news-ok").addEventListener("click", () => {
  $("news").hidden = true;
  config.seen_version = version;
  save();
});

function firstLook() {
  const fresh = !config.welcomed && !config.apps.length && !config.everything;
  if (fresh) {
    step = 0;
    renderWelcome();
    $("welcome").hidden = false;
  } else if (config.seen_version !== version) {
    if (NEWS[version]) showNews();
    else {
      config.seen_version = version;
      save();
    }
  }
}

// ---------- everything

function render() {
  chooseLang();
  applyI18n();
  renderMaster();
  renderMode();
  renderApps();
  renderExclude();
  renderLook();
  renderWhen();
  renderPause();
  renderProfiles();
  renderKeys();
  renderGeneral();
  renderUpdate();
  setInspecting(!!status.inspecting);
  $("footer").textContent = t("footer", { v: version }) + (elevated ? t("footer.admin") : "");
  if (!$("welcome").hidden) renderWelcome();
}

$("master").addEventListener("change", (e) => {
  config.enabled = e.target.checked;
  save();
  renderStates();
});

async function init() {
  const s = await invoke("get_state");
  config = s.config;
  status = s.status;
  version = s.version;
  elevated = s.elevated;
  update = s.update;
  systemLang = s.system_lang;
  presets = s.presets || [];
  crash = s.crash || null;
  profileChanged = !!s.profile_changed;
  packaged = !!s.packaged;
  if (packaged) config.autostart = await invoke("get_autostart").catch(() => config.autostart);
  screens = await invoke("list_screens").catch(() => []);
  render();
  showPage(page);
  firstLook();
  await listen("status", (e) => {
    const wasInspecting = status.inspecting;
    status = e.payload;
    config.enabled = status.enabled;
    renderStates();
    renderPause();
    if (wasInspecting && !status.inspecting) {
      picking = "";
      setInspecting(false);
    }
  });
  // Changes made outside this window: hotkeys, a drawn area, the tray.
  await listen("config", (e) => {
    config = e.payload;
    render();
    checkProfile();
  });
  await listen("inspected", (e) => {
    showPage("tools");
    status.inspecting = false;
    picking = "";
    setInspecting(false);
    showReport(e.payload);
  });
  await listen("area", (e) => {
    const { exe, ok, reason } = e.payload;
    if (ok) {
      open.add(key(exe));
      renderApps();
      toast(t("draw.saved"));
    } else if (reason === "not_open") {
      toast(t("draw.not_open", { app: nameOf(exe) }));
    } else if (reason === "no_part") {
      toast(t("tools.part.none"));
    }
  });
  await listen("update", (e) => {
    update = e.payload;
    renderUpdate();
  });
  // The engine failed and started again.
  await listen("crash", (e) => {
    crash = e.payload || null;
    renderMaster();
  });
  await listen("presets", (e) => {
    presets = e.payload;
    renderApps();
  });
  // Screens come and go (a laptop docked, a projector plugged in).
  window.addEventListener("focus", async () => {
    screens = await invoke("list_screens").catch(() => screens);
    renderScreens();
  });
}

init();
