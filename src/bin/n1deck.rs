#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

use n1deck::service::{self, Control};
use std::process::Command;
use tokio::sync::mpsc;

const HELP: &str = "\
n1deck – schlanker Ersatz für VSD Craft (VSD/Basicolor N1)

  n1deck                Dienst starten (Tray-Icon, läuft im Hintergrund)
  n1deck list           alle HID-Geräte mit VID:PID anzeigen
  n1deck probe          Rohcodes von Tasten/Regler ausgeben
  n1deck learn          Tasten und Regler geführt einlernen
  n1deck config         Konfigurationsfenster öffnen
  n1deck autostart on|off   Start mit Windows ein-/ausschalten
";

fn runtime() -> tokio::runtime::Runtime {
    // block_in_place in mirajazz braucht den Multi-Thread-Scheduler; 1 Worker reicht.
    tokio::runtime::Builder::new_multi_thread()
        .worker_threads(1)
        .enable_all()
        .build()
        .expect("Tokio-Runtime")
}

#[cfg(windows)]
fn attach_console() {
    use windows_sys::Win32::System::Console::{AttachConsole, ATTACH_PARENT_PROCESS};
    unsafe {
        AttachConsole(ATTACH_PARENT_PROCESS);
    }
}

fn config_exe() -> std::path::PathBuf {
    let mut p = std::env::current_exe().unwrap_or_default();
    p.set_file_name(format!("n1deck-config{}", std::env::consts::EXE_SUFFIX));
    p
}

fn autostart(on: bool) -> anyhow::Result<()> {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const KEY: &str = r"HKCU\Software\Microsoft\Windows\CurrentVersion\Run";
        let mut cmd = Command::new("reg");
        if on {
            let exe = std::env::current_exe()?;
            cmd.args(["add", KEY, "/v", "n1deck", "/t", "REG_SZ", "/d", &format!("\"{}\"", exe.display()), "/f"]);
        } else {
            cmd.args(["delete", KEY, "/v", "n1deck", "/f"]);
        }
        let status = cmd.creation_flags(0x0800_0000).status()?;
        anyhow::ensure!(status.success(), "reg.exe meldete einen Fehler");
        println!("Autostart {}.", if on { "eingeschaltet" } else { "ausgeschaltet" });
        Ok(())
    }
    #[cfg(not(windows))]
    {
        let _ = on;
        anyhow::bail!("Autostart ist nur unter Windows umgesetzt")
    }
}

fn daemon() {
    let rt = runtime();
    let (tx, rx) = mpsc::unbounded_channel();

    #[cfg(windows)]
    {
        let worker = std::thread::spawn(move || rt.block_on(service::run(rx)));
        tray::run(tx, worker);
    }
    #[cfg(not(windows))]
    {
        let _keep = tx;
        rt.block_on(service::run(rx));
    }
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    #[cfg(windows)]
    if !args.is_empty() {
        attach_console();
    }

    let result: anyhow::Result<()> = match args.first().map(String::as_str) {
        None | Some("run") => {
            daemon();
            Ok(())
        }
        Some("list") => runtime().block_on(service::list()),
        Some("probe") => runtime().block_on(service::probe()),
        Some("learn") => runtime().block_on(service::learn()),
        Some("config") => Command::new(config_exe()).spawn().map(|_| ()).map_err(Into::into),
        Some("autostart") => match args.get(1).map(String::as_str) {
            Some("on") => autostart(true),
            Some("off") => autostart(false),
            _ => Err(anyhow::anyhow!("Benutzung: n1deck autostart on|off")),
        },
        _ => {
            print!("{HELP}");
            Ok(())
        }
    };
    if let Err(e) = result {
        eprintln!("Fehler: {e:#}");
        std::process::exit(1);
    }
    let _ = Control::Reload; // Typ im Binary referenziert (Tray nutzt ihn nur unter Windows)
}

#[cfg(windows)]
mod tray {
    use super::{config_exe, Control};
    use std::time::{Duration, Instant};
    use tao::event_loop::{ControlFlow, EventLoopBuilder};
    use tokio::sync::mpsc::UnboundedSender;
    use tray_icon::menu::{Menu, MenuEvent, MenuItem};
    use tray_icon::{Icon, TrayIconBuilder};

    fn icon() -> Icon {
        let img = image::load_from_memory(include_bytes!("../../assets/logo.png")).expect("Logo").resize(32, 32, image::imageops::FilterType::Lanczos3).to_rgba8();
        Icon::from_rgba(img.into_raw(), 32, 32).expect("Icon")
    }

    pub fn run(tx: UnboundedSender<Control>, worker: std::thread::JoinHandle<()>) {
        let event_loop = EventLoopBuilder::new().build();
        let menu = Menu::new();
        let item_cfg = MenuItem::new("n1deck öffnen …", true, None);
        let item_reload = MenuItem::new("Config neu laden", true, None);
        let item_quit = MenuItem::new("Beenden", true, None);
        menu.append_items(&[&item_cfg, &item_reload, &item_quit]).expect("Menü");
        let _tray = TrayIconBuilder::new()
            .with_menu(Box::new(menu))
            .with_tooltip("n1deck – N1 Stream Controller läuft")
            .with_icon(icon())
            .build()
            .expect("Tray-Icon");

        let (cfg_id, reload_id, quit_id) = (item_cfg.id().clone(), item_reload.id().clone(), item_quit.id().clone());
        let mut gui: Option<std::process::Child> = None;
        let mut worker = Some(worker);

        event_loop.run(move |_event, _target, flow| {
            *flow = ControlFlow::WaitUntil(Instant::now() + Duration::from_millis(250));
            while let Ok(ev) = MenuEvent::receiver().try_recv() {
                if ev.id == cfg_id {
                    let running = gui.as_mut().map_or(false, |c| matches!(c.try_wait(), Ok(None)));
                    if !running {
                        gui = std::process::Command::new(config_exe()).spawn().ok();
                    }
                } else if ev.id == reload_id {
                    let _ = tx.send(Control::Reload);
                } else if ev.id == quit_id {
                    let _ = tx.send(Control::Quit);
                    if let Some(w) = worker.take() {
                        let _ = w.join();
                    }
                    *flow = ControlFlow::Exit;
                }
            }
        });
    }
}
