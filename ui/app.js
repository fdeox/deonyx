// Settings window. Talks to the Rust side through Tauri commands; every
// change is saved and applied right away.

const { invoke } = window.__TAURI__.core;
const { listen } = window.__TAURI__.event;

let config = { enabled: true, apps: [], exclude: [], everything: false, style: "gray", warm: 0, schedule: "always", from: "20:00", to: "07:00", language: "auto", updates: true };
let status = { enabled: true, waiting: false, running: {}, admin: {}, system: {}, inspecting: false };
let version = "";
let elevated = false;
let update = null;
const open = new Set();

const $ = (id) => document.getElementById(id);
const key = (exe) => String(exe || "").toLowerCase();
const esc = (s) => String(s ?? "").replace(/[&<>"']/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" })[c]);
const stem = (exe) => String(exe || "").replace(/\.exe$/i, "");

let saveTimer = 0;
function save() {
  clearTimeout(saveTimer);
  saveTimer = setTimeout(() => invoke("set_config", { config }).catch((e) => console.error(e)), 120);
}

let toastTimer = 0;
function toast(msg) {
  const el = $("toast");
  el.textContent = msg;
  el.hidden = false;
  clearTimeout(toastTimer);
  toastTimer = setTimeout(() => (el.hidden = true), 3200);
}

function chooseLang() {
  const sys = (navigator.language || "en").toLowerCase().startsWith("tr") ? "tr" : "en";
  LANG = config.language === "tr" || config.language === "en" ? config.language : sys;
}

function iconHtml(app) {
  if (app.icon) return `<div class="icon"><img src="${app.icon}" alt=""></div>`;
  return `<div class="icon"><span>${esc((app.name || app.exe || "?").charAt(0).toUpperCase())}</span></div>`;
}

function nameOf(exe) {
  const k = key(exe);
  return config.apps.find((a) => key(a.exe) === k)?.name || config.exclude.find((a) => key(a.exe) === k)?.name || stem(exe);
}

// ---------- status lines

function stateText(app) {
  if (!config.enabled) return { live: false, text: t("state.paused") };
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
    if (status.waiting) sub = t("master.waiting");
    else if (total === 1) sub = t("master.count.one");
    else if (total > 1) sub = t("master.count.many", { n: total });
    else sub = t("master.none");
  }
  $("master-sub").textContent = sub;
}

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
        <div class="check areas">
          <span><strong>${esc(t("app.areas"))}</strong>
          <small>${esc(areas ? t("app.areas.some", { n: areas }) : t("app.areas.none"))}</small></span>
          <span class="actions">
            ${areas ? `<button class="quiet" data-act="clear">${esc(t("app.clear"))}</button>` : ""}
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
  } else if (act === "clear") {
    app.areas = [];
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

function addApp(r) {
  const k = key(r.exe);
  if (!config.apps.some((a) => key(a.exe) === k)) {
    config.apps.push({ exe: r.exe, name: r.name, enabled: true, darkness: 0.6, text: 0.7, protect: true, areas: [], icon: r.icon || undefined });
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

// ---------- program picker, for both lists

const PICKERS = { apps: "picker", exclude: "exclude-picker" };

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
  const onPick = which === "apps" ? addApp : addExclude;
  const added = new Set((which === "apps" ? config.apps : config.exclude).map((a) => key(a.exe)));
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

// ---------- look

function renderLook() {
  document.querySelectorAll("#style button").forEach((b) => b.classList.toggle("on", b.dataset.style === (config.style || "gray")));
  const w = Math.round((config.warm || 0) * 100);
  $("warm").value = w;
  $("warm-val").textContent = w ? `${w}%` : t("off");
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
});

// ---------- when

function renderWhen() {
  $("schedule").value = config.schedule || "always";
  $("times-row").hidden = config.schedule !== "times";
  $("from").value = config.from || "20:00";
  $("to").value = config.to || "07:00";
  $("when-hint").textContent = t(`when.hint.${config.schedule || "always"}`);
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

// ---------- why isn't a window dark?

function setInspecting(on) {
  $("inspect").textContent = on ? t("tools.cancel") : t("tools.check");
  $("inspect-hint").textContent = on ? t("tools.waiting") : t("tools.inspect.hint");
  $("inspect").classList.toggle("primary", on);
}

$("inspect").addEventListener("click", () => {
  const on = !status.inspecting;
  status.inspecting = on;
  setInspecting(on);
  if (on) $("inspect-result").hidden = true;
  invoke("inspect", { on });
});

function showReport(r) {
  const name = r.exe ? nameOf(r.exe) : "?";
  const actions = {
    not_listed: ["why.add", () => invoke("describe_app", { path: r.path }).then(addApp)],
    fullscreen: ["why.add", () => invoke("describe_app", { path: r.path }).then(addApp)],
    excluded: ["why.include", () => { config.exclude = config.exclude.filter((x) => key(x.exe) !== key(r.exe)); save(); renderExclude(); }],
    app_off: ["why.turn_on", () => { const a = config.apps.find((x) => key(x.exe) === key(r.exe)); if (a) { a.enabled = true; save(); renderApps(); } }],
    off: ["why.turn_on", () => { config.enabled = true; save(); renderMaster(); renderStates(); }],
    admin: ["why.run_admin", () => $("admin").click()],
  };
  const act = actions[r.reason];
  const box = $("inspect-result");
  box.innerHTML = `
    <div class="result-text">
      <strong>${esc(t(`why.${r.reason}`, { app: name }))}</strong>
      <small>${esc(r.title || r.exe || "")}</small>
    </div>
    ${act ? `<button class="primary" id="why-act">${esc(t(act[0]))}</button>` : ""}`;
  box.hidden = false;
  if (act) $("why-act").onclick = () => { act[1](); box.hidden = true; };
}

// ---------- general

function renderGeneral() {
  $("autostart").checked = !!config.autostart;
  $("admin").checked = !!config.admin;
  $("language").value = config.language || "auto";
  $("updates").checked = config.updates !== false;
}

$("autostart").addEventListener("change", (e) => {
  const on = e.target.checked;
  invoke("set_autostart", { on }).then(() => { config.autostart = on; }).catch(() => { e.target.checked = !on; });
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
  renderGeneral();
  renderUpdate();
  setInspecting(!!status.inspecting);
  $("footer").textContent = t("footer", { v: version }) + (elevated ? t("footer.admin") : "");
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
  render();
  await listen("status", (e) => {
    const wasInspecting = status.inspecting;
    status = e.payload;
    config.enabled = status.enabled;
    renderStates();
    if (wasInspecting && !status.inspecting) setInspecting(false);
  });
  // Changes made outside this window: Ctrl+Alt+F, a drawn area, the tray.
  await listen("config", (e) => {
    config = e.payload;
    render();
  });
  await listen("inspected", (e) => {
    status.inspecting = false;
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
    }
  });
  await listen("update", (e) => {
    update = e.payload;
    renderUpdate();
  });
}

init();
