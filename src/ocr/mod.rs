//! Text recognition, pure Rust: PaddleOCR PP-OCR models (Apache-2.0, ONNX exports from
//! huggingface.co/monkt/paddleocr-onnx) run on the `rten` inference engine. The detector
//! finds text lines, a per-script recognizer reads them (CTC); "auto" fuses the East-Slavic
//! and the English recognizers word by word, so mixed Russian/English text and code both come
//! out right. Character positions come from the CTC time steps, which is what makes the
//! invisible text selectable.
pub mod layout;

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock};

use rten::Model;
use rten_tensor::prelude::*;
use rten_tensor::{NdTensor, Tensor};
use sha2::{Digest, Sha256};
use tiny_skia::Pixmap;

use crate::geom::{R, r};

const REPO: &str = "https://huggingface.co/monkt/paddleocr-onnx/resolve/7b02d0a30a07ba2b92ad1ff5a8941ae2c633de65";

pub struct ModelFile {
    pub name: &'static str, // file name in our models folder
    pub path: &'static str, // path in the upstream repository
    pub size: u64,
    pub sha256: &'static str,
}

pub const DETECTOR: ModelFile = ModelFile {
    name: "det.onnx",
    path: "detection/v3/det.onnx",
    size: 2429873,
    sha256: "ee40e80071ba3a320d4efda75f3e22047a7d049e9bf7bcaaf9daea23fc21b935",
};

pub struct Script {
    pub key: &'static str,
    pub label: &'static str, // English source string for tr()
    pub rec: ModelFile,
    pub dict: ModelFile,
}

macro_rules! script {
    ($key:literal, $label:literal, $rs:literal, $rh:literal, $ds:literal, $dh:literal) => {
        Script {
            key: $key,
            label: $label,
            rec: ModelFile {
                name: concat!("rec-", $key, ".onnx"),
                path: concat!("languages/", $key, "/rec.onnx"),
                size: $rs,
                sha256: $rh,
            },
            dict: ModelFile {
                name: concat!("dict-", $key, ".txt"),
                path: concat!("languages/", $key, "/dict.txt"),
                size: $ds,
                sha256: $dh,
            },
        }
    };
}

