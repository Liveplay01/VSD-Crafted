#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

use ab_glyph::FontVec;
use eframe::egui::{self, Color32, CornerRadius, RichText, Stroke, StrokeKind, TextureHandle};
use n1deck::actions::Action;
use n1deck::config::{Config, KeyCfg, Page, PressEffect, KEY_COUNT};
use n1deck::keys::MediaKey;
use n1deck::render::{key_size, load_font, parse_color, render_key};
use std::collections::HashMap;
use std::sync::mpsc::{channel, Receiver, Sender};

// Farbrollen (Light Theme, macOS-nah). Weiss auf ACCENT = 4.6:1, MUTED auf BG = 4.9:1.
const BG: Color32 = Color32::from_rgb(0xf5, 0xf5, 0xf7);
const SIDEBAR: Color32 = Color32::from_rgb(0xec, 0xec, 0xf0);
const SURFACE: Color32 = Color32::WHITE;
const BORDER: Color32 = Color32::from_rgb(0xdc, 0xdc, 0xe0);
const RAIL: Color32 = Color32::from_rgb(0xdc, 0xdc, 0xe0);
const TEXT: Color32 = Color32::from_rgb(0x1d, 0x1d, 0x1f);
const MUTED: Color32 = Color32::from_rgb(0x6e, 0x6e, 0x73);
const ACCENT: Color32 = Color32::from_rgb(0x00, 0x71, 0xe3);
const DANGER: Color32 = Color32::from_rgb(0xd7, 0x00, 0x15);

#[derive(Clone, Copy, PartialEq)]
enum Sel {
    Key(usize),
    KnobPress,
    KnobCw,
    KnobCcw,
}

struct App {
    cfg: Config,
    page: String,
    sel: Option<Sel>,
    new_page: String,
    status: String,
    font: Option<FontVec>,
    textures: HashMap<String, TextureHandle>,
    logo: TextureHandle,
    lib_open: bool,
    autostart: bool,
    lib_target: Option<(String, usize)>,
    lib_query: String,
    lib_msg: String,
    lib_results: Vec<String>,
    lib_thumbs: HashMap<String, TextureHandle>,
    ev_tx: Sender<Ev>,
    ev_rx: Receiver<Ev>,
}

/// Ergebnisse aus Hintergrund-Threads (Netzwerk blockiert nie die UI).
enum Ev {
    Search(Result<Vec<String>, String>),
    Thumb(String, egui::ColorImage),
    Picked(Result<String, String>),
}

fn apply_theme(ctx: &egui::Context) {
    let mut v = egui::Visuals::light();
    v.panel_fill = BG;
    v.window_fill = SURFACE;
    v.extreme_bg_color = SURFACE;
    v.override_text_color = Some(TEXT);
    v.selection.bg_fill = ACCENT;
    v.selection.stroke = Stroke::new(1.0, Color32::WHITE);
    let r = CornerRadius::same(6);
    for w in [&mut v.widgets.noninteractive, &mut v.widgets.inactive, &mut v.widgets.hovered, &mut v.widgets.active] {
        w.corner_radius = r;
    }
    v.widgets.noninteractive.bg_stroke = Stroke::new(1.0, BORDER);
    v.widgets.inactive.bg_fill = RAIL; // Slider-Schiene
    v.widgets.inactive.weak_bg_fill = SURFACE;
    v.widgets.inactive.bg_stroke = Stroke::new(1.0, BORDER);
    v.widgets.hovered.bg_fill = Color32::from_rgb(0xd0, 0xd0, 0xd6);
    v.widgets.hovered.weak_bg_fill = Color32::from_rgb(0xf0, 0xf0, 0xf3);
    v.widgets.hovered.bg_stroke = Stroke::new(1.0, Color32::from_rgb(0xc4, 0xc4, 0xca));
    v.widgets.active.bg_fill = Color32::from_rgb(0xc4, 0xc4, 0xca);
    v.widgets.active.weak_bg_fill = Color32::from_rgb(0xe4, 0xe4, 0xe8);
    ctx.set_visuals(v);
    ctx.style_mut(|s| {
        s.spacing.item_spacing = egui::vec2(8.0, 8.0);
        s.spacing.button_padding = egui::vec2(12.0, 4.0);
        s.spacing.interact_size.y = 28.0;
        s.spacing.slider_width = 140.0;
        s.text_styles.insert(egui::TextStyle::Body, egui::FontId::proportional(13.0));
        s.text_styles.insert(egui::TextStyle::Button, egui::FontId::proportional(13.0));
        s.text_styles.insert(egui::TextStyle::Small, egui::FontId::proportional(11.0));
        s.text_styles.insert(egui::TextStyle::Heading, egui::FontId::proportional(17.0));
    });
}

