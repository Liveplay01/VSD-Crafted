//! Icon-Import fuer die Config-GUI: beliebige Bildformate + SVG, plus Online-Bibliothek (Iconify).
//! Alles wird als PNG im Config-Ordner abgelegt, damit der Dienst nur PNG lesen muss.

use anyhow::{anyhow, Context, Result};
use std::io::Read;
use std::path::{Path, PathBuf};

const API: &str = "https://api.iconify.design";
const MAX_DOWNLOAD: u64 = 5 * 1024 * 1024;

pub fn icons_dir() -> Result<PathBuf> {
    let dir = crate::config::config_path().parent().context("kein Config-Ordner")?.join("icons");
    std::fs::create_dir_all(&dir)?;
    Ok(dir)
}

fn get(url: &str) -> Result<Vec<u8>> {
    let resp = ureq::get(url).timeout(std::time::Duration::from_secs(10)).call().map_err(|e| anyhow!("{e}"))?;
    let mut buf = Vec::new();
    resp.into_reader().take(MAX_DOWNLOAD).read_to_end(&mut buf)?;
    Ok(buf)
}

/// SVG -> RGBA (nicht vormultipliziert) in `px` x `px`, Seitenverhaeltnis erhalten, zentriert.
pub fn rasterize_svg(data: &[u8], px: u32) -> Result<image::RgbaImage> {
    let tree = resvg::usvg::Tree::from_data(data, &resvg::usvg::Options::default()).map_err(|e| anyhow!("SVG ungueltig: {e}"))?;
    let size = tree.size();
    let scale = px as f32 / size.width().max(size.height());
    let mut pix = resvg::tiny_skia::Pixmap::new(px, px).context("Pixmap")?;
    let dx = (px as f32 - size.width() * scale) / 2.0;
    let dy = (px as f32 - size.height() * scale) / 2.0;
    let t = resvg::tiny_skia::Transform::from_scale(scale, scale).post_translate(dx, dy);
    resvg::render(&tree, t, &mut pix.as_mut());
    let mut img = image::RgbaImage::new(px, px);
    for (o, p) in img.pixels_mut().zip(pix.pixels()) {
        let c = p.demultiply();
        *o = image::Rgba([c.red(), c.green(), c.blue(), c.alpha()]);
    }
    Ok(img)
}

/// Beliebige Datei (png, jpg, webp, gif, bmp, ico, svg) als PNG in den Icon-Ordner.
pub fn import_file(src: &Path) -> Result<String> {
    let ext = src.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
    let stem = src.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| "icon".into());
    let dst = icons_dir()?.join(format!("{stem}.png"));
    if ext == "svg" {
        rasterize_svg(&std::fs::read(src)?, 144)?.save(&dst)?;
    } else {
        image::open(src).context("Bildformat nicht lesbar")?.save(&dst)?;
    }
    Ok(dst.to_string_lossy().into_owned())
}

/// Namen wie "mdi:home". Sucht in der Iconify-Bibliothek.
pub fn search(query: &str) -> Result<Vec<String>> {
    let q: String = query.chars().filter(|c| c.is_alphanumeric() || *c == '-' || *c == ' ').collect::<String>().replace(' ', "%20");
    let v: serde_json::Value = serde_json::from_slice(&get(&format!("{API}/search?query={q}&limit=48"))?)?;
    Ok(v["icons"].as_array().map(|a| a.iter().filter_map(|s| s.as_str().map(String::from)).collect()).unwrap_or_default())
}

fn svg_url(name: &str) -> Result<String> {
    let (prefix, icon) = name.split_once(':').context("Name ohne Praefix")?;
    let ok = |s: &str| !s.is_empty() && s.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
    if !ok(prefix) || !ok(icon) {
        return Err(anyhow!("ungueltiger Icon-Name"));
    }
    // Weiss, damit es auf den dunklen Tasten lesbar ist.
    Ok(format!("{API}/{prefix}/{icon}.svg?color=%23ffffff&height=144"))
}

pub fn fetch_rgba(name: &str, px: u32) -> Result<image::RgbaImage> {
    rasterize_svg(&get(&svg_url(name)?)?, px)
}

/// Laedt das Icon herunter und speichert es als PNG im Icon-Ordner.
pub fn download(name: &str) -> Result<String> {
    let dst = icons_dir()?.join(format!("{}.png", name.replace(':', "-")));
    fetch_rgba(name, 144)?.save(&dst)?;
    Ok(dst.to_string_lossy().into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn svg_rasterizes_centered() {
        let svg = br##"<svg xmlns="http://www.w3.org/2000/svg" width="20" height="10"><rect width="20" height="10" fill="#fff"/></svg>"##;
        let img = rasterize_svg(svg, 100).unwrap();
        assert_eq!(img.get_pixel(50, 50)[3], 255);
        assert_eq!(img.get_pixel(50, 5)[3], 0); // Letterbox oben bleibt transparent
    }

    #[test]
    fn name_validation() {
        assert!(svg_url("mdi:home").is_ok());
        assert!(svg_url("../x:y").is_err());
        assert!(svg_url("nocolon").is_err());
    }

    #[test]
    #[ignore = "braucht Internet"]
    fn online_search_and_download() {
        let r = search("discord").unwrap();
        assert!(!r.is_empty());
        assert_eq!(fetch_rgba(&r[0], 64).unwrap().width(), 64);
    }
}
