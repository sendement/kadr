//! Floating panels that dock around the selection: tools, actions, properties, layers.
use egui::{Color32, Id, Order, Pos2, Rect, Sense, Stroke, Vec2, pos2, vec2};
use serde_json::{Value, json};

use crate::color::Color;
use crate::config::{Config, Obj};
use crate::i18n::{tr, trf};
use crate::model::{BLEND_MODES, Style, type_name};
use crate::shapes;

use super::canvas::{Action, Canvas, Mode};
use super::colorpicker::{self, Change, ColorPopup};
use super::style::{self, DISABLED, TEXT, TEXT_DIM};

pub const TOOLS: &[(&str, &str, &str)] = &[
    ("pointer", "Pointer: select and move layers, adjust the selection", "V"),
    ("pen", "Brush", "P"),
    ("highlighter", "Highlighter", "H"),
    ("line", "Line (Shift — 15° steps)", "L"),
    ("arrow", "Arrow (Shift — 15° steps)", "A"),
    ("rect", "Rectangle (Shift — square)", "R"),
    ("ellipse", "Ellipse (Shift — circle)", "E"),
    ("text", "Text", "T"),
    (
        "mark",
        "Numbered mark: click — place, drag — pull out with an arrow",
        "M",
    ),
    ("blur", "Blur / pixelate", "B"),
    (
        "zoom",
        "Magnifier: select an area, then drag the copy aside; copy corners — magnification",
        "Z",
    ),
];

fn accent32(ui: &egui::Ui) -> Color32 {
    ui.visuals().widgets.active.bg_fill
}

/// Square icon button: hover tint, accent fill when checked.
pub fn icon_button(ui: &mut egui::Ui, icon: &str, tip: &str, checked: bool, enabled: bool) -> egui::Response {
    icon_button_sized(ui, icon, tip, checked, enabled, 20.0)
}

pub fn icon_button_sized(
    ui: &mut egui::Ui,
    icon: &str,
    tip: &str,
    checked: bool,
    enabled: bool,
    icon_size: f32,
) -> egui::Response {
    let size = Vec2::splat(icon_size + 9.0);
    let (rect, resp) = ui.allocate_exact_size(size, if enabled { Sense::click() } else { Sense::hover() });
    let accent = accent32(ui);
    if checked {
        ui.painter().rect_filled(rect, 8.0, accent);
    } else if resp.hovered() && enabled {
        ui.painter().rect_filled(rect, 8.0, Color32::from_white_alpha(24));
    }
    let color = if !enabled {
        "#6a6a72".to_string()
    } else if checked {
        let on = style::on(accent);
        format!("#{:02x}{:02x}{:02x}", on.r(), on.g(), on.b())
    } else {
        "#e6e6ea".to_string()
    };
    let img = crate::icons::image(ui.ctx(), icon, &color, icon_size);
    img.paint_at(ui, Rect::from_center_size(rect.center(), Vec2::splat(icon_size)));
    let resp = if tip.is_empty() { resp } else { resp.on_hover_text(tip) };
    if enabled {
        resp.on_hover_cursor(egui::CursorIcon::PointingHand)
    } else {
        resp
    }
}

/// Accent button with icon + text (the main "Copy" action).
fn primary_button(ui: &mut egui::Ui, icon: &str, text: &str, tip: &str) -> egui::Response {
    let accent = accent32(ui);
    let fg = style::on(accent);
    let galley = ui
        .painter()
        .layout_no_wrap(text.to_string(), egui::FontId::proportional(13.0), fg);
    let size = vec2(20.0 + 6.0 + galley.size().x + 20.0, 29.0);
    let (rect, resp) = ui.allocate_exact_size(size, Sense::click());
    let fill = if resp.hovered() {
        accent.gamma_multiply(0.85)
    } else {
        accent
    };
    ui.painter().rect_filled(rect, 8.0, fill);
    let col = format!("#{:02x}{:02x}{:02x}", fg.r(), fg.g(), fg.b());
    crate::icons::image(ui.ctx(), icon, &col, 18.0).paint_at(
        ui,
        Rect::from_min_size(pos2(rect.left() + 9.0, rect.center().y - 9.0), Vec2::splat(18.0)),
    );
    ui.painter().galley(
        pos2(rect.left() + 32.0, rect.center().y - galley.size().y / 2.0),
        galley,
        fg,
    );
    resp.on_hover_text(tip).on_hover_cursor(egui::CursorIcon::PointingHand)
}