fn section(ui: &mut egui::Ui, text: &str) {
    ui.label(RichText::new(text).size(11.0).color(MUTED));
}

/// Eingerueckte Gruppe wie in macOS-Einstellungen.
fn group<R>(ui: &mut egui::Ui, add: impl FnOnce(&mut egui::Ui) -> R) -> R {
    egui::Frame::new()
        .fill(SURFACE)
        .stroke(Stroke::new(1.0, BORDER))
        .corner_radius(10)
        .inner_margin(12)
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            add(ui)
        })
        .inner
}

fn primary(text: &str) -> egui::Button<'static> {
    egui::Button::new(RichText::new(text).color(Color32::WHITE)).fill(ACCENT).stroke(Stroke::NONE)
}

impl App {
    fn new(cc: &eframe::CreationContext<'_>) -> Self {
        apply_theme(&cc.egui_ctx);
        let cfg = Config::load_or_create();
        let page = if cfg.pages.contains_key(&cfg.start_page) {
            cfg.start_page.clone()
        } else {
            cfg.pages.keys().next().cloned().unwrap_or_else(|| "main".into())
        };
        let font = load_font(&cfg);
        let (ev_tx, ev_rx) = channel();
        let logo = {
            let w = window_icon();
            cc.egui_ctx.load_texture("logo", egui::ColorImage::from_rgba_unmultiplied([w.width as usize, w.height as usize], &w.rgba), egui::TextureOptions::LINEAR)
        };
        Self { cfg, page, sel: None, new_page: String::new(), status: String::new(), font,
            textures: HashMap::new(),
            logo,
            lib_open: false,
            autostart: autostart_enabled(),
            lib_target: None,
            lib_query: String::new(),
            lib_msg: String::new(),
            lib_results: vec![],
            lib_thumbs: HashMap::new(),
            ev_tx,
            ev_rx,
        }
    }

    fn save(&mut self) {
        for p in self.cfg.pages.values_mut() {
            p.keys.retain(|k| !(k.label.is_empty() && k.icon.is_empty() && k.color.is_empty() && k.action == Action::None));
            p.keys.sort_by_key(|k| k.key);
        }
        self.status = match self.cfg.save() {
            Ok(()) => "Gespeichert – der Dienst übernimmt die Änderung automatisch.".into(),
            Err(e) => format!("Speichern fehlgeschlagen: {e:#}"),
        };
    }

    /// Vorschau = exakt das Bild, das auf die Taste gesendet wird.
    fn texture(&mut self, ctx: &egui::Context, i: usize) -> TextureHandle {
        let k = self.cfg.key_on_page(&self.page, i).cloned();
        let sig = format!("{i}|{k:?}");
        if let Some(t) = self.textures.get(&sig) {
            return t.clone();
        }
        let img = render_key(k.as_ref(), i, self.font.as_ref()).to_rgb8();
        let size = [img.width() as usize, img.height() as usize];
        let t = ctx.load_texture(&sig, egui::ColorImage::from_rgb(size, img.as_raw()), egui::TextureOptions::LINEAR);
        if self.textures.len() > 400 {
            self.textures.clear();
        }
        self.textures.insert(sig, t.clone());
        t
    }

    fn key_button(&mut self, ui: &mut egui::Ui, i: usize) {
        let tex = self.texture(ui.ctx(), i);
        let (w, h) = key_size(i);
        let (rect, resp) = ui.allocate_exact_size(egui::vec2(w as f32, h as f32), egui::Sense::click());
        let radius = CornerRadius::same(10);
        let empty = self.cfg.key_on_page(&self.page, i).is_none();
        if empty {
            ui.painter().rect_filled(rect, radius, SURFACE);
            ui.painter().text(rect.center(), egui::Align2::CENTER_CENTER, i.to_string(), egui::FontId::proportional(13.0), MUTED);
            ui.painter().rect_stroke(rect, radius, Stroke::new(1.0, BORDER), StrokeKind::Inside);
        } else {
            let uv = egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0));
            egui::Image::new((tex.id(), rect.size())).corner_radius(radius).uv(uv).paint_at(ui, rect);
        }
        if self.sel == Some(Sel::Key(i)) {
            ui.painter().rect_stroke(rect.expand(3.0), CornerRadius::same(12), Stroke::new(2.0, ACCENT), StrokeKind::Outside);
        } else if resp.hovered() {
            ui.painter().rect_stroke(rect, radius, Stroke::new(1.5, Color32::from_rgb(0xa0, 0xa0, 0x9a)), StrokeKind::Inside);
        }
        if resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
            self.sel = Some(Sel::Key(i));
        }
    }
}

