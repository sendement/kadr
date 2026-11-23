//! Text for layers: system fonts via fontdb, shaping via rustybuzz, glyph outlines as
//! tiny-skia paths (so text gets the same outline/fill/blend treatment as shapes).
use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};

use fontdb::{Database, Family, ID, Query, Stretch, Style as FStyle, Weight};
use tiny_skia::{Path, PathBuilder};

static DB: OnceLock<Database> = OnceLock::new();
static DATA: OnceLock<Mutex<HashMap<ID, Arc<dyn AsRef<[u8]> + Send + Sync>>>> = OnceLock::new();
static FALLBACK: OnceLock<Mutex<HashMap<(ID, char), Option<ID>>>> = OnceLock::new();

fn fc_match(pattern: &str) -> Option<String> {
    let out = std::process::Command::new("fc-match")
        .args([pattern, "-f", "%{family[0]}"])
        .output()
        .ok()?;
    let s = String::from_utf8(out.stdout).ok()?;
    (!s.trim().is_empty()).then(|| s.trim().to_string())
}

/// Load the system font database (call early on a thread: it takes ~100 ms).
pub fn db() -> &'static Database {
    DB.get_or_init(|| {
        let mut db = Database::new();
        db.load_system_fonts();
        // fontdb's generic families default to Windows names; ask fontconfig instead
        if let Some(f) = fc_match("sans-serif") {
            db.set_sans_serif_family(f);
        }
        if let Some(f) = fc_match("serif") {
            db.set_serif_family(f);
        }
        if let Some(f) = fc_match("monospace") {
            db.set_monospace_family(f);
        }
        db
    })
}

pub fn families() -> Vec<String> {
    let mut v: Vec<String> = db()
        .faces()
        .filter_map(|f| f.families.first().map(|(n, _)| n.clone()))
        .collect();
    v.sort_by_key(|a| a.to_lowercase());
    v.dedup();
    v
}

fn data(id: ID) -> Option<Arc<dyn AsRef<[u8]> + Send + Sync>> {
    let map = DATA.get_or_init(Default::default);
    let mut m = map.lock().unwrap();
    if let Some(d) = m.get(&id) {
        return Some(d.clone());
    }
    let d: Arc<dyn AsRef<[u8]> + Send + Sync> = match &db().face(id)?.source {
        fontdb::Source::File(path) => Arc::new(std::fs::read(path).ok()?),
        fontdb::Source::Binary(b) => b.clone(),
        fontdb::Source::SharedFile(_, b) => b.clone(),
    };
    m.insert(id, d.clone());
    Some(d)
}

fn face_index(id: ID) -> u32 {
    db().face(id).map_or(0, |f| f.index)
}

pub fn query(family: &str, bold: bool, italic: bool) -> Option<ID> {
    let generic = match family.to_lowercase().as_str() {
        "" | "sans serif" | "sans-serif" | "sans" => Some(Family::SansSerif),
        "serif" => Some(Family::Serif),
        "monospace" | "mono" => Some(Family::Monospace),
        _ => None,
    };
    let fams: Vec<Family> = match generic {
        Some(g) => vec![g],
        None => vec![Family::Name(family), Family::SansSerif],
    };
    db().query(&Query {
        families: &fams,
        weight: if bold { Weight::BOLD } else { Weight::NORMAL },
        stretch: Stretch::Normal,
        style: if italic { FStyle::Italic } else { FStyle::Normal },
    })
    .or_else(|| db().faces().next().map(|f| f.id))
}

fn has_glyph(id: ID, c: char) -> bool {
    let Some(d) = data(id) else { return false };
    ttf_parser::Face::parse((*d).as_ref(), face_index(id))
        .ok()
        .and_then(|f| f.glyph_index(c))
        .is_some()
}

pub fn fallback_for(primary: ID, c: char) -> ID {
    if has_glyph(primary, c) {
        primary
    } else {
        fallback(primary, c, false)
    }
}

/// A face that can draw `c` when `primary` cannot (CJK in a Latin font etc.).
fn fallback(primary: ID, c: char, bold: bool) -> ID {
    let cache = FALLBACK.get_or_init(Default::default);
    if let Some(v) = cache.lock().unwrap().get(&(primary, c)) {
        return v.unwrap_or(primary);
    }
    let mut found = None;
    let preferred = [
        "Noto Sans",
        "Noto Sans CJK SC",
        "Noto Sans CJK JP",
        "Noto Sans CJK KR",
        "DejaVu Sans",
        "Noto Color Emoji",
        "Noto Sans Symbols",
        "Noto Sans Symbols 2",
    ];
    for fam in preferred {
        if let Some(id) = db().query(&Query {
            families: &[Family::Name(fam)],
            weight: if bold { Weight::BOLD } else { Weight::NORMAL },
            ..Default::default()
        }) && has_glyph(id, c)
        {
            found = Some(id);
            break;
        }
    }
    if found.is_none() {
        found = db().faces().map(|f| f.id).find(|&id| has_glyph(id, c));
    }
    cache.lock().unwrap().insert((primary, c), found);
    found.unwrap_or(primary)
}