fn sep(ui: &mut egui::Ui, vertical_panel: bool) {
    let c = Color32::from_white_alpha(30);
    if vertical_panel {
        let (r, _) = ui.allocate_exact_size(vec2(ui.available_width().min(29.0), 7.0), Sense::hover());
        ui.painter().hline(r.x_range(), r.center().y, Stroke::new(1.0, c));
    } else {
        let (r, _) = ui.allocate_exact_size(vec2(7.0, 26.0), Sense::hover());
        ui.painter().vline(r.center().x, r.y_range(), Stroke::new(1.0, c));
    }
}

fn label(ui: &mut egui::Ui, t: &str) {
    ui.label(egui::RichText::new(tr(t)).size(11.5).color(TEXT_DIM));
}

fn area(id: &str, idx: usize, pos: Pos2, order: Order) -> egui::Area {
    egui::Area::new(Id::new((id, idx)))
        .fixed_pos(pos)
        .order(order)
        .constrain(false)
}

/// Draw every panel of the canvas; positions follow the selection (last frame's sizes).
pub fn show(cv: &mut Canvas, ctx: &egui::Context, cfg: &mut Config) {
    cv.panel_rects.clear();
    let Some(crop) = cv.doc.crop else { return };
    if cv.mode != Mode::Edit {
        return;
    }
    let bounds = cv.rect;
    let cw = cv.rect_s(crop.f());
    let (w_, h_, g) = (bounds.width(), bounds.height(), 8.0);
    let (bx, by) = (bounds.left(), bounds.top());
    let [tb, ab, pb, lb] = cv.panel_sizes;

    // toolbar: right of the selection, else left, else inside
    let mut side = None;
    let mut x = if bounds.right() - cw.right() >= tb.x + 2.0 * g {
        side = Some("right");
        cw.right() + g
    } else if cw.left() - bx >= tb.x + 2.0 * g {
        side = Some("left");
        cw.left() - g - tb.x
    } else {
        cw.right() - g - tb.x
    };
    x = x.min(bx + w_ - tb.x - g).max(bx + g);
    let y = cw.top().min(by + h_ - tb.y - g).max(by + g);
    let mut tb_pos = pos2(x, y);

    let mut below = cw.bottom() + g;
    let mut above = cw.top() - g;
    let mut in_bottom = cw.bottom() - g;
    let mut in_top = cw.top() + g;
    let mut place_h = |size: Vec2, prefer_below: bool, align_right: bool| -> Pos2 {
        let h = size.y;
        let prefs = if prefer_below {
            ["below", "above"]
        } else {
            ["above", "below"]
        };
        let mut y = None;
        for side in prefs {
            if side == "below" && below + h + g <= by + h_ {
                y = Some(below);
                below += h + g;
                break;
            }
            if side == "above" && above - h >= by + g {
                y = Some(above - h);
                above -= h + g;
                break;
            }
        }
        let y = y.unwrap_or_else(|| {
            if prefer_below {
                in_bottom -= h + g;
                in_bottom + g
            } else {
                let v = in_top;
                in_top += h + g;
                v
            }
        });
        let x = if align_right { cw.right() - size.x } else { cw.left() };
        pos2(
            x.min(bx + w_ - size.x - g).max(bx + g),
            y.min(by + h_ - h - g).max(by + g),
        )
    };
    let ab_pos = place_h(ab, true, true);
    let props_target = cv.props_target(cfg);
    let pb_pos = place_h(pb, false, false);

    let mut lb_pos = Pos2::ZERO;
    if cv.layers_visible {
        let x = if side != Some("left") && cw.left() - bx >= lb.x + 2.0 * g {
            cw.left() - g - lb.x
        } else if side == Some("right") && bounds.right() - (cw.right() + tb.x + g) >= lb.x + 2.0 * g {
            cw.right() + tb.x + 2.0 * g
        } else if side == Some("left") && bounds.right() - cw.right() >= lb.x + 2.0 * g {
            cw.right() + g
        } else if side != Some("left") {
            cw.left() + g
        } else {
            cw.right() - lb.x - g - if side.is_none() { tb.x + g } else { 0.0 }
        };
        lb_pos = pos2(
            x.min(bx + w_ - lb.x - g).max(bx + g),
            cw.top().min(by + h_ - lb.y - g).max(by + g),
        );
    }
    // inside-placement fallback can stack panels; push vertical ones clear of horizontal bars
    let ab_r = Rect::from_min_size(ab_pos, ab);
    let pb_r = Rect::from_min_size(pb_pos, pb);
    for (pos, size) in [(&mut tb_pos, tb), (&mut lb_pos, lb)] {
        for (hr, is_props) in [(pb_r, true), (ab_r, false)] {
            if is_props && props_target.is_none() {
                continue;
            }
            let vr = Rect::from_min_size(*pos, size);
            if size != Vec2::ZERO && vr.intersects(hr) {
                let y = if hr.center().y < vr.center().y || is_props {
                    hr.bottom() + g
                } else {
                    hr.top() - g - size.y
                };
                pos.y = y.min(by + h_ - size.y - g).max(by + g);
            }
        }
    }

    let idx = cv.index;
    let r = area("toolbar", idx, tb_pos, Order::Middle).show(ctx, |ui| {
        style::panel_frame().show(ui, |ui| {
            ui.spacing_mut().item_spacing = vec2(3.0, 3.0);
            ui.vertical(|ui| {
                for (name, tip, key) in TOOLS {
                    if icon_button(ui, name, &format!("{}  [{key}]", tr(tip)), cv.tool == *name, true).clicked() {
                        cv.set_tool(name);
                    }
                }
            });
        });
    });
    cv.panel_sizes[0] = r.response.rect.size();
    cv.panel_rects.push(r.response.rect);

    let r = area("actions", idx, ab_pos, Order::Middle).show(ctx, |ui| {
        style::panel_frame().show(ui, |ui| {
            ui.spacing_mut().item_spacing = vec2(3.0, 3.0);
            ui.horizontal(|ui| action_bar(cv, ui, cfg));
        });
    });
    cv.panel_sizes[1] = r.response.rect.size();
    cv.panel_rects.push(r.response.rect);

    if let Some((kind, st, layer)) = props_target {
        let r = area("props", idx, pb_pos, Order::Middle).show(ctx, |ui| {
            style::panel_frame().show(ui, |ui| {
                ui.spacing_mut().item_spacing = vec2(4.0, 3.0);
                ui.horizontal(|ui| props_bar(cv, ui, cfg, &kind, st, layer));
            });
        });
        cv.panel_sizes[2] = r.response.rect.size();
        cv.panel_rects.push(r.response.rect);
    } else {
        cv.panel_sizes[2] = Vec2::ZERO;
    }

    if cv.layers_visible {
        let r = area("layers", idx, lb_pos, Order::Middle).show(ctx, |ui| {
            style::panel_frame().show(ui, |ui| layers_panel(cv, ui));
        });
        cv.panel_sizes[3] = r.response.rect.size();
        cv.panel_rects.push(r.response.rect);
    } else {
        cv.panel_sizes[3] = Vec2::ZERO;
    }

    let size_now = cv.panel_sizes;
    if size_now != [tb, ab, pb, lb] {
        ctx.request_repaint();
    }

    // color popup on top of everything
    if cv.popup.is_some() {
        let recent = cfg.recent_colors();
        let mut popup = cv.popup.take().unwrap();
        let size = ctx
            .memory(|m| m.area_rect(Id::new(("color-popup", idx))))
            .map_or(vec2(224.0, 330.0), |r| r.size());
        let pos = colorpicker::anchor_pos(popup.anchor, size, bounds);
        let mut change = Change::None;
        let r = area("color-popup", idx, pos, Order::Foreground).show(ctx, |ui| {
            style::panel_frame()
                .inner_margin(egui::Margin::same(10))
                .show(ui, |ui| {
                    change = popup.ui(ui, &recent);
                });
        });
        cv.panel_rects.push(r.response.rect);
        match change {
            Change::Set(c) => {
                cv.apply_style(cfg, &popup.key, c.map_or(Value::Null, |c| json!(c.argb())));
                cv.popup = Some(popup);
            }
            Change::Eyedropper => {
                cv.start_pick(popup.key.clone());
                cv.popup = Some(popup);
            }
            Change::None => cv.popup = Some(popup),
        }
    }
}