fn key_mut(page: &mut Page, idx: usize) -> &mut KeyCfg {
    if let Some(pos) = page.keys.iter().position(|k| k.key == idx) {
        &mut page.keys[pos]
    } else {
        page.keys.push(KeyCfg { key: idx, ..Default::default() });
        page.keys.last_mut().unwrap()
    }
}

/// Beliebiges Bild (auch WebP, GIF, SVG …) wird als PNG in den Config-Ordner importiert.
fn import_icon() -> Result<Option<String>, String> {
    let Some(src) = rfd::FileDialog::new()
        .add_filter("Bilder", &["png", "jpg", "jpeg", "webp", "gif", "bmp", "ico", "svg"])
        .pick_file()
    else {
        return Ok(None);
    };
    n1deck::iconlib::import_file(&src).map(Some).map_err(|e| format!("{e:#}"))
}

const KINDS: [(&str, &str); 9] = [
    ("none", "Nichts"),
    ("open", "Öffnen (Datei/Ordner/URL)"),
    ("run", "Programm starten"),
    ("hotkey", "Tastenkombination"),
    ("media", "Medien / Lautstärke"),
    ("page", "Seite wechseln"),
    ("obs", "OBS-Befehl"),
    ("multi", "Mehrere nacheinander"),
    ("cycle", "Zirkel (bei jedem Druck die nächste)"),
];

// Discord kennt keine festen globalen Shortcuts: dieselben Kombinationen
// müssen unter Discord → Einstellungen → Tastenkombinationen eingetragen werden.
const DISCORD: [(&str, &str); 3] = [
    ("Mikrofon stumm", "ctrl+shift+m"),
    ("Kopfhörer stumm", "ctrl+shift+d"),
    ("Overlay", "ctrl+shift+o"),
];

/// Startmenue-Verknuepfungen (Name, Pfad), einmal beim ersten Oeffnen gescannt.
fn start_menu_apps() -> &'static [(String, String)] {
    static APPS: std::sync::OnceLock<Vec<(String, String)>> = std::sync::OnceLock::new();
    APPS.get_or_init(|| {
        fn walk(dir: &std::path::Path, depth: u32, out: &mut Vec<(String, String)>) {
            let Ok(rd) = std::fs::read_dir(dir) else { return };
            for e in rd.flatten() {
                let p = e.path();
                if p.is_dir() {
                    if depth < 3 {
                        walk(&p, depth + 1, out);
                    }
                } else if p.extension().is_some_and(|x| x.eq_ignore_ascii_case("lnk")) {
                    let name = p.file_stem().unwrap_or_default().to_string_lossy().into_owned();
                    let l = name.to_lowercase();
                    if !l.contains("uninstall") && !l.contains("deinstall") {
                        out.push((name, p.to_string_lossy().into_owned()));
                    }
                }
            }
        }
        let mut v = Vec::new();
        for var in ["ProgramData", "APPDATA"] {
            if let Some(b) = std::env::var_os(var) {
                walk(&std::path::PathBuf::from(b).join(r"Microsoft\Windows\Start Menu\Programs"), 0, &mut v);
            }
        }
        v.sort_by_key(|(n, _)| n.to_lowercase());
        v.dedup_by(|a, b| a.0.eq_ignore_ascii_case(&b.0));
        v
    })
}

/// Menue mit Suchfeld; liefert den gewaehlten Eintrag (Anzeige, Wert).
fn pick_menu(ui: &mut egui::Ui, title: &str, items: &[(String, String)]) -> Option<String> {
    let mut picked = None;
    ui.menu_button(title, |ui| {
        let id = ui.id().with("filter");
        let mut q: String = ui.memory_mut(|m| m.data.get_temp(id).unwrap_or_default());
        ui.add(egui::TextEdit::singleline(&mut q).hint_text("Suchen…").desired_width(220.0));
        ui.memory_mut(|m| m.data.insert_temp(id, q.clone()));
        let ql = q.to_lowercase();
        egui::ScrollArea::vertical().max_height(260.0).show(ui, |ui| {
            let mut n = 0;
            for (name, val) in items.iter().filter(|(n, _)| ql.is_empty() || n.to_lowercase().contains(&ql)) {
                n += 1;
                if ui.button(name).clicked() {
                    picked = Some(val.clone());
                    ui.close_menu();
                }
            }
            if n == 0 {
                ui.label(RichText::new("Nichts gefunden").color(MUTED));
            }
        });
    });
    picked
}

