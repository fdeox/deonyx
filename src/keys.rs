// Hotkeys, kept as text like "Ctrl+Alt+D" so the settings file stays
// readable, and turned into what RegisterHotKey wants.

use windows::Win32::UI::Input::KeyboardAndMouse::*;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Combo {
    pub mods: HOT_KEY_MODIFIERS,
    pub vk: u32,
}

/// "Ctrl+Alt+D" -> Ctrl, Alt and the D key. None for an empty or unusable
/// text. A plain letter without Ctrl, Alt or Win would steal it from typing,
/// so only F keys may stand alone.
pub fn parse(text: &str) -> Option<Combo> {
    let mut mods = HOT_KEY_MODIFIERS(0);
    let mut vk = None;
    for part in text.split('+').map(str::trim).filter(|p| !p.is_empty()) {
        match part.to_ascii_lowercase().as_str() {
            "ctrl" | "control" => mods |= MOD_CONTROL,
            "alt" => mods |= MOD_ALT,
            "shift" => mods |= MOD_SHIFT,
            "win" => mods |= MOD_WIN,
            k => vk = Some(key_code(k)?),
        }
    }
    let vk = vk?;
    let f_key = (0x70..=0x87).contains(&vk);
    if mods.0 & (MOD_CONTROL.0 | MOD_ALT.0 | MOD_WIN.0) == 0 && !f_key {
        return None;
    }
    Some(Combo { mods, vk })
}

fn key_code(k: &str) -> Option<u32> {
    let b = k.as_bytes();
    if b.len() == 1 && b[0].is_ascii_alphanumeric() {
        return Some(b[0].to_ascii_uppercase() as u32);
    }
    if let Some(n) = k.strip_prefix('f').and_then(|n| n.parse::<u32>().ok()) {
        return (1..=24).contains(&n).then_some(0x6F + n);
    }
    Some(match k {
        "space" => 0x20,
        "pause" => 0x13,
        "insert" => 0x2D,
        "delete" => 0x2E,
        "home" => 0x24,
        "end" => 0x23,
        "pageup" => 0x21,
        "pagedown" => 0x22,
        "left" => 0x25,
        "up" => 0x26,
        "right" => 0x27,
        "down" => 0x28,
        _ => return None,
    })
}

/// Whether the key of a combo is held down right now.
pub fn held(vk: u32) -> bool {
    unsafe { GetAsyncKeyState(vk as i32) as u16 & 0x8000 != 0 }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_hotkeys() {
        let c = parse("Ctrl+Alt+D").unwrap();
        assert_eq!(c.vk, b'D' as u32);
        assert_eq!(c.mods, MOD_CONTROL | MOD_ALT);
        assert_eq!(parse("ctrl + shift + f5").unwrap().vk, 0x74);
        assert_eq!(parse("F13").unwrap().vk, 0x7C);
        assert!(parse("").is_none());
        assert!(parse("Shift+D").is_none());
        assert!(parse("Ctrl+Alt+Ş").is_none());
    }
}
