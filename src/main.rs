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
mod cli;
mod config;
mod desk;
mod engine;
mod gpu;
mod helper;
mod jumplist;
mod keys;
mod layer;
mod log;
mod photos;
mod presets;
mod protect;
mod screens;
mod select;
mod store;
mod sun;
mod win;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use serde::Serialize;
use tauri::image::Image;
use tauri::menu::{CheckMenuItem, IsMenuItem, Menu, MenuItem, PredefinedMenuItem, Submenu};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Emitter, Manager, PhysicalPosition, RunEvent, Theme, WebviewUrl, WebviewWindowBuilder, Wry};
use tauri_plugin_autostart::ManagerExt as _;
use tauri_plugin_dialog::DialogExt;
use tauri_plugin_updater::UpdaterExt;

use config::{AppRule, Config, Excluded};
use engine::{Cmd, Engine, Request, Role, Status};

struct State {
    cfg: Mutex<Config>,
    status: Mutex<Status>,
    engine: Engine,
    /// A newer version, once found.
    update: Mutex<Option<String>>,
    presets: Mutex<Vec<presets::Preset>>,
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
    presets: Vec<presets::Preset>,
    /// The last crash's report, until dismissed.
    crash: Option<String>,
    /// The settings differ from the profile in use.
    profile_changed: bool,
    /// Installed from the Microsoft Store: it updates Deonyx, and there's no
    /// administrator mode.
    packaged: bool,
}

/// Tray texts, in the settings' language: the switch, Settings, Quit, the
/// tooltip when on and off, pause for an hour, resume, the app list, the
/// profiles, updating to a new version ({v}) and its tooltip.
fn words(lang: &str) -> [&'static str; 11] {
    match lang {
        "tr" => ["Uygulamaları karart", "Ayarlar", "Deonyx'ten çık", "Deonyx: açık", "Deonyx: kapalı", "1 saat duraklat", "Devam et", "Uygulamalar", "Profiller", "{v} sürümüne güncelle", "Deonyx: güncelleme hazır"],
        "de" => ["Apps abdunkeln", "Einstellungen", "Deonyx beenden", "Deonyx: an", "Deonyx: aus", "1 Stunde pausieren", "Fortsetzen", "Apps", "Profile", "Auf {v} aktualisieren", "Deonyx: Update bereit"],
        "fr" => ["Assombrir les applis", "Paramètres", "Quitter Deonyx", "Deonyx : activé", "Deonyx : désactivé", "Pause d'une heure", "Reprendre", "Applis", "Profils", "Mettre à jour vers {v}", "Deonyx : mise à jour prête"],
        "es" => ["Oscurecer apps", "Ajustes", "Salir de Deonyx", "Deonyx: activado", "Deonyx: desactivado", "Pausar 1 hora", "Reanudar", "Apps", "Perfiles", "Actualizar a {v}", "Deonyx: actualización lista"],
        "it" => ["Scurisci le app", "Impostazioni", "Esci da Deonyx", "Deonyx: attivo", "Deonyx: disattivato", "Pausa di 1 ora", "Riprendi", "App", "Profili", "Aggiorna a {v}", "Deonyx: aggiornamento pronto"],
        "pt" => ["Escurecer apps", "Configurações", "Sair do Deonyx", "Deonyx: ligado", "Deonyx: desligado", "Pausar por 1 hora", "Retomar", "Apps", "Perfis", "Atualizar para {v}", "Deonyx: atualização pronta"],
        "ru" => ["Затемнять приложения", "Настройки", "Выйти из Deonyx", "Deonyx: вкл.", "Deonyx: выкл.", "Пауза на 1 час", "Продолжить", "Приложения", "Профили", "Обновить до {v}", "Deonyx: обновление готово"],
        "uk" => ["Затемнювати застосунки", "Налаштування", "Вийти з Deonyx", "Deonyx: увімк.", "Deonyx: вимк.", "Пауза на 1 годину", "Продовжити", "Застосунки", "Профілі", "Оновити до {v}", "Deonyx: оновлення готове"],
        "nl" => ["Apps donker maken", "Instellingen", "Deonyx afsluiten", "Deonyx: aan", "Deonyx: uit", "1 uur pauzeren", "Hervatten", "Apps", "Profielen", "Bijwerken naar {v}", "Deonyx: update klaar"],
        "ja" => ["アプリを暗くする", "設定", "Deonyx を終了", "Deonyx: オン", "Deonyx: オフ", "1 時間一時停止", "再開", "アプリ", "プロファイル", "{v} に更新", "Deonyx: 更新の準備ができました"],
        "ko" => ["앱 어둡게 하기", "설정", "Deonyx 종료", "Deonyx: 켜짐", "Deonyx: 꺼짐", "1시간 일시 중지", "다시 시작", "앱", "프로필", "{v}(으)로 업데이트", "Deonyx: 업데이트 준비됨"],
        "zh" => ["调暗应用", "设置", "退出 Deonyx", "Deonyx：开", "Deonyx：关", "暂停 1 小时", "继续", "应用", "配置文件", "更新到 {v}", "Deonyx：更新已就绪"],
        _ => ["Darken apps", "Settings", "Quit Deonyx", "Deonyx: on", "Deonyx: off", "Pause for 1 hour", "Resume", "Apps", "Profiles", "Update to {v}", "Deonyx: update ready"],
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
        presets: s.presets.lock().unwrap().clone(),
        crash: log::crash_report(),
        profile_changed: config_changed(&s),
        packaged: store::packaged(),
    }
}

