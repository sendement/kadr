//! Inline color picker (drawn inside the canvas: separate toplevels would hide under the overlay).
use egui::{Color32, Pos2, Rect, Sense, Stroke, Vec2, pos2, vec2};

use crate::color::Color;
use crate::i18n::tr;

pub const PALETTE: &[&str] = &[
    "#ffff3b30",
    "#ffff9500",
    "#ffffcc00",
    "#ff34c759",
    "#ff00c7be",
    "#ff30b0ff",
    "#ff007aff",
    "#ff5856d6",
    "#ffaf52de",
    "#ffff2d55",
    "#ffffffff",
    "#ff8e8e93",
    "#ff3a3a3c",
    "#ff000000",
];

pub struct ColorPopup {
    pub key: String,
    pub allow_none: bool,
    pub anchor: Rect,
    pub h: f32,
    pub s: f32,
    pub v: f32,
    pub a: f32,
    pub hex: String,
    /// color chosen with the eyedropper, applied on the next frame
    pub picked: Option<Color>,
    pub current: Option<Color>,
}

pub enum Change {
    None,
    Set(Option<Color>),
    Eyedropper,
}

impl ColorPopup {
    pub fn new(key: &str, color: Option<Color>, allow_none: bool, anchor: Rect) -> ColorPopup {
        let mut p = ColorPopup {
            key: key.into(),
            allow_none,
            anchor,
            h: 0.0,
            s: 1.0,
            v: 1.0,
            a: 1.0,
            hex: String::new(),
            picked: None,
            current: color,
        };
        p.load(color.unwrap_or(Color::rgb(0xff, 0x3b, 0x30)));
        p
    }

    fn load(&mut self, c: Color) {
        let (h, s, v, a) = c.to_hsv();
        if h >= 0.0 {
            self.h = h;
        }
        self.s = s;
        self.v = v;
        self.a = a;
        self.hex = if c.a < 255 { c.argb() } else { c.rgb_name() };
    }

    fn color(&self) -> Color {
        Color::from_hsv(self.h.min(0.9999), self.s, self.v, self.a)
    }