pub const SCRIPTS: &[Script] = &[
    script!(
        "eslav",
        "Russian, Ukrainian, Belarusian",
        7870092,
        "dc6bf0e855247decce214ba6dae5bc135fa0ad725a5918a7fcfb59fad6c9cdee",
        1663,
        "3e95f1581557162870cacdba5af91a4c6be2890710d395b0c3c7578e7ee5e6eb"
    ),
    script!(
        "english",
        "English",
        7830888,
        "4e16deb22c4da6468bdca539b2cd3c8687825538b67109177c47d359ab994cd7",
        1416,
        "e025a66d31f327ba0c232e03f407ae8d105e1e709e7ccb3f408aa778c24e70d6"
    ),
    script!(
        "latin",
        "European languages (Latin script)",
        7862832,
        "614ffc2d6d3902d360fad7f1b0dd455ee45e877069d14c4e51a99dc4ef144409",
        1634,
        "3c0a8a79b612653c25f765271714f71281e4e955962c153e272b7b8c1d2b13ff"
    ),
    script!(
        "greek",
        "Greek",
        7791200,
        "13373f736dbb229e96945fc41c2573403d91503b0775c7b7294839e0c5f3a7a3",
        1103,
        "31defc62c0c3ad3674a82da6192226a2ba98ef4ff014a7045cb88d59f9c3de31"
    ),
    script!(
        "chinese",
        "Chinese, Japanese",
        84468836,
        "26fa4f47060f58e25962b9af6beaee05c8182b90e026c4ecc6db165d9dfdc38a",
        74012,
        "d1979e9f794c464c0d2e0b70a7fe14dd978e9dc644c0e71f14158cdf8342af1b"
    ),
    script!(
        "korean",
        "Korean",
        13401252,
        "322f140154c820fcb83c3d24cfe42c9ec70dd1a1834163306a7338136e4f1eaa",
        47451,
        "a88071c68c01707489baa79ebe0405b7beb5cca229f4fc94cc3ef992328802d7"
    ),
    script!(
        "arabic",
        "Arabic",
        8978664,
        "7982d371612785238fd99080cff36354deaec84fdc6ff7da9c82af4243fa0c9a",
        405,
        "637c27c88512c22089bef927b34ada08f748dc132ac70facd68d8202384c2726"
    ),
    script!(
        "hindi",
        "Hindi",
        8980224,
        "43df175fa3c877fbf7bcc4e5bd1e203e24ec450cd3ea96c9e802c86e39a4d4cf",
        508,
        "b5f1be6d8bbff1a19fb96c5d4ca96a423380234bb7d2ce0e07b5838adb4d18ea"
    ),
    script!(
        "thai",
        "Thai",
        7873480,
        "2b6e56b1872200349e227574c25aeb0e0f9af9b8356e9ff5f75ac543a535669a",
        1767,
        "57f5406f94bb6688fb7077f7be65f08bbd71cecf48c01ea26c522cb5c4836b7a"
    ),
    script!(
        "tamil",
        "Tamil",
        8970084,
        "fba9a00af746c8f3ef4f091cf966880222cd7245a60f6a953670593630c0ca4f",
        352,
        "e93e694814afd9ff1e918b6f4bb4267fb4c65a9344ee5d7464f9135b90c0a270"
    ),
    script!(
        "telugu",
        "Telugu",
        8976064,
        "669946d9ad4de93e8d1b13f7e72c59cc2cc2de91bb24e22aad7503a6cc5757ee",
        429,
        "ee9946a60d7701977474e89883694a4ca20eaa3868a23334bf7940dcd008f0f9"
    ),
];

/// Recognizers a language choice needs ("auto" = East Slavic + English, fused).
pub fn scripts_for(lang: &str) -> Vec<&'static Script> {
    let keys: &[&str] = if lang == "auto" || lang.is_empty() {
        &["eslav", "english"]
    } else {
        std::slice::from_ref(&lang)
    };
    keys.iter()
        .filter_map(|k| SCRIPTS.iter().find(|s| s.key == *k))
        .collect()
}

fn user_dir() -> PathBuf {
    crate::config::data_dir().join("models")
}

/// Where model files are looked up: the user's data folder (downloads), then the package.
fn find(f: &ModelFile) -> Option<PathBuf> {
    let mut dirs = vec![user_dir(), PathBuf::from("/usr/share/kadr/models")];
    if let Ok(d) = std::env::var("KADR_MODELS") {
        dirs.insert(0, PathBuf::from(d));
    }
    dirs.into_iter().map(|d| d.join(f.name)).find(|p| p.is_file())
}

/// Files still missing for `lang` (with sizes, for the download button).
pub fn missing(lang: &str) -> Vec<&'static ModelFile> {
    let mut v: Vec<&'static ModelFile> = Vec::new();
    if find(&DETECTOR).is_none() {
        v.push(&DETECTOR);
    }
    for s in scripts_for(lang) {
        for f in [&s.rec, &s.dict] {
            if find(f).is_none() {
                v.push(f);
            }
        }
    }
    v
}