#[derive(Clone, Copy, Debug)]
pub struct Metrics {
    pub ascent: f32,
    pub height: f32, // ascent + descent (QFontMetricsF::height)
}

pub struct Font {
    pub id: ID,
    pub size: f32,
    pub bold: bool,
}

impl Font {
    pub fn new(family: &str, size: f32, bold: bool, italic: bool) -> Option<Font> {
        Some(Font {
            id: query(family, bold, italic)?,
            size,
            bold,
        })
    }

    pub fn metrics(&self) -> Metrics {
        let d = data(self.id);
        let face = d
            .as_ref()
            .and_then(|d| ttf_parser::Face::parse((**d).as_ref(), face_index(self.id)).ok());
        match face {
            Some(f) => {
                let k = self.size / f.units_per_em() as f32;
                let a = f.ascender() as f32 * k;
                let ds = -(f.descender() as f32) * k;
                Metrics {
                    ascent: a,
                    height: a + ds,
                }
            }
            None => Metrics {
                ascent: self.size * 0.8,
                height: self.size,
            },
        }
    }

    /// Split into runs of one face each (primary + per-character fallbacks).
    fn runs(&self, text: &str) -> Vec<(ID, String)> {
        let mut runs: Vec<(ID, String)> = Vec::new();
        for c in text.chars() {
            let id = if c.is_whitespace() || has_glyph(self.id, c) {
                self.id
            } else {
                fallback(self.id, c, self.bold)
            };
            match runs.last_mut() {
                Some((rid, s)) if *rid == id => s.push(c),
                _ => runs.push((id, c.to_string())),
            }
        }
        runs
    }

    pub fn advance(&self, text: &str) -> f32 {
        self.shape(text, 0.0, 0.0, None)
    }

    /// Shape `text` with its baseline at (x, y); appends glyph outlines to `pb` when given.
    /// Returns the advance width.
    pub fn shape(&self, text: &str, x: f32, y: f32, mut pb: Option<&mut PathBuilder>) -> f32 {
        let mut pen = x;
        for (id, run) in self.runs(text) {
            let Some(d) = data(id) else { continue };
            let bytes: &[u8] = (*d).as_ref();
            let Some(face) = rustybuzz::Face::from_slice(bytes, face_index(id)) else {
                continue;
            };
            let k = self.size / face.units_per_em() as f32;
            let mut buf = rustybuzz::UnicodeBuffer::new();
            buf.push_str(&run);
            let out = rustybuzz::shape(&face, &[], buf);
            for (info, pos) in out.glyph_infos().iter().zip(out.glyph_positions()) {
                if let Some(pb) = pb.as_deref_mut() {
                    let gx = pen + pos.x_offset as f32 * k;
                    let gy = y - pos.y_offset as f32 * k;
                    let mut ob = Outline { pb, k, x: gx, y: gy };
                    face.outline_glyph(ttf_parser::GlyphId(info.glyph_id as u16), &mut ob);
                }
                pen += pos.x_advance as f32 * k;
            }
        }
        pen - x
    }

    pub fn path(&self, text: &str, x: f32, y: f32) -> Option<Path> {
        let mut pb = PathBuilder::new();
        self.shape(text, x, y, Some(&mut pb));
        pb.finish()
    }
}

struct Outline<'a> {
    pb: &'a mut PathBuilder,
    k: f32,
    x: f32,
    y: f32,
}

impl Outline<'_> {
    fn t(&self, x: f32, y: f32) -> (f32, f32) {
        (self.x + x * self.k, self.y - y * self.k)
    }
}

impl ttf_parser::OutlineBuilder for Outline<'_> {
    fn move_to(&mut self, x: f32, y: f32) {
        let (a, b) = self.t(x, y);
        self.pb.move_to(a, b);
    }
    fn line_to(&mut self, x: f32, y: f32) {
        let (a, b) = self.t(x, y);
        self.pb.line_to(a, b);
    }
    fn quad_to(&mut self, x1: f32, y1: f32, x: f32, y: f32) {
        let (a, b) = self.t(x1, y1);
        let (c, d) = self.t(x, y);
        self.pb.quad_to(a, b, c, d);
    }
    fn curve_to(&mut self, x1: f32, y1: f32, x2: f32, y2: f32, x: f32, y: f32) {
        let (a, b) = self.t(x1, y1);
        let (c, d) = self.t(x2, y2);
        let (e, f) = self.t(x, y);
        self.pb.cubic_to(a, b, c, d, e, f);
    }
    fn close(&mut self) {
        self.pb.close();
    }
}