fn config_changed(s: &State) -> bool {
    s.cfg.lock().unwrap().profile_changed()
}

/// Whether the settings differ from the profile in use, after a change.
#[tauri::command]
fn profile_changed(s: tauri::State<State>) -> bool {
    config_changed(&s)
}

/// Puts the jump list tasks on the Start menu shortcut and the taskbar, in
/// that language. Test copies would replace the real one: they leave it
/// alone, or with DEONYX_JUMPLIST_TEST try theirs under another id and take
/// it off again.
fn update_jump_list(lang: &'static str) {
    let test = std::env::var_os("DEONYX_JUMPLIST_TEST").is_some();
    if (testing() && !test) || store::packaged() {
        return;
    }
    let id = if testing() { "com.fdeox.deonyx.test" } else { "com.fdeox.deonyx" };
    std::thread::spawn(move || unsafe {
        use windows::Win32::System::Com::{CoInitializeEx, COINIT_APARTMENTTHREADED};
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
        match jumplist::set(id, lang) {
            Ok(()) if testing() => {
                let gone = jumplist::forget(id).is_ok();
                note!("jump list ready ({lang}), removed again: {gone}");
            }
            Ok(()) => {}
            Err(e) => note!("jump list: {e}"),
        }
    });
}

/// Keeps the settings as they are now under a name, and uses that profile.
#[tauri::command]
fn save_profile(app: AppHandle, name: String) -> Result<(), String> {
    let name = name.trim().to_string();
    if name.is_empty() {
        return Err("empty".into());
    }
    let mut c = app.state::<State>().cfg.lock().unwrap().clone();
    let name = c.profiles.iter().find(|p| p.name.eq_ignore_ascii_case(&name)).map_or(name, |p| p.name.clone());
    let p = c.to_profile(&name);
    match c.profiles.iter_mut().find(|q| q.name == name) {
        Some(q) => *q = p,
        None => c.profiles.push(p),
    }
    c.profile = name;
    apply_and_tell(&app, c);
    Ok(())
}

#[tauri::command]
fn use_profile(app: AppHandle, name: String) -> bool {
    let mut c = app.state::<State>().cfg.lock().unwrap().clone();
    let found = c.use_profile(&name);
    if found {
        apply_and_tell(&app, c);
    }
    found
}

/// Forgets a profile; the settings stay as they are.
#[tauri::command]
fn delete_profile(app: AppHandle, name: String) {
    let mut c = app.state::<State>().cfg.lock().unwrap().clone();
    c.profiles.retain(|p| p.name != name);
    if c.profile == name {
        c.profile.clear();
    }
    apply_and_tell(&app, c);
}

#[tauri::command]
fn dismiss_crash() {
    log::forget_crash();
}

/// From the settings or the tray window; the other one hears about it.
#[tauri::command]
fn set_config(app: AppHandle, window: tauri::Window, config: Config) -> Result<(), String> {
    apply(&app, config.clone())?;
    let other = if window.label() == "flyout" { "main" } else { "flyout" };
    let _ = app.emit_to(other, "config", &config);
    Ok(())
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
    if store::packaged() {
        return store::starts_with_windows();
    }
    s.cfg.lock().unwrap().autostart
}