/// Download (and verify) files into ~/.local/share/kadr/models; `progress(done, total)`.
pub fn download(files: &[&ModelFile], progress: impl Fn(u64, u64)) -> Result<(), String> {
    use std::io::{Read, Write};
    let dir = user_dir();
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let total: u64 = files.iter().map(|f| f.size).sum();
    let mut done = 0u64;
    for f in files {
        let url = format!("{REPO}/{}", f.path);
        let resp = ureq::get(&url).call().map_err(|e| format!("{url}: {e}"))?;
        let mut body = resp.into_body();
        let mut reader = body.as_reader();
        let tmp = dir.join(format!("{}.part", f.name));
        let mut out = std::fs::File::create(&tmp).map_err(|e| e.to_string())?;
        let mut hasher = Sha256::new();
        let mut buf = vec![0u8; 64 * 1024];
        loop {
            let n = reader.read(&mut buf).map_err(|e| e.to_string())?;
            if n == 0 {
                break;
            }
            hasher.update(&buf[..n]);
            out.write_all(&buf[..n]).map_err(|e| e.to_string())?;
            done += n as u64;
            progress(done, total);
        }
        let sum: String = hasher.finalize().iter().map(|b| format!("{b:02x}")).collect();
        if sum != f.sha256 {
            let _ = std::fs::remove_file(&tmp);
            return Err(format!("{}: checksum mismatch", f.name));
        }
        std::fs::rename(&tmp, dir.join(f.name)).map_err(|e| e.to_string())?;
    }
    Ok(())
}

// --- engine -------------------------------------------------------------------

struct Rec {
    model: Model,
    dict: Vec<String>,
}

static MODELS: OnceLock<Mutex<HashMap<&'static str, Arc<Rec>>>> = OnceLock::new();
static DET: OnceLock<Mutex<Option<Arc<Model>>>> = OnceLock::new();

fn detector() -> Result<Arc<Model>, String> {
    let slot = DET.get_or_init(Default::default);
    let mut g = slot.lock().unwrap();
    if let Some(m) = g.as_ref() {
        return Ok(m.clone());
    }
    let p = find(&DETECTOR).ok_or("text detection model is missing")?;
    let m = Arc::new(Model::load_file(&p).map_err(|e| format!("{}: {e}", p.display()))?);
    *g = Some(m.clone());
    Ok(m)
}

fn recognizer(s: &'static Script) -> Result<Arc<Rec>, String> {
    let slot = MODELS.get_or_init(Default::default);
    if let Some(r) = slot.lock().unwrap().get(s.key) {
        return Ok(r.clone());
    }
    let p = find(&s.rec).ok_or_else(|| format!("{} model is missing", s.key))?;
    let d = find(&s.dict).ok_or_else(|| format!("{} dictionary is missing", s.key))?;
    let model = Model::load_file(&p).map_err(|e| format!("{}: {e}", p.display()))?;
    let dict = std::fs::read_to_string(&d)
        .map_err(|e| e.to_string())?
        .lines()
        .map(String::from)
        .collect();
    let r = Arc::new(Rec { model, dict });
    slot.lock().unwrap().insert(s.key, r.clone());
    Ok(r)
}

/// One recognized character, x range in image pixels.
#[derive(Clone, Debug, PartialEq)]
pub struct OChar {
    pub c: char,
    pub x0: f32,
    pub x1: f32,
}

/// One detected text segment (a line, or part of a line split at a wide gap).
#[derive(Clone, Debug)]
pub struct OLine {
    pub rect: R, // image pixels
    pub chars: Vec<OChar>,
}

#[derive(Clone, Debug, Default)]
pub struct OcrResult {
    pub lines: Vec<OLine>, // reading order
}

fn rgb_at(img: &Pixmap, x: u32, y: u32) -> [f32; 3] {
    let p = img
        .pixel(x.min(img.width() - 1), y.min(img.height() - 1))
        .unwrap()
        .demultiply();
    [
        p.red() as f32 / 255.0,
        p.green() as f32 / 255.0,
        p.blue() as f32 / 255.0,
    ]
}

