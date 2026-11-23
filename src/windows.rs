//! Regular windows (eframe): the layer editor for a history item, the history gallery and
//! pinned always-on-top shots. Each runs as its own process.
use std::path::PathBuf;
use std::sync::mpsc;
use std::time::{Duration, Instant};

use egui::{Color32, Rect, Sense, Stroke, Vec2, pos2, vec2};

use crate::config::Config;
use crate::history;
use crate::i18n::{tr, trf};
use crate::ui::canvas::{Action, Canvas};
use crate::ui::{panels, style};

pub fn fmt_date(iso: &str) -> String {
    let Ok(dt) = chrono::NaiveDateTime::parse_from_str(iso, "%Y-%m-%dT%H:%M:%S") else {
        return iso.to_string();
    };
    let f = match crate::i18n::current() {
        "en" => "%-m/%-d/%y %-I:%M %p",
        "ru" | "uk" | "de" => "%d.%m.%Y %H:%M",
        "fr" | "es" => "%d/%m/%Y %H:%M",
        "ko" => "%Y. %-m. %-d. %H:%M",
        _ => "%Y/%m/%d %H:%M",
    };
    dt.format(f).to_string()
}

/// Hyprland tiles new toplevels: float + size + center ours once mapped (title identifies it).
fn float_later(title: String, w: i32, h: i32, pos: Option<(i32, i32)>, pin: bool) {
    std::thread::spawn(move || {
        for _ in 0..30 {
            std::thread::sleep(Duration::from_millis(100));
            if crate::hypr::float(&title, w, h, pos, pin) {
                return;
            }
        }
    });
}

fn native(title: &str, size: [f32; 2]) -> eframe::NativeOptions {
    eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title(title)
            .with_app_id("kadr")
            .with_inner_size(size),
        renderer: eframe::Renderer::Glow,
        ..Default::default()
    }
}

fn setup(ctx: &egui::Context, cfg: &Config) {
    style::setup_fonts(ctx);
    style::setup(ctx, cfg.accent());
}

fn save_dialog(cfg: &Config, name: &str, title: &str) -> mpsc::Receiver<Option<PathBuf>> {
    let (tx, rx) = mpsc::channel();
    let dir = crate::sys::save_dir(cfg);
    let (name, title) = (name.to_string(), title.to_string());
    std::thread::spawn(move || {
        let p = rfd::FileDialog::new()
            .set_title(&title)
            .set_directory(dir)
            .set_file_name(&name)
            .add_filter("PNG", &["png"])
            .add_filter("JPEG", &["jpg", "jpeg"])
            .add_filter("WebP", &["webp"])
            .save_file();
        let _ = tx.send(p);
    });
    rx
}

// ------------------------------------------------------------------ editor

struct Editor {
    cv: Canvas,
    cfg: Config,
    id: String,
    confirm_close: bool,
    allow_close: bool,
    export: Option<mpsc::Receiver<Option<PathBuf>>>,
}

pub fn editor(cfg: Config, id: String) -> i32 {
    let doc = match history::load(&id) {
        Ok(d) => d,
        Err(e) => {
            eprintln!("kadr: {e}");
            return 1;
        }
    };
    let title = format!(
        "Kadr — {}",
        fmt_date(doc.meta.get("created").and_then(|v| v.as_str()).unwrap_or(""))
    );
    let cv = Canvas::new(doc, &cfg, true, 1.0, vec![]);
    float_later(title.clone(), 1400, 900, None, false);
    let app = Editor {
        cv,
        cfg,
        id,
        confirm_close: false,
        allow_close: false,
        export: None,
    };
    let r = eframe::run_native(
        "kadr",
        native(&title, [1400.0, 900.0]),
        Box::new(|cc| {
            setup(&cc.egui_ctx, &app.cfg);
            Ok(Box::new(app))
        }),
    );
    if let Err(e) = r {
        eprintln!("kadr: {e}");
        return 1;
    }
    0
}

impl Editor {
    fn action(&mut self, a: Action) {
        let img = self.cv.result();
        match a {
            Action::Copy => {
                crate::sys::copy_image(&img);
                self.cv.toast(tr("Copied to clipboard"));
            }
            Action::Save => {
                let id = self.id.clone();
                match history::save(&mut self.cv.doc, &img, Some(&id), 0) {
                    Ok(_) => self.cv.toast(tr("Saved to history")),
                    Err(e) => self.cv.toast(e.to_string()),
                }
            }
            Action::Export => {
                self.export = Some(save_dialog(&self.cfg, &format!("Kadr_{}.png", self.id), &tr("Export")))
            }
            Action::Pin => crate::sys::spawn_self(&["pin", &self.id]),
            _ => {}
        }
    }
}

