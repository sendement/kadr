//! Document = captured base image + crop + stack of editable layers.
//! The JSON layout matches the Python version, so old history items keep opening.
use std::sync::Arc;
use std::time::Instant;

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::color::Color;
use crate::config::Obj;
use crate::geom::{IR, P, R};
use crate::i18n::{tr, trf};

pub const BLEND_MODES: &[(&str, &str)] = &[
    ("normal", "Normal"),
    ("multiply", "Multiply"),
    ("screen", "Screen"),
    ("overlay", "Overlay"),
    ("darken", "Darken"),
    ("lighten", "Lighten"),
    ("color_dodge", "Color Dodge"),
    ("color_burn", "Color Burn"),
    ("hard_light", "Hard Light"),
    ("soft_light", "Soft Light"),
    ("difference", "Difference"),
    ("exclusion", "Exclusion"),
    ("plus", "Linear Dodge (Add)"),
];

pub fn type_name(kind: &str) -> &'static str {
    match kind {
        "pen" => "Brush",
        "highlighter" => "Highlighter",
        "line" => "Line",
        "arrow" => "Arrow",
        "rect" => "Rectangle",
        "ellipse" => "Ellipse",
        "text" => "Text",
        "mark" => "Mark",
        "blur" => "Blur",
        "zoom" => "Magnifier",
        "ocr" => "Text recognition",
        _ => "Layer",
    }
}

/// Geometry in image pixels; which fields are set depends on the layer type.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct Geom {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pts: Option<Vec<[f32; 2]>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub p1: Option<[f32; 2]>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub p2: Option<[f32; 2]>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub r: Option<[f32; 4]>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pos: Option<[f32; 2]>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub anchor: Option<[f32; 2]>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub src: Option<[f32; 4]>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dst: Option<[f32; 4]>,
}

impl Geom {
    pub fn p1(&self) -> P {
        P::from(self.p1.unwrap_or_default())
    }
    pub fn p2(&self) -> P {
        P::from(self.p2.unwrap_or_default())
    }
    pub fn r(&self) -> R {
        R::from(self.r.unwrap_or_default())
    }
    pub fn pos(&self) -> P {
        P::from(self.pos.unwrap_or_default())
    }
    pub fn anchor(&self) -> P {
        P::from(self.anchor.or(self.pos).unwrap_or_default())
    }
    pub fn src(&self) -> R {
        R::from(self.src.unwrap_or_default())
    }
    pub fn dst(&self) -> R {
        R::from(self.dst.unwrap_or_default())
    }
    pub fn text(&self) -> &str {
        self.text.as_deref().unwrap_or("")
    }
    pub fn pts(&self) -> &[[f32; 2]] {
        self.pts.as_deref().unwrap_or(&[])
    }
}

/// Typed access to a style object (tool-specific look, free-form JSON like in Python).
pub trait Style {
    fn f(&self, k: &str, def: f32) -> f32;
    fn b(&self, k: &str) -> bool;
    fn bd(&self, k: &str, def: bool) -> bool;
    fn s<'a>(&'a self, k: &str, def: &'a str) -> &'a str;
    /// None for a missing key and for an explicit null ("no color")
    fn color(&self, k: &str) -> Option<Color>;
    fn has(&self, k: &str) -> bool;
}

impl Style for Obj {
    fn f(&self, k: &str, def: f32) -> f32 {
        self.get(k).and_then(Value::as_f64).map_or(def, |v| v as f32)
    }
    fn b(&self, k: &str) -> bool {
        self.bd(k, false)
    }
    fn bd(&self, k: &str, def: bool) -> bool {
        match self.get(k) {
            Some(Value::Bool(b)) => *b,
            Some(Value::Null) => false,
            Some(Value::Number(n)) => n.as_f64().is_some_and(|v| v != 0.0),
            _ => def,
        }
    }
    fn s<'a>(&'a self, k: &str, def: &'a str) -> &'a str {
        self.get(k).and_then(Value::as_str).unwrap_or(def)
    }
    fn color(&self, k: &str) -> Option<Color> {
        self.get(k).and_then(Value::as_str).and_then(Color::parse)
    }
    fn has(&self, k: &str) -> bool {
        self.contains_key(k)
    }
}

