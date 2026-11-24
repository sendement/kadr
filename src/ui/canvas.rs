//! Interactive canvas shared by the capture overlay and the history editor.
use std::collections::HashSet;
use std::time::{Duration, Instant};

use egui::{Color32, CursorIcon, Key, Modifiers, PointerButton, Pos2, Rect, Stroke, TextureHandle, Vec2, pos2, vec2};
use serde_json::Value;
use tiny_skia::Pixmap;

use crate::color::Color;
use crate::config::{Config, Obj};
use crate::geom::{IR, P, R, p};
use crate::i18n::{tr, trf};
use crate::model::{Document, Geom, Layer, Snapshot, Style};
use crate::shapes;

use super::colorpicker::ColorPopup;

pub const HANDLE_R: f32 = 6.0;
pub const TOOL_KEYS: &[(Key, &str)] = &[
    (Key::V, "pointer"),
    (Key::P, "pen"),
    (Key::H, "highlighter"),
    (Key::L, "line"),
    (Key::A, "arrow"),
    (Key::R, "rect"),
    (Key::E, "ellipse"),
    (Key::T, "text"),
    (Key::M, "mark"),
    (Key::B, "blur"),
    (Key::Z, "zoom"),
    (Key::O, "ocr"),
];
const DOUBLE_CLICK: Duration = Duration::from_millis(400);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    Copy,
    Save,
    Export,
    Edit,
    Pin,
    Cancel,
    /// the OCR tool copied text (in `Canvas::copied_text`)
    CopyText,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Idle,
    Selecting,
    Edit,
}

enum DragKind {
    Pan {
        last: Pos2,
    },
    Select {
        start: P,
        end: P,
        moved: bool,
        had_crop: bool,
    },
    Draw {
        id: String,
        start: P,
    },
    MarkNew {
        id: String,
    },
    OcrSelect,
    Move {
        ids: Vec<String>,
        last: P,
        whole: bool,
        part: Option<&'static str>,
    },
    Handle {
        id: String,
        key: String,
        orig: Geom,
    },
    Crop {
        key: String,
        orig: R,
    },
    CropMove {
        start: P,
        orig: R,
    },
}

struct Drag {
    kind: DragKind,
    snap: Option<Snapshot>,
    pushed: bool,
}

/// Multi-monitor coordination for the overlay (one canvas per output).
#[derive(Default)]
pub struct Ctl {
    pub active: usize,
    pub locked: Option<usize>,
    /// the locked screen may be abandoned while nothing is drawn on it yet
    pub locked_movable: bool,
    /// canvas whose selection must be reset (another screen took over)
    pub reset: Option<usize>,
    /// canvas that wants to become active
    pub activate: Option<usize>,
}

impl Ctl {
    fn can_start(&mut self, me: usize) -> bool {
        match self.locked {
            None => true,
            Some(l) if l == me => true,
            Some(l) => {
                if self.locked_movable {
                    self.reset = Some(l);
                    self.locked = None;
                    true
                } else {
                    false
                }
            }
        }
    }
    fn lock(&mut self, me: usize) {
        self.locked = Some(me);
        if self.active != me {
            self.activate = Some(me);
        }
    }
    fn hover(&mut self, me: usize) {
        if self.active != me && (self.locked == Some(me) || self.locked.is_none() || self.locked_movable) {
            self.activate = Some(me);
        }
    }
}

pub struct TextEditState {
    pub id: String,
    snap: Snapshot,
    new: bool,
    orig: String,
}

pub struct Canvas {
    pub doc: Document,
    pub editor: bool,
    pub index: usize,
    pub accent: Color,
    pub view_scale: f32, // points per image pixel
    pub offset: Vec2,    // widget-local points
    pub windows: Vec<(R, String)>,
    pub active: bool,
    pub mode: Mode,
    pub tool: String,
    pub selected: Option<String>,
    pub multi: HashSet<String>,
    drag: Option<Drag>,
    pub text_edit: Option<TextEditState>,
    caret_t0: Instant,
    pub hover_win: Option<usize>,
    pub mouse: Option<Pos2>, // widget-local
    cache: Option<(u64, IR, Pixmap)>,
    tex: Option<(u64, IR, TextureHandle)>,
    base_tex: Option<(TextureHandle, egui::TextureOptions)>,
    loupe_tex: Option<TextureHandle>,
    below: Option<(IR, usize, Pixmap)>,
    pub pick: Option<String>, // eyedropper target: color popup key
    pub popup: Option<ColorPopup>,
    pub layers_visible: bool,
    toast: Option<(String, Instant)>,
    pub user_view: bool,
    pub live_copy: bool,
    live_due: Option<Instant>,
    live_crc: Option<u64>,
    pending_click: Option<(Instant, Option<usize>)>,
    last_press: Option<(Instant, Pos2)>,
    actions: Vec<Action>,
    pub rect: Rect, // widget rect in screen points
    pub panel_rects: Vec<Rect>,
    pub panel_sizes: [Vec2; 4], // toolbar, actions, props, layers (last frame)
    pub props_key: Option<(String, Option<String>)>,
    pub rename: Option<(String, String)>,
    pub layer_drag: Option<String>,
    pub fitted_for: Option<Vec2>,
    pub wheel_acc: f32,
    started: Instant,
    pub ocr: Option<super::ocr_tool::OcrState>,
    pub copied_text: Option<String>,
}

impl Canvas {
    pub fn new(doc: Document, cfg: &Config, editor: bool, out_scale: f32, windows: Vec<(R, String)>) -> Canvas {
        let mode = if editor || doc.crop.is_some() {
            Mode::Edit
        } else {
            Mode::Idle
        };
        Canvas {
            doc,
            editor,
            index: 0,
            accent: cfg.accent(),
            view_scale: 1.0 / out_scale,
            offset: Vec2::ZERO,
            windows,
            active: editor,
            mode,
            tool: "pointer".into(),
            selected: None,
            multi: HashSet::new(),
            drag: None,
            text_edit: None,
            caret_t0: Instant::now(),
            hover_win: None,
            mouse: None,
            cache: None,
            tex: None,
            base_tex: None,
            loupe_tex: None,
            below: None,
            pick: None,
            popup: None,
            layers_visible: cfg.bool("layers_visible", false),
            toast: None,
            user_view: false,
            live_copy: cfg.bool("live_copy", false),
            live_due: None,
            live_crc: None,
            pending_click: None,
            last_press: None,
            actions: vec![],
            rect: Rect::ZERO,
            panel_rects: vec![],
            panel_sizes: [Vec2::ZERO; 4],
            props_key: None,
            rename: None,
            layer_drag: None,
            fitted_for: None,
            wheel_acc: 0.0,
            started: Instant::now(),
            ocr: None,
            copied_text: None,
        }
    }

    pub fn started_secs(&self) -> f32 {
        self.started.elapsed().as_secs_f32()
    }

    pub fn accent32(&self) -> Color32 {
        self.accent.egui()
    }

    pub fn set_selected(&mut self, id: Option<String>) {
        self.selected = id;
        self.multi.clear();
    }

    pub fn sel_ids(&self) -> HashSet<String> {
        let mut ids = self.multi.clone();
        if let Some(s) = &self.selected {
            ids.insert(s.clone());
        }
        ids
    }

    pub fn sel_layers(&self) -> Vec<&Layer> {
        let ids = self.sel_ids();
        self.doc.layers.iter().filter(|l| ids.contains(&l.id)).collect()
    }

    pub fn selected_layer(&self) -> Option<&Layer> {
        self.selected.as_ref().and_then(|id| self.doc.layer(id))
    }

    // ------------------------------------------------------------------ view
    pub fn to_img(&self, q: Pos2) -> P {
        p(
            (q.x - self.offset.x) / self.view_scale,
            (q.y - self.offset.y) / self.view_scale,
        )
    }

    pub fn to_w(&self, q: P) -> Pos2 {
        pos2(
            q.x * self.view_scale + self.offset.x,
            q.y * self.view_scale + self.offset.y,
        )
    }

    /// image px -> screen points (absolute, for painting)
    pub fn to_s(&self, q: P) -> Pos2 {
        self.to_w(q) + self.rect.min.to_vec2()
    }

    pub fn rect_w(&self, rr: R) -> Rect {
        Rect::from_two_pos(self.to_w(rr.tl()), self.to_w(rr.br()))
    }

    pub fn rect_s(&self, rr: R) -> Rect {
        self.rect_w(rr).translate(self.rect.min.to_vec2())
    }

    fn tol(&self) -> f32 {
        5.0 / self.view_scale
    }

    fn img_rect(&self) -> R {
        self.doc.base_rect().f()
    }

    fn clamp_img(&self, q: P) -> P {
        let r = self.img_rect();
        p(q.x.clamp(0.0, r.w), q.y.clamp(0.0, r.h))
    }