impl eframe::App for Editor {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        self.cv.rect = ui.max_rect();
        self.cv.input(&ctx, &mut self.cfg, None, !self.confirm_close);
        self.cv.paint(&ctx);
        panels::show(&mut self.cv, &ctx, &mut self.cfg);
        for a in self.cv.take_actions() {
            self.action(a);
        }
        if let Some(rx) = &self.export {
            match rx.try_recv() {
                Ok(Some(path)) => {
                    let img = self.cv.result();
                    match crate::imgio::save_any(&img, &path) {
                        Ok(()) => self.cv.toast(trf("Exported: {path}", &[("path", &path.display())])),
                        Err(e) => self.cv.toast(e),
                    }
                    self.export = None;
                }
                Ok(None) | Err(mpsc::TryRecvError::Disconnected) => self.export = None,
                Err(mpsc::TryRecvError::Empty) => ctx.request_repaint_after(Duration::from_millis(100)),
            }
        }
        if ctx.input(|i| i.viewport().close_requested()) {
            self.cv.finish_text();
            if self.cv.doc.dirty && !self.allow_close {
                ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
                self.confirm_close = true;
            } else {
                self.cfg.save();
            }
        }
        if self.confirm_close {
            let modal = egui::Modal::new(egui::Id::new("save-changes")).show(&ctx, |ui| {
                ui.set_width(320.0);
                ui.label(egui::RichText::new(tr("Save changes to history?")).size(14.0));
                ui.add_space(10.0);
                ui.horizontal(|ui| {
                    if ui.button(tr("Save")).clicked() {
                        self.action(Action::Save);
                        self.close(&ctx);
                    }
                    if ui.button(tr("Discard")).clicked() {
                        self.close(&ctx);
                    }
                    if ui.button(tr("Cancel")).clicked() {
                        self.confirm_close = false;
                    }
                });
            });
            if modal.should_close() {
                self.confirm_close = false;
            }
        }
    }
}

impl Editor {
    fn close(&mut self, ctx: &egui::Context) {
        self.allow_close = true;
        self.confirm_close = false;
        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
    }
}

// ----------------------------------------------------------------- history

struct Thumb {
    id: String,
    dir: PathBuf,
    caption: String,
    tex: Option<egui::TextureHandle>,
}

struct HistoryApp {
    cfg: Config,
    items: Vec<Thumb>,
    selected: Vec<String>,
    anchor: Option<String>,
    confirm_delete: bool,
    stamp: Option<std::time::SystemTime>,
    checked: Instant,
    export: Option<(Vec<String>, mpsc::Receiver<Option<PathBuf>>)>,
    rx: mpsc::Receiver<String>,
    title: String,
}

fn history_socket() -> PathBuf {
    let rt = std::env::var_os("XDG_RUNTIME_DIR").map_or_else(std::env::temp_dir, PathBuf::from);
    rt.join(format!("kadr-history-{}.sock", unsafe { libc::getuid() }))
}

/// Ask a running history window to come forward (or switch language); true if one answered.
pub fn poke_history(msg: &str) -> bool {
    use std::io::Write;
    match std::os::unix::net::UnixStream::connect(history_socket()) {
        Ok(mut s) => s.write_all(msg.as_bytes()).is_ok(),
        Err(_) => false,
    }
}

pub fn history(cfg: Config) -> i32 {
    if poke_history("raise") {
        return 0;
    }
    let path = history_socket();
    let _ = std::fs::remove_file(&path);
    let (tx, rx) = mpsc::channel();
    let title = format!("Kadr — {}", tr("Screenshot history"));
    let listener = std::os::unix::net::UnixListener::bind(&path).ok();
    let app = HistoryApp {
        cfg,
        items: vec![],
        selected: vec![],
        anchor: None,
        confirm_delete: false,
        stamp: None,
        checked: Instant::now(),
        export: None,
        rx,
        title: title.clone(),
    };
    float_later(title.clone(), 1180, 760, None, false);
    let r = eframe::run_native(
        "kadr",
        native(&title, [1180.0, 760.0]),
        Box::new(move |cc| {
            setup(&cc.egui_ctx, &app.cfg);
            if let Some(l) = listener {
                let ctx = cc.egui_ctx.clone();
                std::thread::spawn(move || {
                    use std::io::Read;
                    for s in l.incoming().flatten() {
                        let mut s = s;
                        let mut msg = String::new();
                        let _ = s.read_to_string(&mut msg);
                        let _ = tx.send(msg);
                        ctx.request_repaint();
                    }
                });
            }
            let mut app = app;
            app.reload(&cc.egui_ctx);
            Ok(Box::new(app))
        }),
    );
    let _ = std::fs::remove_file(&path);
    if let Err(e) = r {
        eprintln!("kadr: {e}");
        return 1;
    }
    0
}