fn default_true() -> bool {
    true
}
fn default_one() -> f32 {
    1.0
}
fn default_normal() -> String {
    "normal".into()
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Layer {
    #[serde(rename = "type")]
    pub kind: String,
    pub g: Geom,
    pub style: Obj,
    #[serde(default = "new_id")]
    pub id: String,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default = "default_true")]
    pub visible: bool,
    #[serde(default = "default_one")]
    pub opacity: f32,
    #[serde(default = "default_normal")]
    pub blend: String,
    #[serde(default)]
    pub seq: i64,
    #[serde(default)]
    pub locked: bool,
}

pub fn new_id() -> String {
    format!("{:010x}", fastrand::u64(..) & 0xff_ffff_ffff)
}

#[derive(Clone)]
pub struct Snapshot {
    pub crop: Option<IR>,
    pub layers: Vec<Layer>,
    pub seq: i64,
}

pub struct Document {
    pub base: Arc<tiny_skia::Pixmap>,
    pub crop: Option<IR>,
    pub layers: Vec<Layer>, // bottom -> top
    pub meta: Map<String, Value>,
    pub seq: i64,
    pub undo_stack: Vec<Snapshot>,
    pub redo_stack: Vec<Snapshot>,
    last_key: Option<String>,
    last_time: Option<Instant>,
    pub dirty: bool,
    /// bumped on every change, lets views cache renders
    pub rev: u64,
}

impl Document {
    pub fn new(base: Arc<tiny_skia::Pixmap>, crop: Option<IR>, layers: Vec<Layer>, meta: Map<String, Value>) -> Self {
        let seq = layers.iter().map(|l| l.seq).max().unwrap_or(0);
        Document {
            base,
            crop,
            layers,
            meta,
            seq,
            undo_stack: vec![],
            redo_stack: vec![],
            last_key: None,
            last_time: None,
            dirty: false,
            rev: 0,
        }
    }

    pub fn base_rect(&self) -> IR {
        IR::new(0, 0, self.base.width() as i32, self.base.height() as i32)
    }

    pub fn touch(&mut self) {
        self.rev = self.rev.wrapping_add(1);
    }

    // --- layers -------------------------------------------------------
    pub fn new_layer(&mut self, kind: &str, g: Geom, style: &Obj) -> usize {
        self.seq += 1;
        let blend = if kind == "highlighter" { "multiply" } else { "normal" };
        self.layers.push(Layer {
            kind: kind.into(),
            g,
            style: style.clone(),
            id: new_id(),
            name: None,
            visible: true,
            opacity: 1.0,
            blend: blend.into(),
            seq: self.seq,
            locked: false,
        });
        self.touch();
        self.layers.len() - 1
    }

    pub fn find(&self, id: &str) -> Option<usize> {
        self.layers.iter().position(|l| l.id == id)
    }

    pub fn layer(&self, id: &str) -> Option<&Layer> {
        self.layers.iter().find(|l| l.id == id)
    }

    pub fn layer_mut(&mut self, id: &str) -> Option<&mut Layer> {
        self.layers.iter_mut().find(|l| l.id == id)
    }

    pub fn mark_start(&self) -> i64 {
        self.meta.get("mark_start").and_then(Value::as_i64).unwrap_or(1)
    }

    pub fn mark_numbers(&self) -> std::collections::HashMap<String, i64> {
        let mut marks: Vec<&Layer> = self.layers.iter().filter(|l| l.kind == "mark").collect();
        marks.sort_by_key(|l| l.seq);
        let start = self.mark_start();
        marks
            .into_iter()
            .enumerate()
            .map(|(i, l)| (l.id.clone(), i as i64 + start))
            .collect()
    }

