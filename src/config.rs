// Settings, kept in %APPDATA%\Deonyx\config.json.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// An area of a window kept in its real colours, drawn by the user. Stored
/// as distances from the window's edges, so it follows the window when it is
/// resized the way side panels and 3D views do.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Area {
    pub left: i32,
    pub top: i32,
    pub right: i32,
    pub bottom: i32,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct AppRule {
    /// File name of the program, e.g. "SketchUp.exe".
    pub exe: String,
    /// Name shown in the settings window.
    pub name: String,
    pub enabled: bool,
    /// 0..1: how dark white backgrounds become.
    pub darkness: f32,
    /// 0..1: how bright dark text becomes.
    pub text: f32,
    /// Keep 3D views and pictures in their real colours.
    pub protect: bool,
    /// Find photos and videos in the window and keep them in real colours.
    pub photos: bool,
    /// Areas the user drew to keep in real colours.
    pub areas: Vec<Area>,
    /// Its own hotkey that switches it on and off, like "Ctrl+Alt+1".
    pub hotkey: String,
    /// The program's icon as a small PNG data URL, for the settings window.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
}

impl Default for AppRule {
    fn default() -> Self {
        AppRule {
            exe: String::new(),
            name: String::new(),
            enabled: true,
            darkness: 0.6,
            text: 0.7,
            protect: true,
            photos: true,
            areas: Vec::new(),
            hotkey: String::new(),
            icon: None,
        }
    }
}

impl AppRule {
    pub fn key(&self) -> String {
        self.exe.to_ascii_lowercase()
    }

    /// OKLab lightness that white and black become, for a look.
    pub fn lightness(&self, style: &str) -> (f32, f32) {
        let d = self.darkness.clamp(0.0, 1.0);
        let t = self.text.clamp(0.0, 1.0);
        match style {
            // Backgrounds go (nearly) pitch black: OLED screens switch those
            // pixels off.
            "black" => (0.10 * (1.0 - d), 0.74 + 0.22 * t),
            // Lower contrast: grey on grey, easy on tired eyes.
            "soft" => (0.46 - 0.20 * d, 0.70 + 0.16 * t),
            // For long reading: softer still, and warm (see engine::look).
            "paper" => (0.40 - 0.18 * d, 0.66 + 0.14 * t),
            _ => (0.40 - 0.28 * d, 0.74 + 0.22 * t),
        }
    }
}

/// A program left alone in "every light window" mode.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Excluded {
    pub exe: String,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
}

/// The hotkeys, as text like "Ctrl+Alt+D". An empty one is off.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Keys {
    /// Darkening on and off.
    pub all: String,
    /// The app in front on and off.
    pub front: String,
    /// Held down, every window shows its real colours.
    pub peek: String,
    /// The window under the mouse on and off, whatever its app does.
    pub window: String,
}

impl Default for Keys {
    fn default() -> Self {
        Keys { all: "Ctrl+Alt+D".into(), front: "Ctrl+Alt+F".into(), peek: "Ctrl+Alt+P".into(), window: "Ctrl+Alt+W".into() }
    }
}

/// Where the user lives, for darkening from sunset to sunrise.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Place {
    pub name: String,
    pub lat: f64,
    pub lon: f64,
}

/// A screen with its own look, or none at all.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Screen {
    /// Stays the same across restarts and cable swaps (see screens.rs).
    pub id: String,
    pub name: String,
    /// "" for the same look as everywhere, else "gray", "black" or "soft".
    pub style: String,
    /// Nothing on this screen is darkened.
    pub off: bool,
}

