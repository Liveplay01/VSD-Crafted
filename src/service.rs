//! Gerätezugriff und Hintergrunddienst.
use crate::actions::{self, Action};
use crate::config::{Config, InputMap, Twist, KEY_COUNT};
use crate::render;
use anyhow::{bail, Result};
use futures_lite::StreamExt;
use mirajazz::{
    device::{list_devices, new_hid_backend, Device, DeviceQuery},
    types::{DeviceInput, HidDeviceInfo, ImageFormat, ImageMirroring, ImageMode, ImageRotation},
};
use std::time::{Duration, Instant, SystemTime};
use tokio::sync::mpsc;

const USAGE_PAGE: u16 = 65440;
const USAGE_ID: u16 = 1;
/// Protokollversion 3, laut mirajazz-N1-Beispiel.
const PROTOCOL_VERSION: usize = 3;
/// Betriebsmodus "Software" (1 = Ziffernblock, 2 = Rechner).
const MODE_SOFTWARE: u8 = 3;

pub enum Control {
    Reload,
    Quit,
}

#[derive(Clone, Copy, Debug)]
pub struct Raw {
    pub code: u8,
    pub state: u8,
}

#[derive(Debug, PartialEq, Eq)]
enum Input {
    Key(usize),
    KnobPress,
    KnobCw,
    KnobCcw,
}

fn image_format(key: usize) -> ImageFormat {
    ImageFormat {
        mode: ImageMode::JPEG,
        size: if key >= 15 { (64, 64) } else { (96, 96) },
        rotation: ImageRotation::Rot0,
        mirror: ImageMirroring::None,
    }
}

fn classify(map: &InputMap, raw: Raw) -> Option<Input> {
    if raw.code == 0 {
        return None;
    }
    // Drehungen zuerst: Sie dürfen nicht als Tastendruck gelten.
    if map.knob_cw.code != 0 && raw.code == map.knob_cw.code && raw.state == map.knob_cw.state {
        return Some(Input::KnobCw);
    }
    if map.knob_ccw.code != 0 && raw.code == map.knob_ccw.code && raw.state == map.knob_ccw.state {
        return Some(Input::KnobCcw);
    }
    if raw.state == 0 {
        return None;
    }
    if map.knob_press != 0 && raw.code == map.knob_press {
        return Some(Input::KnobPress);
    }
    map.button_codes.iter().position(|c| *c == raw.code).map(Input::Key)
}

/// Bekannte VID/PID-Kombinationen des N1 (erste = aus der Config).
/// 6603:1000 stammt aus dem mirajazz-Beispiel, 5548:1038/1032 aus den VSD-Craft-Logs,
/// 5548:1002 ("HOTSPOTEKUSB HID DEMO", usage 65440/1) von `n1deck list` am echten Gerät.
const KNOWN_IDS: [(u16, u16); 4] = [(0x6603, 0x1000), (0x5548, 0x1038), (0x5548, 0x1032), (0x5548, 0x1002)];

pub async fn find_device(cfg: &Config) -> Option<mirajazz::types::HidDevice> {
    let mut ids = vec![(cfg.device.vid, cfg.device.pid)];
    ids.extend(KNOWN_IDS);
    let queries: Vec<DeviceQuery> = ids
        .into_iter()
        .map(|(v, p)| DeviceQuery::new(USAGE_PAGE, USAGE_ID, v, p))
        .collect();
    list_devices(&queries).await.ok()?.into_iter().next()
}

async fn open(info: &HidDeviceInfo) -> Result<Device> {
    let device = Device::connect(info, PROTOCOL_VERSION, KEY_COUNT, 0).await?;
    device.set_mode(MODE_SOFTWARE).await?;
    tokio::time::sleep(Duration::from_millis(50)).await;
    Ok(device)
}

