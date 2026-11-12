use crate::actions::Action;
use crate::keys::MediaKey;
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, path::PathBuf};

/// 15 Tasten (3 breit, 5 tief) + 3 kleine Displays in der oberen Reihe.
pub const KEY_COUNT: usize = 18;

pub fn config_path() -> PathBuf {
    let base = dirs::config_dir().unwrap_or_else(|| PathBuf::from("."));
    base.join("n1deck").join("config.toml")
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(default)]
pub struct Config {
    pub brightness: u8,
    /// Sekunden zwischen Keep-Alive-Paketen, 0 = aus.
    pub keep_alive_secs: u64,
    pub start_page: String,
    /// Schriftart (.ttf) für Beschriftungen; leer = Systemschrift suchen.
    pub font: String,
    pub device: DeviceCfg,
    pub input: InputMap,
    pub obs: ObsConfig,
    pub knob: KnobCfg,
    pub pages: BTreeMap<String, Page>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(default)]
pub struct DeviceCfg {
    pub vid: u16,
    pub pid: u16,
}

/// Zuordnung Rohcode -> logische Eingabe. Wird mit `n1deck learn` erzeugt.
#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(default)]
pub struct InputMap {
    /// Position = Tastenindex (0..18), Wert = Rohcode (0 = nicht zugeordnet).
    pub button_codes: Vec<u8>,
    pub knob_press: u8,
    pub knob_cw: Twist,
    pub knob_ccw: Twist,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, Default, PartialEq, Eq)]
#[serde(default)]
pub struct Twist {
    pub code: u8,
    pub state: u8,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(default)]
pub struct ObsConfig {
    pub host: String,
    pub port: u16,
    pub password: String,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(default)]
pub struct KnobCfg {
    pub press: Action,
    pub cw: Action,
    pub ccw: Action,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(default)]
pub struct Page {
    /// Programmnamen (z. B. "ArcRaiders.exe"): läuft eins davon, wird die Seite automatisch aktiv.
    pub exe: Vec<String>,
    pub keys: Vec<KeyCfg>,
}

/// Kurzes Feedback auf dem Tasten-Display beim Drücken.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, Default, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum PressEffect {
    None,
    #[default]
    Brighten,
    Darken,
    Flash,
}

impl PressEffect {
    pub const ALL: [PressEffect; 4] = [Self::None, Self::Brighten, Self::Darken, Self::Flash];
    pub fn label(self) -> &'static str {
        match self {
            Self::None => "Keiner",
            Self::Brighten => "Aufhellen",
            Self::Darken => "Abdunkeln",
            Self::Flash => "Aufblitzen",
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(default)]
pub struct KeyCfg {
    pub key: usize,
    pub label: String,
    /// Pfad zu PNG/JPG, optional.
    pub icon: String,
    /// Hintergrund als #rrggbb.
    pub color: String,
    pub action: Action,
    pub effect: PressEffect,
    /// Ladeanimation, bis ein per Taste gestartetes Programm läuft.
    pub loading: bool,
}

impl Default for KeyCfg {
    fn default() -> Self {
        Self { key: 0, label: String::new(), icon: String::new(), color: String::new(), action: Action::None, effect: PressEffect::default(), loading: true }
    }
}

impl Default for DeviceCfg {
    fn default() -> Self {
        // Werte aus dem mirajazz-N1-Beispiel; mit `n1deck list` prüfen.
        Self { vid: 0x6603, pid: 0x1000 }
    }
}

impl Default for ObsConfig {
    fn default() -> Self {
        Self { host: "127.0.0.1".into(), port: 4455, password: String::new() }
    }
}

impl Default for KnobCfg {
    fn default() -> Self {
        Self {
            press: Action::Media { action: MediaKey::Mute },
            cw: Action::Media { action: MediaKey::VolUp },
            ccw: Action::Media { action: MediaKey::VolDown },
        }
    }
}

fn key(i: usize, label: &str, color: &str, action: Action) -> KeyCfg {
    KeyCfg { key: i, label: label.into(), icon: String::new(), color: color.into(), action, ..Default::default() }
}

impl Default for Config {
    fn default() -> Self {
        let mut pages = BTreeMap::new();
        pages.insert(
            "main".to_string(),
            Page {
                exe: vec![],
                keys: vec![
                    key(0, "Explorer", "#1f3a5f", Action::Run { program: "explorer.exe".into(), args: vec![] }),
                    key(1, "Rechner", "#3b3b3b", Action::Run { program: "calc.exe".into(), args: vec![] }),
                    key(2, "Zurück", "#2d5a3d", Action::Media { action: MediaKey::Prev }),
                    key(3, "Play", "#2d5a3d", Action::Media { action: MediaKey::PlayPause }),
                    key(4, "Weiter", "#2d5a3d", Action::Media { action: MediaKey::Next }),
                    key(14, "Seite 2", "#5a2d4a", Action::Page { to: "zwei".into() }),
                ],
            },
        );
        pages.insert(
            "zwei".to_string(),
            Page {
                exe: vec![],
                keys: vec![
                    key(0, "Start", "#7a1f1f", Action::Obs { request: "ToggleStream".into(), data: String::new() }),
                    key(1, "Rec", "#7a1f1f", Action::Obs { request: "ToggleRecord".into(), data: String::new() }),
                    key(14, "Zurück", "#5a2d4a", Action::Page { to: "main".into() }),
                ],
            },
        );
        Self {
            brightness: 60,
            keep_alive_secs: 10,
            start_page: "main".into(),
            font: String::new(),
            device: DeviceCfg::default(),
            input: InputMap::default(),
            obs: ObsConfig::default(),
            knob: KnobCfg::default(),
            pages,
        }
    }
}

impl Config {
    pub fn load() -> anyhow::Result<Config> {
        let path = config_path();
        let text = std::fs::read_to_string(&path)?;
        Ok(toml::from_str(&text)?)
    }

    /// Lädt die Config oder legt eine Standard-Config an.
    pub fn load_or_create() -> Config {
        match Config::load() {
            Ok(c) => c,
            Err(_) => {
                let c = Config::default();
                let _ = c.save();
                c
            }
        }
    }

    pub fn save(&self) -> anyhow::Result<()> {
        let path = config_path();
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        std::fs::write(path, toml::to_string_pretty(self)?)?;
        Ok(())
    }

    pub fn mtime() -> Option<std::time::SystemTime> {
        std::fs::metadata(config_path()).and_then(|m| m.modified()).ok()
    }

    pub fn key_on_page(&self, page: &str, key: usize) -> Option<&KeyCfg> {
        self.pages.get(page)?.keys.iter().find(|k| k.key == key)
    }
}
