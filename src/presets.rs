// Ready settings for well-known apps: how to find their 3D views, a darkness
// that suits them, render windows best left alone, and where an app's own
// dark mode is. presets.json from the repository is built in; with updates
// on, a newer copy comes from GitHub, so new apps arrive without a new
// version of Deonyx. Anyone can add one there.

use serde::{Deserialize, Serialize};

const BUILT_IN: &str = include_str!("../presets.json");
const URL: &str = "https://raw.githubusercontent.com/fdeox/deonyx/main/presets.json";

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Preset {
    /// Lower-case file name, like "sketchup.exe".
    pub exe: String,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub darkness: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub protect: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub photos: Option<bool>,
    /// Words in the class names or automation ids of its 3D views and
    /// canvases, for protect.rs.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub views: Vec<String>,
    /// Its windows are all picture (a render window): left alone in "every
    /// light window" mode.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub leave: bool,
    /// Where the app's own dark mode is switched on.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub native: Option<String>,
}

fn parse(text: &str) -> Option<Vec<Preset>> {
    let mut list: Vec<Preset> = serde_json::from_str(text).ok()?;
    list.retain(|p| !p.exe.is_empty());
    for p in &mut list {
        p.exe = p.exe.to_ascii_lowercase();
    }
    Some(list)
}

fn cache() -> std::path::PathBuf {
    crate::config::dir().join("presets.json")
}

/// The built-in list, with the downloaded one on top.
pub fn load() -> Vec<Preset> {
    let mut list = parse(BUILT_IN).unwrap_or_default();
    if let Some(newer) = std::fs::read_to_string(cache()).ok().and_then(|t| parse(&t)) {
        for p in newer {
            match list.iter_mut().find(|q| q.exe == p.exe) {
                Some(q) => *q = p,
                None => list.push(p),
            }
        }
    }
    list
}

/// Fetches the newest list from GitHub and keeps it. Blocks: call it off
/// the main thread. True if it got one.
pub fn refresh() -> bool {
    use windows::core::HSTRING;
    use windows::Win32::System::Com::Urlmon::URLDownloadToFileW;
    let tmp = crate::config::dir().join("presets.download");
    let _ = std::fs::create_dir_all(crate::config::dir());
    // The query keeps Windows' download cache from answering instead.
    let url = HSTRING::from(format!("{URL}?t={}", crate::config::now_secs()));
    let path = HSTRING::from(tmp.as_os_str());
    if unsafe { URLDownloadToFileW(None, &url, &path, 0, None) }.is_err() {
        return false;
    }
    let ok = std::fs::read_to_string(&tmp).ok().filter(|t| t.len() < 1 << 20 && parse(t).is_some()).is_some();
    if ok {
        let _ = std::fs::rename(&tmp, cache());
    } else {
        let _ = std::fs::remove_file(&tmp);
    }
    ok
}

/// Words for finding 3D views, from every preset.
pub fn view_words(list: &[Preset]) -> Vec<String> {
    let mut words: Vec<String> = list.iter().flat_map(|p| p.views.iter().map(|w| w.to_ascii_lowercase())).collect();
    words.sort();
    words.dedup();
    words
}

/// Apps whose windows are best left alone.
pub fn left_alone(list: &[Preset]) -> Vec<String> {
    list.iter().filter(|p| p.leave).map(|p| p.exe.clone()).collect()
}

#[cfg(test)]
mod tests {
    #[test]
    fn built_in_list_reads() {
        let list = super::parse(super::BUILT_IN).expect("presets.json");
        assert!(list.iter().any(|p| p.exe == "sketchup.exe"));
    }
}