/// Bilinear sample of the source rectangle `src` into a `w`x`h` BGR tensor, normalized.
fn sample(img: &Pixmap, src: R, w: usize, h: usize, mean: [f32; 3], std: [f32; 3]) -> NdTensor<f32, 4> {
    let mut t = NdTensor::<f32, 4>::zeros([1, 3, h, w]);
    let (sx, sy) = (src.w / w as f32, src.h / h as f32);
    for y in 0..h {
        let fy = (src.y + (y as f32 + 0.5) * sy - 0.5).max(0.0);
        let (y0, wy) = (fy.floor() as u32, fy.fract());
        for x in 0..w {
            let fx = (src.x + (x as f32 + 0.5) * sx - 0.5).max(0.0);
            let (x0, wx) = (fx.floor() as u32, fx.fract());
            let a = rgb_at(img, x0, y0);
            let b = rgb_at(img, x0 + 1, y0);
            let c = rgb_at(img, x0, y0 + 1);
            let d = rgb_at(img, x0 + 1, y0 + 1);
            for ch in 0..3 {
                let v = (a[ch] * (1.0 - wx) + b[ch] * wx) * (1.0 - wy) + (c[ch] * (1.0 - wx) + d[ch] * wx) * wy;
                // PaddleOCR models were trained on BGR (cv2) input
                let k = 2 - ch;
                t[[0, k, y, x]] = (v - mean[k]) / std[k];
            }
        }
    }
    t
}

/// Text line boxes (image pixels) via the DB detector; `scale` upsamples small UI text.
fn detect(img: &Pixmap, scale: f32) -> Result<Vec<R>, String> {
    let det = detector()?;
    let (w, h) = (img.width() as f32, img.height() as f32);
    let dw = (((w * scale) / 32.0).ceil() as usize).max(1) * 32;
    let dh = (((h * scale) / 32.0).ceil() as usize).max(1) * 32;
    let (kx, ky) = (dw as f32 / w, dh as f32 / h);
    // pad instead of stretching: sample only the image part, the rest stays at "white"
    let inp = sample(
        img,
        r(0.0, 0.0, dw as f32 / kx, dh as f32 / ky),
        dw,
        dh,
        [0.485, 0.456, 0.406],
        [0.229, 0.224, 0.225],
    );
    let out: Tensor<f32> = det
        .run_one(inp.view().into(), None)
        .map_err(|e| e.to_string())?
        .try_into()
        .map_err(|e: rten::TryFromValueError| e.to_string())?;
    let prob = out.reshaped([dh, dw]).to_tensor();
    let mut seen = vec![false; dw * dh];
    let mut boxes = Vec::new();
    for y0 in 0..dh {
        for x0 in 0..dw {
            if seen[y0 * dw + x0] || prob[[y0, x0]] <= 0.3 {
                continue;
            }
            seen[y0 * dw + x0] = true;
            let mut stack = vec![(x0, y0)];
            let (mut l, mut t, mut rr, mut b, mut n, mut sum) = (x0, y0, x0, y0, 0usize, 0f32);
            while let Some((x, y)) = stack.pop() {
                n += 1;
                sum += prob[[y, x]];
                l = l.min(x);
                rr = rr.max(x);
                t = t.min(y);
                b = b.max(y);
                let neighbours = [(x.wrapping_sub(1), y), (x + 1, y), (x, y.wrapping_sub(1)), (x, y + 1)];
                for (nx, ny) in neighbours {
                    if nx < dw && ny < dh && !seen[ny * dw + nx] && prob[[ny, nx]] > 0.3 {
                        seen[ny * dw + nx] = true;
                        stack.push((nx, ny));
                    }
                }
            }
            if sum / (n as f32) < 0.5 || b - t < 2 || n < 6 {
                continue;
            }
            // the detector shrinks text regions; grow them back ("unclip", ratio 1.5)
            let (bw, bh) = ((rr - l + 1) as f32, (b - t + 1) as f32);
            let d = bw * bh * 1.5 / (2.0 * (bw + bh));
            let rect = R::ltrb(
                (l as f32 - d) / kx,
                (t as f32 - d) / ky,
                (rr as f32 + 1.0 + d) / kx,
                (b as f32 + 1.0 + d) / ky,
            );
            boxes.push(rect.intersect(r(0.0, 0.0, w, h)));
        }
    }
    boxes.retain(|b| b.w >= 3.0 && b.h >= 4.0);
    Ok(boxes)
}

