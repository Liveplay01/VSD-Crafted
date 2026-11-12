//! Rendert Tasten-Bilder (Hintergrund, Icon, Beschriftung) ohne schwere Abhängigkeiten.
use crate::config::{Config, KeyCfg, PressEffect};
use ab_glyph::{point, Font, FontVec, PxScale, ScaleFont};
use image::{imageops, DynamicImage, Rgb, RgbImage, Rgba, RgbaImage};

/// 15 Haupttasten 96x96, obere Reihe 64x64 (laut mirajazz-N1-Beispiel).
pub fn key_size(key: usize) -> (u32, u32) {
    if key >= 15 {
        (64, 64)
    } else {
        (96, 96)
    }
}

pub fn parse_color(s: &str) -> Rgb<u8> {
    let h = s.trim().trim_start_matches('#');
    if h.len() == 6 {
        if let Ok(v) = u32::from_str_radix(h, 16) {
            return Rgb([(v >> 16) as u8, (v >> 8) as u8, v as u8]);
        }
    }
    Rgb([30, 30, 34])
}

pub fn load_font(cfg: &Config) -> Option<FontVec> {
    let mut candidates = vec![cfg.font.clone()];
    candidates.extend(
        [
            r"C:\Windows\Fonts\segoeui.ttf",
            r"C:\Windows\Fonts\arial.ttf",
            "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf",
        ]
        .map(String::from),
    );
    candidates
        .into_iter()
        .filter(|p| !p.is_empty())
        .find_map(|p| std::fs::read(p).ok().and_then(|b| FontVec::try_from_vec(b).ok()))
}

fn blend(img: &mut RgbImage, x: i32, y: i32, color: Rgb<u8>, a: f32) {
    if x < 0 || y < 0 || x >= img.width() as i32 || y >= img.height() as i32 {
        return;
    }
    let p = img.get_pixel_mut(x as u32, y as u32);
    for i in 0..3 {
        p[i] = (p[i] as f32 * (1.0 - a) + color[i] as f32 * a).round() as u8;
    }
}

/// Zentrierter Text; schrumpft, bis er in die Breite passt.
fn draw_text(img: &mut RgbImage, font: &FontVec, text: &str, mut px: f32, y_top: f32, color: Rgb<u8>) {
    let max_w = img.width() as f32 - 6.0;
    let width = |px: f32| -> f32 {
        let s = font.as_scaled(PxScale::from(px));
        text.chars().map(|c| s.h_advance(font.glyph_id(c))).sum()
    };
    while width(px) > max_w && px > 8.0 {
        px -= 1.0;
    }
    let scale = PxScale::from(px);
    let scaled = font.as_scaled(scale);
    let mut x = (img.width() as f32 - width(px)) / 2.0;
    let baseline = y_top + scaled.ascent();
    for c in text.chars() {
        let id = font.glyph_id(c);
        let glyph = id.with_scale_and_position(scale, point(x, baseline));
        x += scaled.h_advance(id);
        if let Some(og) = font.outline_glyph(glyph) {
            let b = og.px_bounds();
            og.draw(|gx, gy, cov| blend(img, b.min.x as i32 + gx as i32, b.min.y as i32 + gy as i32, color, cov));
        }
    }
}

pub fn render_key(k: Option<&KeyCfg>, key: usize, font: Option<&FontVec>) -> DynamicImage {
    let (w, h) = key_size(key);
    let Some(k) = k else {
        return DynamicImage::ImageRgb8(RgbImage::from_pixel(w, h, Rgb([0, 0, 0])));
    };
    let bg = if k.color.is_empty() { Rgb([30, 30, 34]) } else { parse_color(&k.color) };
    let mut img = RgbImage::from_pixel(w, h, bg);

    let has_label = !k.label.is_empty() && font.is_some();
    let icon_h = if has_label { (h as f32 * 0.62) as u32 } else { h - 8 };

    if !k.icon.is_empty() {
        if let Ok(icon) = image::open(&k.icon) {
            let icon = icon.resize(w - 8, icon_h, imageops::FilterType::Triangle).to_rgba8();
            let mut layer = RgbaImage::from_pixel(w, h, Rgba([0, 0, 0, 0]));
            let ox = (w - icon.width()) / 2;
            let oy = 4 + (icon_h.saturating_sub(icon.height())) / 2;
            imageops::overlay(&mut layer, &icon, ox as i64, oy as i64);
            for (x, y, p) in layer.enumerate_pixels() {
                if p[3] > 0 {
                    blend(&mut img, x as i32, y as i32, Rgb([p[0], p[1], p[2]]), p[3] as f32 / 255.0);
                }
            }
        }
    }
    if has_label {
        let px = if k.icon.is_empty() { h as f32 * 0.22 } else { h as f32 * 0.17 };
        let y = if k.icon.is_empty() { (h as f32 - px) / 2.0 - 2.0 } else { h as f32 - px - 8.0 };
        draw_text(&mut img, font.unwrap(), &k.label, px, y, Rgb([245, 245, 245]));
    }
    DynamicImage::ImageRgb8(img)
}