fn edit_action(ui: &mut egui::Ui, action: &mut Action, pages: &[String], nested: bool) {
    let current = KINDS.iter().find(|(k, _)| *k == action.kind()).map(|(_, l)| *l).unwrap_or("?");
    let mut chosen = action.kind();
    egui::ComboBox::from_id_salt("kind").width(260.0).selected_text(current).show_ui(ui, |ui| {
        for (k, label) in KINDS {
            if nested && (k == "multi" || k == "cycle") {
                continue;
            }
            ui.selectable_value(&mut chosen, k, label);
        }
    });
    if chosen != action.kind() {
        *action = match chosen {
            "open" => Action::Open { target: String::new() },
            "run" => Action::Run { program: String::new(), args: vec![] },
            "hotkey" => Action::Hotkey { keys: String::new() },
            "media" => Action::Media { action: MediaKey::PlayPause },
            "page" => Action::Page { to: pages.first().cloned().unwrap_or_default() },
            "obs" => Action::Obs { request: "ToggleRecord".into(), data: String::new() },
            "multi" => Action::Multi { actions: vec![Action::None] },
            "cycle" => Action::Cycle { actions: vec![Action::None, Action::None] },
            _ => Action::None,
        };
    }
    let mut switch: Option<Action> = None;
    match action {
        Action::None => {}
        Action::Open { target } => {
            ui.label("Ziel (Pfad oder https://…)");
            ui.add(egui::TextEdit::singleline(target).hint_text("Datei, Ordner, App oder Webadresse").desired_width(f32::INFINITY));
            ui.horizontal(|ui| {
                if ui.button("Datei…").clicked() {
                    if let Some(f) = rfd::FileDialog::new().pick_file() {
                        *target = f.to_string_lossy().into_owned();
                    }
                }
                if ui.button("Ordner…").clicked() {
                    if let Some(f) = rfd::FileDialog::new().pick_folder() {
                        *target = f.to_string_lossy().into_owned();
                    }
                }
                let apps: Vec<(String, String)> = start_menu_apps().to_vec();
                if let Some(lnk) = pick_menu(ui, "Installierte Apps…", &apps) {
                    *target = lnk;
                }
            });
        }
        Action::Run { program, args } => {
            ui.label("Programm");
            ui.add(egui::TextEdit::singleline(program).hint_text("Pfad zur .exe wählen oder eintippen").desired_width(f32::INFINITY));
            ui.horizontal(|ui| {
                if ui.button("Durchsuchen…").clicked() {
                    if let Some(f) = rfd::FileDialog::new().add_filter("Programme", &["exe", "bat", "cmd"]).pick_file() {
                        *program = f.to_string_lossy().into_owned();
                    }
                }
                let apps: Vec<(String, String)> = start_menu_apps().to_vec();
                if let Some(lnk) = pick_menu(ui, "Installierte Apps…", &apps) {
                    switch = Some(Action::Open { target: lnk });
                }
            });
            ui.label("Argumente (durch Leerzeichen getrennt)");
            let mut joined = args.join(" ");
            if ui.add(egui::TextEdit::singleline(&mut joined).desired_width(f32::INFINITY)).changed() {
                *args = joined.split_whitespace().map(String::from).collect();
            }
        }
        Action::Hotkey { keys } => {
            ui.label("z. B. ctrl+shift+m, alt+f4, win+d");
            ui.add(egui::TextEdit::singleline(keys).desired_width(f32::INFINITY));
            if !keys.is_empty() {
                if let Err(e) = n1deck::keys::parse_chord(keys) {
                    ui.colored_label(DANGER, format!("{e}"));
                }
            }
            ui.label(RichText::new("Discord-Vorlagen").size(12.0).color(MUTED));
            ui.horizontal_wrapped(|ui| {
                for (label, chord) in DISCORD {
                    if ui.button(label).clicked() {
                        *keys = chord.into();
                    }
                }
            });
            ui.label(RichText::new("Dieselbe Kombination in Discord unter Einstellungen → Tastenkombinationen eintragen.").size(12.0).color(MUTED));
        }
        Action::Media { action } => {
            let label = |m: MediaKey| match m {
                MediaKey::PlayPause => "Play/Pause",
                MediaKey::Next => "Nächster Titel",
                MediaKey::Prev => "Voriger Titel",
                MediaKey::Stop => "Stopp",
                MediaKey::Mute => "Stumm",
                MediaKey::VolUp => "Lauter",
                MediaKey::VolDown => "Leiser",
            };
            egui::ComboBox::from_id_salt("media").width(260.0).selected_text(label(*action)).show_ui(ui, |ui| {
                for m in MediaKey::ALL {
                    ui.selectable_value(action, m, label(m));
                }
            });
        }
        Action::Page { to } => {
            egui::ComboBox::from_id_salt("page").width(260.0).selected_text(to.as_str()).show_ui(ui, |ui| {
                for p in pages {
                    ui.selectable_value(to, p.clone(), p);
                }
            });
        }
        Action::Obs { request, data } => {
            ui.label("Request (z. B. ToggleStream, ToggleRecord, SetCurrentProgramScene)");
            ui.add(egui::TextEdit::singleline(request).desired_width(f32::INFINITY));
            ui.label("Daten als JSON (optional), z. B. {\"sceneName\":\"Kamera\"}");
            ui.add(egui::TextEdit::singleline(data).desired_width(f32::INFINITY));
        }
        Action::Multi { .. } | Action::Cycle { .. } => {
            let is_cycle = matches!(action, Action::Cycle { .. });
            let actions = match action {
                Action::Multi { actions } | Action::Cycle { actions } => actions,
                _ => unreachable!(),
            };
            ui.label(RichText::new(if is_cycle { "Reihenfolge der Druck-Schritte" } else { "Ablauf" }).size(12.0).color(MUTED));
            let mut remove = None;
            for (n, a) in actions.iter_mut().enumerate() {
                egui::Frame::new().fill(SURFACE).stroke(Stroke::new(1.0, BORDER)).corner_radius(8).inner_margin(12).show(ui, |ui| {
                    ui.push_id(n, |ui| {
                        ui.horizontal(|ui| {
                            ui.label(RichText::new(format!("{}.", n + 1)).color(MUTED));
                            if ui.small_button("Entfernen").clicked() {
                                remove = Some(n);
                            }
                        });
                        edit_action(ui, a, pages, true);
                    });
                });
            }
            if let Some(n) = remove {
                actions.remove(n);
            }
            if ui.button("+ Schritt").clicked() {
                actions.push(Action::None);
            }
        }
    }
    if let Some(a) = switch {
        *action = a;
    }
}

