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

mod admin;
mod apps;
mod config;
mod engine;
mod gpu;
mod helper;
mod layer;
mod protect;
mod select;
mod win;

use std::sync::Mutex;

use serde::Serialize;
use tauri::image::Image;
use tauri::menu::{CheckMenuItem, Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Emitter, Manager, RunEvent, Theme, WebviewUrl, WebviewWindowBuilder, Wry};
use tauri_plugin_autostart::ManagerExt as _;
use tauri_plugin_dialog::DialogExt;
use tauri_plugin_updater::UpdaterExt;

use config::{AppRule, Config, Excluded};
use engine::{Cmd, Engine, Request, Role, Status};

struct Tray {
    toggle: CheckMenuItem<Wry>,
    settings: MenuItem<Wry>,
    quit: MenuItem<Wry>,
}

struct State {
    cfg: Mutex<Config>,
    status: Mutex<Status>,
    engine: Engine,
    tray: Mutex<Option<Tray>>,
    /// A newer version, once found.
    update: Mutex<Option<String>>,
}

#[derive(Serialize)]
struct Snapshot {
    config: Config,
    status: Status,
    version: &'static str,
    elevated: bool,
    update: Option<String>,
    lang: &'static str,
    /// What "Same as Windows" means here.
    system_lang: &'static str,
}

/// Tray texts, in the settings' language: the switch, Settings, Quit, and
/// the tooltip when on and off.
fn words(lang: &str) -> [&'static str; 5] {
    match lang {
        "tr" => ["Uygulamaları karart", "Ayarlar", "Deonyx'ten çık", "Deonyx: açık", "Deonyx: kapalı"],
        "de" => ["Apps abdunkeln", "Einstellungen", "Deonyx beenden", "Deonyx: an", "Deonyx: aus"],
        "fr" => ["Assombrir les applis", "Paramètres", "Quitter Deonyx", "Deonyx : activé", "Deonyx : désactivé"],
        "es" => ["Oscurecer apps", "Ajustes", "Salir de Deonyx", "Deonyx: activado", "Deonyx: desactivado"],
        "it" => ["Scurisci le app", "Impostazioni", "Esci da Deonyx", "Deonyx: attivo", "Deonyx: disattivato"],
        "pt" => ["Escurecer apps", "Configurações", "Sair do Deonyx", "Deonyx: ligado", "Deonyx: desligado"],
        "ru" => ["Затемнять приложения", "Настройки", "Выйти из Deonyx", "Deonyx: вкл.", "Deonyx: выкл."],
        "uk" => ["Затемнювати застосунки", "Налаштування", "Вийти з Deonyx", "Deonyx: увімк.", "Deonyx: вимк."],
        "nl" => ["Apps donker maken", "Instellingen", "Deonyx afsluiten", "Deonyx: aan", "Deonyx: uit"],
        "ja" => ["アプリを暗くする", "設定", "Deonyx を終了", "Deonyx: オン", "Deonyx: オフ"],
        "ko" => ["앱 어둡게 하기", "설정", "Deonyx 종료", "Deonyx: 켜짐", "Deonyx: 꺼짐"],
        "zh" => ["调暗应用", "设置", "退出 Deonyx", "Deonyx：开", "Deonyx：关"],
        _ => ["Darken apps", "Settings", "Quit Deonyx", "Deonyx: on", "Deonyx: off"],
    }
}