/// Ink the detector missed (lone characters such as a closing brace): pixels that stand out
/// from their neighbourhood, outside the detected lines, grouped into text-sized boxes.
fn leftover_boxes(img: &Pixmap, found: &[R]) -> Vec<R> {
    let (w, h) = (img.width() as usize, img.height() as usize);
    if found.is_empty() || w < 8 || h < 8 {
        return vec![];
    }
    let line_h = {
        let mut v: Vec<f32> = found.iter().map(|b| b.h).collect();
        v.sort_by(|a, b| a.partial_cmp(b).unwrap());
        v[v.len() / 2]
    };
    let gray: Vec<f32> = img
        .pixels()
        .iter()
        .map(|p| {
            let c = p.demultiply();
            0.299 * c.red() as f32 + 0.587 * c.green() as f32 + 0.114 * c.blue() as f32
        })
        .collect();
    // integral image for local means
    let mut integ = vec![0f64; (w + 1) * (h + 1)];
    for y in 0..h {
        let mut row = 0f64;
        for x in 0..w {
            row += gray[y * w + x] as f64;
            integ[(y + 1) * (w + 1) + x + 1] = integ[y * (w + 1) + x + 1] + row;
        }
    }
    let rad = (line_h as usize).clamp(6, 40);
    let mut ink = vec![false; w * h];
    for y in 0..h {
        let (y0, y1) = (y.saturating_sub(rad), (y + rad + 1).min(h));
        for x in 0..w {
            let (x0, x1) = (x.saturating_sub(rad), (x + rad + 1).min(w));
            let sum = integ[y1 * (w + 1) + x1] - integ[y0 * (w + 1) + x1] - integ[y1 * (w + 1) + x0]
                + integ[y0 * (w + 1) + x0];
            let m = sum / ((y1 - y0) * (x1 - x0)) as f64;
            ink[y * w + x] = (gray[y * w + x] as f64 - m).abs() > 45.0;
        }
    }
    for b in found {
        let g = b.grow(2.0);
        for y in (g.y.max(0.0) as usize)..(g.bottom().min(h as f32) as usize) {
            for x in (g.x.max(0.0) as usize)..(g.right().min(w as f32) as usize) {
                ink[y * w + x] = false;
            }
        }
    }
    let mut seen = vec![false; w * h];
    let mut comps: Vec<R> = Vec::new();
    for y0 in 0..h {
        for x0 in 0..w {
            if !ink[y0 * w + x0] || seen[y0 * w + x0] {
                continue;
            }
            seen[y0 * w + x0] = true;
            let mut stack = vec![(x0, y0)];
            let (mut l, mut t, mut rr, mut b) = (x0, y0, x0, y0);
            let mut n = 0;
            while let Some((x, y)) = stack.pop() {
                n += 1;
                l = l.min(x);
                rr = rr.max(x);
                t = t.min(y);
                b = b.max(y);
                for dy in -1i32..=1 {
                    for dx in -1i32..=1 {
                        let (nx, ny) = (x as i32 + dx, y as i32 + dy);
                        if nx >= 0 && ny >= 0 && (nx as usize) < w && (ny as usize) < h {
                            let i = ny as usize * w + nx as usize;
                            if ink[i] && !seen[i] {
                                seen[i] = true;
                                stack.push((nx as usize, ny as usize));
                            }
                        }
                    }
                }
            }
            if n >= 4 {
                comps.push(R::ltrb(l as f32, t as f32, rr as f32 + 1.0, b as f32 + 1.0));
            }
        }
    }
    // glue neighbouring glyph pieces into words
    comps.sort_by(|a, b| a.x.partial_cmp(&b.x).unwrap());
    let mut merged: Vec<R> = Vec::new();
    for c in comps {
        if let Some(m) = merged.iter_mut().find(|m| {
            let vo = m.bottom().min(c.bottom()) - m.y.max(c.y);
            vo > -0.2 * line_h && c.x - m.right() < 0.6 * line_h
        }) {
            *m = m.union(c);
        } else {
            merged.push(c);
        }
    }
    merged
        .into_iter()
        .filter(|m| m.h >= 0.35 * line_h && m.h <= 1.3 * line_h && m.w <= 12.0 * line_h)
        .map(|m| {
            // a text-line-shaped box around the glyphs, like the detector would give
            let pad = ((line_h - m.h) / 2.0).max(0.0);
            R::ltrb(
                m.x - 0.3 * line_h,
                m.y - pad,
                m.right() + 0.3 * line_h,
                m.bottom() + pad,
            )
            .intersect(r(0.0, 0.0, w as f32, h as f32))
        })
        .collect()
}

