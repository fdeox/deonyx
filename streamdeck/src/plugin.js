// Deonyx for Stream Deck. Each key runs the Deonyx command line (deonyx
// --toggle, --pause, --app, --window, --profile), and the keys show the
// state that `deonyx --status` reports, checked every two seconds while any
// of them is on screen.

import streamDeck, { SingletonAction } from "@elgato/streamdeck";
import { execFile } from "node:child_process";
import { existsSync } from "node:fs";
import { join } from "node:path";

// Where Deonyx is installed: for this user (the installer's default), for
// everyone, or with Scoop. Else it has to be on the PATH.
function findDeonyx() {
  const env = process.env;
  const places = [
    env.LOCALAPPDATA && join(env.LOCALAPPDATA, "Deonyx", "deonyx.exe"),
    env.ProgramFiles && join(env.ProgramFiles, "Deonyx", "deonyx.exe"),
    env.USERPROFILE && join(env.USERPROFILE, "scoop", "apps", "deonyx", "current", "deonyx.exe"),
  ].filter(Boolean);
  return places.find((p) => existsSync(p)) || "deonyx.exe";
}

let exe = findDeonyx();

function deonyx(...args) {
  return new Promise((resolve) => {
    execFile(exe, args, { windowsHide: true, timeout: 5000 }, (err, stdout) => {
      resolve({ ok: !err, out: String(stdout || "").trim() });
    });
  });
}

// What Deonyx said last: { running, on, paused, profile, profiles, apps, ... }.
let state = { running: false };

const key = (exe) => String(exe || "").trim().toLowerCase();

async function check() {
  const r = await deonyx("--status");
  try {
    state = JSON.parse(r.out);
  } catch {
    state = { running: false };
  }
  // Installed or moved since: look again.
  if (!state.running) exe = findDeonyx();
  await Promise.all(ACTIONS.map((a) => a.show()));
}

// Checks run only while keys that show a state are visible.
const visible = new Set();
let timer = 0;

function appeared(id) {
  visible.add(id);
  if (!timer) timer = setInterval(check, 2000);
  check();
}

function disappeared(id) {
  visible.delete(id);
  if (!visible.size && timer) {
    clearInterval(timer);
    timer = 0;
  }
}

// Sends the apps and profiles Deonyx knows to the settings panel, for its
// suggestions.
async function suggest() {
  await streamDeck.ui.sendToPropertyInspector({ apps: Object.keys(state.apps || {}), profiles: state.profiles || [] });
}

class Base extends SingletonAction {
  onWillAppear(ev) {
    appeared(ev.action.id);
  }

  onWillDisappear(ev) {
    disappeared(ev.action.id);
  }

  onPropertyInspectorDidAppear() {
    return suggest();
  }

  // After a press: a tick on the key, or a warning when Deonyx didn't take it.
  async done(ev, r) {
    if (!r.ok) return ev.action.showAlert();
    await check();
    if (!this.showsState) await ev.action.showOk();
  }

  async show() {}
}

class Toggle extends Base {
  manifestId = "com.fdeox.deonyx.toggle";
  showsState = true;

  async onKeyDown(ev) {
    await this.done(ev, await deonyx("--toggle"));
  }

  async show() {
    for (const a of this.actions) if (a.isKey()) await a.setState(state.running && state.on ? 0 : 1);
  }
}

class Pause extends Base {
  manifestId = "com.fdeox.deonyx.pause";
  showsState = true;

  async onKeyDown(ev) {
    const minutes = Math.max(1, Math.round(Number(ev.payload.settings.minutes) || 60));
    await this.done(ev, state.paused ? await deonyx("--resume") : await deonyx("--pause", String(minutes)));
  }

  async show() {
    for (const a of this.actions) if (a.isKey()) await a.setState(state.paused ? 1 : 0);
  }
}

class App extends Base {
  manifestId = "com.fdeox.deonyx.app";
  showsState = true;

  async onKeyDown(ev) {
    const { app, mode } = ev.payload.settings;
    if (!key(app)) return ev.action.showAlert();
    const args = ["--app", String(app).trim()];
    if (mode === "on" || mode === "off") args.push(mode);
    await this.done(ev, await deonyx(...args));
  }

  async show() {
    for (const a of this.actions) {
      if (!a.isKey()) continue;
      const { app } = await a.getSettings();
      await a.setState(state.running && state.on && state.apps?.[key(app)] ? 0 : 1);
    }
  }
}

class Window extends Base {
  manifestId = "com.fdeox.deonyx.window";
  showsState = false;

  // The window under the mouse: Stream Deck keys don't move it.
  async onKeyDown(ev) {
    await this.done(ev, await deonyx("--window"));
  }
}

class Profile extends Base {
  manifestId = "com.fdeox.deonyx.profile";
  showsState = true;

  async onKeyDown(ev) {
    const name = String(ev.payload.settings.profile || "").trim();
    if (!name) return ev.action.showAlert();
    await this.done(ev, await deonyx("--profile", name));
  }

  onDidReceiveSettings() {
    return this.show();
  }

  async show() {
    for (const a of this.actions) {
      if (!a.isKey()) continue;
      const { profile } = await a.getSettings();
      const name = String(profile || "").trim();
      await a.setState(name && key(state.profile) === key(name) ? 1 : 0);
    }
  }
}

const ACTIONS = [new Toggle(), new Pause(), new App(), new Window(), new Profile()];
for (const a of ACTIONS) streamDeck.actions.registerAction(a);
streamDeck.connect();