/// Liest Rohpakete in einem eigenen Task und schickt sie über einen Kanal.
fn spawn_reader(device: &Device) -> mpsc::UnboundedReceiver<Result<Raw, String>> {
    let reader = device.get_reader(|_, _| Ok(DeviceInput::NoData));
    let (tx, rx) = mpsc::unbounded_channel();
    tokio::spawn(async move {
        loop {
            match reader.raw_read_data(512).await {
                Ok(data) => {
                    if std::env::var_os("N1DECK_DEBUG").is_some() { eprintln!("raw {:?}", &data[..data.len().min(16)]); }
                    if data.len() > 10 && data.starts_with(&[65, 67, 75]) {
                        let _ = tx.send(Ok(Raw { code: data[9], state: data[10] }));
                    }
                }
                Err(e) => {
                    let _ = tx.send(Err(e.to_string()));
                    break;
                }
            }
        }
    });
    rx
}

async fn draw_images(device: &Device, images: Vec<(usize, image::DynamicImage)>) -> Result<()> {
    device.clear_all_button_images().await?;
    for (i, img) in images {
        device.set_button_image(i as u8, image_format(i), img).await?;
    }
    device.flush().await?;
    Ok(())
}

/// Einzelne Taste neu zeichnen (nicht die ganze Seite: Komplett-Redraws ließen das Gerät abstürzen).
async fn put(device: &Device, key: usize, img: image::DynamicImage) -> Result<()> {
    device.set_button_image(key as u8, image_format(key), img).await?;
    device.flush().await?;
    Ok(())
}

const LOAD_TIMEOUT: Duration = Duration::from_secs(15);
const EFFECT_MS: u64 = 120;

/// Startet die Aktion ein Programm? `Some(Some(exe))` = bekannte Exe, `Some(None)` = unbekannt (z. B. .lnk).
fn launch_wait(a: &Action) -> Option<Option<String>> {
    let t = match a {
        Action::Run { program, .. } => program,
        Action::Open { target } => target,
        _ => return None,
    };
    let name = std::path::Path::new(t).file_name()?.to_string_lossy().to_lowercase();
    if name.ends_with(".exe") {
        Some(Some(name))
    } else if matches!(a, Action::Open { .. }) && [".lnk", ".bat", ".cmd", ".url"].iter().any(|e| name.ends_with(e)) {
        Some(None)
    } else {
        None
    }
}

/// Ladeanimation einer Taste, bis das gestartete Programm läuft.
struct Spin {
    page: String,
    base: image::DynamicImage,
    started: Instant,
    frame: usize,
    wait: Option<String>,
    baseline: std::collections::HashSet<String>,
}

impl Spin {
    fn done(&self, running: &std::collections::HashSet<String>) -> bool {
        let t = self.started.elapsed();
        if t >= LOAD_TIMEOUT {
            return true;
        }
        match &self.wait {
            // Lief die Exe schon vorher, gibt es kein neues Signal: nach kurzer Zeit aufhören.
            Some(n) => running.contains(n) && (!self.baseline.contains(n) || t > Duration::from_millis(1500)),
            None => t > Duration::from_secs(1) && running.iter().any(|p| !self.baseline.contains(p)),
        }
    }
}

async fn draw_page(device: &Device, cfg: &Config, page: &str) -> Result<()> {
    // Schrift nur kurz im Speicher halten.
    let font = render::load_font(cfg);
    let images = (0..KEY_COUNT)
        .map(|i| (i, render::render_key(cfg.key_on_page(page, i), i, font.as_ref())))
        .collect();
    drop(font);
    draw_images(device, images).await
}

enum End {
    Quit,
    Disconnected,
}

fn reload(cfg: &mut Config, mtime: &mut Option<SystemTime>) -> bool {
    match Config::load() {
        Ok(c) => {
            *cfg = c;
            *mtime = Config::mtime();
            true
        }
        Err(e) => {
            eprintln!("Config konnte nicht geladen werden: {e:#}");
            *mtime = Config::mtime();
            false
        }
    }
}

