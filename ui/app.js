// Settings window. Talks to the Rust side through Tauri commands; every
// change is saved and applied right away.

const { invoke } = window.__TAURI__.core;
const { listen } = window.__TAURI__.event;

let config = { enabled: true, apps: [] };
let status = { enabled: true, running: {} };
let version = "";
let elevated = false;
const open = new Set();

const $ = (id) => document.getElementById(id);
const key = (exe) => exe.toLowerCase();
const esc = (s) => String(s ?? "").replace(/[&<>"']/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" })[c]);

let saveTimer = 0;
function save() {
  clearTimeout(saveTimer);
  saveTimer = setTimeout(() => invoke("set_config", { config }).catch((e) => console.error(e)), 120);
}

function iconHtml(app) {
  if (app.icon) return `<div class="icon"><img src="${app.icon}" alt=""></div>`;
  return `<div class="icon"><span>${esc((app.name || app.exe || "?").charAt(0).toUpperCase())}</span></div>`;
}

function stateText(app) {
  if (!config.enabled) return { live: false, text: "Paused" };
  if (!app.enabled) return { live: false, text: "Off" };
  const n = status.running?.[key(app.exe)] ?? 0;
  const sys = status.system?.[key(app.exe)] ?? 0;
  const service = "belong to a Windows service, which Windows lets no app draw over";
  if (n > 0) {
    const text = n === 1 ? "Darkening 1 window" : `Darkening ${n} windows`;
    return { live: true, text: sys > 0 ? `${text}. Others ${service}.` : text };
  }
  if (sys > 0) return { live: false, text: `Its open windows ${service}.` };
  if ((status.admin?.[key(app.exe)] ?? 0) > 0) return { live: false, text: "Runs as administrator. Turn on Run as administrator below." };
  return { live: false, text: "Waiting for it to open" };
}

function renderMaster() {
  $("master").checked = config.enabled;
  $("master-label").textContent = config.enabled ? "On" : "Off";
}

function renderApps() {
  const list = $("apps");
  if (!config.apps.length) {
    list.innerHTML = `<div class="empty"><strong>No apps yet</strong>Add a program and Deonyx will darken it whenever it's open.</div>`;
    return;
  }
  list.innerHTML = config.apps.map((app, i) => {
    const s = stateText(app);
    const isOpen = open.has(key(app.exe));
    return `
    <div class="app ${isOpen ? "open" : ""}" data-i="${i}">
      <div class="app-head" data-act="expand">
        ${iconHtml(app)}
        <div class="app-name">
          <strong>${esc(app.name || app.exe)}</strong>
          <small class="state"><span class="dot ${s.live ? "live" : ""}"></span><span data-state>${s.text}</span></small>
        </div>
        <span class="chev" aria-hidden="true">&#9654;</span>
        <input type="checkbox" class="switch" data-act="enabled" ${app.enabled ? "checked" : ""} aria-label="Darken ${esc(app.name)}">
      </div>
      ${isOpen ? `
      <div class="app-body">
        <label class="slider"><span>Background darkness</span>
          <input type="range" min="0" max="100" step="1" value="${Math.round(app.darkness * 100)}" data-act="darkness">
          <span data-val="darkness">${Math.round(app.darkness * 100)}%</span></label>
        <label class="slider"><span>Text brightness</span>
          <input type="range" min="0" max="100" step="1" value="${Math.round(app.text * 100)}" data-act="text">
          <span data-val="text">${Math.round(app.text * 100)}%</span></label>
        <label class="check">
          <input type="checkbox" class="switch" data-act="protect" ${app.protect ? "checked" : ""}>
          <span><strong>Keep 3D views and pictures in real colors</strong>
          <small>Found automatically, so models, textures and photos look right.</small></span>
        </label>
        <div class="app-foot">
          <small>${esc(app.exe)}</small>
          <button class="danger" data-act="remove">Remove</button>
        </div>
      </div>` : ""}
    </div>`;
  }).join("");
}

function renderStates() {
  document.querySelectorAll(".app").forEach((el) => {
    const app = config.apps[+el.dataset.i];
    if (!app) return;
    const s = stateText(app);
    el.querySelector("[data-state]").textContent = s.text;
    el.querySelector(".dot").classList.toggle("live", s.live);
  });
}

function render() {
  renderMaster();
  renderApps();
  $("footer").textContent = `Deonyx ${version}${elevated ? " · running as administrator" : ""} · free software under the GPL-3.0 · made by fdeox`;
  $("admin").checked = !!config.admin;
}

// App list interactions, delegated.
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
  } else if (act === "enabled" || act === "protect") {
    app[act] = e.target.checked;
    save();
    renderStates();
  }
});

$("master").addEventListener("change", (e) => {
  config.enabled = e.target.checked;
  renderMaster();
  renderStates();
  save();
});

$("autostart").addEventListener("change", (e) => {
  const on = e.target.checked;
  invoke("set_autostart", { on }).then(() => { config.autostart = on; }).catch(() => { e.target.checked = !on; });
});

const ADMIN_NOTE = $("admin-note").textContent;
$("admin").addEventListener("change", async (e) => {
  const sw = e.target;
  const on = sw.checked;
  sw.disabled = true;
  $("admin-note").textContent = "Waiting for Windows…";
  try {
    await invoke("set_admin", { on });
    config.admin = on;
    $("admin-note").textContent = on === elevated ? ADMIN_NOTE : "Restarting Deonyx…";
  } catch (err) {
    sw.checked = !on;
    $("admin-note").textContent = String(err);
  }
  sw.disabled = false;
});