impl App {
    fn start_search(&mut self) {
        let q = self.lib_query.trim().to_string();
        if q.is_empty() {
            return;
        }
        self.lib_msg = "Suche …".into();
        let tx = self.ev_tx.clone();
        std::thread::spawn(move || {
            let _ = tx.send(Ev::Search(n1deck::iconlib::search(&q).map_err(|e| format!("{e:#}"))));
        });
    }

    fn pick(&mut self, name: String) {
        self.lib_msg = format!("Lade {name} …");
        let tx = self.ev_tx.clone();
        std::thread::spawn(move || {
            let _ = tx.send(Ev::Picked(n1deck::iconlib::download(&name).map_err(|e| format!("{e:#}"))));
        });
    }

    fn handle_events(&mut self, ctx: &egui::Context) {
        while let Ok(ev) = self.ev_rx.try_recv() {
            match ev {
                Ev::Search(Ok(list)) => {
                    self.lib_msg = if list.is_empty() { "Nichts gefunden.".into() } else { String::new() };
                    for n in list.iter().filter(|n| !self.lib_thumbs.contains_key(*n)) {
                        let (tx, n, ctx) = (self.ev_tx.clone(), n.clone(), ctx.clone());
                        std::thread::spawn(move || {
                            if let Ok(img) = n1deck::iconlib::fetch_rgba(&n, 64) {
                                let ci = egui::ColorImage::from_rgba_unmultiplied([64, 64], img.as_raw());
                                let _ = tx.send(Ev::Thumb(n, ci));
                                ctx.request_repaint();
                            }
                        });
                    }
                    self.lib_results = list;
                }
                Ev::Search(Err(e)) => self.lib_msg = format!("Offline? {e}"),
                Ev::Thumb(n, ci) => {
                    let t = ctx.load_texture(format!("lib-{n}"), ci, egui::TextureOptions::LINEAR);
                    self.lib_thumbs.insert(n, t);
                }
                Ev::Picked(Ok(path)) => {
                    if let Some((page, i)) = self.lib_target.clone() {
                        if let Some(p) = self.cfg.pages.get_mut(&page) {
                            key_mut(p, i).icon = path;
                        }
                    }
                    self.lib_msg = "Icon übernommen. Mit Speichern aktivieren.".into();
                }
                Ev::Picked(Err(e)) => self.lib_msg = format!("Download fehlgeschlagen: {e}"),
            }
        }
    }