fn dir_stamp() -> Option<std::time::SystemTime> {
    let d = crate::config::history_dir();
    let mut t = std::fs::metadata(&d).ok()?.modified().ok()?;
    // edits rewrite doc.json inside item folders: the newest few are enough to notice them
    if let Ok(rd) = std::fs::read_dir(&d) {
        for e in rd.flatten() {
            if let Ok(m) = e.path().join("doc.json").metadata().and_then(|m| m.modified()) {
                t = t.max(m);
            }
        }
    }
    Some(t)
}

impl HistoryApp {
    fn reload(&mut self, _ctx: &egui::Context) {
        self.stamp = dir_stamp();
        let old: std::collections::HashMap<String, Option<egui::TextureHandle>> =
            self.items.drain(..).map(|t| (t.id, t.tex)).collect();
        for it in history::list_items() {
            let m = &it.meta;
            let size = m.get("size").and_then(|s| s.as_array()).map_or("?×?".to_string(), |a| {
                format!(
                    "{}×{}",
                    a.first().and_then(|v| v.as_i64()).unwrap_or(0),
                    a.get(1).and_then(|v| v.as_i64()).unwrap_or(0)
                )
            });
            let mut caption = format!(
                "{}\n{size} · {}",
                fmt_date(m.get("created").and_then(|v| v.as_str()).unwrap_or("")),
                trf("layers: {n}", &[("n", &it.layers)])
            );
            if let Some(o) = m.get("output").and_then(|v| v.as_str()) {
                caption.push_str(&format!(" · {o}"));
            }
            let tex = old.get(&it.id).cloned().flatten();
            self.items.push(Thumb {
                id: it.id,
                dir: it.dir,
                caption,
                tex,
            });
        }
        self.selected.retain(|s| self.items.iter().any(|t| &t.id == s));
    }

    fn open(&self, ids: &[String]) {
        for id in ids.iter().take(4) {
            crate::sys::spawn_self(&["edit", id]);
        }
    }

    fn copy(&self) {
        if let Some(id) = self.selected.first()
            && let Ok(doc) = history::load(id)
        {
            crate::sys::copy_image(&crate::shapes::render_result(&doc));
        }
    }

    fn export(&mut self) {
        if self.selected.is_empty() {
            return;
        }
        let ids = self.selected.clone();
        let rx = if ids.len() == 1 {
            save_dialog(&self.cfg, &format!("Kadr_{}.png", ids[0]), &tr("Export"))
        } else {
            let (tx, rx) = mpsc::channel();
            let dir = crate::sys::save_dir(&self.cfg);
            let title = tr("Export folder");
            std::thread::spawn(move || {
                let _ = tx.send(
                    rfd::FileDialog::new()
                        .set_title(&title)
                        .set_directory(dir)
                        .pick_folder(),
                );
            });
            rx
        };
        self.export = Some((ids, rx));
    }

    fn folder(&self) {
        let p = self
            .selected
            .first()
            .map_or_else(crate::config::history_dir, |id| history::item_dir(id));
        crate::sys::open_path(&p);
    }

    fn delete(&mut self, ctx: &egui::Context) {
        for id in std::mem::take(&mut self.selected) {
            history::delete(&id);
        }
        self.reload(ctx);
    }

    fn thumb(&mut self, ctx: &egui::Context, i: usize) -> Option<egui::TextureHandle> {
        let t = &mut self.items[i];
        if t.tex.is_none() {
            let img = image::open(t.dir.join("thumb.jpg")).ok()?.to_rgba8();
            let ci =
                egui::ColorImage::from_rgba_unmultiplied([img.width() as usize, img.height() as usize], img.as_raw());
            t.tex = Some(ctx.load_texture(format!("thumb-{}", t.id), ci, egui::TextureOptions::LINEAR));
        }
        t.tex.clone()
    }

