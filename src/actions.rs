use crate::config::ObsConfig;
use crate::keys::{self, MediaKey};
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Action {
    #[default]
    None,
    /// Datei, Ordner, URL oder Programm mit dem Standardprogramm öffnen.
    Open { target: String },
    /// Programm mit Argumenten starten.
    Run {
        program: String,
        #[serde(default)]
        args: Vec<String>,
    },
    /// Tastenkombination, z. B. "ctrl+shift+m".
    Hotkey { keys: String },
    /// Medien-/Lautstärketaste.
    Media { action: MediaKey },
    /// Zu einer anderen Seite wechseln.
    Page { to: String },
    /// OBS-WebSocket-Request (z. B. ToggleRecord). `data` = JSON-String, optional.
    Obs {
        request: String,
        #[serde(default)]
        data: String,
    },
    /// Mehrere Aktionen nacheinander ausführen.
    Multi { actions: Vec<Action> },
    /// Bei jedem Druck die nächste Aktion der Liste (Zirkel/Umschalter).
    Cycle { actions: Vec<Action> },
}

impl Action {
    pub fn kind(&self) -> &'static str {
        match self {
            Action::None => "none",
            Action::Open { .. } => "open",
            Action::Run { .. } => "run",
            Action::Hotkey { .. } => "hotkey",
            Action::Media { .. } => "media",
            Action::Page { .. } => "page",
            Action::Obs { .. } => "obs",
            Action::Multi { .. } => "multi",
            Action::Cycle { .. } => "cycle",
        }
    }
}

/// Führt eine Aktion aus. Seitenwechsel behandelt der Dienst selbst.
pub async fn execute(action: &Action, obs: &ObsConfig) {
    let result: anyhow::Result<()> = async {
        match action {
            Action::None | Action::Page { .. } | Action::Cycle { .. } => {}
            Action::Multi { actions } => {
                for a in actions {
                    Box::pin(execute(a, obs)).await;
                    tokio::time::sleep(std::time::Duration::from_millis(60)).await;
                }
            }
            Action::Open { target } => open::that_detached(target)?,
            Action::Run { program, args } => {
                std::process::Command::new(program).args(args).spawn()?;
            }
            Action::Hotkey { keys: k } => keys::send_chord(&keys::parse_chord(k)?),
            Action::Media { action } => keys::tap(action.vk()),
            Action::Obs { request, data } => {
                let data = if data.trim().is_empty() { None } else { Some(data.as_str()) };
                crate::obs::request(obs, request, data).await?;
            }
        }
        Ok(())
    }
    .await;
    if let Err(e) = result {
        eprintln!("Aktion fehlgeschlagen ({action:?}): {e:#}");
    }
}