    pub fn fit_view(&mut self, ppp: f32) {
        if !self.editor || self.rect.width() < 50.0 {
            return;
        }
        let r = self.doc.crop.unwrap_or_else(|| self.doc.base_rect()).f();
        let w = Rect::from_min_size(Pos2::ZERO, self.rect.size());
        let avail = Rect::from_min_max(
            w.min + vec2(64.0, 58.0),
            w.max - vec2(20.0 + if self.layers_visible { 250.0 } else { 0.0 }, 58.0),
        );
        let s = (avail.width() / r.w).min(avail.height() / r.h).min(1.0 / ppp);
        self.view_scale = s.max(0.02);
        self.offset = avail.center().to_vec2() - vec2(r.center().x, r.center().y) * self.view_scale;
        self.user_view = false;
    }

    fn zoom_at(&mut self, factor: f32, q: Pos2) {
        let ip = self.to_img(q);
        self.view_scale = (self.view_scale * factor).clamp(0.02, 32.0);
        self.offset = q.to_vec2() - vec2(ip.x, ip.y) * self.view_scale;
        self.user_view = true;
    }

    // ----------------------------------------------------------------- state
    pub fn enter_edit(&mut self) {
        self.mode = Mode::Edit;
        self.props_key = None;
        if self.editor {
            self.fitted_for = None;
        }
        self.invalidate();
    }

    pub fn reset_selection(&mut self, ctl: Option<&mut Ctl>) {
        self.finish_text();
        self.doc.crop = None;
        self.ocr = None;
        self.doc.layers.clear();
        self.doc.undo_stack.clear();
        self.doc.redo_stack.clear();
        self.doc.touch();
        self.set_selected(None);
        self.mode = Mode::Idle;
        self.popup = None;
        if let Some(c) = ctl
            && c.locked == Some(self.index)
        {
            c.locked = None;
        }
    }

    /// Nothing drawn and no edit in progress: another screen may take over.
    pub fn movable(&self) -> bool {
        self.doc.layers.is_empty() && self.drag.is_none() && self.text_edit.is_none()
    }

    pub fn invalidate(&mut self) {
        self.doc.touch();
        if self.live_copy && self.mode == Mode::Edit && self.doc.crop.is_some() {
            self.live_due = Some(Instant::now() + Duration::from_millis(350));
        }
    }

    pub fn toggle_live_copy(&mut self, cfg: &mut Config) {
        self.live_copy = !self.live_copy;
        cfg.set("live_copy", self.live_copy);
        if self.live_copy {
            self.live_crc = None;
            self.live_due = Some(Instant::now());
            self.toast(tr("Live clipboard: the final image is updated on every change"));
        } else {
            self.live_due = None;
            self.toast(tr("Live clipboard off"));
        }
    }

    fn live_copy_tick(&mut self) {
        let Some(due) = self.live_due else { return };
        if Instant::now() < due {
            return;
        }
        if !self.live_copy || self.mode != Mode::Edit || self.doc.crop.is_none() {
            self.live_due = None;
            return;
        }
        if self.drag.is_some() || self.text_edit.is_some() {
            // wait until the edit settles
            self.live_due = Some(Instant::now() + Duration::from_millis(350));
            return;
        }
        self.live_due = None;
        self.ensure_cache();
        if let Some((_, _, img)) = &self.cache {
            let mut h = std::collections::hash_map::DefaultHasher::new();
            std::hash::Hash::hash(img.data(), &mut h);
            std::hash::Hash::hash(&(img.width(), img.height()), &mut h);
            let crc = std::hash::Hasher::finish(&h);
            if Some(crc) != self.live_crc {
                self.live_crc = Some(crc);
                crate::sys::copy_image(img);
            }
        }
    }

    pub fn toast(&mut self, text: String) {
        self.toast = Some((text, Instant::now()));
    }

    pub fn take_actions(&mut self) -> Vec<Action> {
        std::mem::take(&mut self.actions)
    }

    pub fn request(&mut self, a: Action) {
        self.actions.push(a);
    }

    pub fn start_pick(&mut self, key: String) {
        self.pick = Some(key);
        self.toast(tr("Eyedropper: click on the screenshot (Esc — cancel)"));
    }

    pub fn color_at(&mut self, ip: P) -> Color {
        let (x, y) = (ip.x.floor() as i32, ip.y.floor() as i32);
        if let Some(crop) = self.doc.crop
            && crop.contains(x, y)
        {
            self.ensure_cache();
            if let Some((_, c, img)) = &self.cache
                && let Some(px) = img.pixel((x - c.x) as u32, (y - c.y) as u32)
            {
                let d = px.demultiply();
                return Color::rgb(d.red(), d.green(), d.blue());
            }
        }
        if let Some(px) = (x >= 0 && y >= 0)
            .then(|| self.doc.base.pixel(x as u32, y as u32))
            .flatten()
        {
            let d = px.demultiply();
            return Color::rgb(d.red(), d.green(), d.blue());
        }
        Color::rgb(0, 0, 0)
    }

    // ----------------------------------------------------------------- tools
    pub fn set_tool(&mut self, name: &str) {
        self.finish_text();
        if let Some(o) = &mut self.ocr {
            o.sel = None;
        }
        self.tool = name.to_string();
        if let Some(l) = self.selected_layer()
            && name != "pointer"
            && l.kind != name
        {
            self.set_selected(None);
        }
        self.props_key = None;
    }

    /// (kind, style, layer id) the properties bar edits: the selected layer or the active tool.
    pub fn props_target(&self, cfg: &Config) -> Option<(String, Obj, Option<String>)> {
        if let Some(l) = self.selected_layer()
            && (self.tool == "pointer" || self.tool == l.kind)
        {
            return Some((l.kind.clone(), l.style.clone(), Some(l.id.clone())));
        }
        cfg.has_style(&self.tool)
            .then(|| (self.tool.clone(), cfg.style(&self.tool), None))
    }

    pub fn apply_style(&mut self, cfg: &mut Config, key: &str, value: Value) {
        let Some((kind, _, layer)) = self.props_target(cfg) else {
            return;
        };
        cfg.style_mut(&kind).insert(key.into(), value.clone());
        if layer.is_some() {
            let ids: Vec<String> = {
                let sel: Vec<&Layer> = self.sel_layers().into_iter().filter(|l| l.style.has(key)).collect();
                if sel.is_empty() {
                    layer.into_iter().collect()
                } else {
                    sel.iter().map(|l| l.id.clone()).collect()
                }
            };
            self.doc.push_undo(None, Some(format!("style:{}:{key}", ids.join(","))));
            for id in &ids {
                if let Some(l) = self.doc.layer_mut(id) {
                    l.style.insert(key.into(), value.clone());
                }
            }
            self.invalidate();
        }
    }

    pub fn set_layer_prop(&mut self, prop: &str, value: Value, layer: Option<String>) {
        let ids: Vec<String> = match layer {
            Some(id) => vec![id],
            None => self.sel_layers().iter().map(|l| l.id.clone()).collect(),
        };
        if ids.is_empty() {
            return;
        }
        self.doc.push_undo(None, Some(format!("prop:{}:{prop}", ids.join(","))));
        for id in &ids {
            if let Some(l) = self.doc.layer_mut(id) {
                match prop {
                    "blend" => l.blend = value.as_str().unwrap_or("normal").to_string(),
                    "opacity" => l.opacity = value.as_f64().unwrap_or(1.0) as f32,
                    "visible" => l.visible = value.as_bool().unwrap_or(true),
                    "name" => l.name = value.as_str().map(String::from),
                    _ => {}
                }
            }
        }
        self.invalidate();
        if prop == "name" || prop == "visible" {
            self.props_key = None;
        }
    }

    pub fn select(&mut self, id: Option<String>, add: bool) {
        if add && let Some(id) = &id {
            let mut ids = self.sel_ids();
            if !ids.remove(id) {
                ids.insert(id.clone());
            }
            let cur = ids.contains(id).then(|| id.clone());
            self.select_many(ids, cur);
            return;
        }
        if id == self.selected && self.multi.is_empty() {
            return;
        }
        if let Some(te) = &self.text_edit
            && id.as_ref() != Some(&te.id)
        {
            self.finish_text();
        }
        self.set_selected(id);
        if let Some(l) = self.selected_layer()
            && self.tool != "pointer"
            && self.tool != l.kind
        {
            self.tool = "pointer".into();
        }
        self.props_key = None;
    }

    pub fn select_many(&mut self, ids: HashSet<String>, current: Option<String>) {
        self.finish_text();
        let layers: Vec<String> = self
            .doc
            .layers
            .iter()
            .filter(|l| ids.contains(&l.id))
            .map(|l| l.id.clone())
            .collect();
        let cur = current.filter(|c| ids.contains(c)).or_else(|| layers.last().cloned());
        self.selected = cur.clone();
        self.multi = if layers.len() > 1 {
            layers.into_iter().collect()
        } else {
            HashSet::new()
        };
        if let Some(l) = cur.and_then(|c| self.doc.layer(&c).map(|l| l.kind.clone()))
            && self.tool != "pointer"
            && self.tool != l
        {
            self.tool = "pointer".into();
        }
        self.props_key = None;
    }