/// Icons and decorations read as stray letters ("Я", "Q", "т E >"): what the detector found
/// must be at least a confident word, leftover ink must look like code punctuation or a word.
fn looks_like_icon(chars: &[OChar], conf: f32, leftover: bool) -> bool {
    let visible: Vec<char> = chars.iter().map(|c| c.c).filter(|c| !c.is_whitespace()).collect();
    let letters = visible.iter().filter(|c| c.is_alphabetic()).count();
    let longest_word = chars
        .split(|c| !c.c.is_alphanumeric())
        .map(|w| w.len())
        .max()
        .unwrap_or(0);
    if leftover {
        let code = visible
            .iter()
            .all(|c| "{}()[];,.:=<>+-*/\\|&!?\"'`#%$@_~^".contains(*c) || c.is_ascii_digit());
        return conf < 0.85 || !(code || longest_word >= 3);
    }
    // a lone letter or a few scattered single letters, without any real word
    letters > 0 && longest_word <= 1 && visible.len() <= 4 && (conf < 0.97 || visible.len() > 1)
}

/// Raw CTC read of one box: chars with x ranges and confidences.
fn read(img: &Pixmap, rect: R, rec: &Rec) -> Result<Vec<(OChar, f32)>, String> {
    let th = 48usize;
    let tw = ((rect.w * th as f32 / rect.h.max(1.0)).round() as usize).clamp(8, 3200);
    let inp = sample(img, rect, tw, th, [0.5; 3], [0.5; 3]);
    let out: Tensor<f32> = rec
        .model
        .run_one(inp.view().into(), None)
        .map_err(|e| e.to_string())?
        .try_into()
        .map_err(|e: rten::TryFromValueError| e.to_string())?;
    let (steps, classes) = (out.shape()[1], out.shape()[2]);
    let step_w = rect.w / steps as f32;
    let mut emitted: Vec<(char, usize, f32)> = Vec::new();
    let mut prev = 0;
    for t in 0..steps {
        let (mut best, mut bv) = (0, f32::MIN);
        for c in 0..classes {
            let v = out[[0, t, c]];
            if v > bv {
                bv = v;
                best = c;
            }
        }
        if best != 0 && best != prev {
            // past the dictionary is the space class
            let s = rec.dict.get(best - 1).map_or(" ", |s| s.as_str());
            for ch in s.chars() {
                emitted.push((ch, t, bv));
            }
        }
        prev = best;
    }
    let centers: Vec<f32> = emitted
        .iter()
        .map(|(_, t, _)| rect.x + (*t as f32 + 0.5) * step_w)
        .collect();
    let mut out = Vec::new();
    for (i, (c, _, p)) in emitted.iter().enumerate() {
        let x0 = if i == 0 {
            (centers[0] - step_w * 1.5).max(rect.x)
        } else {
            (centers[i - 1] + centers[i]) / 2.0
        };
        let x1 = if i + 1 == centers.len() {
            (centers[i] + step_w * 1.5).min(rect.right())
        } else {
            (centers[i] + centers[i + 1]) / 2.0
        };
        out.push((OChar { c: *c, x0, x1 }, *p));
    }
    Ok(out)
}