/// Close the color popup, remembering its color among the recent ones.
pub fn close_popup(cv: &mut Canvas, cfg: &mut Config) {
    if let Some(p) = cv.popup.take()
        && let Some(c) = p.current
    {
        cfg.add_recent_color(&c.argb());
    }
}

fn action_bar(cv: &mut Canvas, ui: &mut egui::Ui, cfg: &mut Config) {
    if icon_button(
        ui,
        "undo",
        &format!("{}  [Ctrl+Z]", tr("Undo")),
        false,
        !cv.doc.undo_stack.is_empty(),
    )
    .clicked()
    {
        cv.undo();
    }
    if icon_button(
        ui,
        "redo",
        &format!("{}  [Ctrl+Shift+Z]", tr("Redo")),
        false,
        !cv.doc.redo_stack.is_empty(),
    )
    .clicked()
    {
        cv.redo();
    }
    sep(ui, false);
    if icon_button(
        ui,
        "layers",
        &format!("{}  [Tab]", tr("Layers panel")),
        cv.layers_visible,
        true,
    )
    .clicked()
    {
        cv.toggle_layers(cfg);
    }
    sep(ui, false);
    if icon_button(
        ui,
        "clipboard",
        &tr("Live clipboard: after every change\nthe final image is copied to the clipboard"),
        cv.live_copy,
        true,
    )
    .clicked()
    {
        cv.toggle_live_copy(cfg);
    }
    if cv.editor {
        if icon_button(
            ui,
            "export",
            &format!("{}  [Ctrl+Shift+S]", tr("Export to file…")),
            false,
            true,
        )
        .clicked()
        {
            cv.request(Action::Export);
        }
        if icon_button(ui, "save", &format!("{}  [Ctrl+S]", tr("Save to history")), false, true).clicked() {
            cv.request(Action::Save);
        }
    } else {
        if icon_button(ui, "edit", &format!("{}  [Ctrl+E]", tr("Open in editor")), false, true).clicked() {
            cv.request(Action::Edit);
        }
        if icon_button(ui, "pin", &tr("Pin on top of other windows"), false, true).clicked() {
            cv.request(Action::Pin);
        }
        if icon_button(ui, "save", &format!("{}  [Ctrl+S]", tr("Save to file")), false, true).clicked() {
            cv.request(Action::Save);
        }
    }
    if cv.tool == "ocr" {
        // with the text tool the main action copies text, not the picture
        let tip = format!(
            "{}  [Enter / Ctrl+C]",
            tr("Copy the selected text (or all of it) as text")
        );
        if primary_button(ui, "ocr", &tr("Copy text"), &tip).clicked() {
            cv.ocr_copy();
        }
    } else if primary_button(
        ui,
        "copy",
        &tr("Copy"),
        &format!("{}  [Enter / Ctrl+C]", tr("Copy to clipboard")),
    )
    .clicked()
    {
        cv.request(Action::Copy);
    }
    if !cv.editor {
        sep(ui, false);
        if icon_button(ui, "close", &format!("{}  [Esc]", tr("Cancel")), false, true).clicked() {
            cv.request(Action::Cancel);
        }
    }
}