    fn layer_at(&self, ip: P, types: Option<&[&str]>) -> Option<String> {
        // a generous grab zone: anything within ~12px on screen, plus the box around selected layers
        let tol = 12.0 / self.view_scale;
        let pad = 8.0 / self.view_scale;
        let ok = |l: &Layer| l.visible && !l.locked && types.is_none_or(|t| t.contains(&l.kind.as_str()));
        for l in self.sel_layers().iter().rev() {
            // zoom/mark bounds span empty space between their parts: those need a precise hit
            if ok(l) && l.kind != "zoom" && l.kind != "mark" && shapes::bounds(l).grow(pad).contains(ip) {
                return Some(l.id.clone());
            }
        }
        self.doc
            .layers
            .iter()
            .rev()
            .find(|l| ok(l) && shapes::hit(l, ip, tol).is_some())
            .map(|l| l.id.clone())
    }

    fn handle_at(&self, q: Pos2) -> Option<String> {
        if self.text_edit.is_some() || !self.multi.is_empty() {
            return None;
        }
        let l = self.selected_layer()?;
        shapes::handles(l)
            .into_iter()
            .find(|(_, pt)| self.to_w(*pt).distance(q) <= HANDLE_R + 3.0)
            .map(|(k, _)| k)
    }

    fn crop_handle_at(&self, q: Pos2) -> Option<String> {
        if self.mode != Mode::Edit {
            return None;
        }
        let cw = self.rect_w(self.doc.crop?.f());
        let cwr = R::of(cw);
        for (key, pt) in shapes::rect_handle_points(cwr) {
            if (pt.x - q.x).abs() <= 7.0 && (pt.y - q.y).abs() <= 7.0 {
                return Some(key.to_string());
            }
        }
        if self.tool == "pointer" && cw.expand(5.0).contains(q) && !cw.shrink(5.0).contains(q) {
            let mut key = String::new();
            if (q.y - cw.top()).abs() <= 5.0 {
                key.push('t');
            } else if (q.y - cw.bottom()).abs() <= 5.0 {
                key.push('b');
            }
            if (q.x - cw.left()).abs() <= 5.0 {
                key.push('l');
            } else if (q.x - cw.right()).abs() <= 5.0 {
                key.push('r');
            }
            return (!key.is_empty()).then_some(key);
        }
        None
    }

    fn new_layer(&mut self, cfg: &Config, kind: &str, g: Geom, push: bool) -> (String, Snapshot) {
        let snap = self.doc.snapshot();
        let idx = self.doc.new_layer(kind, g, &cfg.style(kind));
        if push {
            self.doc.push_undo(Some(snap.clone()), None);
        }
        let id = self.doc.layers[idx].id.clone();
        self.set_selected(Some(id.clone()));
        (id, snap)
    }

    fn discard_last(&mut self) {
        if let Some(s) = self.doc.undo_stack.pop() {
            self.doc.restore(s);
        }
        self.set_selected(None);
    }

    // ------------------------------------------------------------------ text
    fn start_text(&mut self, id: String, snap: Snapshot, new: bool) {
        let orig = self.doc.layer(&id).map(|l| l.g.text().to_string()).unwrap_or_default();
        self.text_edit = Some(TextEditState {
            id: id.clone(),
            snap,
            new,
            orig,
        });
        self.set_selected(Some(id));
        self.caret_t0 = Instant::now();
        self.props_key = None;
        self.invalidate();
    }

    pub fn finish_text(&mut self) {
        let Some(t) = self.text_edit.take() else { return };
        let text = self.doc.layer(&t.id).map(|l| l.g.text().to_string());
        match text {
            Some(s) if s.trim().is_empty() => {
                if let Some(i) = self.doc.find(&t.id) {
                    self.doc.layers.remove(i);
                }
                if !t.new {
                    self.doc.push_undo(Some(t.snap), None);
                }
                self.set_selected(None);
            }
            Some(s) if t.new || s != t.orig => self.doc.push_undo(Some(t.snap), None),
            _ => {}
        }
        self.props_key = None;
        self.invalidate();
    }

    fn text_key(&mut self, ev: &egui::Event, ctx: &egui::Context) {
        let Some(id) = self.text_edit.as_ref().map(|t| t.id.clone()) else {
            return;
        };
        let Some(mut txt) = self.doc.layer(&id).map(|l| l.g.text().to_string()) else {
            return;
        };
        match ev {
            egui::Event::Key {
                key,
                pressed: true,
                modifiers,
                ..
            } => {
                let ctrl = modifiers.ctrl;
                match key {
                    Key::Escape => return self.finish_text(),
                    Key::Enter if ctrl => return self.finish_text(),
                    Key::Enter => txt.push('\n'),
                    Key::Backspace if ctrl => {
                        let t = txt.trim_end().to_string();
                        txt = match t.rfind([' ', '\n']) {
                            Some(i) => t[..=i].to_string(),
                            None => String::new(),
                        };
                    }
                    Key::Backspace => {
                        txt.pop();
                    }
                    Key::Z if ctrl => {
                        self.finish_text();
                        self.undo();
                        return;
                    }
                    _ => return,
                }
            }
            egui::Event::Text(s) => txt.push_str(s),
            egui::Event::Paste(s) => txt.push_str(s),
            _ => return,
        }
        if let Some(l) = self.doc.layer_mut(&id) {
            l.g.text = Some(txt);
        }
        self.caret_t0 = Instant::now();
        self.invalidate();
        ctx.request_repaint();
    }

    // --------------------------------------------------------------- editing
    fn begin_drag(&mut self, kind: DragKind, layer: Option<&str>, pushed: bool) {
        self.drag = Some(Drag {
            kind,
            snap: if pushed { None } else { Some(self.doc.snapshot()) },
            pushed,
        });
        self.below = None;
        if let (Some(id), Some(crop)) = (layer, self.doc.crop)
            && let Some(idx) = self.doc.find(id)
        {
            let img = shapes::composite(&self.doc, crop, &self.doc.layers[..idx], None);
            self.below = Some((crop, idx, img));
        }
    }

    fn changed(&mut self) {
        if let Some(d) = &mut self.drag
            && !d.pushed
        {
            d.pushed = true;
            let snap = d.snap.take();
            self.doc.push_undo(snap, None);
        }
        self.invalidate();
    }

    pub fn undo(&mut self) {
        self.finish_text();
        if self.doc.undo() {
            self.after_history();
        }
    }

    pub fn redo(&mut self) {
        self.finish_text();
        if self.doc.redo() {
            self.after_history();
        }
    }

    fn after_history(&mut self) {
        let sel = self.selected.clone().filter(|s| self.doc.layer(s).is_some());
        let multi: HashSet<String> = self
            .multi
            .iter()
            .filter(|i| self.doc.layer(i).is_some())
            .cloned()
            .collect();
        self.selected = sel;
        self.multi = multi;
        self.below = None;
        if self.doc.crop.is_none() && !self.editor {
            self.reset_selection(None);
            return;
        }
        self.props_key = None;
        self.invalidate();
    }

    pub fn delete_selected(&mut self) {
        let ids = self.sel_ids();
        if ids.is_empty() {
            return;
        }
        self.doc.push_undo(None, None);
        self.doc.layers.retain(|l| !ids.contains(&l.id));
        self.set_selected(None);
        self.props_key = None;
        self.invalidate();
    }

    pub fn duplicate_selected(&mut self) {
        let src: Vec<Layer> = self.sel_layers().into_iter().cloned().collect();
        if src.is_empty() {
            return;
        }
        self.doc.push_undo(None, None);
        let mut copies = Vec::new();
        for l in src.iter().rev() {
            let mut c = l.clone();
            c.id = crate::model::new_id();
            self.doc.seq += 1;
            c.seq = self.doc.seq;
            if let Some(n) = &mut c.name {
                n.push_str(&tr(" (copy)"));
            }
            shapes::translate(&mut c, 16.0, 16.0, true, None);
            let at = self.doc.find(&l.id).map_or(self.doc.layers.len(), |i| i + 1);
            copies.push(c.id.clone());
            self.doc.layers.insert(at, c);
        }
        let first = copies.first().cloned();
        self.select_many(copies.into_iter().collect(), first);
        self.invalidate();
    }

    pub fn move_selected(&mut self, dir: i32) {
        let ids = self.sel_ids();
        if ids.is_empty() {
            return;
        }
        let snap = self.doc.snapshot();
        let n = self.doc.layers.len() as i32;
        let mut moved = false;
        let order: Vec<i32> = if dir > 0 {
            (0..n).rev().collect()
        } else {
            (0..n).collect()
        };
        for i in order {
            let j = i + dir;
            if j >= 0
                && j < n
                && ids.contains(&self.doc.layers[i as usize].id)
                && !ids.contains(&self.doc.layers[j as usize].id)
            {
                self.doc.layers.swap(i as usize, j as usize);
                moved = true;
            }
        }
        if moved {
            self.doc.push_undo(Some(snap), None);
            self.invalidate();
        }
    }