/// Druck-Effekt: Bild Richtung Weiß (heller/Blitz) oder Schwarz (dunkler) mischen.
pub fn press_effect(img: &DynamicImage, e: PressEffect) -> DynamicImage {
    let (target, a) = match e {
        PressEffect::None => return img.clone(),
        PressEffect::Brighten => (255.0, 0.35),
        PressEffect::Darken => (0.0, 0.5),
        PressEffect::Flash => (255.0, 0.8),
    };
    let mut rgb = img.to_rgb8();
    for c in rgb.pixels_mut().flat_map(|p| p.0.iter_mut()) {
        *c = (*c as f32 * (1.0 - a) + target * a).round() as u8;
    }
    DynamicImage::ImageRgb8(rgb)
}

/// Ladeanimation: abgedunkelte Taste mit 8 Punkten im Kreis, einer läuft hell voraus.
pub fn spinner_frame(base: &DynamicImage, frame: usize) -> DynamicImage {
    let mut img = press_effect(base, PressEffect::Darken).to_rgb8();
    let (w, h) = (img.width() as f32, img.height() as f32);
    let (cx, cy, r, dot) = (w / 2.0, h / 2.0, w * 0.22, w * 0.06);
    for i in 0..8usize {
        let ang = i as f32 / 8.0 * std::f32::consts::TAU - std::f32::consts::FRAC_PI_2;
        let (px, py) = (cx + r * ang.cos(), cy + r * ang.sin());
        let behind = (frame + 8 - i) % 8; // 0 = Kopf, danach verblassend
        let a = (1.0 - behind as f32 / 8.0).powi(2).max(0.12);
        for y in (py - dot - 1.0) as i32..=(py + dot + 1.0) as i32 {
            for x in (px - dot - 1.0) as i32..=(px + dot + 1.0) as i32 {
                let d = ((x as f32 - px).powi(2) + (y as f32 - py).powi(2)).sqrt();
                if d <= dot {
                    blend(&mut img, x, y, Rgb([255, 255, 255]), a);
                }
            }
        }
    }
    DynamicImage::ImageRgb8(img)
}

// 5x7-Ziffern, damit `learn` ohne Systemschrift funktioniert.
const DIGITS: [[u8; 7]; 10] = [
    [0b01110, 0b10001, 0b10011, 0b10101, 0b11001, 0b10001, 0b01110],
    [0b00100, 0b01100, 0b00100, 0b00100, 0b00100, 0b00100, 0b01110],
    [0b01110, 0b10001, 0b00001, 0b00010, 0b00100, 0b01000, 0b11111],
    [0b11111, 0b00010, 0b00100, 0b00010, 0b00001, 0b10001, 0b01110],
    [0b00010, 0b00110, 0b01010, 0b10010, 0b11111, 0b00010, 0b00010],
    [0b11111, 0b10000, 0b11110, 0b00001, 0b00001, 0b10001, 0b01110],
    [0b00110, 0b01000, 0b10000, 0b11110, 0b10001, 0b10001, 0b01110],
    [0b11111, 0b00001, 0b00010, 0b00100, 0b01000, 0b01000, 0b01000],
    [0b01110, 0b10001, 0b10001, 0b01110, 0b10001, 0b10001, 0b01110],
    [0b01110, 0b10001, 0b10001, 0b01111, 0b00001, 0b00010, 0b01100],
];

/// Große Zahl auf farbigem Grund (für den Lernmodus).
pub fn render_number(n: usize, key: usize, highlight: bool) -> DynamicImage {
    let (w, h) = key_size(key);
    let bg = if highlight { Rgb([200, 120, 0]) } else { Rgb([20, 20, 24]) };
    let mut img = RgbImage::from_pixel(w, h, bg);
    let digits: Vec<usize> = n.to_string().bytes().map(|b| (b - b'0') as usize).collect();
    let cells = digits.len() * 6 - 1;
    let scale = ((w as usize * 7 / 10) / cells).max(1) as u32;
    let total_w = cells as u32 * scale;
    let (x0, y0) = ((w - total_w) / 2, (h - 7 * scale) / 2);
    for (di, d) in digits.iter().enumerate() {
        for (row, bits) in DIGITS[*d].iter().enumerate() {
            for col in 0..5u32 {
                if bits >> (4 - col) & 1 == 1 {
                    for dy in 0..scale {
                        for dx in 0..scale {
                            let x = x0 + (di as u32 * 6 + col) * scale + dx;
                            let y = y0 + row as u32 * scale + dy;
                            img.put_pixel(x, y, Rgb([255, 255, 255]));
                        }
                    }
                }
            }
        }
    }
    DynamicImage::ImageRgb8(img)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn color_parsing() {
        assert_eq!(parse_color("#ff8000"), Rgb([255, 128, 0]));
        assert_eq!(parse_color("kaputt"), Rgb([30, 30, 34]));
    }

    #[test]
    fn effects_change_pixels() {
        let base = DynamicImage::ImageRgb8(RgbImage::from_pixel(96, 96, Rgb([100, 100, 100])));
        assert!(press_effect(&base, PressEffect::Brighten).to_rgb8().get_pixel(0, 0)[0] > 100);
        assert!(press_effect(&base, PressEffect::Darken).to_rgb8().get_pixel(0, 0)[0] < 100);
        assert_eq!(press_effect(&base, PressEffect::None).to_rgb8().get_pixel(0, 0)[0], 100);
        assert_ne!(spinner_frame(&base, 0).to_rgb8().into_raw(), spinner_frame(&base, 3).to_rgb8().into_raw());
    }

    #[test]
    fn number_image_has_right_size() {
        assert_eq!(render_number(17, 17, true).width(), 64);
        assert_eq!(render_number(7, 0, false).height(), 96);
    }
}
