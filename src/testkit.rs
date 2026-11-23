//! Headless test harness: a canvas driven by synthetic input and rendered with egui_glow into
//! an offscreen framebuffer (surfaceless EGL on the render node). Never touches the screen;
//! the base image is synthetic.
#![cfg(test)]
use std::os::fd::AsRawFd;
use std::sync::Arc;

use glow::HasContext;
use khronos_egl as egl;

use crate::config::Config;
use crate::model::Document;
use crate::ui::canvas::{Canvas, Ctl};
use crate::ui::{panels, style};

#[link(name = "gbm")]
unsafe extern "C" {
    fn gbm_create_device(fd: libc::c_int) -> *mut libc::c_void;
}

pub struct Gpu {
    pub gl: Arc<glow::Context>,
    _file: std::fs::File,
}

pub fn gpu() -> Gpu {
    let node = std::fs::read_dir("/dev/dri")
        .unwrap()
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.file_name().unwrap().to_string_lossy().starts_with("renderD"))
        .min()
        .expect("no render node");
    let file = std::fs::OpenOptions::new().read(true).write(true).open(node).unwrap();
    let e = egl::Instance::new(egl::Static);
    let gbm = unsafe { gbm_create_device(file.as_raw_fd()) };
    let dpy = unsafe { e.get_platform_display(0x31D7, gbm, &[egl::ATTRIB_NONE]) }.unwrap();
    e.initialize(dpy).unwrap();
    e.bind_api(egl::OPENGL_ES_API).unwrap();
    let cfg = e
        .choose_first_config(dpy, &[egl::RENDERABLE_TYPE, egl::OPENGL_ES3_BIT, egl::NONE])
        .unwrap();
    let ctx = e
        .create_context(
            dpy,
            cfg.unwrap_or_else(|| unsafe { egl::Config::from_ptr(std::ptr::null_mut()) }),
            None,
            &[egl::CONTEXT_MAJOR_VERSION, 3, egl::NONE],
        )
        .unwrap();
    e.make_current(dpy, None, None, Some(ctx)).unwrap();
    let gl = unsafe {
        glow::Context::from_loader_function(|s| e.get_proc_address(s).map_or(std::ptr::null(), |f| f as *const _))
    };
    std::mem::forget(e);
    Gpu {
        gl: Arc::new(gl),
        _file: file,
    }
}

/// Synthetic "desktop": gradient wallpaper with a few flat "windows" and some text-like bars.
pub fn fake_desktop(w: u32, h: u32) -> tiny_skia::Pixmap {
    let mut pm = tiny_skia::Pixmap::new(w, h).unwrap();
    for (i, px) in pm.pixels_mut().iter_mut().enumerate() {
        let (x, y) = ((i as u32 % w) as f32 / w as f32, (i as u32 / w) as f32 / h as f32);
        *px = tiny_skia::ColorU8::from_rgba(
            (40.0 + 80.0 * x) as u8,
            (60.0 + 60.0 * y) as u8,
            (110.0 + 90.0 * (1.0 - x)) as u8,
            255,
        )
        .premultiply();
    }
    let mut paint = tiny_skia::Paint::default();
    for (rx, ry, rw, rh, c) in [
        (80.0, 60.0, 520.0, 380.0, (236, 236, 240)),
        (660.0, 120.0, 520.0, 520.0, (30, 32, 40)),
    ] {
        paint.set_color_rgba8(c.0, c.1, c.2, 255);
        pm.fill_rect(
            tiny_skia::Rect::from_xywh(rx, ry, rw, rh).unwrap(),
            &paint,
            tiny_skia::Transform::identity(),
            None,
        );
        paint.set_color_rgba8(90, 140, 230, 255);
        pm.fill_rect(
            tiny_skia::Rect::from_xywh(rx, ry, rw, 28.0).unwrap(),
            &paint,
            tiny_skia::Transform::identity(),
            None,
        );
        for k in 0..8 {
            paint.set_color_rgba8(if c.0 > 128 { 60 } else { 200 }, 90, 110, 255);
            let bw = 120.0 + ((k * 37) % 200) as f32;
            pm.fill_rect(
                tiny_skia::Rect::from_xywh(rx + 24.0, ry + 56.0 + k as f32 * 34.0, bw, 12.0).unwrap(),
                &paint,
                tiny_skia::Transform::identity(),
                None,
            );
        }
    }
    pm
}

pub struct Harness {
    pub ctx: egui::Context,
    pub canvas: Canvas,
    pub cfg: Config,
    pub ctl: Ctl,
    gpu: Gpu,
    painter: egui_glow::Painter,
    pub size: (u32, u32),
    pub ppp: f32,
    t: f64,
    pending: Vec<egui::Event>,
    pub mods: egui::Modifiers,
}

impl Harness {
    pub fn new(editor: bool, size: (u32, u32)) -> Harness {
        crate::i18n::setup("en");
        let gpu = gpu();
        let painter = egui_glow::Painter::new(gpu.gl.clone(), "", None, false).unwrap();
        let mut cfg = Config::load();
        // isolated from the user's settings
        cfg.v = Config::default_for_tests();
        let base = Arc::new(fake_desktop(size.0, size.1));
        let mut meta = serde_json::Map::new();
        meta.insert("scale".into(), 1.0.into());
        let doc = Document::new(base, None, vec![], meta);
        let wins = vec![(crate::geom::r(80.0, 60.0, 520.0, 380.0), "Fake — Window".to_string())];
        let mut canvas = Canvas::new(doc, &cfg, editor, 1.0, wins);
        canvas.active = true;
        let ctx = egui::Context::default();
        style::setup_fonts(&ctx);
        style::setup(&ctx, cfg.accent());
        Harness {
            ctx,
            canvas,
            cfg,
            ctl: Ctl::default(),
            gpu,
            painter,
            size,
            ppp: 1.0,
            t: 0.0,
            pending: vec![],
            mods: egui::Modifiers::NONE,
        }
    }