/// A named set of what to darken and how, to switch between in one go
/// (day, night, presenting).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Profile {
    pub name: String,
    pub everything: bool,
    pub apps: Vec<AppRule>,
    pub exclude: Vec<Excluded>,
    pub style: String,
    pub warm: f32,
    pub screens: Vec<Screen>,
    pub share_original: bool,
    pub readable: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub enabled: bool,
    pub apps: Vec<AppRule>,
    /// Darken every light window, not only the apps in the list.
    pub everything: bool,
    pub exclude: Vec<Excluded>,
    /// "gray", "black" or "soft".
    pub style: String,
    /// 0..1 warm tint, like Night light.
    pub warm: f32,
    /// "always", "windows" (when Windows uses dark mode), "times" or "sun"
    /// (from sunset to sunrise at `place`).
    pub schedule: String,
    /// "HH:MM", for schedule "times". May wrap past midnight.
    pub from: String,
    pub to: String,
    /// Start with Windows.
    pub autostart: bool,
    /// Run as administrator, through a scheduled task (admin.rs).
    pub admin: bool,
    /// "auto" (the same as Windows) or one of LANGS.
    pub language: String,
    /// Look for a new version now and then.
    pub updates: bool,
    pub keys: Keys,
    /// Screen sharing and screenshots show the real colours; only the user
    /// sees the dark ones.
    pub share_original: bool,
    /// On battery: "keep" going, draw "slow"er, or "pause".
    pub battery: String,
    pub place: Place,
    pub screens: Vec<Screen>,
    /// Paused from the tray until this time (seconds since 1970), 0 if not.
    pub paused_until: u64,
    /// The welcome steps were shown.
    pub welcomed: bool,
    /// The last version whose "What's new" was shown.
    pub seen_version: String,
    /// Faint text is lifted a little more, so it stays readable when dark.
    pub readable: bool,
    /// Screen sharing in real colours on its own while a meeting or
    /// streaming app shares (see engine::SHARE_APPS).
    pub share_auto: bool,
    /// While one of these runs (a game, a video editor), Deonyx pauses.
    pub pause_apps: Vec<Excluded>,
    pub profiles: Vec<Profile>,
    /// The profile last switched to, if any.
    pub profile: String,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            enabled: true,
            apps: Vec::new(),
            everything: false,
            exclude: Vec::new(),
            style: "gray".into(),
            warm: 0.0,
            schedule: "always".into(),
            from: "20:00".into(),
            to: "07:00".into(),
            autostart: false,
            admin: false,
            language: "auto".into(),
            updates: true,
            keys: Keys::default(),
            share_original: false,
            battery: "keep".into(),
            place: Place::default(),
            screens: Vec::new(),
            paused_until: 0,
            welcomed: false,
            seen_version: String::new(),
            readable: true,
            share_auto: true,
            pause_apps: Vec::new(),
            profiles: Vec::new(),
            profile: String::new(),
        }
    }
}

/// The languages Deonyx speaks.
pub const LANGS: &[&str] = &["en", "tr", "de", "fr", "es", "it", "pt", "ru", "uk", "nl", "ja", "ko", "zh"];

/// Seconds since 1970.
pub fn now_secs() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

impl Config {
    /// The parts of these settings a profile keeps.
    pub fn to_profile(&self, name: &str) -> Profile {
        Profile {
            name: name.to_string(),
            everything: self.everything,
            apps: self.apps.clone(),
            exclude: self.exclude.clone(),
            style: self.style.clone(),
            warm: self.warm,
            screens: self.screens.clone(),
            share_original: self.share_original,
            readable: self.readable,
        }
    }

    /// Changes made while a profile is in use belong to that profile.
    pub fn keep_profile(&mut self) {
        if self.profile.is_empty() {
            return;
        }
        let p = self.to_profile(&self.profile);
        match self.profiles.iter_mut().find(|q| q.name == p.name) {
            Some(q) => *q = p,
            None => self.profile.clear(),
        }
    }

    /// Switches to a profile. False if there's none of that name.
    pub fn use_profile(&mut self, name: &str) -> bool {
        let Some(p) = self.profiles.iter().find(|p| p.name.eq_ignore_ascii_case(name)).cloned() else { return false };
        self.everything = p.everything;
        self.apps = p.apps;
        self.exclude = p.exclude;
        self.style = p.style;
        self.warm = p.warm;
        self.screens = p.screens;
        self.share_original = p.share_original;
        self.readable = p.readable;
        self.profile = p.name;
        true
    }

    /// Paused from the tray right now.
    pub fn paused(&self) -> bool {
        self.paused_until > now_secs()
    }

    /// Language actually used, one of LANGS.
    pub fn lang(&self) -> &'static str {
        LANGS.iter().find(|l| **l == self.language).copied().unwrap_or_else(crate::win::system_language)
    }
}

/// DEONYX_CONFIG_DIR overrides the folder, so tests never touch the real one.
pub fn dir() -> PathBuf {
    if let Some(d) = std::env::var_os("DEONYX_CONFIG_DIR") {
        return PathBuf::from(d);
    }
    let base = std::env::var_os("APPDATA").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("."));
    base.join("Deonyx")
}

fn path() -> PathBuf {
    dir().join("config.json")
}

pub fn load() -> Config {
    let p = path();
    let Ok(text) = std::fs::read_to_string(&p) else { return Config::default() };
    match serde_json::from_str(&text) {
        Ok(c) => c,
        Err(e) => {
            // Keep the unreadable file instead of overwriting it with defaults.
            let aside = p.with_extension(format!("unreadable-{}.json", now_secs()));
            let _ = std::fs::copy(&p, aside);
            eprintln!("config.json could not be read: {e}");
            Config::default()
        }
    }
}

/// Writes to a temporary file first, so a crash never leaves half a file.
pub fn save(c: &Config) -> std::io::Result<()> {
    let p = path();
    std::fs::create_dir_all(dir())?;
    let tmp = p.with_extension("json.tmp");
    std::fs::write(&tmp, serde_json::to_string_pretty(c).unwrap_or_default())?;
    std::fs::rename(&tmp, &p)
}
