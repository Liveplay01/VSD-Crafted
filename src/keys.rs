//! Tastatur-Events: Parser für "ctrl+shift+m" und Senden per SendInput (Windows).
use anyhow::{bail, Result};
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum MediaKey {
    PlayPause,
    Next,
    Prev,
    Stop,
    Mute,
    VolUp,
    VolDown,
}

impl MediaKey {
    pub const ALL: [MediaKey; 7] = [
        MediaKey::PlayPause,
        MediaKey::Next,
        MediaKey::Prev,
        MediaKey::Stop,
        MediaKey::Mute,
        MediaKey::VolUp,
        MediaKey::VolDown,
    ];

    pub fn vk(self) -> u16 {
        match self {
            MediaKey::Mute => 0xAD,
            MediaKey::VolDown => 0xAE,
            MediaKey::VolUp => 0xAF,
            MediaKey::Next => 0xB0,
            MediaKey::Prev => 0xB1,
            MediaKey::Stop => 0xB2,
            MediaKey::PlayPause => 0xB3,
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
pub struct Chord {
    pub mods: Vec<u16>,
    pub key: u16,
}

pub fn parse_chord(s: &str) -> Result<Chord> {
    let mut mods = Vec::new();
    let mut key = None;
    for part in s.split('+').map(|p| p.trim().to_lowercase()) {
        let m = match part.as_str() {
            "ctrl" | "strg" | "control" => Some(0x11),
            "shift" | "umschalt" => Some(0x10),
            "alt" => Some(0x12),
            "win" | "meta" | "super" => Some(0x5B),
            _ => None,
        };
        if let Some(m) = m {
            mods.push(m);
            continue;
        }
        let vk = match part.as_str() {
            "enter" | "return" => 0x0D,
            "esc" | "escape" => 0x1B,
            "space" | "leertaste" => 0x20,
            "tab" => 0x09,
            "backspace" => 0x08,
            "delete" | "entf" => 0x2E,
            "insert" => 0x2D,
            "home" => 0x24,
            "end" => 0x23,
            "pageup" => 0x21,
            "pagedown" => 0x22,
            "left" => 0x25,
            "up" => 0x26,
            "right" => 0x27,
            "down" => 0x28,
            "printscreen" | "print" => 0x2C,
            p if p.len() == 1 && p.as_bytes()[0].is_ascii_alphanumeric() => {
                p.as_bytes()[0].to_ascii_uppercase() as u16
            }
            p if p.starts_with('f') && p[1..].parse::<u16>().map_or(false, |n| (1..=24).contains(&n)) => {
                0x6F + p[1..].parse::<u16>().unwrap()
            }
            other => bail!("Unbekannte Taste: {other}"),
        };
        if key.replace(vk).is_some() {
            bail!("Mehr als eine Haupttaste in '{s}'");
        }
    }
    match key {
        Some(key) => Ok(Chord { mods, key }),
        None => bail!("Keine Haupttaste in '{s}'"),
    }
}

#[cfg(windows)]
fn send_vk(vk: u16, up: bool) {
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::*;
    let input = INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: vk,
                wScan: 0,
                dwFlags: if up { KEYEVENTF_KEYUP } else { 0 },
                time: 0,
                dwExtraInfo: 0,
            },
        },
    };
    unsafe {
        SendInput(1, &input, std::mem::size_of::<INPUT>() as i32);
    }
}

#[cfg(not(windows))]
fn send_vk(vk: u16, up: bool) {
    eprintln!("[stub] vk=0x{vk:02X} up={up} (nur unter Windows aktiv)");
}

pub fn tap(vk: u16) {
    send_vk(vk, false);
    send_vk(vk, true);
}

pub fn send_chord(c: &Chord) {
    for m in &c.mods {
        send_vk(*m, false);
    }
    tap(c.key);
    for m in c.mods.iter().rev() {
        send_vk(*m, true);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_chord() {
        assert_eq!(parse_chord("ctrl+shift+m").unwrap(), Chord { mods: vec![0x11, 0x10], key: b'M' as u16 });
        assert_eq!(parse_chord("F5").unwrap().key, 0x74);
        assert_eq!(parse_chord("alt+f4").unwrap().key, 0x73);
        assert!(parse_chord("ctrl+").is_err());
        assert!(parse_chord("a+b").is_err());
    }
}