    pub fn push(&mut self, ev: egui::Event) {
        self.pending.push(ev);
    }

    /// One egui pass (+ optional readback of the frame as a pixmap).
    pub fn frame_px(&mut self, read: bool) -> Option<tiny_skia::Pixmap> {
        self.t += 1.0 / 30.0;
        let (w, h) = self.size;
        let lw = w as f32 / self.ppp;
        let lh = h as f32 / self.ppp;
        let mut raw = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(lw, lh))),
            time: Some(self.t),
            events: std::mem::take(&mut self.pending),
            focused: true,
            max_texture_side: Some(self.painter.max_texture_side()),
            ..Default::default()
        };
        raw.viewports
            .entry(egui::ViewportId::ROOT)
            .or_default()
            .native_pixels_per_point = Some(self.ppp);
        let (cv, cfg, ctl) = (&mut self.canvas, &mut self.cfg, &mut self.ctl);
        let editor = cv.editor;
        let mut out = self.ctx.run_ui(raw, |ui| {
            let ctx = &ui.ctx().clone();
            cv.rect = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(lw, lh));
            cv.input(ctx, cfg, if editor { None } else { Some(ctl) }, true);
            cv.paint(ctx);
            panels::show(cv, ctx, cfg);
        });
        let gl = &self.gpu.gl;
        unsafe {
            let fbo = gl.create_framebuffer().unwrap();
            let rb = gl.create_renderbuffer().unwrap();
            gl.bind_renderbuffer(glow::RENDERBUFFER, Some(rb));
            gl.renderbuffer_storage(glow::RENDERBUFFER, glow::RGBA8, w as i32, h as i32);
            gl.bind_framebuffer(glow::FRAMEBUFFER, Some(fbo));
            gl.framebuffer_renderbuffer(glow::FRAMEBUFFER, glow::COLOR_ATTACHMENT0, glow::RENDERBUFFER, Some(rb));
            let prims = self.ctx.tessellate(std::mem::take(&mut out.shapes), self.ppp);
            egui_glow::painter::clear(gl, [w, h], [0.0, 0.0, 0.0, 1.0]);
            self.painter
                .paint_and_update_textures([w, h], self.ppp, &prims, &mut out.textures_delta);
            let res = if read {
                let mut data = vec![0u8; (w * h * 4) as usize];
                gl.bind_framebuffer(glow::FRAMEBUFFER, Some(fbo));
                gl.read_pixels(
                    0,
                    0,
                    w as i32,
                    h as i32,
                    glow::RGBA,
                    glow::UNSIGNED_BYTE,
                    glow::PixelPackData::Slice(Some(&mut data)),
                );
                // GL rows are bottom-up
                let row = (w * 4) as usize;
                let mut flipped = vec![0u8; data.len()];
                for y in 0..h as usize {
                    flipped[y * row..(y + 1) * row]
                        .copy_from_slice(&data[(h as usize - 1 - y) * row..(h as usize - y) * row]);
                }
                for px in flipped.chunks_exact_mut(4) {
                    px[3] = 255;
                }
                tiny_skia::Pixmap::from_vec(flipped, tiny_skia::IntSize::from_wh(w, h).unwrap())
            } else {
                None
            };
            gl.bind_framebuffer(glow::FRAMEBUFFER, None);
            gl.delete_framebuffer(fbo);
            gl.delete_renderbuffer(rb);
            res
        }
    }

    pub fn frame(&mut self) {
        self.frame_px(false);
    }

    pub fn snapshot(&mut self, name: &str) -> std::path::PathBuf {
        let img = self.frame_px(true).unwrap();
        let dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target/shots");
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join(format!("{name}.png"));
        crate::imgio::save_png(&img, &p).unwrap();
        p
    }

    pub fn move_to(&mut self, x: f32, y: f32) {
        self.push(egui::Event::PointerMoved(egui::pos2(x, y)));
        self.frame();
    }

    pub fn button(&mut self, x: f32, y: f32, button: egui::PointerButton, pressed: bool) {
        let m = self.mods;
        self.push(egui::Event::PointerButton {
            pos: egui::pos2(x, y),
            button,
            pressed,
            modifiers: m,
        });
        self.frame();
    }

    pub fn click(&mut self, x: f32, y: f32) {
        self.move_to(x, y);
        self.button(x, y, egui::PointerButton::Primary, true);
        self.button(x, y, egui::PointerButton::Primary, false);
        // let the double-click window pass
        std::thread::sleep(std::time::Duration::from_millis(420));
        self.frame();
    }

    pub fn drag(&mut self, from: (f32, f32), to: (f32, f32)) {
        self.move_to(from.0, from.1);
        self.button(from.0, from.1, egui::PointerButton::Primary, true);
        for i in 1..=8 {
            let k = i as f32 / 8.0;
            self.move_to(from.0 + (to.0 - from.0) * k, from.1 + (to.1 - from.1) * k);
        }
        self.button(to.0, to.1, egui::PointerButton::Primary, false);
        std::thread::sleep(std::time::Duration::from_millis(420));
        self.frame();
    }

    pub fn key(&mut self, key: egui::Key) {
        let m = self.mods;
        self.push(egui::Event::Key {
            key,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: m,
        });
        self.push(egui::Event::Key {
            key,
            physical_key: None,
            pressed: false,
            repeat: false,
            modifiers: m,
        });
        self.frame();
    }

    pub fn type_text(&mut self, s: &str) {
        self.push(egui::Event::Text(s.to_string()));
        self.frame();
    }
}