fn color_button(cv: &mut Canvas, ui: &mut egui::Ui, st: &Obj, key: &str, tip: &str, allow_none: bool) {
    let c = st.color(key);
    let resp = colorpicker::swatch_button(ui, c, vec2(28.0, 28.0), 6.0).on_hover_text(tr(tip));
    if resp.clicked() {
        let same = cv.popup.as_ref().is_some_and(|p| p.key == key);
        cv.popup = if same {
            None
        } else {
            Some(ColorPopup::new(key, c, allow_none, resp.rect))
        };
    }
}

fn slider(
    cv: &mut Canvas,
    ui: &mut egui::Ui,
    cfg: &mut Config,
    st: &Obj,
    key: &str,
    lo: i64,
    hi: i64,
    tip: &str,
    width: f32,
) {
    if !tip.is_empty() {
        label(ui, tip);
    }
    let mut v = (st.f(key, lo as f32).round() as i64).clamp(lo, hi);
    ui.spacing_mut().slider_width = width;
    let r = ui.add(egui::Slider::new(&mut v, lo..=hi).show_value(false));
    ui.add_sized(
        vec2(30.0, 20.0),
        egui::Label::new(egui::RichText::new(v.to_string()).size(11.5).color(TEXT_DIM)),
    );
    if r.changed() {
        cv.apply_style(cfg, key, json!(v));
    }
}