    fn library_window(&mut self, ctx: &egui::Context) {
        if !self.lib_open {
            return;
        }
        let mut open = true;
        let mut pick = None;
        let mut search = false;
        egui::Window::new("Icon-Bibliothek").open(&mut open).default_size([520.0, 460.0]).collapsible(false).show(ctx, |ui| {
            ui.horizontal(|ui| {
                let r = ui.add(egui::TextEdit::singleline(&mut self.lib_query).hint_text("Suchen, z. B. discord, home, music").desired_width(340.0));
                if (ui.add(primary("Suchen")).clicked() || (r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)))) && !self.lib_query.trim().is_empty() {
                    search = true;
                }
            });
            ui.label(RichText::new("Quelle: Iconify (Open-Source-Icons). Ein Klick lädt das Icon herunter und setzt es auf die Taste.").size(11.0).color(MUTED));
            if !self.lib_msg.is_empty() {
                ui.label(RichText::new(&self.lib_msg).color(MUTED));
            }
            egui::ScrollArea::vertical().show(ui, |ui| {
                ui.horizontal_wrapped(|ui| {
                    for n in &self.lib_results {
                        let (rect, resp) = ui.allocate_exact_size(egui::vec2(56.0, 56.0), egui::Sense::click());
                        ui.painter().rect_filled(rect, CornerRadius::same(8), Color32::from_rgb(0x2a, 0x2a, 0x2e));
                        if let Some(t) = self.lib_thumbs.get(n) {
                            egui::Image::new((t.id(), egui::vec2(40.0, 40.0))).paint_at(ui, rect.shrink(8.0));
                        }
                        if resp.hovered() {
                            ui.painter().rect_stroke(rect, CornerRadius::same(8), Stroke::new(2.0, ACCENT), StrokeKind::Inside);
                        }
                        if resp.on_hover_text(n.as_str()).on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                            pick = Some(n.clone());
                        }
                    }
                });
            });
        });
        self.lib_open = open;
        if search {
            self.start_search();
        }
        if let Some(n) = pick {
            self.pick(n);
        }
    }
}

impl eframe::App for App {
    fn clear_color(&self, _: &egui::Visuals) -> [f32; 4] {
        [0xf5 as f32 / 255.0, 0xf5 as f32 / 255.0, 0xf7 as f32 / 255.0, 1.0]
    }

    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.handle_events(ctx);
        let page_names: Vec<String> = self.cfg.pages.keys().cloned().collect();