fn is_cyr(c: char) -> bool {
    matches!(c, '\u{0400}'..='\u{04FF}')
}

const HOMOGLYPHS: &[(char, char)] = &[
    ('a', 'а'),
    ('e', 'е'),
    ('o', 'о'),
    ('p', 'р'),
    ('c', 'с'),
    ('x', 'х'),
    ('y', 'у'),
    ('A', 'А'),
    ('B', 'В'),
    ('E', 'Е'),
    ('K', 'К'),
    ('M', 'М'),
    ('H', 'Н'),
    ('O', 'О'),
    ('P', 'Р'),
    ('C', 'С'),
    ('T', 'Т'),
    ('X', 'Х'),
];

/// Letter runs that mix scripts are OCR noise: bring lookalike letters to the run's majority
/// script (runs, not words: `vec!["кадр",` must not count `vec` against `кадр`).
fn fix_homoglyphs(chars: &mut [OChar]) {
    let mut start = 0;
    while start < chars.len() {
        if !chars[start].c.is_alphabetic() {
            start += 1;
            continue;
        }
        let end = chars[start..]
            .iter()
            .position(|c| !c.c.is_alphabetic())
            .map_or(chars.len(), |p| start + p);
        let word = &mut chars[start..end];
        let cyr = word.iter().filter(|c| is_cyr(c.c)).count();
        let lat = word.iter().filter(|c| c.c.is_ascii_alphabetic()).count();
        if cyr > 0 && lat > 0 {
            for ch in word.iter_mut() {
                ch.c = if cyr >= lat {
                    HOMOGLYPHS.iter().find(|(l, _)| *l == ch.c).map_or(ch.c, |(_, c)| *c)
                } else {
                    HOMOGLYPHS.iter().find(|(_, c)| *c == ch.c).map_or(ch.c, |(l, _)| *l)
                };
            }
        }
        start = end;
    }
}

fn mean(v: &[(OChar, f32)]) -> f32 {
    if v.is_empty() {
        0.0
    } else {
        v.iter().map(|x| x.1).sum::<f32>() / v.len() as f32
    }
}

/// Maximal runs of letters (start, end) whose script is Cyrillic (`cyr`) or not.
fn letter_runs(v: &[(OChar, f32)], cyr: bool) -> Vec<(usize, usize)> {
    let mut runs = Vec::new();
    let mut i = 0;
    while i < v.len() {
        if !v[i].0.c.is_alphabetic() {
            i += 1;
            continue;
        }
        let s = i;
        while i < v.len() && v[i].0.c.is_alphabetic() {
            i += 1;
        }
        let c = v[s..i].iter().filter(|x| is_cyr(x.0.c)).count();
        let l = (i - s) - c;
        if (c > 0 && c >= l) == cyr {
            runs.push((s, i));
        }
    }
    runs
}

/// Replace the parts of `base` covered by `runs` of `other` with those runs.
fn splice(base: Vec<(OChar, f32)>, other: &[(OChar, f32)], runs: &[(usize, usize)]) -> Vec<(OChar, f32)> {
    let spans: Vec<(f32, f32)> = runs.iter().map(|&(s, e)| (other[s].0.x0, other[e - 1].0.x1)).collect();
    let inside = |c: &OChar| {
        let m = (c.x0 + c.x1) / 2.0;
        spans.iter().any(|&(a, b)| m >= a && m <= b)
    };
    let mut out: Vec<(OChar, f32)> = base
        .into_iter()
        .filter(|(c, _)| c.c.is_whitespace() || !inside(c))
        .collect();
    // spaces the base saw inside a spliced word are gone with it
    out.retain(|(c, _)| !(c.c.is_whitespace() && spans.iter().any(|&(a, b)| c.x0 >= a && c.x1 <= b)));
    for &(s, e) in runs {
        out.extend(other[s..e].iter().cloned());
    }
    out.sort_by(|a, b| a.0.x0.partial_cmp(&b.0.x0).unwrap_or(std::cmp::Ordering::Equal));
    out
}