    /// New bottom->top order by ids (layers panel drag).
    pub fn reorder(&mut self, ids: &[String]) {
        let layers: Option<Vec<Layer>> = ids.iter().map(|i| self.doc.layer(i).cloned()).collect();
        let Some(layers) = layers else { return };
        if layers.len() != self.doc.layers.len() || layers.iter().zip(&self.doc.layers).all(|(a, b)| a.id == b.id) {
            return;
        }
        self.doc.push_undo(None, None);
        self.doc.layers = layers;
        self.invalidate();
    }

    fn nudge(&mut self, dx: f32, dy: f32) {
        let ids: Vec<String> = self.sel_layers().iter().map(|l| l.id.clone()).collect();
        if !ids.is_empty() {
            self.doc.push_undo(None, Some(format!("nudge:{}", ids.join(","))));
            for id in &ids {
                if let Some(l) = self.doc.layer_mut(id) {
                    shapes::translate(l, dx, dy, true, None);
                }
            }
        } else if let Some(c) = self.doc.crop {
            self.doc.push_undo(None, Some("nudge:crop".into()));
            let r = c.translated(dx as i32, dy as i32);
            if self.doc.base_rect().contains_rect(r) {
                self.doc.crop = Some(r);
            }
        }
        self.invalidate();
    }

    fn select_full_screen(&mut self) {
        if self.mode == Mode::Edit {
            self.doc.push_undo(None, None);
        }
        self.doc.crop = Some(self.doc.base_rect());
        self.hover_win = None;
        self.enter_edit();
    }

    // --------------------------------------------------------------- mouse
    fn press(&mut self, q: Pos2, button: PointerButton, mods: Modifiers, cfg: &mut Config, mut ctl: Option<&mut Ctl>) {
        self.mouse = Some(q);
        if let Some(c) = ctl.as_deref_mut() {
            c.hover(self.index);
        }
        let ip = self.to_img(q);
        if let Some(key) = self.pick.take() {
            if button == PointerButton::Primary {
                let c = self.color_at(ip);
                if let Some(pp) = &mut self.popup
                    && pp.key == key
                {
                    pp.picked = Some(c);
                }
            }
            return;
        }
        if self.popup.is_some() {
            super::panels::close_popup(self, cfg);
        }
        if button == PointerButton::Middle && self.editor {
            self.drag = Some(Drag {
                kind: DragKind::Pan { last: q },
                snap: None,
                pushed: true,
            });
            return;
        }
        if button == PointerButton::Secondary {
            if self.text_edit.is_some() {
                self.finish_text();
            } else if self.mode == Mode::Edit && !self.editor && self.doc.layers.is_empty() {
                self.reset_selection(ctl);
            } else if self.tool != "pointer" {
                self.set_tool("pointer");
            } else {
                self.select(None, false);
            }
            return;
        }
        if button != PointerButton::Primary {
            return;
        }
        if self.mode == Mode::Idle {
            if let Some(c) = ctl {
                if !c.can_start(self.index) {
                    self.toast(tr("Another screen already has drawings — finish or cancel them (Esc)"));
                    return;
                }
                c.lock(self.index);
            }
            let s = self.clamp_img(ip);
            self.drag = Some(Drag {
                kind: DragKind::Select {
                    start: s,
                    end: s,
                    moved: false,
                    had_crop: false,
                },
                snap: None,
                pushed: true,
            });
            self.mode = Mode::Selecting;
            return;
        }
        if self.mode != Mode::Edit {
            return;
        }
        if let Some(te) = &self.text_edit {
            let tol = self.tol();
            if self
                .doc
                .layer(&te.id)
                .is_some_and(|l| shapes::hit(l, ip, tol).is_some())
            {
                return;
            }
            self.finish_text();
            if self.tool == "text" {
                return;
            }
        }
        if let Some(key) = self.handle_at(q) {
            let id = self.selected.clone().unwrap();
            let orig = self.doc.layer(&id).map(|l| l.g.clone()).unwrap_or_default();
            self.begin_drag(
                DragKind::Handle {
                    id: id.clone(),
                    key,
                    orig,
                },
                Some(&id),
                false,
            );
            return;
        }
        if let Some(key) = self.crop_handle_at(q) {
            let orig = self.doc.crop.unwrap().f();
            self.begin_drag(DragKind::Crop { key, orig }, None, false);
            return;
        }
        let crop = self.doc.crop.map(|c| c.f());
        if !self.editor && !crop.is_some_and(|c| c.contains(ip)) {
            self.select(None, false);
            let s = self.clamp_img(ip);
            self.drag = Some(Drag {
                kind: DragKind::Select {
                    start: s,
                    end: s,
                    moved: false,
                    had_crop: true,
                },
                snap: Some(self.doc.snapshot()),
                pushed: false,
            });
            self.mode = Mode::Selecting;
            return;
        }
        let t = self.tool.clone();
        let (x, y) = (ip.x, ip.y);
        if t == "ocr" {
            if self.ocr_press(q) {
                self.drag = Some(Drag {
                    kind: DragKind::OcrSelect,
                    snap: None,
                    pushed: true,
                });
            }
            return;
        }
        match t.as_str() {
            "pointer" => {
                if let Some(id) = self.layer_at(ip, None) {
                    if mods.shift {
                        self.select(Some(id), true);
                        return;
                    }
                    let mut group: Vec<String> = self.sel_layers().iter().map(|l| l.id.clone()).collect();
                    if !group.contains(&id) || group.len() < 2 {
                        self.select(Some(id.clone()), false);
                        group = vec![id.clone()];
                    }
                    let kind = self.doc.layer(&id).map(|l| l.kind.clone()).unwrap_or_default();
                    let partial = kind == "mark" || kind == "zoom";
                    let whole = group.len() > 1 || !partial || mods.ctrl;
                    let tol = self.tol();
                    let part = if whole {
                        None
                    } else {
                        self.doc.layer(&id).and_then(|l| shapes::hit(l, ip, tol))
                    };
                    let first = group[0].clone();
                    self.begin_drag(
                        DragKind::Move {
                            ids: group,
                            last: ip,
                            whole,
                            part,
                        },
                        Some(&first),
                        false,
                    );
                    return;
                }
                self.select(None, false);
                if let Some(c) = crop
                    && c.contains(ip)
                {
                    self.begin_drag(DragKind::CropMove { start: ip, orig: c }, None, false);
                }
            }
            "mark" => {
                if let Some(id) = self.layer_at(ip, Some(&["mark"])) {
                    self.select(Some(id.clone()), false);
                    self.begin_drag(
                        DragKind::Move {
                            ids: vec![id.clone()],
                            last: ip,
                            whole: mods.ctrl,
                            part: None,
                        },
                        Some(&id),
                        false,
                    );
                    return;
                }
                let g = Geom {
                    anchor: Some([x, y]),
                    pos: Some([x, y]),
                    ..Default::default()
                };
                let (id, _) = self.new_layer(cfg, "mark", g, true);
                self.begin_drag(DragKind::MarkNew { id: id.clone() }, Some(&id), true);
                self.props_key = None;
            }
            "zoom" => {
                if let Some(id) = self.layer_at(ip, Some(&["zoom"])) {
                    self.select(Some(id.clone()), false);
                    let tol = self.tol();
                    let part = if mods.ctrl {
                        None
                    } else {
                        self.doc.layer(&id).and_then(|l| shapes::hit(l, ip, tol))
                    };
                    self.begin_drag(
                        DragKind::Move {
                            ids: vec![id.clone()],
                            last: ip,
                            whole: mods.ctrl,
                            part,
                        },
                        Some(&id),
                        false,
                    );
                    return;
                }
                let g = Geom {
                    src: Some([x, y, 0.0, 0.0]),
                    dst: Some([x, y, 0.0, 0.0]),
                    ..Default::default()
                };
                let (id, _) = self.new_layer(cfg, "zoom", g, true);
                self.begin_drag(
                    DragKind::Draw {
                        id: id.clone(),
                        start: ip,
                    },
                    Some(&id),
                    true,
                );
                self.props_key = None;
            }
            "text" => {
                if let Some(id) = self.layer_at(ip, Some(&["text"])) {
                    self.select(Some(id.clone()), false);
                    let snap = self.doc.snapshot();
                    self.start_text(id, snap, false);
                    return;
                }
                let size = cfg.style("text").f("size", 26.0);
                let g = Geom {
                    pos: Some([x, y - size * 0.65]),
                    text: Some(String::new()),
                    ..Default::default()
                };
                let (id, snap) = self.new_layer(cfg, "text", g, false);
                self.start_text(id, snap, true);
            }
            _ => {
                let g = match t.as_str() {
                    "pen" | "highlighter" => Geom {
                        pts: Some(vec![[x, y]]),
                        ..Default::default()
                    },
                    "line" | "arrow" => Geom {
                        p1: Some([x, y]),
                        p2: Some([x, y]),
                        ..Default::default()
                    },
                    _ => Geom {
                        r: Some([x, y, 0.0, 0.0]),
                        ..Default::default()
                    },
                };
                let (id, _) = self.new_layer(cfg, &t, g, true);
                self.begin_drag(
                    DragKind::Draw {
                        id: id.clone(),
                        start: ip,
                    },
                    Some(&id),
                    true,
                );
                self.props_key = None;
            }
        }
    }