    pub fn display_name(&self, layer: &Layer) -> String {
        if let Some(n) = &layer.name
            && !n.is_empty()
        {
            return n.clone();
        }
        if layer.kind == "mark" {
            let n = self
                .mark_numbers()
                .get(&layer.id)
                .map_or("?".to_string(), |n| n.to_string());
            return trf("Mark #{n}", &[("n", &n)]);
        }
        let mut same: Vec<&Layer> = self.layers.iter().filter(|l| l.kind == layer.kind).collect();
        same.sort_by_key(|l| l.seq);
        let idx = same.iter().position(|l| l.id == layer.id).unwrap_or(0);
        format!("{} {}", tr(type_name(&layer.kind)), idx + 1)
    }

    // --- undo ---------------------------------------------------------
    pub fn snapshot(&self) -> Snapshot {
        Snapshot {
            crop: self.crop,
            layers: self.layers.clone(),
            seq: self.seq,
        }
    }

    pub fn restore(&mut self, s: Snapshot) {
        self.crop = s.crop;
        self.layers = s.layers;
        self.seq = s.seq;
        self.touch();
    }

    /// Record state before a change. Same `key` within 0.8s coalesces (slider drags).
    pub fn push_undo(&mut self, snap: Option<Snapshot>, key: Option<String>) {
        let now = Instant::now();
        self.dirty = true;
        self.touch();
        if let (Some(k), Some(lk), Some(t)) = (&key, &self.last_key, self.last_time)
            && k == lk
            && now.duration_since(t).as_secs_f32() < 0.8
        {
            self.last_time = Some(now);
            return;
        }
        self.last_key = key;
        self.last_time = Some(now);
        let s = snap.unwrap_or_else(|| self.snapshot());
        self.undo_stack.push(s);
        if self.undo_stack.len() > 200 {
            self.undo_stack.remove(0);
        }
        self.redo_stack.clear();
    }

    pub fn undo(&mut self) -> bool {
        let Some(s) = self.undo_stack.pop() else { return false };
        self.redo_stack.push(self.snapshot());
        self.restore(s);
        self.last_key = None;
        self.dirty = true;
        true
    }

    pub fn redo(&mut self) -> bool {
        let Some(s) = self.redo_stack.pop() else { return false };
        self.undo_stack.push(self.snapshot());
        self.restore(s);
        self.last_key = None;
        self.dirty = true;
        true
    }

    // --- persistence --------------------------------------------------
    pub fn to_json(&self) -> Value {
        serde_json::json!({
            "version": 1,
            "meta": self.meta,
            "seq": self.seq,
            "crop": self.crop.map(|c| c.arr()),
            "layers": self.layers,
        })
    }

    pub fn from_json(base: Arc<tiny_skia::Pixmap>, d: &Value) -> Document {
        let crop = d
            .get("crop")
            .and_then(|c| serde_json::from_value::<[i32; 4]>(c.clone()).ok())
            .map(|c| IR::new(c[0], c[1], c[2], c[3]));
        let layers: Vec<Layer> = d
            .get("layers")
            .and_then(Value::as_array)
            .map(|a| {
                a.iter()
                    .filter_map(|l| serde_json::from_value(l.clone()).ok())
                    .collect()
            })
            .unwrap_or_default();
        let meta = d.get("meta").and_then(Value::as_object).cloned().unwrap_or_default();
        let mut doc = Document::new(base, crop, layers, meta);
        doc.seq = doc.seq.max(d.get("seq").and_then(Value::as_i64).unwrap_or(0));
        doc
    }

    pub fn meta_f(&self, k: &str, def: f32) -> f32 {
        self.meta.get(k).and_then(Value::as_f64).map_or(def, |v| v as f32)
    }
}