/// "auto": East-Slavic and English reads of the same line, combined by letter runs. The
/// script with more letters is the base; runs of the other script come from the model that
/// knows it (English is also better at code punctuation).
fn fuse(es: Vec<(OChar, f32)>, en: Vec<(OChar, f32)>) -> (Vec<OChar>, f32) {
    let cyr = es.iter().filter(|(c, _)| is_cyr(c.c)).count();
    let lat = es.iter().filter(|(c, _)| c.c.is_alphabetic() && !is_cyr(c.c)).count();
    let fused = if cyr == 0 {
        if mean(&en) + 0.05 >= mean(&es) { en } else { es }
    } else if cyr >= lat {
        // Cyrillic text: take Latin words from the English read where it is confident
        let runs: Vec<(usize, usize)> = letter_runs(&en, false)
            .into_iter()
            .filter(|&(s, e)| {
                let (a, b) = (en[s].0.x0, en[e - 1].0.x1);
                let es_part: Vec<(OChar, f32)> = es
                    .iter()
                    .filter(|(c, _)| (c.x0 + c.x1) / 2.0 >= a && (c.x0 + c.x1) / 2.0 <= b)
                    .cloned()
                    .collect();
                let es_lat = es_part
                    .iter()
                    .filter(|(c, _)| c.c.is_alphabetic() && !is_cyr(c.c))
                    .count();
                es_lat * 2 >= es_part.iter().filter(|(c, _)| c.c.is_alphabetic()).count().max(1)
                    && mean(&en[s..e]) >= mean(&es_part)
            })
            .collect();
        splice(es, &en, &runs)
    } else {
        // Latin text or code with a few Russian words
        let runs = letter_runs(&es, true);
        splice(en, &es, &runs)
    };
    let conf = mean(&fused);
    (fused.into_iter().map(|x| x.0).collect(), conf)
}

/// Recognize all text in `img`. `out_scale` = physical pixels per logical point (HiDPI text
/// is already big enough; 1x screens get upsampled for the detector).
pub fn recognize(img: &Pixmap, lang: &str, out_scale: f32) -> Result<OcrResult, String> {
    let scripts = scripts_for(lang);
    if scripts.is_empty() {
        return Err(format!("unknown language {lang}"));
    }
    let recs: Vec<Arc<Rec>> = scripts.iter().map(|s| recognizer(s)).collect::<Result<_, _>>()?;
    let area = (img.width() * img.height()).max(1) as f32;
    let scale = (2.0 / out_scale.max(0.5))
        .clamp(1.0, 2.0)
        .min((9.0e6 / area).sqrt())
        .max(0.5);
    let boxes = detect(img, scale)?;
    let extra = leftover_boxes(img, &boxes);
    let n_found = boxes.len();
    let mut lines = Vec::new();
    for (i, b) in boxes.into_iter().chain(extra).enumerate() {
        let leftover = i >= n_found;
        let (chars, conf) = if recs.len() == 2 && scripts[0].key == "eslav" {
            fuse(read(img, b, &recs[0])?, read(img, b, &recs[1])?)
        } else {
            let v = read(img, b, &recs[0])?;
            let c = mean(&v);
            (v.into_iter().map(|x| x.0).collect(), c)
        };
        let mut chars = chars;
        if scripts.iter().any(|s| s.key == "eslav") {
            fix_homoglyphs(&mut chars);
        }
        // trim and drop empty / pure-noise reads
        while chars.first().is_some_and(|c| c.c.is_whitespace()) {
            chars.remove(0);
        }
        while chars.last().is_some_and(|c| c.c.is_whitespace()) {
            chars.pop();
        }
        if chars.is_empty() || conf < 0.3 || looks_like_icon(&chars, conf, leftover) {
            continue;
        }
        lines.push(OLine { rect: b, chars });
    }
    layout::order(&mut lines);
    Ok(OcrResult { lines })
}
