//! The text recognition tool: recognizes the selection in the background, keeps the text
//! invisible over the picture, lets it be selected with the mouse and copied as text.
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use egui::{Color32, Pos2, Stroke, vec2};
use serde_json::json;

use crate::config::Config;
use crate::geom::{IR, P};
use crate::i18n::{tr, trf};
use crate::model::Style;
use crate::ocr::layout::{self, Pos};
use crate::ocr::{self, OcrResult};

use super::canvas::{Action, Canvas};

type Slot = Arc<Mutex<Option<Result<OcrResult, String>>>>;

pub struct Download {
    pub done: Arc<AtomicU64>,
    pub total: u64,
    pub result: Arc<Mutex<Option<Result<(), String>>>>,
}

pub struct OcrState {
    pub region: IR,
    pub lang: String,
    job: Option<Slot>,
    pub result: Option<Arc<OcrResult>>,
    pub error: Option<String>,
    pub sel: Option<(Pos, Pos)>,
    pub finished_at: Option<Instant>,
    pub download: Option<Download>,
}

pub fn lang_of(cfg: &Config) -> String {
    let l = cfg.style("ocr").s("lang", "auto").to_string();
    if l == "auto" || ocr::SCRIPTS.iter().any(|s| s.key == l) {
        l
    } else {
        "auto".into()
    }
}

pub fn lang_label(key: &str) -> String {
    if key == "auto" {
        return tr("Automatic (Russian + English)");
    }
    ocr::SCRIPTS
        .iter()
        .find(|s| s.key == key)
        .map_or(key.to_string(), |s| tr(s.label))
}

fn mb(bytes: u64) -> String {
    format!("{:.0}", (bytes as f64 / 1_048_576.0).max(1.0))
}

impl Canvas {
    pub fn ocr_lines(&self) -> &[ocr::OLine] {
        self.ocr
            .as_ref()
            .and_then(|o| o.result.as_deref())
            .map_or(&[], |r| &r.lines)
    }

    /// Keep the recognition in step with the selection and the chosen language.
    pub fn ocr_tick(&mut self, ctx: &egui::Context, cfg: &Config) {
        if self.tool != "ocr" {
            return;
        }
        let Some(crop) = self.doc.crop else { return };
        let lang = lang_of(cfg);
        let stale = self.ocr.as_ref().is_none_or(|o| o.region != crop || o.lang != lang);
        if stale {
            let download = self.ocr.take().and_then(|o| o.download);
            self.ocr = Some(OcrState {
                region: crop,
                lang: lang.clone(),
                job: None,
                result: None,
                error: None,
                sel: None,
                finished_at: None,
                download,
            });
        }
        let o = self.ocr.as_mut().unwrap();
        // a finished download restarts recognition
        if let Some(d) = &o.download {
            let r = d.result.lock().unwrap().take();
            match r {
                Some(Ok(())) => {
                    o.download = None;
                    o.error = None;
                }
                Some(Err(e)) => {
                    o.download = None;
                    o.error = Some(e);
                }
                None => {
                    ctx.request_repaint_after(Duration::from_millis(100));
                    return;
                }
            }
        }
        if let Some(job) = &o.job {
            let r = job.lock().unwrap().take();
            match r {
                Some(Ok(res)) => {
                    let n = res.lines.len();
                    o.result = Some(Arc::new(res));
                    o.job = None;
                    o.finished_at = Some(Instant::now());
                    self.toast(if n == 0 {
                        tr("No text found")
                    } else {
                        tr("Text recognized: drag to select it, Ctrl+C copies")
                    });
                }
                Some(Err(e)) => {
                    o.error = Some(e);
                    o.job = None;
                }
                None => ctx.request_repaint_after(Duration::from_millis(100)),
            }
            return;
        }
        if o.result.is_some() || o.error.is_some() || !ocr::missing(&lang).is_empty() {
            return;
        }
        // recognize the clean capture, not the annotations drawn over it
        let img = crate::shapes::crop_pixmap(&self.doc.base, crop);
        let scale = self.doc.meta_f("scale", 1.0);
        let slot: Slot = Arc::new(Mutex::new(None));
        let (s2, ctx2) = (slot.clone(), ctx.clone());
        std::thread::spawn(move || {
            let mut r = ocr::recognize(&img, &lang, scale);
            if let Ok(res) = &mut r {
                // crop-local -> image pixels
                for l in &mut res.lines {
                    l.rect = l.rect.translate(P {
                        x: crop.x as f32,
                        y: crop.y as f32,
                    });
                    for c in &mut l.chars {
                        c.x0 += crop.x as f32;
                        c.x1 += crop.x as f32;
                    }
                }
            }
            *s2.lock().unwrap() = Some(r);
            ctx2.request_repaint();
        });
        o.job = Some(slot);
    }