#[tauri::command]
fn get_state(s: tauri::State<State>) -> Snapshot {
    let config = s.cfg.lock().unwrap().clone();
    Snapshot {
        lang: config.lang(),
        config,
        status: s.status.lock().unwrap().clone(),
        version: env!("CARGO_PKG_VERSION"),
        elevated: admin::elevated(),
        update: s.update.lock().unwrap().clone(),
        system_lang: win::system_language(),
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
fn installed_apps() -> Vec<apps::Running> {
    apps::installed()
}

#[tauri::command(async)]
fn app_icons(paths: Vec<String>) -> Vec<Option<String>> {
    apps::icons(&paths)
}

#[tauri::command(async)]
fn describe_app(path: String) -> apps::Running {
    apps::describe(&path)
}

#[tauri::command(async)]
fn pick_exe(app: AppHandle) -> Option<apps::Running> {
    let picked = app.dialog().file().add_filter("Programs", &["exe"]).blocking_pick_file()?;
    let path = picked.into_path().ok()?;
    Some(apps::describe(&path.to_string_lossy()))
}

#[tauri::command]
fn get_autostart(s: tauri::State<State>) -> bool {
    s.cfg.lock().unwrap().autostart
}

#[tauri::command(async)]
fn set_autostart(app: AppHandle, on: bool) -> Result<(), String> {
    let mut c = app.state::<State>().cfg.lock().unwrap().clone();
    c.autostart = on;
    sync_startup(&app, &c)?;
    apply(&app, c)
}

/// Turns running as administrator on or off. Windows asks the user first;
/// Deonyx then restarts with or without administrator rights.
#[tauri::command(async)]
fn set_admin(app: AppHandle, on: bool) -> Result<(), String> {
    let mut c = app.state::<State>().cfg.lock().unwrap().clone();
    if c.admin == on {
        return Ok(());
    }
    let elevated = admin::elevated();
    if on {
        if elevated {
            admin::create_task(c.autostart)?;
        } else {
            admin::run_elevated(if c.autostart { "--admin-task create-logon" } else { "--admin-task create" })?;
        }
    } else if elevated {
        admin::delete_task()?;
    } else {
        admin::run_elevated("--admin-task delete")?;
    }
    c.admin = on;
    let _ = sync_startup(&app, &c);
    apply(&app, c)?;
    if on != elevated {
        // Come back with the settings open, now with (or without) the rights.
        let _ = std::fs::write(config::dir().join("open-settings"), b"");
        if on {
            admin::relaunch_elevated();
        } else {
            admin::relaunch_normal();
        }
        quit_soon(&app);
    }
    Ok(())
}

/// "Why isn't this window dark?": the engine waits for a click.
#[tauri::command]
fn inspect(app: AppHandle, on: bool) {
    let s = app.state::<State>();
    s.engine.send(if on { Cmd::Inspect } else { Cmd::StopInspect });
}

/// Let the user draw an area of an app's window to keep in real colours.
#[tauri::command]
fn draw_area(app: AppHandle, exe: String, hint: String) {
    app.state::<State>().engine.send(Cmd::DrawArea { exe, hint });
}

#[tauri::command]
async fn check_update(app: AppHandle) -> Result<Option<String>, String> {
    let found = app.updater().map_err(|e| e.to_string())?.check().await.map_err(|e| e.to_string())?;
    let version = found.map(|u| u.version);
    *app.state::<State>().update.lock().unwrap() = version.clone();
    Ok(version)
}

/// Downloads the new version and starts its installer, which replaces and
/// restarts Deonyx.
#[tauri::command]
async fn install_update(app: AppHandle) -> Result<(), String> {
    let Some(update) = app.updater().map_err(|e| e.to_string())?.check().await.map_err(|e| e.to_string())? else {
        return Err("no update".into());
    };
    app.state::<State>().engine.send(Cmd::Quit);
    update.download_and_install(|_, _| {}, || {}).await.map_err(|e| e.to_string())?;
    app.restart();
}

fn quit_soon(app: &AppHandle) {
    let a = app.clone();
    std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_millis(400));
        a.state::<State>().engine.send(Cmd::Quit);
        a.exit(0);
    });
}

/// Starting with Windows goes through the Run key normally, and through the
/// task's logon trigger when running as administrator (Windows doesn't start
/// administrator programs from the Run key).
fn sync_startup(app: &AppHandle, c: &Config) -> Result<(), String> {
    let auto = app.autolaunch();
    if c.admin {
        let _ = auto.disable();
        if admin::elevated() {
            admin::create_task(c.autostart)
        } else {
            admin::run_elevated(if c.autostart { "--admin-task create-logon" } else { "--admin-task create" })
        }
    } else if c.autostart {
        auto.enable().map_err(|e| e.to_string())
    } else {
        if auto.is_enabled().unwrap_or(false) {
            auto.disable().map_err(|e| e.to_string())?;
        }
        Ok(())
    }
}

/// Saves the settings and hands them to the engine.
fn apply(app: &AppHandle, config: Config) -> Result<(), String> {
    let s = app.state::<State>();
    config::save(&config).map_err(|e| e.to_string())?;
    if let Some(t) = s.tray.lock().unwrap().as_ref() {
        let w = words(config.lang());
        let _ = t.toggle.set_checked(config.enabled);
        let _ = t.toggle.set_text(w[0]);
        let _ = t.settings.set_text(w[1]);
        let _ = t.quit.set_text(w[2]);
    }
    *s.cfg.lock().unwrap() = config.clone();
    s.engine.send(Cmd::Config(config));
    Ok(())
}

/// Like apply(), and tells the settings window, for changes made elsewhere.
fn apply_and_tell(app: &AppHandle, config: Config) {
    if apply(app, config.clone()).is_ok() {
        let _ = app.emit("config", &config);
    }
}

fn set_enabled(app: &AppHandle, on: bool) {
    let mut c = app.state::<State>().cfg.lock().unwrap().clone();
    c.enabled = on;
    apply_and_tell(app, c);
}

