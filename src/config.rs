// Settings, kept in %APPDATA%\Deonyx\config.json.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

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
            icon: None,
        }
    }
}

impl AppRule {
    /// OKLab lightness a white background is mapped to.
    pub fn lmin(&self) -> f32 {
        0.40 - 0.28 * self.darkness.clamp(0.0, 1.0)
    }

    /// OKLab lightness black text is mapped to.
    pub fn lmax(&self) -> f32 {
        0.74 + 0.22 * self.text.clamp(0.0, 1.0)
    }

    pub fn key(&self) -> String {
        self.exe.to_ascii_lowercase()
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub enabled: bool,
    pub apps: Vec<AppRule>,
}

impl Default for Config {
    fn default() -> Self {
        Config { enabled: true, apps: Vec::new() }
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
            eprintln!("config.json okunamadi: {e}");
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
