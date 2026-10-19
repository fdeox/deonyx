// Deonyx: dark mode for any app.
//
// Every visible window of the chosen programs (main windows, menus, lists,
// dialogs, tooltips) is captured with Windows.Graphics.Capture, its lightness
// is flipped on the GPU, and the result is shown in a click-through window
// right above it. The program itself is never touched.
//
// This file is the tray app and the settings window; the darkening itself
// runs on its own thread (engine.rs).

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod apps;
mod config;
mod engine;
mod gpu;
mod layer;
mod protect;
mod win;

use std::sync::Mutex;

use serde::Serialize;
use tauri::image::Image;
use tauri::menu::{CheckMenuItem, Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Emitter, Manager, RunEvent, Theme, WebviewUrl, WebviewWindowBuilder, Wry};
use tauri_plugin_autostart::ManagerExt as _;
use tauri_plugin_dialog::DialogExt;

use config::{AppRule, Config};
use engine::{Cmd, Engine, Status};

struct State {
    cfg: Mutex<Config>,
    status: Mutex<Status>,
    engine: Engine,
    toggle: Mutex<Option<CheckMenuItem<Wry>>>,
}

#[derive(Serialize)]
struct Snapshot {
    config: Config,
    status: Status,
    version: &'static str,
}

#[tauri::command]
fn get_state(s: tauri::State<State>) -> Snapshot {
    Snapshot {
        config: s.cfg.lock().unwrap().clone(),
        status: s.status.lock().unwrap().clone(),
        version: env!("CARGO_PKG_VERSION"),
    }
}

#[tauri::command]
fn set_config(app: AppHandle, config: Config) -> Result<(), String> {
    apply(&app, config)
}

#[tauri::command(async)]
fn running_apps() -> Vec<apps::Running> {
    apps::running()
}

#[tauri::command(async)]
fn pick_exe(app: AppHandle) -> Option<apps::Running> {
    let picked = app.dialog().file().set_title("Choose a program").add_filter("Programs", &["exe"]).blocking_pick_file()?;
    let path = picked.into_path().ok()?;
    Some(apps::describe(&path.to_string_lossy()))
}

#[tauri::command]
fn get_autostart(app: AppHandle) -> bool {
    app.autolaunch().is_enabled().unwrap_or(false)
}

#[tauri::command]
fn set_autostart(app: AppHandle, on: bool) -> Result<(), String> {
    let a = app.autolaunch();
    if on { a.enable() } else { a.disable() }.map_err(|e| e.to_string())
}

/// Saves the settings and hands them to the engine.
fn apply(app: &AppHandle, config: Config) -> Result<(), String> {
    let s = app.state::<State>();
    config::save(&config).map_err(|e| e.to_string())?;
    if let Some(t) = s.toggle.lock().unwrap().as_ref() {
        let _ = t.set_checked(config.enabled);
    }
    *s.cfg.lock().unwrap() = config.clone();
    s.engine.send(Cmd::Config(config));
    Ok(())
}

fn set_enabled(app: &AppHandle, on: bool) {
    let mut c = app.state::<State>().cfg.lock().unwrap().clone();
    c.enabled = on;
    let _ = apply(app, c);
}

/// Called on the engine thread whenever something worth showing changes.
fn on_status(app: &AppHandle, st: Status) {
    if let Some(s) = app.try_state::<State>() {
        *s.status.lock().unwrap() = st.clone();
        let mut c = s.cfg.lock().unwrap();
        // The hotkey switches the engine directly; remember it.
        if c.enabled != st.enabled {
            c.enabled = st.enabled;
            let _ = config::save(&c);
            if let Some(t) = s.toggle.lock().unwrap().as_ref() {
                let _ = t.set_checked(st.enabled);
            }
        }
    }
    if let Some(tray) = app.tray_by_id("main") {
        let _ = tray.set_tooltip(Some(if st.enabled { "Deonyx: on" } else { "Deonyx: off" }));
    }
    let _ = app.emit("status", &st);
}

fn open_settings(app: &AppHandle) {
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.unminimize();
        let _ = w.show();
        let _ = w.set_focus();
        return;
    }
    let built = WebviewWindowBuilder::new(app, "main", WebviewUrl::App("index.html".into()))
        .title("Deonyx")
        .inner_size(820.0, 640.0)
        .min_inner_size(640.0, 520.0)
        .theme(Some(Theme::Dark))
        .background_color(tauri::window::Color(18, 18, 22, 255))
        .center()
        .build();
    if let Err(e) = built {
        eprintln!("ayarlar penceresi acilamadi: {e}");
    }
}

fn build_tray(app: &AppHandle, enabled: bool) -> tauri::Result<CheckMenuItem<Wry>> {
    let toggle = CheckMenuItem::with_id(app, "toggle", "Darken apps", true, enabled, Some("Ctrl+Alt+D"))?;
    let settings = MenuItem::with_id(app, "settings", "Settings", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "Quit Deonyx", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&toggle, &PredefinedMenuItem::separator(app)?, &settings, &quit])?;
    let item = toggle.clone();
    TrayIconBuilder::with_id("main")
        .icon(Image::from_bytes(include_bytes!("../icons/tray.png"))?)
        .tooltip(if enabled { "Deonyx: on" } else { "Deonyx: off" })
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(move |app, e| match e.id.as_ref() {
            "toggle" => set_enabled(app, item.is_checked().unwrap_or(true)),
            "settings" => open_settings(app),
            "quit" => {
                app.state::<State>().engine.send(Cmd::Quit);
                app.exit(0);
            }
            _ => {}
        })
        .on_tray_icon_event(|tray, e| {
            if let TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, .. } = e {
                open_settings(tray.app_handle());
            }
        })
        .build(app)?;
    Ok(toggle)
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let hidden = args.iter().any(|a| a == "--hidden");
    let mut cfg = config::load();
    // For testing: --process X darkens X for this run without saving it.
    let mut it = args.iter().skip(1);
    while let Some(a) = it.next() {
        if a == "--process" {
            if let Some(exe) = it.next() {
                if !cfg.apps.iter().any(|r| r.exe.eq_ignore_ascii_case(exe)) {
                    cfg.apps.push(AppRule { exe: exe.clone(), name: exe.clone(), ..Default::default() });
                }
            }
        }
    }

    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| open_settings(app)))
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            Some(vec!["--hidden"]),
        ))
        .plugin(tauri_plugin_dialog::init())
        .setup(move |app| {
            let handle = app.handle().clone();
            let first_run = cfg.apps.is_empty();
            let engine = Engine::start(cfg.clone(), Box::new(move |st| on_status(&handle, st)));
            app.manage(State {
                status: Mutex::new(Status { enabled: cfg.enabled, ..Default::default() }),
                cfg: Mutex::new(cfg.clone()),
                engine,
                toggle: Mutex::new(None),
            });
            let toggle = build_tray(app.handle(), cfg.enabled)?;
            *app.state::<State>().toggle.lock().unwrap() = Some(toggle);
            if !hidden || first_run {
                open_settings(app.handle());
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![get_state, set_config, running_apps, pick_exe, get_autostart, set_autostart])
        .build(tauri::generate_context!())
        .expect("Deonyx failed to start")
        .run(|_app, event| {
            // Closing the settings window keeps Deonyx running in the tray.
            if let RunEvent::ExitRequested { api, code: None, .. } = event {
                api.prevent_exit();
            }
        });
}