fn toggle(cv: &mut Canvas, ui: &mut egui::Ui, cfg: &mut Config, st: &Obj, key: &str, icon: &str, tip: &str) {
    let on = st.b(key);
    if icon_button(ui, icon, &tr(tip), on, true).clicked() {
        cv.apply_style(cfg, key, json!(!on));
    }
}

fn choice(cv: &mut Canvas, ui: &mut egui::Ui, cfg: &mut Config, st: &Obj, key: &str, opts: &[(&str, &str, &str)]) {
    let cur = st.s(key, "");
    for (value, icon, tip) in opts {
        if icon_button(ui, icon, &tr(tip), cur == *value, true).clicked() {
            cv.apply_style(cfg, key, json!(value));
        }
    }
}

fn props_bar(cv: &mut Canvas, ui: &mut egui::Ui, cfg: &mut Config, kind: &str, mut st: Obj, layer: Option<String>) {
    let title = match layer.as_ref().and_then(|id| cv.doc.layer(id)) {
        Some(l) => cv.doc.display_name(l),
        None => tr(type_name(kind)),
    };
    ui.label(egui::RichText::new(title).size(12.0).strong().color(TEXT));
    sep(ui, false);
    match kind {
        "pen" | "highlighter" => {
            color_button(cv, ui, &st, "color", "Color", false);
            slider(cv, ui, cfg, &st, "width", 1, 80, "Width", 90.0);
        }
        "line" | "arrow" => {
            color_button(cv, ui, &st, "color", "Color", false);
            slider(cv, ui, cfg, &st, "width", 1, 40, "Width", 90.0);
            toggle(cv, ui, cfg, &st, "dash", "dash", "Dashed");
            if kind == "arrow" {
                choice(
                    cv,
                    ui,
                    cfg,
                    &st,
                    "heads",
                    &[
                        ("end", "arrow", "Arrowhead on one end"),
                        ("both", "arrow_both", "Arrowheads on both ends"),
                    ],
                );
            }
        }
        "rect" | "ellipse" => {
            label(ui, "Outline");
            color_button(cv, ui, &st, "stroke", "Outline color", true);
            label(ui, "Fill");
            color_button(cv, ui, &st, "fill", "Fill color", true);
            slider(cv, ui, cfg, &st, "width", 0, 40, "Width", 90.0);
            if kind == "rect" {
                slider(cv, ui, cfg, &st, "radius", 0, 60, "Corner radius", 90.0);
            }
            toggle(cv, ui, cfg, &st, "dash", "dash", "Dashed");
        }
        "text" => {
            color_button(cv, ui, &st, "color", "Text color", false);
            let fam = st.s("family", "Sans Serif").to_string();
            egui::ComboBox::from_id_salt(("font", cv.index))
                .width(150.0)
                .height(360.0)
                .selected_text(egui::RichText::new(&fam).size(12.0))
                .show_ui(ui, |ui| {
                    for f in font_families(ui.ctx()).iter() {
                        if ui.selectable_label(*f == fam, f).clicked() {
                            cv.apply_style(cfg, "family", json!(f));
                        }
                    }
                });
            slider(cv, ui, cfg, &st, "size", 8, 160, "Size", 90.0);
            toggle(cv, ui, cfg, &st, "bold", "bold", "Bold");
            toggle(cv, ui, cfg, &st, "italic", "italic", "Italic");
            label(ui, "Stroke");
            color_button(cv, ui, &st, "outline", "Stroke color", true);
            label(ui, "Background");
            color_button(cv, ui, &st, "bg", "Background color", true);
        }
        "mark" => {
            choice(
                cv,
                ui,
                cfg,
                &st,
                "shape",
                &[
                    ("circle", "circle", "Circle"),
                    ("square", "square", "Square"),
                    ("diamond", "diamond", "Diamond"),
                ],
            );
            slider(cv, ui, cfg, &st, "size", 16, 120, "Size", 90.0);
            label(ui, "Number");
            color_button(cv, ui, &st, "text", "Number color", false);
            label(ui, "Background");
            color_button(cv, ui, &st, "fill", "Background color (and arrow)", true);
            label(ui, "Outline");
            color_button(cv, ui, &st, "outline", "Outline color", true);
            slider(cv, ui, cfg, &st, "outline_w", 0, 10, "Width", 90.0);
            let next = cv.doc.mark_numbers().len() as i64 + cv.doc.mark_start();
            ui.label(
                egui::RichText::new(trf("next #{n}", &[("n", &next)]))
                    .size(11.5)
                    .color(TEXT_DIM),
            );
        }
        "zoom" => {
            shapes::zoom_upgrade(&mut st);
            choice(
                cv,
                ui,
                cfg,
                &st,
                "shape",
                &[
                    ("ellipse", "ellipse", "Ellipse"),
                    ("rect", "rect_sharp", "Rectangle"),
                    ("round", "round", "Rounded rectangle"),
                ],
            );
            if st.s("shape", "") == "round" {
                slider(cv, ui, cfg, &st, "radius", 2, 80, "Radius", 60.0);
            }
            sep(ui, false);
            label(ui, "Area");
            color_button(cv, ui, &st, "src_color", "Selected area frame color", false);
            slider(cv, ui, cfg, &st, "src_width", 0, 12, "", 60.0);
            toggle(cv, ui, cfg, &st, "src_dash", "dash", "Dashed area frame");
            sep(ui, false);
            label(ui, "Zoomed");
            color_button(cv, ui, &st, "stroke", "Copy frame color", false);
            slider(cv, ui, cfg, &st, "width", 0, 12, "", 60.0);
            toggle(cv, ui, cfg, &st, "shadow", "shadow", "Shadow under the copy");
            toggle(
                cv,
                ui,
                cfg,
                &st,
                "smooth",
                "smooth",
                "Smooth magnification (otherwise crisp pixels)",
            );
            sep(ui, false);
            toggle(
                cv,
                ui,
                cfg,
                &st,
                "lines",
                "lines",
                "Leader lines from the area to the copy",
            );
            color_button(cv, ui, &st, "line_color", "Leader line color", false);
            slider(cv, ui, cfg, &st, "line_width", 1, 10, "", 60.0);
            toggle(cv, ui, cfg, &st, "dash", "dash", "Dashed leader lines");
            if let Some(l) = layer.as_ref().and_then(|id| cv.doc.layer(id)) {
                let k = l.g.dst().w / l.g.src().w.max(1e-6);
                ui.label(
                    egui::RichText::new(trf("magnification ×{k}", &[("k", &format!("{k:.1}"))]))
                        .size(11.5)
                        .color(TEXT_DIM),
                );
            }
        }
        "blur" => {
            choice(
                cv,
                ui,
                cfg,
                &st,
                "mode",
                &[("pixelate", "blur", "Pixelate"), ("blur", "drop", "Blur")],
            );
            slider(cv, ui, cfg, &st, "strength", 2, 60, "Strength", 90.0);
        }
        "ocr" => cv.ocr_props(ui, cfg),
        _ => {}
    }
}