async fn session(
    info: &HidDeviceInfo,
    cfg: &mut Config,
    mtime: &mut Option<SystemTime>,
    ctl: &mut mpsc::UnboundedReceiver<Control>,
) -> Result<End> {
    let device = open(info).await?;
    let mut page = cfg.start_page.clone();
    device.set_brightness(cfg.brightness).await?;
    draw_page(&device, cfg, &page).await?;
    let mut rx = spawn_reader(&device);
    // Zählerstand pro Zirkel-Taste (Seite, Eingabe).
    let mut cycles: std::collections::HashMap<(String, usize), usize> = Default::default();

    // Seite, die zuletzt durch ein laufendes Programm gewählt wurde.
    let mut auto_page: Option<String> = None;

    let mut down: std::collections::HashSet<u8> = Default::default();
    // Druck-Effekte (zurücksetzen ab Zeitpunkt) und laufende Ladeanimationen.
    let mut restores: Vec<(usize, Instant, image::DynamicImage)> = Vec::new();
    let mut spins: std::collections::HashMap<usize, Spin> = Default::default();
    let mut anim = tokio::time::interval(Duration::from_millis(120));
    anim.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

    let mut tick = tokio::time::interval(Duration::from_secs(2));
    let mut last_keep_alive = Instant::now();

    loop {
        tokio::select! {
            ev = rx.recv() => {
                let raw = match ev {
                    Some(Ok(raw)) => raw,
                    Some(Err(e)) => { eprintln!("Lesefehler: {e}"); return Ok(End::Disconnected); }
                    None => return Ok(End::Disconnected),
                };
                // Das N1 verschluckt manchmal das Drücken (state=1) und meldet nur das Loslassen:
                // Ein Loslassen ohne vorheriges Drücken zählt deshalb als Druck.
                let mut input = classify(&cfg.input, raw);
                if input.is_some() && raw.state != 0 {
                    down.insert(raw.code);
                } else if input.is_none() && raw.state == 0 && !down.remove(&raw.code) {
                    input = classify(&cfg.input, Raw { state: 1, ..raw });
                }
                let action = match input {
                    Some(Input::Key(i)) => cfg.key_on_page(&page, i).map(|k| k.action.clone()),
                    Some(Input::KnobPress) => Some(cfg.knob.press.clone()),
                    Some(Input::KnobCw) => Some(cfg.knob.cw.clone()),
                    Some(Input::KnobCcw) => Some(cfg.knob.ccw.clone()),
                    None => None,
                };
                let action = match action {
                    Some(Action::Cycle { actions }) if !actions.is_empty() => {
                        let slot = match input {
                            Some(Input::Key(i)) => i,
                            Some(Input::KnobPress) => 100,
                            Some(Input::KnobCw) => 101,
                            _ => 102,
                        };
                        let n = cycles.entry((page.clone(), slot)).or_insert(0);
                        let a = actions[*n % actions.len()].clone();
                        *n += 1;
                        Some(a)
                    }
                    a => a,
                };
                // Visuelles Feedback auf der gedrückten Taste.
                if let (Some(Input::Key(i)), Some(a)) = (&input, &action) {
                    if !matches!(a, Action::Page { .. }) {
                        if let Some(k) = cfg.key_on_page(&page, *i).cloned() {
                            let wait = if k.loading { launch_wait(a) } else { None };
                            if k.effect != crate::config::PressEffect::None || wait.is_some() {
                                let font = render::load_font(cfg);
                                let base = render::render_key(Some(&k), *i, font.as_ref());
                                drop(font);
                                if let Some(wait) = wait {
                                    let spin = Spin { page: page.clone(), base, started: Instant::now(), frame: 0, wait, baseline: crate::procs::running() };
                                    put(&device, *i, render::spinner_frame(&spin.base, 0)).await?;
                                    restores.retain(|r| r.0 != *i);
                                    spins.insert(*i, spin);
                                } else if !spins.contains_key(i) {
                                    put(&device, *i, render::press_effect(&base, k.effect)).await?;
                                    restores.retain(|r| r.0 != *i);
                                    restores.push((*i, Instant::now() + Duration::from_millis(EFFECT_MS), base));
                                }
                            }
                        }
                    }
                }
                match action {
                    Some(Action::Page { to }) => {
                        if cfg.pages.contains_key(&to) {
                            page = to;
                            draw_page(&device, cfg, &page).await?;
                        } else {
                            eprintln!("Seite '{to}' existiert nicht");
                        }
                    }
                    Some(a) => {
                        let obs = cfg.obs.clone();
                        tokio::spawn(async move { actions::execute(&a, &obs).await });
                    }
                    None => {}
                }
            }
            c = ctl.recv() => match c {
                Some(Control::Reload) => {
                    if reload(cfg, mtime) {
                        if !cfg.pages.contains_key(&page) { page = cfg.start_page.clone(); }
                        device.set_brightness(cfg.brightness).await?;
                        draw_page(&device, cfg, &page).await?;
                    }
                }
                Some(Control::Quit) | None => {
                    let _ = device.shutdown().await;
                    return Ok(End::Quit);
                }
            },
            _ = anim.tick() => {
                let now = Instant::now();
                let mut i = 0;
                while i < restores.len() {
                    if restores[i].1 <= now {
                        let (k, _, base) = restores.swap_remove(i);
                        put(&device, k, base).await?;
                    } else {
                        i += 1;
                    }
                }
                if !spins.is_empty() {
                    let keys: Vec<usize> = spins.keys().copied().collect();
                    // Prozessliste nur alle 4 Frames (~0,5 s) prüfen.
                    let running = if spins.values().any(|s| s.frame % 4 == 3) { Some(crate::procs::running()) } else { None };
                    for k in keys {
                        let s = spins.get_mut(&k).unwrap();
                        if s.page != page {
                            spins.remove(&k);
                            continue;
                        }
                        s.frame += 1;
                        if s.frame % 4 == 0 && running.as_ref().map_or(false, |r| s.done(r)) || s.started.elapsed() >= LOAD_TIMEOUT {
                            let s = spins.remove(&k).unwrap();
                            put(&device, k, s.base).await?;
                        } else {
                            put(&device, k, render::spinner_frame(&s.base, s.frame)).await?;
                        }
                    }
                }
            }
            _ = tick.tick() => {
                if cfg.keep_alive_secs > 0 && last_keep_alive.elapsed().as_secs() >= cfg.keep_alive_secs {
                    device.keep_alive().await?;
                    last_keep_alive = Instant::now();
                }
                if cfg.pages.values().any(|p| !p.exe.is_empty()) {
                    let running = crate::procs::running();
                    let want = cfg.pages.iter()
                        .find(|(_, p)| p.exe.iter().any(|e| running.contains(&e.trim().to_lowercase())))
                        .map(|(n, _)| n.clone());
                    if want != auto_page {
                        auto_page = want.clone();
                        let target = want.unwrap_or_else(|| cfg.start_page.clone());
                        if target != page && cfg.pages.contains_key(&target) {
                            page = target;
                            draw_page(&device, cfg, &page).await?;
                        }
                    }
                }
                let now = Config::mtime();
                if now != *mtime {
                    if reload(cfg, mtime) {
                        if !cfg.pages.contains_key(&page) { page = cfg.start_page.clone(); }
                        device.set_brightness(cfg.brightness).await?;
                        draw_page(&device, cfg, &page).await?;
                    }
                }
            }
        }
    }
}