// "Add an app" panel.
function addApp(r) {
  const k = key(r.exe);
  if (!config.apps.some((a) => key(a.exe) === k)) {
    config.apps.push({ exe: r.exe, name: r.name, enabled: true, darkness: 0.6, text: 0.7, protect: true, icon: r.icon || undefined });
    save();
  }
  open.add(k);
  closePicker();
  renderApps();
}

function closePicker() {
  $("picker").hidden = true;
  $("picker").innerHTML = "";
}

const fold = (t) => String(t ?? "").toLocaleLowerCase("tr").normalize("NFD").replace(/[\u0300-\u036f]/g, "").replace(/ı/g, "i");

function pickRow(r, i, group, added) {
  return `
    <div class="pick" data-search="${esc(fold(`${r.name} ${r.exe}`))}" data-row="${group}:${i}">
      ${iconHtml(r)}
      <div class="app-name"><strong>${esc(r.name)}</strong><small>${esc(r.title || r.exe)}</small></div>
      ${added.has(key(r.exe)) ? `<small>Added</small>` : `<button data-pick="${group}:${i}">Add</button>`}
    </div>`;
}

// Icons of installed programs arrive in small batches, so the list is usable
// at once even when Windows is slow to open the files.
async function fillIcons(list) {
  for (let start = 0; start < list.length; start += 16) {
    if ($("picker").hidden) return;
    const batch = list.slice(start, start + 16);
    const icons = await invoke("app_icons", { paths: batch.map((r) => r.title) }).catch(() => []);
    icons.forEach((icon, j) => {
      const i = start + j;
      list[i].icon = icon || undefined;
      const box = document.querySelector(`[data-row="installed:${i}"] .icon`);
      if (icon && box) box.innerHTML = `<img src="${icon}" alt="">`;
    });
  }
}

async function openPicker() {
  const p = $("picker");
  p.hidden = false;
  p.innerHTML = `
    <div class="picker-head">
      <input type="search" id="pick-search" placeholder="Search programs" autocomplete="off" spellcheck="false">
      <button class="quiet" data-pick="close">Close</button>
    </div>
    <div class="picker-list">
      <div class="group" data-group="open">Open now</div><div id="pick-open"><small class="pad">Looking for open programs…</small></div>
      <div class="group" data-group="installed">Installed</div><div id="pick-installed"><small class="pad">Looking for installed programs…</small></div>
      <small class="pad" id="pick-none" hidden>Nothing matches. Try Browse below.</small>
    </div>
    <div class="picker-foot"><small>Not listed?</small><button class="quiet" data-pick="browse">Browse for a program…</button></div>`;
  const search = $("pick-search");
  search.focus();
  const lists = { open: [], installed: [] };
  const added = new Set(config.apps.map((a) => key(a.exe)));

  const filter = () => {
    const q = fold(search.value.trim());
    let any = false;
    for (const g of ["open", "installed"]) {
      let shown = 0;
      document.querySelectorAll(`#pick-${g} .pick`).forEach((row) => {
        const hit = !q || row.dataset.search.includes(q);
        row.hidden = !hit;
        if (hit) shown++;
      });
      const loading = !!document.querySelector(`#pick-${g} .pad`);
      p.querySelector(`[data-group="${g}"]`).hidden = !shown && !loading;
      any ||= shown > 0 || loading;
    }
    $("pick-none").hidden = any;
  };
  search.addEventListener("input", filter);
  search.addEventListener("keydown", (e) => {
    if (e.key === "Enter") p.querySelector(".pick:not([hidden]) button")?.click();
    if (e.key === "Escape") closePicker();
  });

  const openReady = invoke("running_apps").then((list) => {
    lists.open = list;
    $("pick-open").innerHTML = list.map((r, i) => pickRow(r, i, "open", added)).join("");
    filter();
    return list;
  });
  // Wait for the open ones: a program shows up under one heading only.
  Promise.all([openReady, invoke("installed_apps")]).then(([open, list]) => {
    const running = new Set(open.map((r) => key(r.exe)));
    lists.installed = list.filter((r) => !running.has(key(r.exe)));
    $("pick-installed").innerHTML = lists.installed.map((r, i) => pickRow(r, i, "installed", added)).join("");
    filter();
    fillIcons(lists.installed);
  });

  p.onclick = async (e) => {
    const what = e.target.dataset.pick;
    if (what === undefined) return;
    if (what === "close") return closePicker();
    if (what === "browse") {
      const r = await invoke("pick_exe");
      if (r) addApp(r);
      return;
    }
    const [g, i] = what.split(":");
    addApp(lists[g][+i]);
  };
}

$("add").addEventListener("click", () => ($("picker").hidden ? openPicker() : closePicker()));

async function init() {
  const s = await invoke("get_state");
  config = s.config;
  status = s.status;
  version = s.version;
  elevated = s.elevated;
  render();
  $("autostart").checked = await invoke("get_autostart");
  await listen("status", (e) => {
    status = e.payload;
    config.enabled = status.enabled;
    renderMaster();
    renderStates();
  });
}

init();