/// Ctrl+Alt+F on an app: on if it was off, off if it was on.
fn toggle_app(app: &AppHandle, exe: String, path: String) {
    let mut c = app.state::<State>().cfg.lock().unwrap().clone();
    let key = exe.to_ascii_lowercase();
    if let Some(r) = c.apps.iter_mut().find(|r| r.key() == key) {
        r.enabled = !r.enabled;
    } else if c.everything {
        if let Some(i) = c.exclude.iter().position(|e| e.exe.eq_ignore_ascii_case(&exe)) {
            c.exclude.remove(i);
        } else {
            let d = apps::describe(&path);
            c.exclude.push(Excluded { exe: d.exe, name: d.name, icon: d.icon });
        }
    } else {
        let d = apps::describe(&path);
        c.apps.push(AppRule { exe: d.exe, name: d.name, icon: d.icon, ..Default::default() });
    }
    apply_and_tell(app, c);
}

fn on_request(app: &AppHandle, r: Request) {
    match r {
        Request::OpenSettings => {
            let a = app.clone();
            let _ = app.run_on_main_thread(move || open_settings(&a));
        }
        Request::ToggleApp { exe, path } => toggle_app(app, exe, path),
        Request::Inspected(report) => {
            let _ = app.emit("inspected", &report);
            let a = app.clone();
            let _ = app.run_on_main_thread(move || open_settings(&a));
        }
        Request::Area { exe, area } => {
            if let Ok(a) = &area {
                let mut c = app.state::<State>().cfg.lock().unwrap().clone();
                let key = exe.to_ascii_lowercase();
                match c.apps.iter_mut().find(|r| r.key() == key) {
                    Some(r) => r.areas.push(*a),
                    None => c.apps.push(AppRule { exe: exe.clone(), name: exe.clone(), areas: vec![*a], ..Default::default() }),
                }
                apply_and_tell(app, c);
            }
            let _ = app.emit("area", serde_json::json!({ "exe": exe, "ok": area.is_ok(), "reason": area.err() }));
            let a = app.clone();
            let _ = app.run_on_main_thread(move || open_settings(&a));
        }
    }
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
            if let Some(t) = s.tray.lock().unwrap().as_ref() {
                let _ = t.toggle.set_checked(st.enabled);
            }
        }
        if let Some(tray) = app.tray_by_id("main") {
            let w = words(c.lang());
            let _ = tray.set_tooltip(Some(if st.enabled { w[3] } else { w[4] }));
        }
    }
    let _ = app.emit("status", &st);
}

/// Test copies run with their own settings folder, next to the real Deonyx,
/// and never take the focus from whatever the user is doing.
fn testing() -> bool {
    std::env::var_os("DEONYX_CONFIG_DIR").is_some()
}

fn open_settings(app: &AppHandle) {
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.unminimize();
        let _ = w.show();
        if !testing() {
            let _ = w.set_focus();
        }
        return;
    }
    // Likely next: "Add an app". Have the installed programs ready by then.
    apps::warm_up();
    let built = WebviewWindowBuilder::new(app, "main", WebviewUrl::App("index.html".into()))
        .title("Deonyx")
        .inner_size(840.0, 700.0)
        .min_inner_size(660.0, 540.0)
        .theme(Some(Theme::Dark))
        .background_color(tauri::window::Color(18, 18, 22, 255))
        .center()
        .focused(!testing())
        .build();
    if let Err(e) = built {
        eprintln!("ayarlar penceresi acilamadi: {e}");
    }
}

