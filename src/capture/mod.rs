//! Capturing every output: KMS first (frozen scanout, no compositor involvement), then
//! wlr-screencopy as the fallback. Output layout comes from Wayland itself, so it works the
//! same on any compositor; Hyprland only adds window rectangles and the cursor position.
pub mod kms;
pub mod wl;

use std::collections::HashMap;
use std::sync::Arc;

use tiny_skia::Pixmap;

use crate::hypr;

/// Tightly packed 8-bit RGBA, rows top-down.
pub struct Rgba {
    pub w: u32,
    pub h: u32,
    pub data: Vec<u8>,
}

#[derive(Clone)]
pub struct Output {
    pub name: String,
    pub x: i32, // logical global position
    pub y: i32,
    pub w: i32, // logical size
    pub h: i32,
    pub scale: f32,
    pub transform: u32,
    pub workspaces: Vec<String>,
    pub image: Option<Arc<Pixmap>>,
}

impl Output {
    pub fn contains(&self, x: f32, y: f32) -> bool {
        x >= self.x as f32 && x < (self.x + self.w) as f32 && y >= self.y as f32 && y < (self.y + self.h) as f32
    }
}

#[derive(Clone, Debug)]
pub struct WindowInfo {
    pub x: f32, // logical global
    pub y: f32,
    pub w: f32,
    pub h: f32,
    pub title: String,
    pub class: String,
}

/// Undo the output transform (wl_output.transform) so the picture is upright.
fn untransform(img: Rgba, t: u32) -> Rgba {
    if t == 0 {
        return img;
    }
    let (w, h) = (img.w as usize, img.h as usize);
    let rot = t % 4; // 90° steps the compositor rotated the content counter-clockwise
    let flip = t >= 4;
    let (ow, oh) = if rot % 2 == 1 { (h, w) } else { (w, h) };
    let mut out = vec![0u8; ow * oh * 4];
    for oy in 0..oh {
        for ox in 0..ow {
            // output pixel -> buffer pixel
            let (mut x, y) = match rot {
                0 => (ox, oy),
                1 => (w - 1 - oy, ox),
                2 => (w - 1 - ox, h - 1 - oy),
                _ => (oy, h - 1 - ox),
            };
            if flip {
                x = w - 1 - x;
            }
            let s = (y * w + x) * 4;
            let d = (oy * ow + ox) * 4;
            out[d..d + 4].copy_from_slice(&img.data[s..s + 4]);
        }
    }
    Rgba {
        w: ow as u32,
        h: oh as u32,
        data: out,
    }
}

fn flip_v(img: &mut Rgba) {
    let row = img.w as usize * 4;
    let h = img.h as usize;
    for y in 0..h / 2 {
        let (a, b) = img.data.split_at_mut((h - 1 - y) * row);
        a[y * row..(y + 1) * row].swap_with_slice(&mut b[..row]);
    }
}

fn to_pixmap(img: Rgba) -> Option<Arc<Pixmap>> {
    // opaque pixels: premultiplied == straight, so the bytes go in as they are
    Pixmap::from_vec(img.data, tiny_skia::IntSize::from_wh(img.w, img.h)?).map(Arc::new)
}

pub struct Capture {
    pub outputs: Vec<Output>,
    pub cursor: Option<(f32, f32)>,
    pub windows: Vec<WindowInfo>,
    pub backend: &'static str,
}

/// Grab all outputs. `with_windows` also asks Hyprland for window rectangles (area picking).
pub fn grab(with_windows: bool) -> Result<Capture, String> {
    let mode = std::env::var("KADR_CAPTURE").unwrap_or_default();
    let mut session = wl::Session::connect()?;
    let mut outputs = session.outputs();
    if outputs.is_empty() {
        return Err("no outputs".into());
    }
    let mut kms_imgs: HashMap<String, Rgba> = HashMap::new();
    if mode != "screencopy" {
        match kms::capture() {
            Ok(m) => kms_imgs = m,
            Err(e) => {
                if mode == "kms" {
                    return Err(e);
                }
                eprintln!("kadr: KMS capture unavailable ({e}), using wlr-screencopy");
            }
        }
    }
    let mut used_screencopy = false;
    for o in &mut outputs {
        let img = match kms_imgs.remove(&o.name) {
            Some(img) => Some(untransform(img, o.transform)),
            None => {
                used_screencopy = true;
                match session.screencopy(&o.name) {
                    Ok((mut img, y_invert)) => {
                        if y_invert {
                            flip_v(&mut img);
                        }
                        Some(untransform(img, o.transform))
                    }
                    Err(e) => {
                        eprintln!("kadr: {}: {e}", o.name);
                        None
                    }
                }
            }
        };
        o.image = img.and_then(to_pixmap);
    }
    let backend = if used_screencopy { "screencopy" } else { "kms" };
    outputs.retain(|o| o.image.is_some());
    if outputs.is_empty() {
        return Err("could not capture any output".into());
    }
    let mut cursor = None;
    let mut windows = vec![];
    if hypr::available() {
        hypr::add_workspaces(&mut outputs);
        cursor = hypr::cursor_pos();
        if with_windows {
            windows = hypr::list_windows(&outputs);
        }
    }
    Ok(Capture {
        outputs,
        cursor,
        windows,
        backend,
    })
}
