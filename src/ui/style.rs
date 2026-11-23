//! Look shared by the overlay and the windows: dark panels, accent highlights, system fonts.
use egui::{Color32, CornerRadius, FontData, FontDefinitions, FontFamily, Stroke, Visuals};

use crate::color::Color;

pub const PANEL_BG: Color32 = Color32::from_rgba_premultiplied(20, 20, 26, 238);
pub const TEXT: Color32 = Color32::from_rgb(0xe6, 0xe6, 0xea);
pub const TEXT_DIM: Color32 = Color32::from_rgb(0xb9, 0xb9, 0xc3);
pub const DISABLED: Color32 = Color32::from_rgb(0x6a, 0x6a, 0x72);
pub const WINDOW_BG: Color32 = Color32::from_rgb(0x14, 0x14, 0x18);

fn face_bytes(id: fontdb::ID) -> Option<(Vec<u8>, u32)> {
    let db = crate::text::db();
    let face = db.face(id)?;
    let bytes = match &face.source {
        fontdb::Source::File(p) => std::fs::read(p).ok()?,
        fontdb::Source::Binary(b) | fontdb::Source::SharedFile(_, b) => (**b).as_ref().to_vec(),
    };
    Some((bytes, face.index))
}

/// System sans-serif as the UI font, with CJK fallbacks found by coverage.
pub fn setup_fonts(ctx: &egui::Context) {
    let mut fonts = FontDefinitions::default();
    let mut added = Vec::new();
    if let Some(id) = crate::text::query("Sans Serif", false, false)
        && let Some((bytes, index)) = face_bytes(id)
    {
        let mut fd = FontData::from_owned(bytes);
        fd.index = index;
        fonts.font_data.insert("system".into(), fd.into());
        added.push(("system".to_string(), true));
    }
    let db = crate::text::db();
    for (i, probe) in ['中', 'あ', '한'].into_iter().enumerate() {
        let id = crate::text::query("Sans Serif", false, false).map(|p| crate::text::fallback_for(p, probe));
        if let Some(id) = id
            && !added.iter().any(|(n, _)| n == &format!("{id:?}"))
            && db.face(id).is_some()
            && let Some((bytes, index)) = face_bytes(id)
        {
            let mut fd = FontData::from_owned(bytes);
            fd.index = index;
            let name = format!("{id:?}");
            fonts.font_data.insert(name.clone(), fd.into());
            added.push((name, false));
        }
        let _ = i;
    }
    for (name, primary) in added {
        let list = fonts.families.entry(FontFamily::Proportional).or_default();
        if primary {
            list.insert(0, name);
        } else {
            list.push(name);
        }
    }
    ctx.set_fonts(fonts);
}

pub fn setup(ctx: &egui::Context, accent: Color) {
    let a = accent.egui();
    let mut v = Visuals::dark();
    v.window_fill = PANEL_BG;
    v.panel_fill = WINDOW_BG;
    v.extreme_bg_color = Color32::from_rgba_unmultiplied(255, 255, 255, 16);
    v.selection.bg_fill = Color32::from_rgba_unmultiplied(a.r(), a.g(), a.b(), 110);
    v.selection.stroke = Stroke::new(1.0, Color32::WHITE);
    v.hyperlink_color = a;
    v.widgets.noninteractive.fg_stroke = Stroke::new(1.0, TEXT_DIM);
    v.widgets.inactive.fg_stroke = Stroke::new(1.0, TEXT);
    v.widgets.inactive.weak_bg_fill = Color32::TRANSPARENT;
    v.widgets.inactive.bg_fill = Color32::from_rgba_unmultiplied(255, 255, 255, 40);
    v.widgets.hovered.weak_bg_fill = Color32::from_rgba_unmultiplied(255, 255, 255, 24);
    v.widgets.hovered.bg_fill = Color32::from_rgba_unmultiplied(255, 255, 255, 60);
    v.widgets.hovered.bg_stroke = Stroke::NONE;
    v.widgets.active.weak_bg_fill = Color32::from_rgba_unmultiplied(255, 255, 255, 36);
    v.widgets.active.bg_fill = a;
    v.widgets.active.bg_stroke = Stroke::NONE;
    for w in [
        &mut v.widgets.inactive,
        &mut v.widgets.hovered,
        &mut v.widgets.active,
        &mut v.widgets.open,
    ] {
        w.corner_radius = CornerRadius::same(7);
    }
    v.slider_trailing_fill = true;
    v.window_corner_radius = CornerRadius::same(12);
    v.menu_corner_radius = CornerRadius::same(8);
    v.popup_shadow = egui::Shadow::NONE;
    ctx.set_visuals(v);
    ctx.all_styles_mut(|s| {
        s.spacing.item_spacing = egui::vec2(4.0, 4.0);
        s.spacing.button_padding = egui::vec2(6.0, 4.0);
        s.spacing.interact_size.y = 26.0;
        s.interaction.tooltip_delay = 0.35;
        s.interaction.selectable_labels = false;
    });
}

pub fn panel_frame() -> egui::Frame {
    egui::Frame::NONE
        .fill(PANEL_BG)
        .stroke(Stroke::new(1.0, Color32::from_rgba_unmultiplied(255, 255, 255, 30)))
        .corner_radius(CornerRadius::same(12))
        .inner_margin(egui::Margin::same(6))
}

/// Text color that reads on top of `bg`.
pub fn on(bg: Color32) -> Color32 {
    let l = 0.299 * bg.r() as f32 + 0.587 * bg.g() as f32 + 0.114 * bg.b() as f32;
    if l > 150.0 {
        Color32::from_rgb(0x10, 0x10, 0x14)
    } else {
        Color32::WHITE
    }
}