    fn moved(&mut self, q: Pos2, mods: Modifiers) {
        self.mouse = Some(q);
        let ip = self.to_img(q);
        let shift = mods.shift;
        let vs = self.view_scale;
        let b = self.img_rect();
        let Some(d) = &mut self.drag else {
            if self.mode == Mode::Idle {
                self.update_hover(ip);
            }
            return;
        };
        if matches!(d.kind, DragKind::OcrSelect) {
            self.ocr_drag(q);
            return;
        }
        match &mut d.kind {
            DragKind::Pan { last } => {
                self.offset += q - *last;
                *last = q;
                self.user_view = true;
            }
            DragKind::Select { start, end, moved, .. } => {
                let r = self.doc.base_rect().f();
                *end = p(ip.x.clamp(0.0, r.w), ip.y.clamp(0.0, r.h));
                if !*moved && start.dist(*end) * vs > 4.0 {
                    *moved = true;
                    self.popup = None;
                }
            }
            DragKind::Draw { id, start } => {
                let (id, start) = (id.clone(), *start);
                if let Some(l) = self.doc.layer_mut(&id) {
                    match l.kind.as_str() {
                        "pen" | "highlighter" => {
                            let pts = l.g.pts.get_or_insert_with(Vec::new);
                            let last = pts.last().copied().unwrap_or([ip.x, ip.y]);
                            if (ip.x - last[0]).hypot(ip.y - last[1]) * vs >= 1.5 {
                                pts.push([ip.x, ip.y]);
                            }
                        }
                        "line" | "arrow" => {
                            let p2 = if shift { shapes::snap_angle(start, ip) } else { ip };
                            l.g.p2 = Some(p2.arr());
                        }
                        kind => {
                            let mut r = shapes::resize_rect(R::pts(start, start), "br", ip, shift);
                            if ip.x < start.x || ip.y < start.y {
                                r = R::pts(start, ip);
                                if shift {
                                    let s = r.w.max(r.h);
                                    r = R::pts(
                                        start,
                                        start + p(s.copysign(ip.x - start.x), s.copysign(ip.y - start.y)),
                                    );
                                }
                            }
                            if kind == "zoom" {
                                l.g.src = Some(r.arr());
                                l.g.dst = Some(r.arr());
                            } else {
                                l.g.r = Some(r.arr());
                            }
                        }
                    }
                }
                self.changed();
            }
            DragKind::MarkNew { id } => {
                let id = id.clone();
                if let Some(l) = self.doc.layer_mut(&id) {
                    l.g.pos = Some(ip.arr());
                }
                self.changed();
            }
            DragKind::Move { ids, last, whole, part } => {
                let (dx, dy) = (ip.x - last.x, ip.y - last.y);
                if dx != 0.0 || dy != 0.0 {
                    *last = ip;
                    let (ids, whole, part) = (ids.clone(), *whole, *part);
                    for id in &ids {
                        if let Some(l) = self.doc.layer_mut(id) {
                            shapes::translate(l, dx, dy, whole, part);
                        }
                    }
                    self.changed();
                }
            }
            DragKind::Handle { id, key, orig } => {
                let (id, key, orig) = (id.clone(), key.clone(), orig.clone());
                if let Some(l) = self.doc.layer_mut(&id) {
                    shapes::drag_handle(l, &key, ip, &orig, shift);
                }
                self.changed();
            }
            DragKind::OcrSelect => {}
            DragKind::Crop { .. } | DragKind::CropMove { .. } => {
                let r = match &d.kind {
                    DragKind::Crop { key, orig } => {
                        let c = p(ip.x.clamp(0.0, b.w), ip.y.clamp(0.0, b.h));
                        shapes::resize_rect(*orig, key, c, false)
                    }
                    DragKind::CropMove { start, orig } => {
                        let mut r = orig.translate(ip - *start);
                        r.x = r.x.min(b.w - r.w).max(0.0);
                        r.y = r.y.min(b.h - r.h).max(0.0);
                        r
                    }
                    _ => unreachable!(),
                };
                let rr = IR::new(
                    r.x.round() as i32,
                    r.y.round() as i32,
                    (r.w.round() as i32).max(1),
                    (r.h.round() as i32).max(1),
                );
                if Some(rr) != self.doc.crop {
                    self.doc.crop = Some(rr);
                    self.changed();
                }
            }
        }
    }

    fn release(&mut self, ctl: Option<&mut Ctl>) {
        let Some(d) = self.drag.take() else { return };
        self.below = None;
        match d.kind {
            DragKind::Pan { .. } => {}
            DragKind::OcrSelect => self.ocr_release(),
            DragKind::Select {
                start,
                end,
                moved,
                had_crop,
            } => {
                if !moved {
                    if had_crop {
                        // plain click outside an existing selection: keep it
                        self.mode = Mode::Edit;
                        return;
                    }
                    // click with no selection yet: wait to see whether it is a double click
                    self.mode = Mode::Idle;
                    if let Some(c) = ctl
                        && c.locked == Some(self.index)
                    {
                        c.locked = None;
                    }
                    self.pending_click = Some((Instant::now() + DOUBLE_CLICK, self.hover_win));
                    return;
                }
                let mut r = R::pts(start, end).aligned().intersect(self.doc.base_rect());
                if r.w < 2 || r.h < 2 {
                    r = self.doc.base_rect();
                }
                if had_crop && self.doc.crop.is_some() {
                    self.doc.push_undo(d.snap, None);
                }
                self.doc.crop = Some(r);
                self.hover_win = None;
                self.enter_edit();
            }
            DragKind::Draw { id, .. } => {
                let mut degenerate = false;
                let mut zoom = false;
                if let Some(l) = self.doc.layer_mut(&id) {
                    match l.kind.as_str() {
                        "line" | "arrow" => degenerate = l.g.p1().dist(l.g.p2()) < 3.0,
                        "rect" | "ellipse" | "blur" => {
                            let r = l.g.r();
                            degenerate = r.w < 3.0 || r.h < 3.0;
                        }
                        "zoom" => {
                            let s = l.g.src();
                            degenerate = s.w < 3.0 || s.h < 3.0;
                            zoom = !degenerate;
                        }
                        _ => {}
                    }
                }
                if degenerate {
                    self.discard_last();
                } else if zoom {
                    self.toast(tr(
                        "Drag the copy aside · copy corners — magnification · Ctrl — move as a whole",
                    ));
                }
                self.props_key = None;
                self.invalidate();
            }
            _ => {
                self.props_key = None;
                self.invalidate();
            }
        }
    }

    fn single_click_tick(&mut self, ctl: Option<&mut Ctl>) {
        let Some((due, win)) = self.pending_click else { return };
        if Instant::now() < due {
            return;
        }
        self.pending_click = None;
        let Some(wi) = win else { return };
        if self.mode != Mode::Idle {
            return;
        }
        if let Some(c) = ctl {
            if !c.can_start(self.index) {
                return;
            }
            c.lock(self.index);
        }
        let Some((r, _)) = self.windows.get(wi) else { return };
        let crop = r.aligned().intersect(self.doc.base_rect());
        if crop.is_empty() {
            return;
        }
        self.doc.crop = Some(crop);
        self.hover_win = None;
        self.enter_edit();
    }

    fn double_click(&mut self, q: Pos2, mods: Modifiers, cfg: &mut Config, mut ctl: Option<&mut Ctl>) {
        let ip = self.to_img(q);
        if !self.editor {
            if self.mode == Mode::Idle {
                self.pending_click = None;
                if let Some(c) = ctl.as_deref_mut() {
                    if !c.can_start(self.index) {
                        return;
                    }
                    c.lock(self.index);
                }
                self.select_full_screen();
                return;
            }
            if self.mode == Mode::Edit
                && !self.doc.crop.is_some_and(|c| c.f().contains(ip))
                && self.crop_handle_at(q).is_none()
            {
                self.select_full_screen();
                return;
            }
        }
        if self.mode == Mode::Edit && self.tool == "ocr" {
            self.ocr_select_word(q);
            return;
        }
        if self.mode == Mode::Edit
            && (self.tool == "pointer" || self.tool == "text")
            && let Some(id) = self.layer_at(ip, Some(&["text"]))
        {
            self.select(Some(id.clone()), false);
            let snap = self.doc.snapshot();
            self.start_text(id, snap, false);
            return;
        }
        if self.mode == Mode::Edit
            && !self.editor
            && self.tool == "pointer"
            && self.layer_at(ip, None).is_none()
            && self.doc.crop.is_some_and(|c| c.f().contains(ip))
        {
            self.request(Action::Copy);
            return;
        }
        // a quick second press with no special meaning is a press: twitchy clicks must not be lost
        self.press(q, PointerButton::Primary, mods, cfg, ctl);
    }