    pub fn ui(&mut self, ui: &mut egui::Ui, recent: &[String]) -> Change {
        let mut change = Change::None;
        if let Some(c) = self.picked.take() {
            self.load(c);
            change = Change::Set(Some(c));
        }
        ui.spacing_mut().item_spacing = vec2(3.0, 3.0);
        egui::Grid::new("palette").spacing(vec2(3.0, 3.0)).show(ui, |ui| {
            for (i, c) in PALETTE.iter().enumerate() {
                let col = Color::parse(c);
                if swatch_button(ui, col, vec2(22.0, 22.0), 5.0)
                    .on_hover_text(col.map_or(String::new(), |c| c.argb()))
                    .clicked()
                {
                    self.load(col.unwrap());
                    change = Change::Set(col);
                }
                if i % 7 == 6 {
                    ui.end_row();
                }
            }
        });
        if !recent.is_empty() {
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new(tr("Recent")).size(11.0));
                for c in recent.iter().take(6) {
                    let col = Color::parse(c);
                    if swatch_button(ui, col, vec2(22.0, 22.0), 5.0).on_hover_text(c).clicked() && col.is_some() {
                        self.load(col.unwrap());
                        change = Change::Set(col);
                    }
                }
            });
        }
        ui.add_space(5.0);
        // saturation/value square
        let (rect, resp) = ui.allocate_exact_size(vec2(200.0, 130.0), Sense::click_and_drag());
        let painter = ui.painter_at(rect);
        let hue = Color::from_hsv(self.h.min(0.9999), 1.0, 1.0, 1.0).egui();
        let mut mesh = egui::Mesh::default();
        let steps = 24;
        for i in 0..steps {
            for j in 0..steps {
                let (s0, s1) = (i as f32 / steps as f32, (i + 1) as f32 / steps as f32);
                let (v0, v1) = (1.0 - j as f32 / steps as f32, 1.0 - (j + 1) as f32 / steps as f32);
                let c = |s: f32, v: f32| Color::from_hsv(self.h.min(0.9999), s, v, 1.0).egui();
                let r = Rect::from_min_max(
                    pos2(rect.left() + s0 * rect.width(), rect.top() + (1.0 - v0) * rect.height()),
                    pos2(rect.left() + s1 * rect.width(), rect.top() + (1.0 - v1) * rect.height()),
                );
                let base = mesh.vertices.len() as u32;
                mesh.colored_vertex(r.left_top(), c(s0, v0));
                mesh.colored_vertex(r.right_top(), c(s1, v0));
                mesh.colored_vertex(r.right_bottom(), c(s1, v1));
                mesh.colored_vertex(r.left_bottom(), c(s0, v1));
                mesh.add_triangle(base, base + 1, base + 2);
                mesh.add_triangle(base, base + 2, base + 3);
            }
        }
        let _ = hue;
        painter.add(mesh);
        let cp = pos2(
            rect.left() + self.s * rect.width(),
            rect.top() + (1.0 - self.v) * rect.height(),
        );
        painter.circle_stroke(cp, 6.0, Stroke::new(3.0, Color32::BLACK));
        painter.circle_stroke(cp, 6.0, Stroke::new(1.5, Color32::WHITE));
        if (resp.dragged() || resp.clicked() || resp.is_pointer_button_down_on())
            && let Some(q) = resp.interact_pointer_pos()
        {
            self.s = ((q.x - rect.left()) / rect.width()).clamp(0.0, 1.0);
            self.v = 1.0 - ((q.y - rect.top()) / rect.height()).clamp(0.0, 1.0);
            change = self.from_ui();
        }
        if let Some(v) = bar(ui, BarKind::Hue, self.h, self.color()) {
            self.h = v;
            change = self.from_ui();
        }
        if let Some(v) = bar(ui, BarKind::Alpha, self.a, self.color()) {
            self.a = v;
            change = self.from_ui();
        }
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            let te = ui.add(
                egui::TextEdit::singleline(&mut self.hex)
                    .desired_width(92.0)
                    .font(egui::TextStyle::Monospace),
            );
            if te.lost_focus()
                && ui.input(|i| i.key_pressed(egui::Key::Enter))
                && let Some(c) = Color::parse(&self.hex)
            {
                self.load(c);
                change = Change::Set(Some(c));
            }
            if super::panels::icon_button(
                ui,
                "eyedropper",
                &tr("Eyedropper: pick a color from the screenshot"),
                false,
                true,
            )
            .clicked()
            {
                change = Change::Eyedropper;
            }
            if self.allow_none && ui.button(tr("None")).on_hover_text(tr("No color")).clicked() {
                change = Change::Set(None);
            }
        });
        if let Change::Set(c) = &change {
            self.current = *c;
        }
        change
    }

    fn from_ui(&mut self) -> Change {
        let c = self.color();
        self.hex = if c.a < 255 { c.argb() } else { c.rgb_name() };
        Change::Set(Some(c))
    }
}

enum BarKind {
    Hue,
    Alpha,
}

fn bar(ui: &mut egui::Ui, kind: BarKind, value: f32, color: Color) -> Option<f32> {
    let (rect, resp) = ui.allocate_exact_size(vec2(200.0, 14.0), Sense::click_and_drag());
    let painter = ui.painter_at(rect.expand(2.0));
    let r = rect.shrink2(vec2(0.0, 2.0));
    let mut mesh = egui::Mesh::default();
    match kind {
        BarKind::Hue => {
            let n = 36;
            for i in 0..n {
                let (a, b) = (i as f32 / n as f32, (i + 1) as f32 / n as f32);
                let c0 = Color::from_hsv(a.min(0.9999), 1.0, 1.0, 1.0).egui();
                let c1 = Color::from_hsv(b.min(0.9999), 1.0, 1.0, 1.0).egui();
                quad(
                    &mut mesh,
                    Rect::from_min_max(
                        pos2(r.left() + a * r.width(), r.top()),
                        pos2(r.left() + b * r.width(), r.bottom()),
                    ),
                    c0,
                    c1,
                );
            }
        }
        BarKind::Alpha => {
            checker(&painter, r, 4.0);
            let c0 = color.with_alpha(0).egui();
            let c1 = color.with_alpha(255).egui();
            quad(&mut mesh, r, c0, c1);
        }
    }
    painter.add(mesh);
    let x = rect.left() + value * (rect.width() - 1.0);
    let k = Rect::from_min_size(pos2(x - 3.0, rect.top()), vec2(6.0, rect.height()));
    painter.rect_stroke(k, 2.0, Stroke::new(3.0, Color32::BLACK), egui::StrokeKind::Middle);
    painter.rect_stroke(k, 2.0, Stroke::new(1.5, Color32::WHITE), egui::StrokeKind::Middle);
    if (resp.dragged() || resp.clicked() || resp.is_pointer_button_down_on())
        && let Some(q) = resp.interact_pointer_pos()
    {
        return Some(((q.x - rect.left()) / rect.width()).clamp(0.0, 1.0));
    }
    None
}