/// Hauptschleife des Dienstes: wartet auf das Gerät, hält die Sitzung, verbindet neu.
pub async fn run(mut ctl: mpsc::UnboundedReceiver<Control>) {
    let mut cfg = Config::load_or_create();
    let mut mtime = Config::mtime();
    loop {
        let info = loop {
            if let Some(i) = find_device(&cfg).await {
                break i;
            }
            match tokio::time::timeout(Duration::from_secs(2), ctl.recv()).await {
                Ok(Some(Control::Quit)) | Ok(None) => return,
                Ok(Some(Control::Reload)) => {
                    reload(&mut cfg, &mut mtime);
                }
                Err(_) => {
                    if Config::mtime() != mtime {
                        reload(&mut cfg, &mut mtime);
                    }
                }
            }
        };
        match session(&info, &mut cfg, &mut mtime, &mut ctl).await {
            Ok(End::Quit) => return,
            Ok(End::Disconnected) => {}
            Err(e) => eprintln!("Sitzung beendet: {e:#}"),
        }
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
}

// ---------------------------------------------------------------- Hilfsbefehle

/// Listet alle HID-Geräte (um VID/PID des N1 zu prüfen).
pub async fn list() -> Result<()> {
    let backend = new_hid_backend();
    let mut stream = backend.enumerate().await?;
    while let Some(d) = stream.next().await {
        println!(
            "{:04X}:{:04X}  usage {}/{}  {}",
            d.vendor_id, d.product_id, d.usage_page, d.usage_id, d.name
        );
    }
    Ok(())
}

async fn connect_for_tools() -> Result<(Device, mpsc::UnboundedReceiver<Result<Raw, String>>, Config)> {
    let cfg = Config::load_or_create();
    let Some(info) = find_device(&cfg).await else {
        bail!(
            "Kein Gerät mit {:04X}:{:04X} gefunden. `n1deck list` zeigt alle HID-Geräte; \
             VID/PID ggf. in der Config unter [device] anpassen. Läuft VSD Craft noch? Bitte beenden.",
            cfg.device.vid,
            cfg.device.pid
        );
    };
    let device = open(&info).await?;
    device.set_brightness(cfg.brightness).await?;
    let rx = spawn_reader(&device);
    Ok((device, rx, cfg))
}

/// Gibt alle Rohcodes aus.
pub async fn probe() -> Result<()> {
    let (device, mut rx, _) = connect_for_tools().await?;
    println!("Drücke Tasten / drehe den Regler. Beenden mit Strg+C.");
    let mut ka = tokio::time::interval(Duration::from_secs(10));
    loop {
        tokio::select! {
            ev = rx.recv() => match ev {
                Some(Ok(r)) => println!("code={:#04x} ({})  state={}", r.code, r.code, r.state),
                Some(Err(e)) => bail!("Lesefehler: {e}"),
                None => return Ok(()),
            },
            _ = ka.tick() => device.keep_alive().await?,
        }
    }
}

/// Wartet auf eine Eingabe und schickt dabei alle 5 s ein keep_alive: Ohne trennt das N1 nach ~30 s.
async fn wait_press(
    device: &Device,
    rx: &mut mpsc::UnboundedReceiver<Result<Raw, String>>,
    accept: impl Fn(Raw) -> bool,
) -> Result<Raw> {
    let start = Instant::now();
    loop {
        if start.elapsed() > Duration::from_secs(60) {
            bail!("Zeitüberschreitung (60 s) – abgebrochen");
        }
        match tokio::time::timeout(Duration::from_secs(5), rx.recv()).await {
            Ok(Some(Ok(r))) if accept(r) => return Ok(r),
            Ok(Some(Ok(_))) => {}
            Ok(Some(Err(e))) => bail!("Lesefehler: {e}"),
            Ok(None) => bail!("Gerät getrennt"),
            Err(_) => device.keep_alive().await?,
        }
    }
}

/// Geführtes Einlernen: Tasten zeigen ihre Nummer, du drückst sie der Reihe nach.
pub async fn learn() -> Result<()> {
    let (device, mut rx, mut cfg) = connect_for_tools().await?;
    let mut map = InputMap { button_codes: vec![0; KEY_COUNT], ..Default::default() };

    println!("Einlernen. Jede Taste zeigt jetzt ihre Nummer (0–17).");
    for i in 0..KEY_COUNT {
        // Nur die geänderten Tasten neu zeichnen: 18 Komplett-Redraws ließen das Gerät abstürzen.
        if i == 0 {
            let images = (0..KEY_COUNT).map(|k| (k, render::render_number(k, k, k == i))).collect();
            draw_images(&device, images).await?;
        } else {
            for k in [i - 1, i] {
                device.set_button_image(k as u8, image_format(k), render::render_number(k, k, k == i)).await?;
            }
            device.flush().await?;
        }
        println!("Drücke die Taste mit der orange hinterlegten Nummer {i} …");
        let used = map.button_codes.clone();
        let r = wait_press(&device, &mut rx, |r| r.state != 0 && r.code != 0 && !used.contains(&r.code)).await?;
        println!("  Taste {i} = Code {:#04x}", r.code);
        map.button_codes[i] = r.code;
        tokio::time::sleep(Duration::from_millis(300)).await; // Loslassen abwarten
        while rx.try_recv().is_ok() {}
    }

    cfg.input = map.clone();
    cfg.save()?; // Tasten sofort sichern, falls ein späterer Schritt scheitert.

    let used = map.button_codes.clone();
    println!("Drücke jetzt den Drehregler (Klick) … (15 s, sonst übersprungen)");
    match tokio::time::timeout(
        Duration::from_secs(15),
        wait_press(&device, &mut rx, |r| r.state != 0 && r.code != 0 && !used.contains(&r.code)),
    )
    .await
    {
        Ok(r) => {
            let r = r?;
            map.knob_press = r.code;
            println!("  Regler-Klick = Code {:#04x}", r.code);
        }
        Err(_) => println!("  Kein Klick erkannt – übersprungen (knob_press bleibt aus)."),
    }
    tokio::time::sleep(Duration::from_millis(300)).await;
    while rx.try_recv().is_ok() {}

    println!("Drehe den Regler EINEN Schritt nach rechts (im Uhrzeigersinn) …");
    let press = map.knob_press;
    let r = wait_press(&device, &mut rx, |r| r.code != 0 && !used.contains(&r.code) && (r.code != press || r.state > 1)).await?;
    map.knob_cw = Twist { code: r.code, state: r.state };
    println!("  rechts = Code {:#04x}, state {}", r.code, r.state);
    tokio::time::sleep(Duration::from_millis(400)).await;
    while rx.try_recv().is_ok() {}

    println!("Drehe den Regler EINEN Schritt nach links …");
    let cw = map.knob_cw;
    let r = wait_press(&device, &mut rx, |r| {
        r.code != 0 && !used.contains(&r.code) && !(r.code == cw.code && r.state == cw.state) && (r.code != press || r.state > 1)
    })
    .await?;
    map.knob_ccw = Twist { code: r.code, state: r.state };
    println!("  links = Code {:#04x}, state {}", r.code, r.state);

    cfg.input = map;
    cfg.save()?;
    let page = cfg.start_page.clone();
    draw_page(&device, &cfg, &page).await?;
    println!("Fertig. Zuordnung in {} gespeichert.", crate::config::config_path().display());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn map() -> InputMap {
        let mut codes = vec![0u8; KEY_COUNT];
        for (i, c) in codes.iter_mut().enumerate() {
            *c = i as u8 + 1;
        }
        InputMap {
            button_codes: codes,
            knob_press: 0x25,
            knob_cw: Twist { code: 0x91, state: 1 },
            knob_ccw: Twist { code: 0x90, state: 1 },
        }
    }

    #[test]
    fn classifies_inputs() {
        let m = map();
        assert_eq!(classify(&m, Raw { code: 3, state: 1 }), Some(Input::Key(2)));
        assert_eq!(classify(&m, Raw { code: 3, state: 0 }), None); // Loslassen
        assert_eq!(classify(&m, Raw { code: 0x25, state: 1 }), Some(Input::KnobPress));
        assert_eq!(classify(&m, Raw { code: 0x91, state: 1 }), Some(Input::KnobCw));
        assert_eq!(classify(&m, Raw { code: 0x90, state: 1 }), Some(Input::KnobCcw));
        assert_eq!(classify(&InputMap::default(), Raw { code: 3, state: 1 }), None);
    }
}