#[tauri::command(async)]
fn set_autostart(app: AppHandle, on: bool) -> Result<(), String> {
    let mut c = app.state::<State>().cfg.lock().unwrap().clone();
    c.autostart = on;
    if store::packaged() {
        store::set_start_with_windows(on)?;
    } else {
        sync_startup(&app, &c)?;
    }
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

/// "Keep a part in real colours": the engine waits for a click on it.
#[tauri::command]
fn pick_area(app: AppHandle, on: bool) {
    let s = app.state::<State>();
    s.engine.send(if on { Cmd::PickArea } else { Cmd::StopInspect });
}

/// Let the user draw an area of an app's window to keep in real colours.
#[tauri::command]
fn draw_area(app: AppHandle, exe: String, hint: String) {
    app.state::<State>().engine.send(Cmd::DrawArea { exe, hint });
}

#[tauri::command]
fn list_screens() -> Vec<screens::Info> {
    screens::list()
}

#[derive(Serialize)]
struct SunTimes {
    /// "times", "light" (the sun stays up) or "dark" (it never rises).
    kind: &'static str,
    rise: String,
    set: String,
}

#[tauri::command]
fn sun_times(lat: f64, lon: f64) -> SunTimes {
    let hhmm = |m: u32| format!("{:02}:{:02}", m / 60, m % 60);
    match sun::today(lat, lon) {
        sun::Day::Times { rise, set } => SunTimes { kind: "times", rise: hhmm(rise), set: hhmm(set) },
        sun::Day::Light => SunTimes { kind: "light", rise: String::new(), set: String::new() },
        sun::Day::Dark => SunTimes { kind: "dark", rise: String::new(), set: String::new() },
    }
}

/// Pauses darkening for a while (minutes), or resumes (0).
#[tauri::command]
fn pause(app: AppHandle, minutes: u64) {
    let mut c = app.state::<State>().cfg.lock().unwrap().clone();
    c.paused_until = if minutes == 0 { 0 } else { config::now_secs() + minutes * 60 };
    apply_and_tell(&app, c);
}

/// Everything a bug report needs, as text for the clipboard.
#[tauri::command]
fn diagnostics(app: AppHandle) -> String {
    let s = app.state::<State>();
    let c = s.cfg.lock().unwrap().clone();
    let st = s.status.lock().unwrap().clone();
    let yes = |b: bool| if b { "yes" } else { "no" };
    let mut out = vec![
        format!("Deonyx {}", env!("CARGO_PKG_VERSION")),
        format!("Windows build {} ({})", desk::windows_build(), std::env::consts::ARCH),
        format!("Administrator: {} (setting {})", yes(admin::elevated()), yes(c.admin)),
        format!("Capture: {}", if st.capture.is_empty() { "window" } else { &st.capture }),
        format!("GPU: {}", gpu_name()),
    ];
    let screens = screens::list();
    out.push(format!("Screens: {}", screens.iter().map(|s| s.name.clone()).collect::<Vec<_>>().join(", ")));
    out.push(format!(
        "Mode: {}, {} app(s), {} left alone",
        if c.everything { "every light window" } else { "chosen apps" },
        c.apps.len(),
        c.exclude.len()
    ));
    out.push(format!(
        "Look: {}, warm {:.0}%, dim dark windows {:.0}%, schedule {}{}, battery {}",
        c.style,
        c.warm * 100.0,
        c.dim * 100.0,
        c.schedule,
        if c.gradual { " (gradual)" } else { "" },
        c.battery
    ));
    out.push(format!(
        "State: {}{}{}{}",
        if st.enabled { "on" } else { "off" },
        if st.waiting { ", waiting for its time" } else { "" },
        if st.paused { ", paused" } else { "" },
        if st.battery { ", paused on battery" } else { "" }
    ));
    let mut running: Vec<_> = st.running.iter().map(|(e, n)| format!("{e} {n}")).collect();
    running.sort();
    out.push(format!("Darkening: {}", if running.is_empty() { "nothing".into() } else { running.join(", ") }));
    out.push(format!(
        "Hotkeys: {} / {} / {}{}",
        c.keys.all,
        c.keys.front,
        c.keys.peek,
        if st.keys_taken.is_empty() { String::new() } else { format!(" (taken: {})", st.keys_taken.join(", ")) }
    ));
    out.push(format!("Apps: {}", c.apps.iter().map(|a| a.exe.clone()).collect::<Vec<_>>().join(", ")));
    out.push("Recent messages:".into());
    out.extend(log::lines().into_iter().map(|l| format!("  {l}")));
    if let Some(crash) = log::crash_report() {
        out.push("Last crash:".into());
        out.extend(crash.lines().map(|l| format!("  {l}")));
    }
    out.join("\n")
}

fn gpu_name() -> String {
    use windows::Win32::Graphics::Dxgi::{CreateDXGIFactory1, IDXGIFactory1};
    unsafe {
        let Ok(f) = CreateDXGIFactory1::<IDXGIFactory1>() else { return "?".into() };
        let Ok(a) = f.EnumAdapters1(0) else { return "?".into() };
        let Ok(d) = a.GetDesc1() else { return "?".into() };
        let n = d.Description.iter().position(|&c| c == 0).unwrap_or(d.Description.len());
        String::from_utf16_lossy(&d.Description[..n])
    }
}

/// Saves the settings to a file the user picks.
#[tauri::command(async)]
fn export_config(app: AppHandle) -> Result<bool, String> {
    let c = app.state::<State>().cfg.lock().unwrap().clone();
    let Some(path) = app.dialog().file().add_filter("Deonyx settings", &["json"]).set_file_name("Deonyx settings.json").blocking_save_file()
    else {
        return Ok(false);
    };
    let path = path.into_path().map_err(|e| e.to_string())?;
    std::fs::write(path, serde_json::to_string_pretty(&c).unwrap_or_default()).map_err(|e| e.to_string())?;
    Ok(true)
}

/// Takes the settings from a file the user picks. Running as administrator
/// and starting with Windows stay as they are here: those change Windows
/// itself and have their own switches.
#[tauri::command(async)]
fn import_config(app: AppHandle) -> Result<Option<Config>, String> {
    let Some(path) = app.dialog().file().add_filter("Deonyx settings", &["json"]).blocking_pick_file() else { return Ok(None) };
    let path = path.into_path().map_err(|e| e.to_string())?;
    let text = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
    let mut c: Config = serde_json::from_str(&text).map_err(|_| "not_settings".to_string())?;
    let now = app.state::<State>().cfg.lock().unwrap().clone();
    c.admin = now.admin;
    c.autostart = now.autostart;
    c.welcomed = true;
    c.seen_version = now.seen_version;
    apply_and_tell(&app, c.clone());
    Ok(Some(c))
}

#[tauri::command]
fn open_settings_cmd(app: AppHandle) {
    open_settings(&app);
    if let Some(f) = app.get_webview_window("flyout") {
        let _ = f.hide();
    }
}

/// When the tray window last went away: a click on the tray icon takes the
/// focus from it first, and that click should close it, not open it again.
static FLYOUT_HIDDEN: Mutex<Option<std::time::Instant>> = Mutex::new(None);
/// The tray window has had the focus since it was last shown.
static FLYOUT_FOCUSED: AtomicBool = AtomicBool::new(false);

#[tauri::command]
fn close_flyout(app: AppHandle) {
    if let Some(f) = app.get_webview_window("flyout") {
        if f.is_visible().unwrap_or(false) {
            *FLYOUT_HIDDEN.lock().unwrap() = Some(std::time::Instant::now());
        }
        let _ = f.hide();
    }
}

/// Installed with Scoop: Scoop updates it, not Deonyx itself.
fn from_scoop() -> bool {
    std::env::current_exe().is_ok_and(|p| p.to_string_lossy().to_ascii_lowercase().contains(r"\scoop\apps\"))
}

#[tauri::command]
async fn check_update(app: AppHandle) -> Result<Option<String>, String> {
    // Scoop and the Microsoft Store update Deonyx themselves.
    if from_scoop() || store::packaged() {
        return Ok(None);
    }
    let found = app.updater().map_err(|e| e.to_string())?.check().await.map_err(|e| e.to_string())?;
    let version = found.map(|u| u.version);
    *app.state::<State>().update.lock().unwrap() = version.clone();
    show_update(&app);
    Ok(version)
}

/// Downloads the new version and starts its installer, which replaces and
/// restarts Deonyx.
#[tauri::command]
async fn install_update(app: AppHandle) -> Result<(), String> {
    if testing() {
        if let Ok(v) = std::env::var("DEONYX_UPDATE_TEST") {
            note!("update test: would install {v}");
            let _ = app.emit("update-test-install", &v);
            return Ok(());
        }
    }
    if store::packaged() {
        return Err("The Microsoft Store updates Deonyx.".into());
    }
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
    let old_lang = std::mem::replace(&mut *s.cfg.lock().unwrap(), config.clone()).lang();
    if config.lang() != old_lang {
        update_jump_list(config.lang());
    }
    refresh_tray(app, &config);
    s.engine.send(Cmd::Config(config));
    Ok(())
}

/// Like apply(), and tells the settings window, for changes made elsewhere.
fn apply_and_tell(app: &AppHandle, config: Config) {
    if apply(app, config).is_ok() {
        let c = app.state::<State>().cfg.lock().unwrap().clone();
        let _ = app.emit("config", &c);
    }
}

fn set_enabled(app: &AppHandle, on: bool) {
    let mut c = app.state::<State>().cfg.lock().unwrap().clone();
    c.enabled = on;
    apply_and_tell(app, c);
}

/// A new app, with its preset's settings when there is one.
fn new_rule(app: &AppHandle, exe: String, name: String, icon: Option<String>) -> AppRule {
    let mut r = AppRule { exe, name, icon, ..Default::default() };
    let key = r.key();
    if let Some(p) = app.state::<State>().presets.lock().unwrap().iter().find(|p| p.exe == key) {
        r.darkness = p.darkness.unwrap_or(r.darkness);
        r.text = p.text.unwrap_or(r.text);
        r.protect = p.protect.unwrap_or(r.protect);
        r.photos = p.photos.unwrap_or(r.photos);
    }
    r
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
        c.apps.push(new_rule(app, d.exe, d.name, d.icon));
    }
    apply_and_tell(app, c);
}

/// From the command line: an app on, off, or switched (on is None).
fn set_app(app: &AppHandle, exe: String, on: Option<bool>) {
    let mut c = app.state::<State>().cfg.lock().unwrap().clone();
    let key = exe.to_ascii_lowercase();
    if let Some(r) = c.apps.iter_mut().find(|r| r.key() == key) {
        r.enabled = on.unwrap_or(!r.enabled);
    } else if c.everything {
        let at = c.exclude.iter().position(|e| e.exe.eq_ignore_ascii_case(&exe));
        match (at, on) {
            (Some(i), Some(true) | None) => {
                c.exclude.remove(i);
            }
            (None, Some(false) | None) => {
                let name = exe.trim_end_matches(".exe").trim_end_matches(".EXE").to_string();
                c.exclude.push(Excluded { exe, name, icon: None });
            }
            _ => return,
        }
    } else if on != Some(false) {
        let name = exe.trim_end_matches(".exe").trim_end_matches(".EXE").to_string();
        c.apps.push(new_rule(app, exe, name, None));
    } else {
        return;
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
        Request::SetApp { exe, on } => set_app(app, exe, on),
        Request::Pause(minutes) => pause(app.clone(), minutes),
        Request::Restarted => {
            // A fresh engine starts from the settings it was born with.
            if let Some(s) = app.try_state::<State>() {
                s.engine.send(Cmd::Presets(s.presets.lock().unwrap().clone()));
                s.engine.send(Cmd::Config(s.cfg.lock().unwrap().clone()));
            }
            let _ = app.emit("crash", log::crash_report());
        }
        Request::Profile(name) => {
            let mut c = app.state::<State>().cfg.lock().unwrap().clone();
            if c.use_profile(&name) {
                apply_and_tell(app, c);
            }
        }
        Request::Inspected(report) => {
            let _ = app.emit("inspected", &report);
            let a = app.clone();
            let _ = app.run_on_main_thread(move || open_settings(&a));
        }
        Request::Area { exe, path, area } => {
            if let Ok(a) = &area {
                let mut c = app.state::<State>().cfg.lock().unwrap().clone();
                let key = exe.to_ascii_lowercase();
                match c.apps.iter_mut().find(|r| r.key() == key) {
                    Some(r) => r.areas.push(*a),
                    None => {
                        let d = if path.is_empty() { None } else { Some(apps::describe(&path)) };
                        let (name, icon) = d.map_or((exe.clone(), None), |d| (d.name, d.icon));
                        let mut r = new_rule(app, exe.clone(), name, icon);
                        r.areas.push(*a);
                        c.apps.push(r);
                    }
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
        let changed = {
            let mut c = s.cfg.lock().unwrap();
            // The hotkey switches the engine directly; remember it.
            if c.enabled != st.enabled {
                c.enabled = st.enabled;
                let _ = config::save(&c);
                Some(c.clone())
            } else {
                None
            }
        };
        if let Some(c) = changed {
            refresh_tray(app, &c);
        }
        if let Some(tray) = app.tray_by_id("main") {
            let cfg = s.cfg.lock().unwrap().clone();
            let _ = tray.set_tooltip(Some(tooltip(app, &cfg)));
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
        note!("settings window failed to open: {e}");
    }
}

/// The small window over the tray: on and off, a pause, the apps, the look.
fn open_flyout(app: &AppHandle, at: PhysicalPosition<f64>) {
    const W: f64 = 320.0;
    const H: f64 = 440.0;
    if FLYOUT_HIDDEN.lock().unwrap().is_some_and(|t| t.elapsed().as_millis() < 300) {
        return;
    }
    let window = match app.get_webview_window("flyout") {
        Some(w) => w,
        None => {
            let built = WebviewWindowBuilder::new(app, "flyout", WebviewUrl::App("flyout.html".into()))
                .title("Deonyx")
                .inner_size(W, H)
                .decorations(false)
                .resizable(false)
                .skip_taskbar(true)
                .always_on_top(true)
                .visible(false)
                .shadow(true)
                .theme(Some(Theme::Dark))
                .background_color(tauri::window::Color(18, 18, 22, 255))
                // The first time, the window is only ready once its page is:
                // showing it earlier doesn't stick.
                .on_page_load(|w, p| {
                    if p.event() == tauri::webview::PageLoadEvent::Finished {
                        let _ = w.show();
                        let _ = w.set_focus();
                    }
                })
                .build();
            match built {
                Ok(w) => {
                    // Gone as soon as the focus goes elsewhere, like Windows' own
                    // tray windows (the page listens too, this also covers its
                    // frame).
                    // Only after it had the focus: Windows may show it without
                    // giving it the focus, and a "lost" one then means nothing.
                    let a = app.clone();
                    w.on_window_event(move |e| match e {
                        tauri::WindowEvent::Focused(true) => FLYOUT_FOCUSED.store(true, Ordering::Relaxed),
                        tauri::WindowEvent::Focused(false) if FLYOUT_FOCUSED.swap(false, Ordering::Relaxed) => {
                            close_flyout(a.clone());
                        }
                        _ => {}
                    });
                    w
                }
                Err(e) => {
                    note!("tray window failed to open: {e}");
                    return open_settings(app);
                }
            }
        }
    };
    // Next to the click, inside the screen's free area, above or below the
    // taskbar.
    if let Ok(Some(m)) = app.monitor_from_point(at.x, at.y) {
        let s = m.scale_factor();
        let wa = m.work_area();
        let (w, h) = (W * s, H * s);
        let (left, top) = (wa.position.x as f64, wa.position.y as f64);
        let (right, bottom) = (left + wa.size.width as f64, top + wa.size.height as f64);
        let x = (at.x - w / 2.0).clamp(left + 8.0, right - w - 8.0);
        let y = if at.y > (top + bottom) / 2.0 { bottom - h - 8.0 } else { top + 8.0 };
        let _ = window.set_position(PhysicalPosition::new(x, y));
    }
    let _ = window.emit("flyout-open", ());
    FLYOUT_FOCUSED.store(false, Ordering::Relaxed);
    let _ = window.show();
    let _ = window.set_focus();
}

/// The tray menu, rebuilt whenever the settings change.
/// A name as a menu shows it: Windows reads "&" as the underlined letter.
fn menu_text(name: &str) -> String {
    name.replace('&', "&&")
}

fn tray_menu(app: &AppHandle, cfg: &Config) -> tauri::Result<Menu<Wry>> {
    let w = words(cfg.lang());
    let accel = if keys::parse(&cfg.keys.all).is_some() { Some(cfg.keys.all.as_str()) } else { None };
    let toggle = CheckMenuItem::with_id(app, "toggle", w[0], true, cfg.enabled, accel)
        .or_else(|_| CheckMenuItem::with_id(app, "toggle", w[0], true, cfg.enabled, None::<&str>))?;
    let pause = if cfg.paused() {
        MenuItem::with_id(app, "resume", w[6], true, None::<&str>)?
    } else {
        MenuItem::with_id(app, "pause", w[5], cfg.enabled, None::<&str>)?
    };
    let checks: Vec<CheckMenuItem<Wry>> = cfg
        .apps
        .iter()
        .map(|a| CheckMenuItem::with_id(app, format!("app:{}", a.key()), menu_text(if a.name.is_empty() { &a.exe } else { &a.name }), true, a.enabled, None::<&str>))
        .collect::<tauri::Result<_>>()?;
    let refs: Vec<&dyn IsMenuItem<Wry>> = checks.iter().map(|c| c as &dyn IsMenuItem<Wry>).collect();
    let apps = Submenu::with_items(app, w[7], !checks.is_empty(), &refs)?;
    let profiles: Vec<CheckMenuItem<Wry>> = cfg
        .profiles
        .iter()
        .map(|p| CheckMenuItem::with_id(app, format!("profile:{}", p.name), menu_text(&p.name), true, cfg.profile == p.name, None::<&str>))
        .collect::<tauri::Result<_>>()?;
    let refs: Vec<&dyn IsMenuItem<Wry>> = profiles.iter().map(|c| c as &dyn IsMenuItem<Wry>).collect();
    let profile_menu = Submenu::with_items(app, w[8], true, &refs)?;
    let settings = MenuItem::with_id(app, "settings", w[1], true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", w[2], true, None::<&str>)?;
    let sep = PredefinedMenuItem::separator(app)?;
    let sep2 = PredefinedMenuItem::separator(app)?;
    let version = app.try_state::<State>().and_then(|s| s.update.lock().unwrap().clone());
    let update = MenuItem::with_id(app, "update", w[9].replace("{v}", version.as_deref().unwrap_or("")), true, None::<&str>)?;
    let sep0 = PredefinedMenuItem::separator(app)?;
    let mut items: Vec<&dyn IsMenuItem<Wry>> = Vec::new();
    if version.is_some() {
        items.extend([&update as &dyn IsMenuItem<Wry>, &sep0]);
    }
    items.extend([&toggle as &dyn IsMenuItem<Wry>, &pause, &apps]);
    if !profiles.is_empty() {
        items.push(&profile_menu);
    }
    items.extend([&sep as &dyn IsMenuItem<Wry>, &settings, &sep2, &quit]);
    Menu::with_items(app, &items)
}

fn refresh_tray(app: &AppHandle, cfg: &Config) {
    if let (Some(tray), Ok(menu)) = (app.tray_by_id("main"), tray_menu(app, cfg)) {
        let _ = tray.set_menu(Some(menu));
        let _ = tray.set_tooltip(Some(tooltip(app, cfg)));
    }
}

/// What the tray icon says on hover: a waiting update first, else on or off.
fn tooltip(app: &AppHandle, cfg: &Config) -> &'static str {
    let w = words(cfg.lang());
    let Some(s) = app.try_state::<State>() else { return if cfg.enabled { w[3] } else { w[4] } };
    if s.update.lock().unwrap().is_some() {
        return w[10];
    }
    let st = s.status.lock().unwrap();
    if st.enabled && !st.paused && st.paused_by.is_empty() {
        w[3]
    } else {
        w[4]
    }
}

const TRAY: &[u8] = include_bytes!("../icons/tray.png");
/// The same gem with a green dot: a new version waits.
const TRAY_UPDATE: &[u8] = include_bytes!("../icons/tray-update.png");

/// A new version was found: the tray gets its dot, menu item and tooltip,
/// and open windows their banner. Installing waits for the user's click.
fn show_update(app: &AppHandle) {
    let Some(s) = app.try_state::<State>() else { return };
    let version = s.update.lock().unwrap().clone();
    let cfg = s.cfg.lock().unwrap().clone();
    refresh_tray(app, &cfg);
    if let Some(tray) = app.tray_by_id("main") {
        if let Ok(icon) = Image::from_bytes(if version.is_some() { TRAY_UPDATE } else { TRAY }) {
            let _ = tray.set_icon(Some(icon));
        }
    }
    if let Some(v) = version {
        let _ = app.emit("update", &v);
    }
}

/// Looks for a new version a little after starting and then every few
/// hours, for as long as updates are on in the settings.
fn watch_updates(app: &AppHandle) {
    let h = app.clone();
    tauri::async_runtime::spawn(async move {
        tokio_sleep(10).await;
        loop {
            let on = h.state::<State>().cfg.lock().unwrap().updates;
            if on && h.state::<State>().update.lock().unwrap().is_none() {
                match check_update(h.clone()).await {
                    Ok(Some(v)) => note!("Deonyx {v} is ready"),
                    Ok(None) => {}
                    Err(e) => note!("update check: {e}"),
                }
            }
            tokio_sleep(UPDATE_EVERY).await;
        }
    });
}

/// Seconds between looks for a new version.
const UPDATE_EVERY: u64 = 6 * 60 * 60;

fn on_menu(app: &AppHandle, id: &str) {
    let c = app.state::<State>().cfg.lock().unwrap().clone();
    match id {
        "toggle" => set_enabled(app, !c.enabled),
        "pause" => pause(app.clone(), 60),
        "resume" => pause(app.clone(), 0),
        "settings" => open_settings(app),
        "update" => {
            let a = app.clone();
            tauri::async_runtime::spawn(async move {
                if let Err(e) = install_update(a).await {
                    note!("update failed: {e}");
                }
            });
        }
        "quit" => {
            app.state::<State>().engine.send(Cmd::Quit);
            app.exit(0);
        }
        _ => {
            if let Some(name) = id.strip_prefix("profile:") {
                let mut c = c;
                if c.use_profile(name) {
                    apply_and_tell(app, c);
                }
            } else if let Some(key) = id.strip_prefix("app:") {
                let mut c = c;
                if let Some(r) = c.apps.iter_mut().find(|r| r.key() == key) {
                    r.enabled = !r.enabled;
                    apply_and_tell(app, c);
                }
            }
        }
    }
}

fn build_tray(app: &AppHandle, cfg: &Config) -> tauri::Result<()> {
    let w = words(cfg.lang());
    TrayIconBuilder::with_id("main")
        .icon(Image::from_bytes(TRAY)?)
        .tooltip(if cfg.enabled { w[3] } else { w[4] })
        .menu(&tray_menu(app, cfg)?)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, e| on_menu(app, e.id.as_ref()))
        .on_tray_icon_event(|tray, e| {
            if let TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, position, .. } = e {
                open_flyout(tray.app_handle(), position);
            }
        })
        .build(app)?;
    Ok(())
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let has = |flag: &str| args.iter().any(|a| a == flag);
    log::keep_crashes();

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
    // A command for the running Deonyx (cli.rs).
    if let Some(code) = cli::run(&args) {
        std::process::exit(code);
    }
    // The id the Start menu shortcut has, so the jump list lands on it.
    if !testing() && !store::packaged() {
        jumplist::claim_app_id();
    }
    // Started by the task right after the old instance quit: let it go first.
    if has("--from-task") {
        std::thread::sleep(std::time::Duration::from_millis(1500));
    }

    let mut cfg = config::load();
    // No administrator task from the Store.
    if store::packaged() && cfg.admin {
        cfg.admin = false;
        let _ = config::save(&cfg);
    }
    if cfg.admin && !admin::elevated() {
        // Running as administrator is on: hand over to the running Deonyx,
        // or start it through the task.
        if admin::ask_running_to_open() || admin::run_task() {
            return;
        }
        // The task is gone (removed outside Deonyx): run without
        // administrator rights, and show the switch as it really is.
        if !admin::task_exists() {
            note!("the administrator task is gone; running without administrator rights");
            cfg.admin = false;
            let _ = config::save(&cfg);
        }
    }
    let marker = config::dir().join("open-settings");
    let reopen = std::fs::remove_file(&marker).is_ok();
    let hidden = (has("--hidden") || store::started_with_windows()) && !reopen;
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
                Arc::new(move |st| on_status(&status_handle, st)),
                Arc::new(move |r| on_request(&request_handle, r)),
            );
            app.manage(State {
                status: Mutex::new(Status { enabled: cfg.enabled, ..Default::default() }),
                cfg: Mutex::new(cfg.clone()),
                engine,
                update: Mutex::new(None),
                presets: Mutex::new(presets::load()),
            });
            // Starting with Windows used to live only in the Run key.
            if !store::packaged() && !cfg.admin && !cfg.autostart && app.autolaunch().is_enabled().unwrap_or(false) {
                let mut c = cfg.clone();
                c.autostart = true;
                let _ = config::save(&c);
                *app.state::<State>().cfg.lock().unwrap() = c;
            }
            build_tray(app.handle(), &cfg)?;
            update_jump_list(cfg.lang());
            if !hidden || first_run {
                open_settings(app.handle());
            }
            // Test copies can open the tray window as if clicked at "x,y".
            if let Some((x, y)) = std::env::var("DEONYX_FLYOUT_AT")
                .ok()
                .filter(|_| testing)
                .and_then(|v| v.split_once(',').and_then(|(x, y)| Some((x.trim().parse().ok()?, y.trim().parse().ok()?))))
            {
                open_flyout(app.handle(), PhysicalPosition::new(x, y));
            }
            if cfg.updates && !testing && !from_scoop() {
                // Newer app presets, then a new version, a little after starting.
                let h = app.handle().clone();
                std::thread::spawn(move || {
                    std::thread::sleep(std::time::Duration::from_secs(8));
                    if presets::refresh() {
                        let list = presets::load();
                        *h.state::<State>().presets.lock().unwrap() = list.clone();
                        h.state::<State>().engine.send(Cmd::Presets(list.clone()));
                        let _ = h.emit("presets", &list);
                    }
                });
                watch_updates(app.handle());
            }
            // Test copies can pretend a new version is out.
            if testing {
                if let Ok(v) = std::env::var("DEONYX_UPDATE_TEST") {
                    let h = app.handle().clone();
                    std::thread::spawn(move || {
                        std::thread::sleep(std::time::Duration::from_secs(2));
                        *h.state::<State>().update.lock().unwrap() = Some(v);
                        show_update(&h);
                    });
                }
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
            pick_area,
            draw_area,
            list_screens,
            sun_times,
            pause,
            save_profile,
            use_profile,
            profile_changed,
            delete_profile,
            dismiss_crash,
            diagnostics,
            export_config,
            import_config,
            open_settings_cmd,
            close_flyout,
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
        Arc::new(move |st| {
            helper::send(hwnd(main_window), Some(hwnd(st.hwnd)), helper::STATUS, &st);
        }),
        Arc::new(|_| {}),
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