    fn click(&mut self, id: &str, mods: egui::Modifiers) {
        if mods.ctrl {
            if let Some(p) = self.selected.iter().position(|s| s == id) {
                self.selected.remove(p);
            } else {
                self.selected.push(id.to_string());
            }
            self.anchor = Some(id.to_string());
        } else if mods.shift
            && let Some(a) = &self.anchor
        {
            let ia = self.items.iter().position(|t| &t.id == a);
            let ib = self.items.iter().position(|t| t.id == id);
            if let (Some(ia), Some(ib)) = (ia, ib) {
                self.selected = self.items[ia.min(ib)..=ia.max(ib)]
                    .iter()
                    .map(|t| t.id.clone())
                    .collect();
            }
        } else {
            self.selected = vec![id.to_string()];
            self.anchor = Some(id.to_string());
        }
    }
}

impl eframe::App for HistoryApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        while let Ok(msg) = self.rx.try_recv() {
            match msg.trim() {
                "raise" => {
                    self.reload(&ctx);
                    let t = self.title.clone();
                    std::thread::spawn(move || crate::hypr::focus(&t));
                    ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
                }
                "lang" => {
                    self.cfg = Config::load();
                    crate::i18n::setup(&self.cfg.str("language"));
                    self.reload(&ctx);
                }
                _ => {}
            }
        }
        if self.checked.elapsed() > Duration::from_secs(2) {
            self.checked = Instant::now();
            if dir_stamp() != self.stamp {
                self.reload(&ctx);
            }
        }
        ctx.request_repaint_after(Duration::from_secs(2));
        if let Some((ids, rx)) = &self.export {
            match rx.try_recv() {
                Ok(Some(path)) => {
                    for id in ids {
                        if let Ok(doc) = history::load(id) {
                            let img = crate::shapes::render_result(&doc);
                            let target = if ids.len() == 1 {
                                path.clone()
                            } else {
                                path.join(format!("Kadr_{id}.png"))
                            };
                            let _ = crate::imgio::save_any(&img, &target);
                        }
                    }
                    self.export = None;
                }
                Ok(None) | Err(mpsc::TryRecvError::Disconnected) => self.export = None,
                Err(mpsc::TryRecvError::Empty) => ctx.request_repaint_after(Duration::from_millis(100)),
            }
        }
        if !ctx.text_edit_focused() && !self.confirm_delete {
            let (del, enter, copy) = ctx.input(|i| {
                (
                    i.key_pressed(egui::Key::Delete),
                    i.key_pressed(egui::Key::Enter),
                    i.modifiers.ctrl && i.key_pressed(egui::Key::C),
                )
            });
            if del && !self.selected.is_empty() {
                self.confirm_delete = true;
            }
            if enter {
                self.open(&self.selected.clone());
            }
            if copy {
                self.copy();
            }
        }
        egui::CentralPanel::default()
            .frame(egui::Frame::NONE.fill(style::WINDOW_BG).inner_margin(egui::Margin {
                left: 12,
                right: 12,
                top: 10,
                bottom: 12,
            }))
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.label(
                        egui::RichText::new(tr("Screenshot history"))
                            .size(15.0)
                            .strong()
                            .color(style::TEXT),
                    );
                    ui.label(
                        egui::RichText::new(trf("Total: {n}", &[("n", &self.items.len())]))
                            .color(Color32::from_rgb(0x8a, 0x8a, 0x94)),
                    );
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let has = !self.selected.is_empty();
                        if text_button(ui, "trash", &tr("Delete"), has).clicked() {
                            self.confirm_delete = true;
                        }
                        if text_button(ui, "folder", &tr("Folder"), true).clicked() {
                            self.folder();
                        }
                        if text_button(ui, "export", &tr("Export"), has).clicked() {
                            self.export();
                        }
                        if text_button(ui, "copy", &tr("Copy"), has).clicked() {
                            self.copy();
                        }
                        if text_button(ui, "edit", &tr("Edit"), has).clicked() {
                            self.open(&self.selected.clone());
                        }
                    });
                });
                ui.add_space(6.0);
                let cell = vec2(350.0, 268.0);
                let cols = ((ui.available_width() / cell.x).floor() as usize).max(1);
                let rows = self.items.len().div_ceil(cols);
                let accent = self.cfg.accent().egui();
                egui::ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .show_rows(ui, cell.y, rows, |ui, range| {
                        for row in range {
                            ui.horizontal(|ui| {
                                for c in 0..cols {
                                    let i = row * cols + c;
                                    if i >= self.items.len() {
                                        break;
                                    }
                                    let (rect, resp) = ui.allocate_exact_size(cell, Sense::click());
                                    let r = rect.shrink(4.0);
                                    let id = self.items[i].id.clone();
                                    let sel = self.selected.contains(&id);
                                    let bg = if sel {
                                        Color32::from_rgba_unmultiplied(accent.r(), accent.g(), accent.b(), 70)
                                    } else if resp.hovered() {
                                        Color32::from_white_alpha(16)
                                    } else {
                                        Color32::from_white_alpha(8)
                                    };
                                    ui.painter().rect_filled(r, 10.0, bg);
                                    if sel {
                                        ui.painter().rect_stroke(
                                            r,
                                            10.0,
                                            Stroke::new(1.5, accent),
                                            egui::StrokeKind::Inside,
                                        );
                                    }
                                    if let Some(t) = self.thumb(&ctx, i) {
                                        let ts = t.size_vec2();
                                        let k = (320.0 / ts.x).min(200.0 / ts.y);
                                        let sz = ts * k;
                                        let ir = Rect::from_center_size(pos2(r.center().x, r.top() + 10.0 + 100.0), sz);
                                        ui.painter().image(
                                            t.id(),
                                            ir,
                                            Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
                                            Color32::WHITE,
                                        );
                                    }
                                    let mut job = egui::text::LayoutJob::simple(
                                        self.items[i].caption.clone(),
                                        egui::FontId::proportional(12.0),
                                        style::TEXT,
                                        r.width() - 16.0,
                                    );
                                    job.halign = egui::Align::Center;
                                    let galley = ui.painter().layout_job(job);
                                    ui.painter()
                                        .galley(pos2(r.center().x, r.top() + 218.0), galley, style::TEXT);
                                    let resp = resp.on_hover_cursor(egui::CursorIcon::PointingHand);
                                    if resp.clicked() {
                                        let m = ui.input(|i| i.modifiers);
                                        self.click(&id, m);
                                    }
                                    if resp.double_clicked() {
                                        self.open(&[id]);
                                    }
                                }
                            });
                        }
                    });
            });
        if self.confirm_delete {
            let n = self.selected.len();
            let modal = egui::Modal::new(egui::Id::new("delete")).show(&ctx, |ui| {
                ui.set_width(360.0);
                ui.label(trf("Delete screenshots: {n}? This cannot be undone.", &[("n", &n)]));
                ui.add_space(10.0);
                ui.horizontal(|ui| {
                    if ui.button(tr("Delete")).clicked() {
                        self.delete(&ctx);
                        self.confirm_delete = false;
                    }
                    if ui.button(tr("Cancel")).clicked() {
                        self.confirm_delete = false;
                    }
                });
            });
            if modal.should_close() {
                self.confirm_delete = false;
            }
        }
    }
}