        egui::TopBottomPanel::top("top")
            .frame(egui::Frame::new().fill(SURFACE).stroke(Stroke::new(1.0, BORDER)).inner_margin(egui::Margin::symmetric(16, 12)))
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.add(egui::Image::new(&self.logo).fit_to_exact_size(egui::vec2(28.0, 28.0)));
                    ui.label(RichText::new("n1deck").heading().strong());
                    ui.label(RichText::new("N1 Stream Controller").color(MUTED));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.add(primary("Speichern")).clicked() {
                            self.save();
                        }
                        ui.add_space(8.0);
                        if ui.checkbox(&mut self.autostart, "Mit Windows starten").changed() {
                            match set_autostart(self.autostart) {
                                Ok(()) => self.status = if self.autostart { "Autostart an".into() } else { "Autostart aus".into() },
                                Err(e) => {
                                    self.autostart = autostart_enabled();
                                    self.status = e;
                                }
                            }
                        }
                        ui.add_space(8.0);
                        ui.add(egui::Slider::new(&mut self.cfg.brightness, 0..=100).suffix(" %"));
                        ui.label(RichText::new("Helligkeit").color(MUTED));
                        if !self.status.is_empty() {
                            ui.add_space(8.0);
                            ui.label(RichText::new(&self.status).size(12.0).color(MUTED));
                        }
                    });
                });
                if self.cfg.input.button_codes.len() < KEY_COUNT || self.cfg.input.button_codes.iter().all(|c| *c == 0) {
                    ui.colored_label(DANGER, "Tasten noch nicht eingelernt: einmal `n1deck learn` im Terminal ausführen.");
                }
            });

        egui::SidePanel::left("pages")
            .exact_width(200.0)
            .resizable(false)
            .show_separator_line(false)
            .frame(egui::Frame::new().fill(SIDEBAR).stroke(Stroke::new(1.0, BORDER)).inner_margin(12))
            .show(ctx, |ui| {
                section(ui, "Seiten");
                for p in &page_names {
                    let label = if *p == self.cfg.start_page { format!("{p}  ·  Start") } else { p.clone() };
                    let sel = self.page == *p;
                    let text = if sel { RichText::new(label).color(Color32::WHITE) } else { RichText::new(label) };
                    let b = egui::Button::new(text)
                        .fill(if sel { ACCENT } else { Color32::TRANSPARENT })
                        .stroke(Stroke::NONE)
                        .min_size(egui::vec2(ui.available_width(), 28.0));
                    if ui.add(b).clicked() {
                        self.page = p.clone();
                    }
                }
                ui.horizontal(|ui| {
                    let w = ui.available_width() - 44.0;
                    let r = ui.add(egui::TextEdit::singleline(&mut self.new_page).hint_text("Neue Seite").desired_width(w));
                    let enter = r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                    if (ui.button("+").clicked() || enter) && !self.new_page.trim().is_empty() {
                        let name = self.new_page.trim().to_string();
                        self.cfg.pages.entry(name.clone()).or_default();
                        self.page = name;
                        self.new_page.clear();
                    }
                });
                ui.add_space(16.0);
                section(ui, "Diese Seite");
                let page = self.cfg.pages.entry(self.page.clone()).or_default();
                let mut joined = page.exe.join(", ");
                ui.label("Automatisch bei Programm");
                if ui.add(egui::TextEdit::singleline(&mut joined).hint_text("ArcRaiders.exe").desired_width(f32::INFINITY)).changed() {
                    page.exe = joined.split(',').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect();
                }
                let procs: Vec<(String, String)> = {
                    let mut v: Vec<String> = n1deck::procs::running().into_iter().filter(|n| !n.starts_with("svc") && n.ends_with(".exe")).collect();
                    v.sort();
                    v.into_iter().map(|n| (n.clone(), n)).collect()
                };
                if let Some(exe) = pick_menu(ui, "Laufendes Programm…", &procs) {
                    let page = self.cfg.pages.entry(self.page.clone()).or_default();
                    if !page.exe.iter().any(|e| e.eq_ignore_ascii_case(&exe)) {
                        page.exe.push(exe);
                    }
                }
                if ui.button("Als Startseite").clicked() {
                    self.cfg.start_page = self.page.clone();
                }
                if ui.button(RichText::new("Seite löschen").color(DANGER)).clicked() && self.cfg.pages.len() > 1 {
                    self.cfg.pages.remove(&self.page);
                    self.page = self.cfg.pages.keys().next().cloned().unwrap_or_default();
                    if !self.cfg.pages.contains_key(&self.cfg.start_page) {
                        self.cfg.start_page = self.page.clone();
                    }
                }
            });

        egui::SidePanel::right("editor")
            .show_separator_line(false)
            .min_width(320.0)
            .default_width(360.0)
            .frame(egui::Frame::new().fill(BG).stroke(Stroke::new(1.0, BORDER)).inner_margin(16))
            .show(ctx, |ui| {
                egui::ScrollArea::vertical().show(ui, |ui| {
                    let pages = page_names.clone();
                    let mut want_lib = false;
                    match self.sel {
                        None => {
                            section(ui, "Belegung");
                            ui.label(RichText::new("Wähle links eine Taste oder den Drehregler.").color(MUTED));
                        }
                        Some(Sel::Key(i)) => {
                            section(ui, &format!("Taste {i} · {}", self.page));
                            let page = self.cfg.pages.entry(self.page.clone()).or_default();
                            let k = key_mut(page, i);
                            group(ui, |ui| {
                            ui.label("Beschriftung");
                            ui.add(egui::TextEdit::singleline(&mut k.label).desired_width(f32::INFINITY));
                            ui.horizontal(|ui| {
                                ui.label("Farbe");
                                let mut c = parse_color(&k.color).0;
                                if ui.color_edit_button_srgb(&mut c).changed() {
                                    k.color = format!("#{:02x}{:02x}{:02x}", c[0], c[1], c[2]);
                                }
                            });
                            ui.label("Icon");
                            ui.horizontal(|ui| {
                                if ui.button("Bild wählen…").clicked() {
                                    match import_icon() {
                                        Ok(Some(p)) => k.icon = p,
                                        Ok(None) => {}
                                        Err(e) => self.status = e,
                                    }
                                }
                                if ui.button("Bibliothek…").clicked() {
                                    want_lib = true;
                                }
                                if !k.icon.is_empty() && ui.button("Entfernen").clicked() {
                                    k.icon.clear();
                                }
                            });
                            if !k.icon.is_empty() {
                                let name = std::path::Path::new(&k.icon).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                                ui.label(RichText::new(name).size(12.0).color(MUTED));
                            }
                            });
                            section(ui, "Aktion");
                            group(ui, |ui| edit_action(ui, &mut k.action, &pages, false));
                            section(ui, "Feedback");
                            group(ui, |ui| {
                                ui.horizontal(|ui| {
                                    ui.label("Druck-Effekt");
                                    egui::ComboBox::from_id_salt("press_effect")
                                        .selected_text(k.effect.label())
                                        .show_ui(ui, |ui| {
                                            for e in PressEffect::ALL {
                                                ui.selectable_value(&mut k.effect, e, e.label());
                                            }
                                        });
                                });
                                if matches!(k.action, Action::Run { .. } | Action::Open { .. }) {
                                    ui.checkbox(&mut k.loading, "Ladeanimation, bis das Programm läuft");
                                }
                            });
                            if ui.button(RichText::new("Taste leeren").color(DANGER)).clicked() {
                                *k = KeyCfg { key: i, ..Default::default() };
                            }
                        }
                        Some(s) => {
                            let (title, action) = match s {
                                Sel::KnobPress => ("Regler drücken", &mut self.cfg.knob.press),
                                Sel::KnobCw => ("Regler nach rechts", &mut self.cfg.knob.cw),
                                _ => ("Regler nach links", &mut self.cfg.knob.ccw),
                            };
                            section(ui, title);
                            group(ui, |ui| edit_action(ui, action, &pages, false));
                        }
                    }
                    if let (true, Some(Sel::Key(i))) = (want_lib, self.sel) {
                        self.lib_open = true;
                        self.lib_target = Some((self.page.clone(), i));
                    }
                });
            });

        self.library_window(ctx);

        egui::CentralPanel::default().frame(egui::Frame::new().fill(BG).inner_margin(24)).show(ctx, |ui| {
            section(ui, "Obere Reihe");
            ui.horizontal(|ui| {
                for i in 15..18 {
                    self.key_button(ui, i);
                }
            });
            ui.add_space(16.0);
            section(ui, "Haupttasten");
            for row in 0..5 {
                ui.horizontal(|ui| {
                    for col in 0..3 {
                        self.key_button(ui, row * 3 + col);
                    }
                });
            }
            ui.add_space(16.0);
            section(ui, "Drehregler · gilt auf allen Seiten");
            ui.horizontal(|ui| {
                for (label, s) in [("Drücken", Sel::KnobPress), ("Rechts", Sel::KnobCw), ("Links", Sel::KnobCcw)] {
                    let on = self.sel == Some(s);
                    let text = if on { RichText::new(label).color(Color32::WHITE) } else { RichText::new(label) };
                    let b = egui::Button::new(text).fill(if on { ACCENT } else { SURFACE }).min_size(egui::vec2(88.0, 32.0));
                    if ui.add(b).clicked() {
                        self.sel = Some(s);
                    }
                }
            });
        });
    }
}