    fn wheel(&mut self, delta: Vec2, q: Pos2, mods: Modifiers, cfg: &mut Config) {
        if delta.y == 0.0 && delta.x == 0.0 {
            return;
        }
        if self.editor && mods.ctrl {
            self.zoom_at(if delta.y > 0.0 { 1.15 } else { 1.0 / 1.15 }, q);
            return;
        }
        if self.mode != Mode::Edit {
            return;
        }
        let target = self.props_target(cfg);
        let key = target
            .as_ref()
            .and_then(|(_, st, _)| ["width", "size", "strength"].into_iter().find(|k| st.has(k)));
        if let (Some(key), Some((_, st, _))) = (key, target) {
            let step = if delta.y > 0.0 { 1.0 } else { -1.0 } * if key == "size" { 2.0 } else { 1.0 };
            let v = (st.f(key, 1.0) + step).max(1.0);
            self.apply_style(cfg, key, serde_json::json!(v as i64));
            self.props_key = None;
            let label = match key {
                "width" => "Width",
                "size" => "Size",
                _ => "Strength",
            };
            self.toast(format!("{}: {}", tr(label), v as i64));
        } else if self.editor {
            self.offset += delta * 25.0;
            self.user_view = true;
        }
    }

    fn update_hover(&mut self, ip: P) {
        self.hover_win = self.windows.iter().position(|(r, _)| r.contains(ip));
    }

    fn cursor(&self) -> CursorIcon {
        if self.pick.is_some() {
            return CursorIcon::Crosshair;
        }
        if !self.editor && !self.active {
            return CursorIcon::Default;
        }
        if matches!(self.mode, Mode::Idle | Mode::Selecting) {
            return CursorIcon::Crosshair;
        }
        if let Some(d) = &self.drag {
            return match &d.kind {
                DragKind::Pan { .. } => CursorIcon::Grabbing,
                DragKind::OcrSelect => CursorIcon::Text,
                DragKind::CropMove { .. } => CursorIcon::Grabbing,
                DragKind::Handle { key, .. } | DragKind::Crop { key, .. } => resize_cursor(key),
                DragKind::Move { .. } => CursorIcon::Move,
                _ => CursorIcon::Crosshair,
            };
        }
        let Some(q) = self.mouse else {
            return if self.tool != "pointer" {
                CursorIcon::Crosshair
            } else {
                CursorIcon::Default
            };
        };
        let ip = self.to_img(q);
        if let Some(k) = self.handle_at(q) {
            return resize_cursor(&k);
        }
        if let Some(k) = self.crop_handle_at(q) {
            return resize_cursor(&k);
        }
        match self.tool.as_str() {
            "pointer" => {
                if self.layer_at(ip, None).is_some() {
                    CursorIcon::Move
                } else if self.doc.crop.is_some_and(|c| c.f().contains(ip)) {
                    CursorIcon::Grab
                } else if self.editor {
                    CursorIcon::Default
                } else {
                    CursorIcon::Crosshair
                }
            }
            "text" => CursorIcon::Text,
            "ocr" if self.ocr_over_text(q) => CursorIcon::Text,
            "mark" if self.layer_at(ip, Some(&["mark"])).is_some() => CursorIcon::Move,
            _ => CursorIcon::Crosshair,
        }
    }

    // ------------------------------------------------------------- keyboard
    fn key(&mut self, key: Key, mods: Modifiers, cfg: &mut Config, ctl: Option<&mut Ctl>) {
        let (ctrl, shift) = (mods.ctrl || mods.command, mods.shift);
        let ocr_sel = self.ocr.as_ref().is_some_and(|o| o.sel.is_some());
        if key == Key::Escape {
            if self.tool == "ocr" && ocr_sel {
                if let Some(o) = &mut self.ocr {
                    o.sel = None;
                }
            } else if self.pick.is_some() {
                self.pick = None;
            } else if self.popup.is_some() {
                super::panels::close_popup(self, cfg);
            } else if self.editor {
                self.select(None, false);
            } else {
                self.request(Action::Cancel);
            }
            return;
        }
        if self.mode != Mode::Edit {
            if key == Key::C
                && !ctrl
                && let Some(m) = self.mouse
            {
                let c = self.color_at(self.to_img(m)).rgb_name();
                crate::sys::copy_text(&c);
                self.toast(trf("Color {c} copied", &[("c", &c)]));
            } else if matches!(key, Key::Enter | Key::Space) || (ctrl && key == Key::A) {
                let ok = match ctl {
                    Some(c) => {
                        let ok = c.can_start(self.index);
                        if ok {
                            c.lock(self.index);
                        }
                        ok
                    }
                    None => true,
                };
                if ok {
                    self.doc.crop = Some(self.doc.base_rect());
                    self.enter_edit();
                }
            }
            return;
        }
        if self.tool == "ocr" {
            match key {
                Key::C if ctrl => return self.ocr_copy(),
                Key::Enter if !ctrl => return self.ocr_copy(),
                Key::A if ctrl => return self.ocr_select_all(),
                _ => {}
            }
        }
        match key {
            Key::Z if ctrl => {
                if shift {
                    self.redo()
                } else {
                    self.undo()
                }
            }
            Key::Y if ctrl => self.redo(),
            Key::C if ctrl => self.request(Action::Copy),
            Key::Enter if !ctrl => self.request(Action::Copy),
            Key::S if ctrl => self.request(if shift && self.editor {
                Action::Export
            } else {
                Action::Save
            }),
            Key::E if ctrl && !self.editor => self.request(Action::Edit),
            Key::D if ctrl => self.duplicate_selected(),
            Key::A if ctrl => {
                let all = self.doc.layers.iter().map(|l| l.id.clone()).collect();
                self.select_many(all, None);
            }
            Key::CloseBracket if ctrl => self.move_selected(1),
            Key::OpenBracket if ctrl => self.move_selected(-1),
            Key::Num0 if ctrl && self.editor => self.fitted_for = None,
            Key::Delete | Key::Backspace => self.delete_selected(),
            Key::Tab => self.toggle_layers(cfg),
            Key::ArrowLeft | Key::ArrowRight | Key::ArrowUp | Key::ArrowDown => {
                let step = if shift { 10.0 } else { 1.0 };
                let (dx, dy) = match key {
                    Key::ArrowLeft => (-step, 0.0),
                    Key::ArrowRight => (step, 0.0),
                    Key::ArrowUp => (0.0, -step),
                    _ => (0.0, step),
                };
                self.nudge(dx, dy);
            }
            _ if !ctrl && !mods.alt => {
                if let Some((_, t)) = TOOL_KEYS.iter().find(|(k, _)| *k == key) {
                    self.set_tool(t);
                }
            }
            _ => {}
        }
    }

    pub fn toggle_layers(&mut self, cfg: &mut Config) {
        self.layers_visible = !self.layers_visible;
        cfg.set("layers_visible", self.layers_visible);
        if self.editor && !self.user_view {
            self.fitted_for = None;
        }
    }