fn build_tray(app: &AppHandle, cfg: &Config) -> tauri::Result<Tray> {
    let w = words(cfg.lang());
    let toggle = CheckMenuItem::with_id(app, "toggle", w[0], true, cfg.enabled, Some("Ctrl+Alt+D"))?;
    let settings = MenuItem::with_id(app, "settings", w[1], true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", w[2], true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&toggle, &PredefinedMenuItem::separator(app)?, &settings, &quit])?;
    let item = toggle.clone();
    TrayIconBuilder::with_id("main")
        .icon(Image::from_bytes(include_bytes!("../icons/tray.png"))?)
        .tooltip(if cfg.enabled { w[3] } else { w[4] })
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
    Ok(Tray { toggle, settings, quit })
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let has = |flag: &str| args.iter().any(|a| a == flag);

    // The copy that darkens normal apps for a Deonyx running as administrator.
    if has("--helper") {
        helper_main(&args);
        return;
    }
    // Run, as administrator, by admin::run_elevated.
    if let Some(i) = args.iter().position(|a| a == "--admin-task") {
        let r = match args.get(i + 1).map(String::as_str) {
            Some("create-logon") => admin::create_task(true),
            Some("create") => admin::create_task(false),
            Some("delete") => admin::delete_task(),
            _ => Err("unknown".into()),
        };
        std::process::exit(if r.is_ok() { 0 } else { 1 });
    }
    // Started by the task right after the old instance quit: let it go first.
    if has("--from-task") {
        std::thread::sleep(std::time::Duration::from_millis(1500));
    }

    let mut cfg = config::load();
    if cfg.admin && !admin::elevated() {
        // Running as administrator is on: hand over to the running Deonyx,
        // or start it through the task.
        if admin::ask_running_to_open() || admin::run_task() {
            return;
        }
    }
    let marker = config::dir().join("open-settings");
    let reopen = std::fs::remove_file(&marker).is_ok();
    let hidden = has("--hidden") && !reopen;
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

    let mut builder = tauri::Builder::default();
    let testing = testing();
    if !testing {
        builder = builder.plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| open_settings(app)));
    }
    builder
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            Some(vec!["--hidden"]),
        ))
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .setup(move |app| {
            let status_handle = app.handle().clone();
            let request_handle = app.handle().clone();
            let first_run = cfg.apps.is_empty() && !cfg.everything;
            let engine = Engine::start(
                cfg.clone(),
                Role::Main,
                Box::new(move |st| on_status(&status_handle, st)),
                Box::new(move |r| on_request(&request_handle, r)),
            );
            app.manage(State {
                status: Mutex::new(Status { enabled: cfg.enabled, ..Default::default() }),
                cfg: Mutex::new(cfg.clone()),
                engine,
                tray: Mutex::new(None),
                update: Mutex::new(None),
            });
            // Starting with Windows used to live only in the Run key.
            if !cfg.admin && !cfg.autostart && app.autolaunch().is_enabled().unwrap_or(false) {
                let mut c = cfg.clone();
                c.autostart = true;
                let _ = config::save(&c);
                *app.state::<State>().cfg.lock().unwrap() = c;
            }
            let tray = build_tray(app.handle(), &cfg)?;
            *app.state::<State>().tray.lock().unwrap() = Some(tray);
            if !hidden || first_run {
                open_settings(app.handle());
            }
            // Look for a new version a little after starting.
            if cfg.updates && !testing {
                let h = app.handle().clone();
                tauri::async_runtime::spawn(async move {
                    tokio_sleep(10).await;
                    if let Ok(Some(v)) = check_update(h.clone()).await {
                        let _ = h.emit("update", &v);
                    }
                });
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            get_state,
            set_config,
            running_apps,
            installed_apps,
            app_icons,
            describe_app,
            pick_exe,
            get_autostart,
            set_autostart,
            set_admin,
            inspect,
            draw_area,
            check_update,
            install_update
        ])
        .build(tauri::generate_context!())
        .expect("Deonyx failed to start")
        .run(|_app, event| {
            // Closing the settings window keeps Deonyx running in the tray.
            if let RunEvent::ExitRequested { api, code: None, .. } = event {
                api.prevent_exit();
            }
        });
}

/// The helper (see helper.rs): no tray and no window, just the engine, for
/// as long as the main Deonyx runs.
fn helper_main(args: &[String]) {
    // Tauri makes the main copy see real pixels on every screen; the helper
    // never starts Tauri, so it asks for that itself.
    unsafe {
        use windows::Win32::UI::HiDpi::{SetProcessDpiAwarenessContext, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2};
        let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
    }
    let arg = |name: &str| args.iter().position(|a| a == name).and_then(|i| args.get(i + 1)).cloned();
    if let Some(dir) = arg("--config-dir") {
        std::env::set_var("DEONYX_CONFIG_DIR", dir);
    }
    let parent: u32 = arg("--parent").and_then(|s| s.parse().ok()).unwrap_or(0);
    // A window handle can't cross threads as one; its number can.
    let main_window: isize = arg("--window").and_then(|s| s.parse().ok()).unwrap_or(0);
    let hwnd = |n: isize| windows::Win32::Foundation::HWND(n as *mut _);
    let engine = Engine::start(
        config::load(),
        Role::Helper { parent },
        Box::new(move |st| {
            helper::send(hwnd(main_window), Some(hwnd(st.hwnd)), helper::STATUS, &st);
        }),
        Box::new(|_| {}),
    );
    helper::wait_for_parent(parent, hwnd(main_window));
    engine.send(Cmd::Quit);
    std::thread::sleep(std::time::Duration::from_millis(300));
}

/// Waits without holding a thread.
async fn tokio_sleep(secs: u64) {
    let (tx, rx) = tauri::async_runtime::channel::<()>(1);
    std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_secs(secs));
        let _ = tx.blocking_send(());
    });
    let mut rx = rx;
    let _ = rx.recv().await;
}