fn window_icon() -> egui::IconData {
    let img = image::load_from_memory(include_bytes!("../../assets/logo.png")).expect("Logo").to_rgba8();
    egui::IconData { width: img.width(), height: img.height(), rgba: img.into_raw() }
}

/// Autostart-Eintrag (HKCU\...\Run) fuer den Dienst n1deck.exe neben dieser Exe.
const RUN_KEY: &str = r"HKCU\Software\Microsoft\Windows\CurrentVersion\Run";

fn reg(args: &[&str]) -> bool {
    use std::os::windows::process::CommandExt;
    std::process::Command::new("reg").args(args).creation_flags(0x0800_0000).output().map(|o| o.status.success()).unwrap_or(false)
}

fn autostart_enabled() -> bool {
    reg(&["query", RUN_KEY, "/v", "n1deck"])
}

fn set_autostart(on: bool) -> Result<(), String> {
    let ok = if on {
        let mut exe = std::env::current_exe().map_err(|e| e.to_string())?;
        exe.set_file_name("n1deck.exe");
        if !exe.exists() {
            return Err("n1deck.exe liegt nicht neben der Config-GUI".into());
        }
        reg(&["add", RUN_KEY, "/v", "n1deck", "/t", "REG_SZ", "/d", &format!("\"{}\"", exe.display()), "/f"])
    } else {
        reg(&["delete", RUN_KEY, "/v", "n1deck", "/f"])
    };
    if ok { Ok(()) } else { Err("Registry-Eintrag fehlgeschlagen".into()) }
}

fn main() -> eframe::Result {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1100.0, 700.0])
            .with_min_inner_size([760.0, 560.0])
            .with_title("n1deck – N1 Stream Controller")
            .with_icon(window_icon()),
        ..Default::default()
    };
    eframe::run_native("n1deck-config", options, Box::new(|cc| Ok(Box::new(App::new(cc)))))
}
