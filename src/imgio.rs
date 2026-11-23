//! Pixmap <-> files/bytes.
use std::io::Cursor;
use std::path::Path;

use image::codecs::png::{CompressionType, FilterType, PngEncoder};
use image::{ImageEncoder, RgbaImage};
use tiny_skia::Pixmap;

pub fn to_rgba(pm: &Pixmap) -> RgbaImage {
    let mut img = RgbaImage::new(pm.width(), pm.height());
    for (dst, src) in img.pixels_mut().zip(pm.pixels()) {
        let c = src.demultiply();
        *dst = image::Rgba([c.red(), c.green(), c.blue(), c.alpha()]);
    }
    img
}

pub fn from_rgba(w: u32, h: u32, data: &[u8]) -> Option<Pixmap> {
    let mut pm = Pixmap::new(w, h)?;
    for (dst, s) in pm.pixels_mut().iter_mut().zip(data.chunks_exact(4)) {
        *dst = tiny_skia::ColorU8::from_rgba(s[0], s[1], s[2], s[3]).premultiply();
    }
    Some(pm)
}

pub fn load(path: &Path) -> Option<Pixmap> {
    let img = image::open(path).ok()?.to_rgba8();
    from_rgba(img.width(), img.height(), img.as_raw())
}

pub fn png_bytes(pm: &Pixmap) -> Vec<u8> {
    let img = to_rgba(pm);
    let mut out = Vec::new();
    let enc = PngEncoder::new_with_quality(&mut out, CompressionType::Fast, FilterType::Adaptive);
    let _ = enc.write_image(img.as_raw(), img.width(), img.height(), image::ExtendedColorType::Rgba8);
    out
}

pub fn save_png(pm: &Pixmap, path: &Path) -> std::io::Result<()> {
    std::fs::write(path, png_bytes(pm))
}

/// Save by extension: png (default), jpg/jpeg, webp.
pub fn save_any(pm: &Pixmap, path: &Path) -> Result<(), String> {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("png")
        .to_lowercase();
    match ext.as_str() {
        "jpg" | "jpeg" => {
            let rgb = image::DynamicImage::ImageRgba8(to_rgba(pm)).to_rgb8();
            let f = std::fs::File::create(path).map_err(|e| e.to_string())?;
            let mut w = std::io::BufWriter::new(f);
            image::codecs::jpeg::JpegEncoder::new_with_quality(&mut w, 92)
                .encode_image(&rgb)
                .map_err(|e| e.to_string())
        }
        "webp" => to_rgba(pm)
            .save_with_format(path, image::ImageFormat::WebP)
            .map_err(|e| e.to_string()),
        _ => save_png(pm, path).map_err(|e| e.to_string()),
    }
}

pub fn thumb_jpg(pm: &Pixmap, path: &Path) {
    let img = image::DynamicImage::ImageRgba8(to_rgba(pm));
    let t = img.resize(360, 240, image::imageops::FilterType::Triangle).to_rgb8();
    let mut buf = Cursor::new(Vec::new());
    if image::codecs::jpeg::JpegEncoder::new_with_quality(&mut buf, 88)
        .encode_image(&t)
        .is_ok()
    {
        let _ = std::fs::write(path, buf.into_inner());
    }
}