fn text_button(ui: &mut egui::Ui, icon: &str, text: &str, enabled: bool) -> egui::Response {
    let color = if enabled { "#e6e6ea" } else { "#6a6a72" };
    let img = crate::icons::image(ui.ctx(), icon, color, 18.0);
    let b = egui::Button::image_and_text(
        img,
        egui::RichText::new(text).color(if enabled { style::TEXT } else { style::DISABLED }),
    );
    ui.add_enabled(enabled, b)
        .on_hover_cursor(egui::CursorIcon::PointingHand)
}

// --------------------------------------------------------------------- pin

struct Pin {
    tex: Option<egui::TextureHandle>,
    img: tiny_skia::Pixmap,
    base: Vec2,
    zoom: f32,
    alpha: f32,
    tag: String,
    cfg: Config,
    save: Option<mpsc::Receiver<Option<PathBuf>>>,
}

pub fn pin(cfg: Config, id: String, at: Option<(i32, i32)>) -> i32 {
    let doc = match history::load(&id) {
        Ok(d) => d,
        Err(e) => {
            eprintln!("kadr: {e}");
            return 1;
        }
    };
    let scale = doc.meta_f("scale", 1.0).max(0.1);
    let img = crate::shapes::render_result(&doc);
    let base = vec2(
        (img.width() as f32 / scale).round(),
        (img.height() as f32 / scale).round(),
    );
    let tag = format!("kadr-pin-{}", std::process::id());
    let opts = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title(&tag)
            .with_app_id("kadr")
            .with_inner_size(base)
            .with_decorations(false)
            .with_transparent(true),
        renderer: eframe::Renderer::Glow,
        ..Default::default()
    };
    float_later(tag.clone(), base.x as i32, base.y as i32, at, true);
    let app = Pin {
        tex: None,
        img,
        base,
        zoom: 1.0,
        alpha: 1.0,
        tag,
        cfg,
        save: None,
    };
    let r = eframe::run_native(
        "kadr",
        opts,
        Box::new(|cc| {
            setup(&cc.egui_ctx, &app.cfg);
            Ok(Box::new(app))
        }),
    );
    if let Err(e) = r {
        eprintln!("kadr: {e}");
        return 1;
    }
    0
}