fn font_families(ctx: &egui::Context) -> std::sync::Arc<Vec<String>> {
    let id = Id::new("kadr-font-families");
    if let Some(v) = ctx.data(|d| d.get_temp::<std::sync::Arc<Vec<String>>>(id)) {
        return v;
    }
    let v = std::sync::Arc::new(crate::text::families());
    ctx.data_mut(|d| d.insert_temp(id, v.clone()));
    v
}

fn layers_panel(cv: &mut Canvas, ui: &mut egui::Ui) {
    ui.set_width(230.0);
    ui.spacing_mut().item_spacing = vec2(2.0, 2.0);
    ui.horizontal(|ui| {
        ui.label(egui::RichText::new(tr("Layers")).size(12.0).strong().color(TEXT));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let has = !cv.sel_ids().is_empty();
            if icon_button_sized(
                ui,
                "trash",
                &format!("{}  [Del]", tr("Delete selected")),
                false,
                has,
                16.0,
            )
            .clicked()
            {
                cv.delete_selected();
            }
            if icon_button_sized(ui, "down", &format!("{}  [Ctrl+[]", tr("Move down")), false, has, 16.0).clicked() {
                cv.move_selected(-1);
            }
            if icon_button_sized(ui, "up", &format!("{}  [Ctrl+]]", tr("Move up")), false, has, 16.0).clicked() {
                cv.move_selected(1);
            }
            if icon_button_sized(
                ui,
                "duplicate",
                &format!("{}  [Ctrl+D]", tr("Duplicate selected")),
                false,
                has,
                16.0,
            )
            .clicked()
            {
                cv.duplicate_selected();
            }
        });
    });
    ui.add_space(2.0);
    let accent = accent32(ui);
    let rows: Vec<(String, String, String, bool)> = cv
        .doc
        .layers
        .iter()
        .rev()
        .map(|l| (l.id.clone(), l.kind.clone(), cv.doc.display_name(l), l.visible))
        .collect();
    let selected = cv.sel_ids();
    let row_h = 28.0;
    let list_h = ((rows.len() + 1) as f32 * (row_h + 2.0)).clamp(60.0, 440.0);
    let mut row_rects: Vec<(String, Rect)> = Vec::new();
    egui::ScrollArea::vertical()
        .max_height(list_h)
        .auto_shrink([false, true])
        .show(ui, |ui| {
            for (id, kind, name, visible) in &rows {
                let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), row_h), Sense::click_and_drag());
                row_rects.push((id.clone(), rect));
                let is_sel = selected.contains(id);
                if is_sel {
                    ui.painter().rect_filled(
                        rect,
                        6.0,
                        Color32::from_rgba_unmultiplied(accent.r(), accent.g(), accent.b(), 90),
                    );
                } else if resp.hovered() {
                    ui.painter().rect_filled(rect, 6.0, Color32::from_white_alpha(14));
                }
                // visibility box
                let cb = Rect::from_center_size(pos2(rect.left() + 13.0, rect.center().y), vec2(15.0, 15.0));
                ui.painter().rect(
                    cb,
                    3.0,
                    if *visible { accent } else { Color32::TRANSPARENT },
                    Stroke::new(1.0, Color32::from_white_alpha(120)),
                    egui::StrokeKind::Inside,
                );
                if *visible {
                    crate::icons::image(ui.ctx(), "check", "#101014", 12.0)
                        .paint_at(ui, Rect::from_center_size(cb.center(), Vec2::splat(12.0)));
                }
                crate::icons::image(ui.ctx(), kind, "#e6e6ea", 16.0).paint_at(
                    ui,
                    Rect::from_center_size(pos2(rect.left() + 36.0, rect.center().y), Vec2::splat(16.0)),
                );
                let text_r = Rect::from_min_max(pos2(rect.left() + 50.0, rect.top()), rect.max);
                let renaming = cv.rename.as_ref().is_some_and(|(rid, _)| rid == id);
                if renaming {
                    let (_, buf) = cv.rename.as_mut().unwrap();
                    let te = ui.put(
                        text_r.shrink2(vec2(0.0, 3.0)),
                        egui::TextEdit::singleline(buf).font(egui::FontId::proportional(12.5)),
                    );
                    te.request_focus();
                    if te.lost_focus() {
                        let (rid, buf) = cv.rename.take().unwrap();
                        if ui.input(|i| !i.key_pressed(egui::Key::Escape)) && !buf.trim().is_empty() {
                            cv.set_layer_prop("name", json!(buf.trim()), Some(rid));
                        }
                    }
                } else {
                    ui.painter().text(
                        pos2(text_r.left(), rect.center().y),
                        egui::Align2::LEFT_CENTER,
                        name,
                        egui::FontId::proportional(12.5),
                        if is_sel { Color32::WHITE } else { TEXT },
                    );
                }
                let resp = resp.on_hover_text(format!(
                    "{}\n{}",
                    tr("Checkbox — visibility, double click — rename, drag — order"),
                    tr("Ctrl/Shift+click — select several")
                ));
                if resp.clicked() {
                    let q = resp.interact_pointer_pos().unwrap_or(rect.center());
                    if cb.expand(3.0).contains(q) {
                        cv.set_layer_prop("visible", json!(!visible), Some(id.clone()));
                    } else {
                        let mods = ui.input(|i| i.modifiers);
                        if mods.ctrl {
                            let mut ids = cv.sel_ids();
                            if !ids.remove(id) {
                                ids.insert(id.clone());
                            }
                            let cur = ids.contains(id).then(|| id.clone());
                            cv.select_many(ids, cur);
                        } else if mods.shift
                            && let Some(anchor) = cv.selected.clone()
                        {
                            let a = rows.iter().position(|r| r.0 == anchor);
                            let b = rows.iter().position(|r| &r.0 == id);
                            if let (Some(a), Some(b)) = (a, b) {
                                let ids = rows[a.min(b)..=a.max(b)].iter().map(|r| r.0.clone()).collect();
                                cv.select_many(ids, Some(anchor));
                            }
                        } else {
                            cv.select(Some(id.clone()), false);
                        }
                    }
                }
                if resp.double_clicked() && !cb.expand(3.0).contains(resp.interact_pointer_pos().unwrap_or_default()) {
                    cv.rename = Some((id.clone(), name.clone()));
                }
                if resp.drag_started() {
                    cv.layer_drag = Some(id.clone());
                }
            }
            // the screenshot itself, always at the bottom
            let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), row_h), Sense::hover());
            crate::icons::image(ui.ctx(), "rect", "#6a6a72", 16.0).paint_at(
                ui,
                Rect::from_center_size(pos2(rect.left() + 36.0, rect.center().y), Vec2::splat(16.0)),
            );
            ui.painter().text(
                pos2(rect.left() + 50.0, rect.center().y),
                egui::Align2::LEFT_CENTER,
                tr("Screenshot (background)"),
                egui::FontId::proportional(12.5),
                DISABLED,
            );
        });
    // drag to reorder: show where the row lands, apply on release
    if let Some(drag_id) = cv.layer_drag.clone() {
        let pointer = ui.input(|i| i.pointer.interact_pos());
        let released = ui.input(|i| i.pointer.any_released());
        if let Some(q) = pointer
            && !row_rects.is_empty()
        {
            let mut slot = row_rects.len();
            for (i, (_, r)) in row_rects.iter().enumerate() {
                if q.y < r.center().y {
                    slot = i;
                    break;
                }
            }
            let y = if slot < row_rects.len() {
                row_rects[slot].1.top() - 1.0
            } else {
                row_rects.last().unwrap().1.bottom() + 1.0
            };
            let xr = row_rects[0].1.x_range();
            ui.painter().hline(xr, y, Stroke::new(2.0, accent));
            if released {
                let mut order: Vec<String> = row_rects.iter().map(|(id, _)| id.clone()).collect(); // top -> bottom
                let from = order.iter().position(|i| *i == drag_id);
                if let Some(from) = from {
                    let item = order.remove(from);
                    let to = if slot > from { slot - 1 } else { slot };
                    order.insert(to.min(order.len()), item);
                    order.reverse();
                    cv.reorder(&order);
                }
                cv.layer_drag = None;
            }
        } else if released {
            cv.layer_drag = None;
        }
    }
    ui.add_space(4.0);
    let sel = cv.selected_layer().map(|l| (l.blend.clone(), l.opacity));
    ui.add_enabled_ui(sel.is_some(), |ui| {
        ui.horizontal(|ui| {
            label(ui, "Blending");
            let cur = sel.as_ref().map_or("normal".to_string(), |s| s.0.clone());
            let cur_name = BLEND_MODES.iter().find(|(k, _)| *k == cur).map_or("Normal", |(_, n)| n);
            egui::ComboBox::from_id_salt(("blend", cv.index))
                .width(150.0)
                .selected_text(tr(cur_name))
                .show_ui(ui, |ui| {
                    for (k, n) in BLEND_MODES {
                        if ui.selectable_label(*k == cur, tr(n)).clicked() {
                            cv.set_layer_prop("blend", json!(k), None);
                        }
                    }
                });
        });
        ui.horizontal(|ui| {
            label(ui, "Opacity");
            let mut v = sel.as_ref().map_or(100, |s| (s.1 * 100.0).round() as i64);
            ui.spacing_mut().slider_width = 110.0;
            if ui.add(egui::Slider::new(&mut v, 0..=100).show_value(false)).changed() {
                cv.set_layer_prop("opacity", json!(v as f64 / 100.0), None);
            }
            ui.label(egui::RichText::new(format!("{v}%")).size(11.5).color(TEXT_DIM));
        });
    });
    let _ = Color::rgb(0, 0, 0);
}