    pub fn ocr_busy(&self) -> bool {
        self.ocr.as_ref().is_some_and(|o| o.job.is_some())
    }

    pub fn start_download(&mut self, ctx: &egui::Context, cfg: &Config) {
        let lang = lang_of(cfg);
        let files = ocr::missing(&lang);
        if files.is_empty() {
            return;
        }
        let total = files.iter().map(|f| f.size).sum();
        let done = Arc::new(AtomicU64::new(0));
        let result = Arc::new(Mutex::new(None));
        let (d2, r2, ctx2) = (done.clone(), result.clone(), ctx.clone());
        std::thread::spawn(move || {
            let r = ocr::download(&files, |n, _| {
                d2.store(n, Ordering::Relaxed);
                ctx2.request_repaint();
            });
            *r2.lock().unwrap() = Some(r);
            ctx2.request_repaint();
        });
        if let Some(o) = &mut self.ocr {
            o.error = None;
            o.download = Some(Download { done, total, result });
        }
    }

    pub fn ocr_hit(&self, q: Pos2) -> Option<Pos> {
        layout::hit(self.ocr_lines(), self.to_img(q))
    }

    /// Start of a mouse selection; false when there is no text to select.
    pub fn ocr_press(&mut self, q: Pos2) -> bool {
        let Some(p) = self.ocr_hit(q) else { return false };
        if let Some(o) = &mut self.ocr {
            o.sel = Some((p, p));
        }
        true
    }

    pub fn ocr_drag(&mut self, q: Pos2) {
        let p = self.ocr_hit(q);
        if let (Some(o), Some(p)) = (&mut self.ocr, p)
            && let Some((a, _)) = o.sel
        {
            o.sel = Some((a, p));
        }
    }

    pub fn ocr_release(&mut self) {
        if let Some(o) = &mut self.ocr
            && o.sel.is_some_and(|(a, b)| a == b)
        {
            o.sel = None;
        }
    }

    pub fn ocr_select_word(&mut self, q: Pos2) {
        let Some(p) = self.ocr_hit(q) else { return };
        let w = layout::word_at(self.ocr_lines(), p);
        if let Some(o) = &mut self.ocr {
            o.sel = Some(w);
        }
    }

    pub fn ocr_select_all(&mut self) {
        let all = layout::all(self.ocr_lines());
        if let Some(o) = &mut self.ocr
            && o.result.is_some()
        {
            o.sel = Some(all);
        }
    }

    /// Copy the selected text (or all of it) as text: Action::CopyText carries it out.
    pub fn ocr_copy(&mut self) {
        if self.ocr_busy() {
            self.toast(tr("Text is still being recognized…"));
            return;
        }
        let lines = self.ocr_lines();
        if lines.is_empty() {
            self.toast(tr("No text found"));
            return;
        }
        let (a, b) = self
            .ocr
            .as_ref()
            .and_then(|o| o.sel)
            .unwrap_or_else(|| layout::all(lines));
        let text = layout::format(lines, a, b);
        if text.is_empty() {
            return;
        }
        self.copied_text = Some(text);
        self.request(Action::CopyText);
    }