impl Pin {
    fn target(&self) -> Vec2 {
        vec2(
            (self.base.x * self.zoom).round().max(40.0),
            (self.base.y * self.zoom).round().max(30.0),
        )
    }

    fn apply_size(&self, ctx: &egui::Context) {
        let t = self.target();
        ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(t));
        let tag = self.tag.clone();
        std::thread::spawn(move || crate::hypr::resize(&tag, t.x as i32, t.y as i32));
    }
}

impl eframe::App for Pin {
    fn clear_color(&self, _: &egui::Visuals) -> [f32; 4] {
        [0.0, 0.0, 0.0, 0.0]
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        let tex = self.tex.get_or_insert_with(|| {
            let ci = egui::ColorImage::from_rgba_premultiplied(
                [self.img.width() as usize, self.img.height() as usize],
                self.img.data(),
            );
            ctx.load_texture("pin", ci, egui::TextureOptions::LINEAR)
        });
        let rect = ui.max_rect();
        let tint = Color32::from_white_alpha((self.alpha * 255.0) as u8);
        ui.painter()
            .image(tex.id(), rect, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), tint);
        ui.painter().rect_stroke(
            rect.shrink(0.5),
            0.0,
            Stroke::new(1.0, self.cfg.accent().egui()),
            egui::StrokeKind::Middle,
        );
        let resp = ui
            .interact(rect, egui::Id::new("pin-area"), Sense::click_and_drag())
            .on_hover_text(tr(
                "Wheel — zoom, Ctrl+wheel — opacity, double click / Esc — close, right click — menu",
            ));
        if resp.drag_started_by(egui::PointerButton::Primary) {
            ctx.send_viewport_cmd(egui::ViewportCommand::StartDrag);
        }
        if resp.double_clicked() || ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
        if ctx.input(|i| i.modifiers.ctrl && i.key_pressed(egui::Key::C)) {
            crate::sys::copy_image(&self.img);
        }
        let (dy, ctrl) = ctx.input(|i| {
            let dy: f32 = i
                .raw
                .events
                .iter()
                .map(|e| match e {
                    egui::Event::MouseWheel { delta, .. } => delta.y,
                    _ => 0.0,
                })
                .sum();
            (dy, i.modifiers.ctrl)
        });
        if dy != 0.0 && resp.hovered() {
            if ctrl {
                self.alpha = (self.alpha + if dy > 0.0 { 0.08 } else { -0.08 }).clamp(0.15, 1.0);
            } else {
                self.zoom = (self.zoom * if dy > 0.0 { 1.1 } else { 1.0 / 1.1 }).clamp(0.1, 8.0);
                self.apply_size(&ctx);
            }
        }
        resp.context_menu(|ui| {
            if ui.button(tr("Copy")).clicked() {
                crate::sys::copy_image(&self.img);
                ui.close();
            }
            if ui.button(tr("Save as…")).clicked() {
                self.save = Some(save_dialog(&self.cfg, "kadr.png", &tr("Save screenshot")));
                ui.close();
            }
            if ui.button(tr("Zoom 100%")).clicked() {
                self.zoom = 1.0;
                self.apply_size(&ctx);
                ui.close();
            }
            ui.separator();
            if ui.button(tr("Close")).clicked() {
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
        });
        if let Some(rx) = &self.save
            && let Ok(r) = rx.try_recv()
        {
            if let Some(p) = r {
                let _ = crate::imgio::save_any(&self.img, &p);
            }
            self.save = None;
        }
        if self.save.is_some() {
            ctx.request_repaint_after(Duration::from_millis(100));
        }
    }
}