    // ------------------------------------------------------------------ frame
    /// Handle this frame's input. `ctl` is the overlay coordinator (None in the editor).
    pub fn input(&mut self, ctx: &egui::Context, cfg: &mut Config, mut ctl: Option<&mut Ctl>, keyboard: bool) {
        let ppp = ctx.pixels_per_point();
        // refit on resize unless the user zoomed/panned; Ctrl+0 and panel toggles reset fitted_for
        let resized = self.fitted_for != Some(self.rect.size()) && !self.user_view;
        if self.editor && (resized || self.fitted_for.is_none()) {
            self.fit_view(ppp);
            self.fitted_for = Some(self.rect.size());
        }
        let over_ui = |q: Pos2, me: &Canvas| me.panel_rects.iter().any(|r| r.contains(q)) || ctx.is_pointer_over_egui();
        let events = ctx.input(|i| i.events.clone());
        let wants_kb = ctx.text_edit_focused();
        let origin = self.rect.min.to_vec2();
        for ev in &events {
            match ev {
                egui::Event::PointerMoved(q) => {
                    if self.drag.is_none() && over_ui(*q, self) {
                        self.mouse = None;
                        continue;
                    }
                    if let Some(c) = ctl.as_deref_mut()
                        && self.drag.is_none()
                    {
                        c.hover(self.index);
                    }
                    let m = ctx.input(|i| i.modifiers);
                    self.moved(*q - origin, m);
                }
                egui::Event::PointerButton {
                    pos,
                    button,
                    pressed,
                    modifiers,
                } => {
                    let q = *pos - origin;
                    if *pressed {
                        if over_ui(*pos, self) || !self.rect.contains(*pos) {
                            continue;
                        }
                        let now = Instant::now();
                        let dbl = *button == PointerButton::Primary
                            && self
                                .last_press
                                .is_some_and(|(t, lp)| now.duration_since(t) < DOUBLE_CLICK && lp.distance(q) < 6.0);
                        if *button == PointerButton::Primary {
                            self.last_press = if dbl { None } else { Some((now, q)) };
                        }
                        if dbl {
                            self.double_click(q, *modifiers, cfg, ctl.as_deref_mut());
                        } else {
                            self.press(q, *button, *modifiers, cfg, ctl.as_deref_mut());
                        }
                    } else if self.drag.is_some() {
                        self.moved(q, *modifiers);
                        self.release(ctl.as_deref_mut());
                    }
                }
                egui::Event::MouseWheel {
                    unit, delta, modifiers, ..
                } => {
                    let Some(q) = ctx.input(|i| i.pointer.hover_pos()) else {
                        continue;
                    };
                    if over_ui(q, self) {
                        continue;
                    }
                    let d = match unit {
                        egui::MouseWheelUnit::Point => {
                            self.wheel_acc += delta.y;
                            if self.wheel_acc.abs() < 40.0 {
                                if self.editor && !modifiers.ctrl {
                                    self.offset += *delta;
                                    self.user_view = true;
                                }
                                continue;
                            }
                            let d = vec2(0.0, self.wheel_acc.signum());
                            self.wheel_acc = 0.0;
                            d
                        }
                        _ => *delta,
                    };
                    self.wheel(d, q - origin, *modifiers, cfg);
                }
                egui::Event::PointerGone => {
                    self.mouse = None;
                }
                _ => {}
            }
            if !keyboard || wants_kb {
                continue;
            }
            if self.text_edit.is_some() {
                self.text_key(ev, ctx);
                continue;
            }
            if let egui::Event::Key {
                key,
                pressed: true,
                modifiers,
                ..
            } = ev
            {
                self.key(*key, *modifiers, cfg, ctl.as_deref_mut());
            }
        }
        if keyboard && !wants_kb {
            // keep Tab for the layers panel instead of egui's focus cycling
            ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Tab));
        }
        self.single_click_tick(ctl.as_deref_mut());
        self.live_copy_tick();
        self.ocr_tick(ctx, cfg);
        if let Some(c) = ctl
            && c.locked == Some(self.index)
        {
            c.locked_movable = self.movable();
        }
        if self.mouse.is_some() || self.drag.is_some() {
            ctx.set_cursor_icon(self.cursor());
        }
        if self.pending_click.is_some() || self.live_due.is_some() {
            ctx.request_repaint_after(Duration::from_millis(30));
        }
        if self.text_edit.is_some() {
            ctx.request_repaint_after(Duration::from_millis(530));
        }
        if let Some((_, t)) = &self.toast
            && t.elapsed() < Duration::from_millis(1600)
        {
            ctx.request_repaint_after(Duration::from_millis(100));
        }
    }

    pub fn current_crop(&self) -> Option<R> {
        if let Some(Drag {
            kind: DragKind::Select { start, end, moved, .. },
            ..
        }) = &self.drag
        {
            return moved.then(|| R::pts(*start, *end));
        }
        if self.mode == Mode::Edit {
            return self.doc.crop.map(|c| c.f());
        }
        None
    }

    pub fn ensure_cache(&mut self) {
        let Some(region) = self.doc.crop else { return };
        if let Some((rev, r, _)) = &self.cache
            && *rev == self.doc.rev
            && *r == region
        {
            return;
        }
        let img = match &self.below {
            Some((r, idx, start)) if *r == region && *idx <= self.doc.layers.len() => {
                shapes::composite(&self.doc, region, &self.doc.layers[*idx..], Some(start))
            }
            _ => shapes::composite(&self.doc, region, &self.doc.layers, None),
        };
        self.cache = Some((self.doc.rev, region, img));
    }

    pub fn result(&mut self) -> Pixmap {
        self.finish_text();
        self.ensure_cache();
        match &self.cache {
            Some((_, _, img)) => img.clone(),
            None => shapes::render_result(&self.doc),
        }
    }

    fn drag_kind_is(&self, f: impl Fn(&DragKind) -> bool) -> bool {
        self.drag.as_ref().is_some_and(|d| f(&d.kind))
    }

    pub fn paint(&mut self, ctx: &egui::Context) {
        let painter = ctx.layer_painter(egui::LayerId::background());
        let s = self.view_scale;
        let accent = self.accent32();
        if self.editor {
            painter.rect_filled(self.rect, 0.0, super::style::WINDOW_BG);
        }
        let ppp = ctx.pixels_per_point();
        let smooth = self.editor && s * ppp < 1.0;
        let opts = if smooth {
            egui::TextureOptions::LINEAR
        } else {
            egui::TextureOptions::NEAREST
        };
        let uv = Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0));
        let img_r = self.rect_s(self.img_rect());
        if self.base_tex.as_ref().is_none_or(|(_, o)| *o != opts) {
            let b = &self.doc.base;
            let img = egui::ColorImage::from_rgba_premultiplied([b.width() as usize, b.height() as usize], b.data());
            match &mut self.base_tex {
                Some((t, o)) => {
                    t.set(img, opts);
                    *o = opts;
                }
                None => self.base_tex = Some((ctx.load_texture(format!("base-{}", self.index), img, opts), opts)),
            }
        }
        if let Some((t, _)) = &self.base_tex {
            painter.image(t.id(), img_r, uv, Color32::WHITE);
        }

        let crop = self.current_crop();
        let dim = Color32::from_black_alpha(if self.editor {
            150
        } else if self.active {
            115
        } else {
            170
        });
        let cw = crop.map(|c| self.rect_s(c));
        match cw {
            Some(c) => {
                let c = c.intersect(img_r);
                for r in [
                    Rect::from_min_max(img_r.min, pos2(img_r.max.x, c.min.y)),
                    Rect::from_min_max(pos2(img_r.min.x, c.max.y), img_r.max),
                    Rect::from_min_max(pos2(img_r.min.x, c.min.y), pos2(c.min.x, c.max.y)),
                    Rect::from_min_max(pos2(c.max.x, c.min.y), pos2(img_r.max.x, c.max.y)),
                ] {
                    if r.is_positive() {
                        painter.rect_filled(r, 0.0, dim);
                    }
                }
            }

            None => {
                painter.rect_filled(img_r, 0.0, dim);
            }
        }

        if self.mode == Mode::Edit && self.doc.crop.is_some() {
            self.ensure_cache();
            let (rev, region) = (self.doc.rev, self.doc.crop.unwrap());
            let stale = !matches!(&self.tex, Some((r, rg, _)) if *r == rev && *rg == region);
            if stale && let Some((_, _, img)) = &self.cache {
                let ci = egui::ColorImage::from_rgba_premultiplied(
                    [img.width() as usize, img.height() as usize],
                    img.data(),
                );
                match &mut self.tex {
                    Some((r, rg, t)) if t.size() == [img.width() as usize, img.height() as usize] => {
                        t.set(ci, opts);
                        *r = rev;
                        *rg = region;
                    }
                    _ => self.tex = Some((rev, region, ctx.load_texture(format!("doc-{}", self.index), ci, opts))),
                }
            }
            if let Some((_, _, t)) = &self.tex {
                painter.image(t.id(), self.rect_s(region.f()), uv, Color32::WHITE);
            }
        }

        if self.mode == Mode::Idle
            && self.active
            && !self.editor
            && let Some(wi) = self.hover_win
            && let Some((wr, title)) = self.windows.get(wi)
        {
            let wr = self.rect_s(*wr);
            painter.rect_filled(
                wr,
                0.0,
                Color32::from_rgba_unmultiplied(accent.r(), accent.g(), accent.b(), 28),
            );
            painter.rect_stroke(wr.shrink(1.0), 0.0, Stroke::new(2.0, accent), egui::StrokeKind::Middle);
            let t = if title.is_empty() { tr("Window") } else { title.clone() };
            pill(
                &painter,
                &t,
                wr.min + vec2(8.0, 8.0),
                egui::Align2::LEFT_TOP,
                Some(self.rect),
                None,
            );
        }

        if let Some(c) = cw {
            painter.rect_stroke(c, 0.0, Stroke::new(1.5, accent), egui::StrokeKind::Middle);
            if self.mode == Mode::Edit
                && !self.drag_kind_is(|k| {
                    matches!(
                        k,
                        DragKind::Draw { .. } | DragKind::Move { .. } | DragKind::MarkNew { .. }
                    )
                })
            {
                for (_, pt) in shapes::rect_handle_points(R::of(c)) {
                    let r = Rect::from_center_size(pt.egui(), vec2(8.0, 8.0));
                    painter.rect(
                        r,
                        2.0,
                        Color32::from_rgb(0xf5, 0xf5, 0xf7),
                        Stroke::new(1.5, accent),
                        egui::StrokeKind::Middle,
                    );
                }
            }
            if self.mode == Mode::Selecting
                || self.drag_kind_is(|k| matches!(k, DragKind::Crop { .. } | DragKind::CropMove { .. }))
            {
                let r = crop.unwrap().aligned();
                pill(
                    &painter,
                    &format!("{} × {}", r.w, r.h),
                    c.min - vec2(0.0, 6.0),
                    egui::Align2::LEFT_BOTTOM,
                    Some(self.rect),
                    None,
                );
            }
        }

        if self.mode == Mode::Edit {
            self.paint_selection(&painter);
            self.paint_ocr(&painter);
        }

        if !self.editor && self.active {
            self.paint_glow(&painter, ctx);
            if self.mode == Mode::Idle {
                pill(
                    &painter,
                    &tr(
                        "Select an area  ·  click — window  ·  double click / Enter — full screen  ·  C — copy color  ·  Esc — cancel",
                    ),
                    pos2(self.rect.center().x, self.rect.min.y + 22.0),
                    egui::Align2::CENTER_TOP,
                    None,
                    None,
                );
            }
        }
        if self.mouse.is_some()
            && ((!self.editor
                && self.active
                && (matches!(self.mode, Mode::Idle | Mode::Selecting) || self.pick.is_some()))
                || (self.editor && self.pick.is_some()))
        {
            self.paint_loupe(ctx, &painter);
        }
        if let Some((t, at)) = &self.toast
            && at.elapsed() < Duration::from_millis(1500)
        {
            pill(
                &painter,
                t,
                pos2(self.rect.center().x, self.rect.max.y - 40.0),
                egui::Align2::CENTER_BOTTOM,
                None,
                Some(accent),
            );
        }
    }

    fn paint_selection(&self, painter: &egui::Painter) {
        let accent = self.accent32();
        let drawing = self.drag_kind_is(|k| matches!(k, DragKind::Draw { .. }));
        if let Some(sel) = self.selected_layer()
            && !drawing
        {
            for l in self.sel_layers() {
                let br = self.rect_s(shapes::bounds(l)).expand(3.0);
                dashed_rect(painter, br, Color32::from_white_alpha(200), 0.0);
                dashed_rect(painter, br, accent, 4.0);
            }
            if self.text_edit.is_none() && self.multi.is_empty() {
                for (key, pt) in shapes::handles(sel) {
                    let fill = if key == "anchor" { accent } else { Color32::WHITE };
                    painter.circle(self.to_s(pt), HANDLE_R - 1.0, fill, Stroke::new(1.5, accent));
                }
            }
        }
        if let Some(te) = &self.text_edit
            && (self.caret_t0.elapsed().as_millis() / 530).is_multiple_of(2)
            && let Some(l) = self.doc.layer(&te.id)
        {
            let (a, b) = shapes::text_caret(l);
            painter.line_segment([self.to_s(a), self.to_s(b)], Stroke::new(2.0, accent));
        }
    }

    fn paint_glow(&self, painter: &egui::Painter, ctx: &egui::Context) {
        let r = self.rect;
        let t = self.started.elapsed().as_secs_f32();
        let pulse = (t * 0.09 * 25.0) % std::f32::consts::TAU;
        let a = 0.7 + 0.3 * pulse.sin();
        let n = 16;
        for i in 0..n {
            let al = (a * 0.85 * (1.0 - i as f32 / n as f32).powf(2.2)).clamp(0.0, 1.0);
            let c = self.accent.alpha_f(al).egui();
            painter.rect_stroke(
                r.shrink(i as f32 + 0.5),
                0.0,
                Stroke::new(1.0, c),
                egui::StrokeKind::Middle,
            );
        }
        painter.rect_stroke(
            r.shrink(1.0),
            0.0,
            Stroke::new(2.0, self.accent32()),
            egui::StrokeKind::Middle,
        );
        ctx.request_repaint_after(Duration::from_millis(40));
    }

    fn paint_loupe(&mut self, ctx: &egui::Context, painter: &egui::Painter) {
        let Some(m) = self.mouse else { return };
        let ip = self.to_img(m);
        let (ix, iy) = (ip.x.floor() as i32, ip.y.floor() as i32);
        let (n, z) = (15i32, 9.0f32);
        let size = n as f32 * z;
        let sub = shapes::crop_pixmap(&self.doc.base, IR::new(ix - n / 2, iy - n / 2, n, n));
        let ci = egui::ColorImage::from_rgba_premultiplied([n as usize, n as usize], sub.data());
        match &mut self.loupe_tex {
            Some(t) => t.set(ci, egui::TextureOptions::NEAREST),
            None => {
                self.loupe_tex =
                    Some(ctx.load_texture(format!("loupe-{}", self.index), ci, egui::TextureOptions::NEAREST))
            }
        }
        let ms = m + self.rect.min.to_vec2();
        let (mut x, mut y) = (ms.x + 26.0, ms.y + 26.0);
        if x + size > self.rect.max.x - 10.0 {
            x = ms.x - 26.0 - size;
        }
        if y + size + 30.0 > self.rect.max.y - 10.0 {
            y = ms.y - 26.0 - size - 30.0;
        }
        let r = Rect::from_min_size(pos2(x, y), vec2(size, size));
        painter.rect_filled(r, 12.0, Color32::BLACK);
        if let Some(t) = &self.loupe_tex {
            let mut mesh = egui::Mesh::with_texture(t.id());
            mesh.add_rect_with_uv(r, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
            painter.add(mesh);
        }
        let grid = Stroke::new(1.0, Color32::from_white_alpha(22));
        for i in 1..n {
            let o = i as f32 * z;
            painter.line_segment([pos2(r.left() + o, r.top()), pos2(r.left() + o, r.bottom())], grid);
            painter.line_segment([pos2(r.left(), r.top() + o), pos2(r.right(), r.top() + o)], grid);
        }
        let c = Rect::from_min_size(
            pos2(r.left() + (n / 2) as f32 * z, r.top() + (n / 2) as f32 * z),
            vec2(z, z),
        );
        painter.rect_stroke(c, 0.0, Stroke::new(3.0, Color32::BLACK), egui::StrokeKind::Middle);
        painter.rect_stroke(c, 0.0, Stroke::new(1.5, Color32::WHITE), egui::StrokeKind::Middle);
        // round off the corners of the magnified square by painting the outside of the radius
        painter.rect_stroke(
            r.expand(3.0),
            15.0,
            Stroke::new(6.0, Color32::from_black_alpha(0)),
            egui::StrokeKind::Middle,
        );
        painter.rect_stroke(r, 12.0, Stroke::new(2.0, self.accent32()), egui::StrokeKind::Middle);
        let col = if ix >= 0 && iy >= 0 && (ix as u32) < self.doc.base.width() && (iy as u32) < self.doc.base.height() {
            let d = self.doc.base.pixel(ix as u32, iy as u32).unwrap().demultiply();
            Color::rgb(d.red(), d.green(), d.blue())
        } else {
            Color::rgb(0, 0, 0)
        };
        pill(
            painter,
            &format!("{ix}, {iy}   {}", col.rgb_name().to_uppercase()),
            pos2(r.center().x, r.bottom() + 6.0),
            egui::Align2::CENTER_TOP,
            None,
            None,
        );
    }
}