    pub fn paint_ocr(&self, painter: &egui::Painter) {
        if self.tool != "ocr" {
            return;
        }
        let Some(o) = &self.ocr else { return };
        let accent = self.accent32();
        let crop = self.rect_s(o.region.f());
        if o.job.is_some() {
            // a scan line sweeping over the selection while the models work
            let t = (self.started_secs() * 0.8).fract();
            let y = crop.top() + crop.height() * t;
            painter.line_segment(
                [egui::pos2(crop.left(), y), egui::pos2(crop.right(), y)],
                Stroke::new(
                    2.0,
                    Color32::from_rgba_unmultiplied(accent.r(), accent.g(), accent.b(), 150),
                ),
            );
            super::canvas::pill(
                painter,
                &tr("Recognizing text…"),
                crop.center_top() + vec2(0.0, 10.0),
                egui::Align2::CENTER_TOP,
                Some(self.rect),
                Some(accent),
            );
        }
        if let Some(d) = &o.download {
            let done = d.done.load(Ordering::Relaxed);
            let pct = (done as f64 / d.total.max(1) as f64 * 100.0).round();
            super::canvas::pill(
                painter,
                &trf("Downloading recognition models… {p}%", &[("p", &pct)]),
                crop.center_top() + vec2(0.0, 10.0),
                egui::Align2::CENTER_TOP,
                Some(self.rect),
                Some(accent),
            );
        }
        let lines = o.result.as_deref().map_or(&[][..], |r| &r.lines[..]);
        // the text itself stays invisible; right after recognition its lines flash once
        if let Some(t) = o.finished_at {
            let k = 1.0 - t.elapsed().as_secs_f32() / 1.2;
            if k > 0.0 {
                let c = Color32::from_rgba_unmultiplied(accent.r(), accent.g(), accent.b(), (k * 110.0) as u8);
                for l in lines {
                    painter.rect_stroke(self.rect_s(l.rect), 3.0, Stroke::new(1.0, c), egui::StrokeKind::Middle);
                }
            }
        }
        if let Some((a, b)) = o.sel {
            let fill = Color32::from_rgba_unmultiplied(accent.r(), accent.g(), accent.b(), 85);
            for r in layout::selection_rects(lines, a, b) {
                painter.rect_filled(self.rect_s(r), 2.0, fill);
            }
        }
    }

    pub fn ocr_over_text(&self, q: Pos2) -> bool {
        let ip = self.to_img(q);
        self.ocr_lines().iter().any(|l| l.rect.grow(2.0).contains(ip))
    }

    /// Properties bar of the tool: language, status, model download.
    pub fn ocr_props(&mut self, ui: &mut egui::Ui, cfg: &mut Config) {
        let lang = lang_of(cfg);
        let missing = ocr::missing(&lang);
        ui.label(
            egui::RichText::new(tr("Language"))
                .size(11.5)
                .color(super::style::TEXT_DIM),
        );
        egui::ComboBox::from_id_salt(("ocr-lang", self.index))
            .width(230.0)
            .selected_text(lang_label(&lang))
            .show_ui(ui, |ui| {
                let keys = std::iter::once("auto").chain(ocr::SCRIPTS.iter().map(|s| s.key));
                for k in keys {
                    let need: u64 = ocr::missing(k).iter().map(|f| f.size).sum();
                    let label = if need > 0 {
                        format!("{}  ·  {}", lang_label(k), trf("download {n} MB", &[("n", &mb(need))]))
                    } else {
                        lang_label(k)
                    };
                    if ui.selectable_label(k == lang, label).clicked() {
                        self.apply_style(cfg, "lang", json!(k));
                    }
                }
            });
        let status = match &self.ocr {
            Some(o) if o.download.is_some() => tr("Downloading…"),
            Some(o) if o.job.is_some() => tr("Recognizing text…"),
            Some(o) if o.error.is_some() => o.error.clone().unwrap(),
            Some(o) if o.result.is_some() => {
                let n = o.result.as_ref().unwrap().lines.len();
                if n == 0 {
                    tr("No text found")
                } else {
                    trf("lines: {n}", &[("n", &n)])
                }
            }
            _ if !missing.is_empty() => tr("Recognition models are not installed"),
            _ => String::new(),
        };
        if !status.is_empty() {
            ui.label(egui::RichText::new(status).size(11.5).color(super::style::TEXT_DIM));
        }
        let downloading = self.ocr.as_ref().is_some_and(|o| o.download.is_some());
        if !missing.is_empty() && !downloading {
            let size: u64 = missing.iter().map(|f| f.size).sum();
            if ui
                .button(trf("Download ({n} MB)", &[("n", &mb(size))]))
                .on_hover_text(tr("Models are fetched from Hugging Face once and checked"))
                .clicked()
            {
                let ctx = ui.ctx().clone();
                self.start_download(&ctx, cfg);
            }
        }
    }
}