fn quad(mesh: &mut egui::Mesh, r: Rect, left: Color32, right: Color32) {
    let base = mesh.vertices.len() as u32;
    mesh.colored_vertex(r.left_top(), left);
    mesh.colored_vertex(r.right_top(), right);
    mesh.colored_vertex(r.right_bottom(), right);
    mesh.colored_vertex(r.left_bottom(), left);
    mesh.add_triangle(base, base + 1, base + 2);
    mesh.add_triangle(base, base + 2, base + 3);
}

pub fn checker(painter: &egui::Painter, r: Rect, size: f32) {
    painter.rect_filled(r, 0.0, Color32::from_rgb(0xcf, 0xcf, 0xcf));
    let mut y = r.top();
    let mut row = 0;
    while y < r.bottom() {
        let mut x = r.left() + if row % 2 == 1 { size } else { 0.0 };
        while x < r.right() {
            let cell = Rect::from_min_size(pos2(x, y), Vec2::splat(size)).intersect(r);
            painter.rect_filled(cell, 0.0, Color32::from_rgb(0x8f, 0x8f, 0x8f));
            x += size * 2.0;
        }
        y += size;
        row += 1;
    }
}

pub fn paint_swatch(painter: &egui::Painter, r: Rect, color: Option<Color>, radius: f32) {
    match color {
        None => {
            painter.rect_filled(r, radius, Color32::from_white_alpha(20));
            painter.line_segment(
                [r.left_bottom() + vec2(4.0, -4.0), r.right_top() + vec2(-4.0, 4.0)],
                Stroke::new(2.0, Color32::from_rgb(0xff, 0x45, 0x3a)),
            );
        }
        Some(c) => {
            if c.a < 255 {
                checker(&painter.with_clip_rect(r), r, 5.0);
            }
            painter.rect_filled(r, radius, c.egui());
        }
    }
    painter.rect_stroke(
        r.shrink(0.5),
        radius,
        Stroke::new(1.0, Color32::from_white_alpha(70)),
        egui::StrokeKind::Middle,
    );
}

pub fn swatch_button(ui: &mut egui::Ui, color: Option<Color>, size: Vec2, radius: f32) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(size, Sense::click());
    paint_swatch(ui.painter(), rect.shrink(2.0), color, radius);
    if resp.hovered() {
        ui.painter().rect_stroke(
            rect.shrink(0.5),
            radius + 1.0,
            Stroke::new(1.5, Color32::from_white_alpha(140)),
            egui::StrokeKind::Middle,
        );
    }
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
}

pub fn anchor_pos(anchor: Rect, size: Vec2, bounds: Rect) -> Pos2 {
    let mut pos = pos2(anchor.left(), anchor.bottom() + 6.0);
    if pos.y + size.y > bounds.bottom() - 8.0 {
        pos.y = anchor.top() - size.y - 6.0;
    }
    pos.x = pos.x.min(bounds.right() - size.x - 8.0).max(bounds.left() + 8.0);
    pos.y = pos.y.max(bounds.top() + 8.0);
    pos
}