fn resize_cursor(key: &str) -> CursorIcon {
    let k = key.strip_prefix("d_").or_else(|| key.strip_prefix("s_")).unwrap_or(key);
    match k {
        "tl" | "br" => CursorIcon::ResizeNwSe,
        "tr" | "bl" => CursorIcon::ResizeNeSw,
        "t" | "b" => CursorIcon::ResizeVertical,
        "l" | "r" => CursorIcon::ResizeHorizontal,
        _ => CursorIcon::Move,
    }
}

fn dashed_rect(painter: &egui::Painter, r: Rect, color: Color32, offset: f32) {
    let pts = [
        r.left_top(),
        r.right_top(),
        r.right_bottom(),
        r.left_bottom(),
        r.left_top(),
    ];
    painter.extend(egui::Shape::dashed_line_with_offset(
        &pts,
        Stroke::new(1.0, color),
        &[4.0],
        &[4.0],
        offset,
    ));
}

/// Rounded label ("pill") anchored at `at`; kept inside `bounds` when given.
pub fn pill(
    painter: &egui::Painter,
    text: &str,
    at: Pos2,
    align: egui::Align2,
    bounds: Option<Rect>,
    color: Option<Color32>,
) -> Rect {
    let font = egui::FontId::proportional(12.5);
    let galley = painter.layout_no_wrap(
        text.to_string(),
        font,
        color.unwrap_or(Color32::from_rgb(0xec, 0xec, 0xf1)),
    );
    let size = galley.size() + vec2(18.0, 8.0);
    let mut r = align.anchor_size(at, size);
    if let Some(b) = bounds {
        let x = r.min.x.min(b.max.x - size.x - 4.0).max(b.min.x + 4.0);
        let y = r.min.y.min(b.max.y - size.y - 4.0).max(b.min.y + 4.0);
        r = Rect::from_min_size(pos2(x, y), size);
    }
    painter.rect(
        r,
        size.y / 2.0,
        Color32::from_rgba_premultiplied(20, 20, 25, 230),
        Stroke::new(1.0, Color32::from_white_alpha(40)),
        egui::StrokeKind::Inside,
    );
    painter.galley(r.center() - galley.size() / 2.0, galley, Color32::WHITE);
    r
}
