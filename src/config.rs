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
    /// Areas the user drew to keep in real colours.
    pub areas: Vec<Area>,
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
            areas: Vec::new(),
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
    /// "always", "windows" (when Windows uses dark mode) or "times".
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
        }
    }
}

/// The languages Deonyx speaks.
pub const LANGS: &[&str] = &["en", "tr", "de", "fr", "es", "it", "pt", "ru", "uk", "nl", "ja", "ko", "zh"];

impl Config {
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
            let aside = p.with_extension(format!(
                "unreadable-{}.json",
                std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
            ));
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
