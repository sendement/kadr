//! Per-layer-type geometry: painting, bounds, hit tests, handles, compositing.
use std::collections::HashMap;

use tiny_skia::{
    BlendMode, FillRule, FilterQuality, LineCap, LineJoin, Paint, Path, PathBuilder, Pattern, Pixmap, PixmapPaint,
    Rect, SpreadMode, Stroke, StrokeDash, Transform,
};

use crate::color::Color;
use crate::config::Obj;
use crate::geom::{IR, P, R, p, r};
use crate::model::{Document, Geom, Layer, Style};
use crate::text::Font;

const RED: Color = Color::rgb(0xff, 0x3b, 0x30);

// --- path helpers ----------------------------------------------------------

pub fn solid(c: Color) -> Paint<'static> {
    let mut paint = Paint::default();
    paint.set_color(c.skia());
    paint.anti_alias = true;
    paint
}

pub fn make_stroke(width: f32, dash: bool, cap: LineCap) -> Stroke {
    Stroke {
        width: width.max(0.01),
        line_cap: cap,
        line_join: LineJoin::Round,
        miter_limit: 4.0,
        dash: if dash {
            StrokeDash::new(vec![2.2 * width.max(0.5), 2.0 * width.max(0.5)], 0.0)
        } else {
            None
        },
    }
}

fn rect(rr: R) -> Option<Rect> {
    Rect::from_xywh(rr.x, rr.y, rr.w.max(0.001), rr.h.max(0.001))
}

pub fn rounded_rect_path(rr: R, rad: f32) -> Option<Path> {
    let rr = rr.norm();
    let rad = rad.min(rr.w / 2.0).min(rr.h / 2.0).max(0.0);
    if rad < 0.01 {
        return Some(PathBuilder::from_rect(rect(rr)?));
    }
    let k = 0.552_284_8 * rad;
    let (l, t, rt, b) = (rr.left(), rr.top(), rr.right(), rr.bottom());
    let mut pb = PathBuilder::new();
    pb.move_to(l + rad, t);
    pb.line_to(rt - rad, t);
    pb.cubic_to(rt - rad + k, t, rt, t + rad - k, rt, t + rad);
    pb.line_to(rt, b - rad);
    pb.cubic_to(rt, b - rad + k, rt - rad + k, b, rt - rad, b);
    pb.line_to(l + rad, b);
    pb.cubic_to(l + rad - k, b, l, b - rad + k, l, b - rad);
    pb.line_to(l, t + rad);
    pb.cubic_to(l, t + rad - k, l + rad - k, t, l + rad, t);
    pb.close();
    pb.finish()
}

pub fn ellipse_path(rr: R) -> Option<Path> {
    PathBuilder::from_oval(rect(rr.norm())?)
}

fn poly_path(pts: &[P], close: bool) -> Option<Path> {
    let mut pb = PathBuilder::new();
    let first = pts.first()?;
    pb.move_to(first.x, first.y);
    for q in &pts[1..] {
        pb.line_to(q.x, q.y);
    }
    if close {
        pb.close();
    }
    pb.finish()
}

fn line_path(a: P, b: P) -> Option<Path> {
    let mut pb = PathBuilder::new();
    pb.move_to(a.x, a.y);
    pb.line_to(b.x, b.y);
    pb.finish()
}

/// Smoothed freehand stroke (quadratic segments through the midpoints).
pub fn stroke_path(pts: &[[f32; 2]]) -> Option<Path> {
    let mut pb = PathBuilder::new();
    let first = pts.first()?;
    pb.move_to(first[0], first[1]);
    if pts.len() == 1 {
        pb.line_to(first[0] + 0.01, first[1]);
        return pb.finish();
    }
    for i in 1..pts.len() - 1 {
        let (a, b) = (pts[i], pts[i + 1]);
        pb.quad_to(a[0], a[1], (a[0] + b[0]) / 2.0, (a[1] + b[1]) / 2.0);
    }
    let last = pts[pts.len() - 1];
    pb.line_to(last[0], last[1]);
    pb.finish()
}

pub fn head_len(width: f32) -> f32 {
    (width * 3.6).max(12.0)
}

/// Slightly notched arrow head polygon pointing along `d` (unit vector).
pub fn arrow_head(tip: P, d: P, hl: f32) -> [P; 4] {
    let n = p(-d.y, d.x);
    let base = tip - d * hl;
    [tip, base + n * (hl * 0.5), tip - d * (hl * 0.72), base - n * (hl * 0.5)]
}

fn fill(pm: &mut Pixmap, path: &Path, paint: &Paint, ts: Transform) {
    pm.fill_path(path, paint, FillRule::Winding, ts, None);
}

fn stroke(pm: &mut Pixmap, path: &Path, paint: &Paint, st: &Stroke, ts: Transform) {
    pm.stroke_path(path, paint, st, ts, None);
}

pub fn draw_arrow(pm: &mut Pixmap, ts: Transform, a: P, b: P, color: Color, width: f32, dash: bool, both: bool) {
    let v = b - a;
    let len = v.len();
    if len < 0.5 {
        return;
    }
    let u = v / len;
    let hl = head_len(width).min(len * if both { 0.45 } else { 0.8 });
    let start = if both { a + u * (hl * 0.6) } else { a };
    let end = b - u * (hl * 0.6);
    let paint = solid(color);
    if let Some(path) = line_path(start, end) {
        stroke(pm, &path, &paint, &make_stroke(width, dash, LineCap::Round), ts);
    }
    let mut heads = vec![arrow_head(b, u, hl)];
    if both {
        heads.push(arrow_head(a, -u, hl));
    }
    for h in heads {
        if let Some(path) = poly_path(&h, true) {
            fill(pm, &path, &paint, ts);
            stroke(pm, &path, &paint, &make_stroke(1.0, false, LineCap::Round), ts);
        }
    }
}

// --- text ------------------------------------------------------------------

pub struct TextLayout {
    pub font: Option<Font>,
    pub lines: Vec<String>,
    pub ascent: f32,
    pub line_h: f32,
    pub pad: f32,
    pub rect: R,
    pub last_w: f32,
}

pub fn text_layout(layer: &Layer) -> TextLayout {
    let st = &layer.style;
    let size = st.f("size", 24.0);
    let font = Font::new(
        st.s("family", "Sans Serif"),
        size.max(4.0).floor(),
        st.b("bold"),
        st.b("italic"),
    );
    let lines: Vec<String> = layer.g.text().split('\n').map(String::from).collect();
    let (ascent, line_h) = font.as_ref().map_or((size * 0.8, size), |f| {
        let m = f.metrics();
        (m.ascent, m.height)
    });
    let widths: Vec<f32> = lines
        .iter()
        .map(|l| font.as_ref().map_or(0.0, |f| f.advance(l)))
        .collect();
    let w = widths.iter().copied().fold(size * 0.4, f32::max);
    let pad = if st.color("bg").is_some() { size * 0.3 } else { 0.0 };
    let pos = layer.g.pos();
    let rect = r(pos.x, pos.y, w + 2.0 * pad, line_h * lines.len() as f32 + 2.0 * pad);
    TextLayout {
        font,
        lines,
        ascent,
        line_h,
        pad,
        rect,
        last_w: *widths.last().unwrap_or(&0.0),
    }
}

pub fn text_caret(layer: &Layer) -> (P, P) {
    let tl = text_layout(layer);
    let x = tl.rect.x + tl.pad + tl.last_w;
    let y = tl.rect.y + tl.pad + tl.line_h * (tl.lines.len() as f32 - 1.0);
    (p(x + 1.0, y), p(x + 1.0, y + tl.line_h))
}

// --- marks -----------------------------------------------------------------

pub fn mark_poly(st: &Obj, c: P) -> Vec<P> {
    let s = st.f("size", 36.0);
    match st.s("shape", "circle") {
        "diamond" => {
            let rr = s * 0.62;
            vec![p(c.x, c.y - rr), p(c.x + rr, c.y), p(c.x, c.y + rr), p(c.x - rr, c.y)]
        }
        "square" => rounded_poly(r(c.x - s / 2.0, c.y - s / 2.0, s, s), s * 0.18),
        _ => ellipse_poly(r(c.x - s / 2.0, c.y - s / 2.0, s, s)),
    }
}

pub fn mark_shape(st: &Obj, c: P) -> Option<Path> {
    let s = st.f("size", 36.0);
    match st.s("shape", "circle") {
        "square" => rounded_rect_path(r(c.x - s / 2.0, c.y - s / 2.0, s, s), s * 0.18),
        "diamond" => poly_path(&mark_poly(st, c), true),
        _ => ellipse_path(r(c.x - s / 2.0, c.y - s / 2.0, s, s)),
    }
}

pub fn mark_has_arrow(layer: &Layer) -> bool {
    layer.g.anchor().dist(layer.g.pos()) > layer.style.f("size", 36.0) * 0.55
}

pub fn mark_arrow_color(st: &Obj) -> Color {
    st.color("fill").or_else(|| st.color("outline")).unwrap_or(RED)
}

fn paint_mark(pm: &mut Pixmap, ts: Transform, layer: &Layer, n: Option<i64>) {
    let st = &layer.style;
    let s = st.f("size", 36.0);
    let (c, a) = (layer.g.pos(), layer.g.anchor());
    if mark_has_arrow(layer) {
        let u = (a - c) / c.dist(a);
        draw_arrow(
            pm,
            ts,
            c + u * (s * 0.45),
            a,
            mark_arrow_color(st),
            (s / 9.0).max(2.0),
            false,
            false,
        );
    }
    let Some(path) = mark_shape(st, c) else { return };
    fill(
        pm,
        &path,
        &solid(Color::rgba(0, 0, 0, 70)),
        ts.pre_translate(s * 0.04, s * 0.07),
    );
    if let Some(f) = st.color("fill") {
        fill(pm, &path, &solid(f), ts);
    }
    let ow = st.f("outline_w", 2.0);
    if let Some(o) = st.color("outline")
        && ow > 0.0
    {
        stroke(pm, &path, &solid(o), &make_stroke(ow, false, LineCap::Square), ts);
    }
    let text = n.map_or("?".to_string(), |n| n.to_string());
    let k = match text.chars().count() {
        1 => 0.52,
        2 => 0.44,
        _ => 0.34,
    };
    if let Some(font) = Font::new("Sans Serif", (s * k).floor().max(4.0), true, false) {
        let m = font.metrics();
        let w = font.advance(&text);
        let base = c.y - m.height / 2.0 + m.ascent;
        if let Some(path) = font.path(&text, c.x - w / 2.0, base) {
            fill(
                pm,
                &path,
                &solid(st.color("text").unwrap_or(Color::rgb(255, 255, 255))),
                ts,
            );
        }
    }
}

// --- painting --------------------------------------------------------------

pub fn paint(pm: &mut Pixmap, ts: Transform, layer: &Layer, mark_no: Option<i64>) {
    let (g, st) = (&layer.g, &layer.style);
    match layer.kind.as_str() {
        "pen" | "highlighter" => {
            let cap = if layer.kind == "highlighter" {
                LineCap::Square
            } else {
                LineCap::Round
            };
            if let Some(path) = stroke_path(g.pts()) {
                let c = st.color("color").unwrap_or(RED);
                stroke(pm, &path, &solid(c), &make_stroke(st.f("width", 4.0), false, cap), ts);
            }
        }
        "line" => {
            if let Some(path) = line_path(g.p1(), g.p2()) {
                let c = st.color("color").unwrap_or(RED);
                stroke(
                    pm,
                    &path,
                    &solid(c),
                    &make_stroke(st.f("width", 4.0), st.b("dash"), LineCap::Round),
                    ts,
                );
            }
        }
        "arrow" => draw_arrow(
            pm,
            ts,
            g.p1(),
            g.p2(),
            st.color("color").unwrap_or(RED),
            st.f("width", 5.0),
            st.b("dash"),
            st.s("heads", "end") == "both",
        ),
        "rect" | "ellipse" => {
            let rr = g.r();
            let path = if layer.kind == "rect" {
                rounded_rect_path(rr, st.f("radius", 0.0))
            } else {
                ellipse_path(rr)
            };
            let Some(path) = path else { return };
            if let Some(f) = st.color("fill") {
                fill(pm, &path, &solid(f), ts);
            }
            let w = st.f("width", 4.0);
            if let Some(c) = st.color("stroke")
                && w > 0.0
            {
                stroke(pm, &path, &solid(c), &make_stroke(w, st.b("dash"), LineCap::Round), ts);
            }
        }
        "text" => {
            let tl = text_layout(layer);
            if let Some(bg) = st.color("bg")
                && let Some(path) = rounded_rect_path(tl.rect, tl.pad * 0.9)
            {
                fill(pm, &path, &solid(bg), ts);
            }
            let Some(font) = &tl.font else { return };
            let mut pb = PathBuilder::new();
            for (i, line) in tl.lines.iter().enumerate() {
                font.shape(
                    line,
                    tl.rect.x + tl.pad,
                    tl.rect.y + tl.pad + tl.ascent + i as f32 * tl.line_h,
                    Some(&mut pb),
                );
            }
            let Some(path) = pb.finish() else { return };
            if let Some(o) = st.color("outline") {
                let w = (st.f("size", 24.0) / 7.0).max(2.0);
                stroke(pm, &path, &solid(o), &make_stroke(w, false, LineCap::Round), ts);
            }
            fill(pm, &path, &solid(st.color("color").unwrap_or(RED)), ts);
        }
        "mark" => paint_mark(pm, ts, layer, mark_no),
        _ => {}
    }
}

// --- polygons for hit tests and magnifier connectors -------------------------

pub fn ellipse_poly(rr: R) -> Vec<P> {
    let c = rr.center();
    let n = 72;
    (0..n)
        .map(|i| {
            let a = i as f32 / n as f32 * std::f32::consts::TAU;
            p(c.x + rr.w / 2.0 * a.cos(), c.y + rr.h / 2.0 * a.sin())
        })
        .collect()
}

pub fn rounded_poly(rr: R, rad: f32) -> Vec<P> {
    let rad = rad.min(rr.w / 2.0).min(rr.h / 2.0).max(0.0);
    if rad < 0.01 {
        return vec![rr.tl(), p(rr.right(), rr.top()), rr.br(), p(rr.left(), rr.bottom())];
    }
    let mut v = Vec::new();
    let corners = [
        (p(rr.right() - rad, rr.top() + rad), -90.0f32),
        (p(rr.right() - rad, rr.bottom() - rad), 0.0),
        (p(rr.left() + rad, rr.bottom() - rad), 90.0),
        (p(rr.left() + rad, rr.top() + rad), 180.0),
    ];
    for (c, start) in corners {
        for i in 0..=8 {
            let a = (start + i as f32 * 90.0 / 8.0).to_radians();
            v.push(p(c.x + rad * a.cos(), c.y + rad * a.sin()));
        }
    }
    v
}

pub fn poly_contains(poly: &[P], q: P) -> bool {
    let mut inside = false;
    let mut j = poly.len().wrapping_sub(1);
    for i in 0..poly.len() {
        let (a, b) = (poly[i], poly[j]);
        if (a.y > q.y) != (b.y > q.y) && q.x < (b.x - a.x) * (q.y - a.y) / (b.y - a.y) + a.x {
            inside = !inside;
        }
        j = i;
    }
    inside
}

pub fn seg_dist(q: P, a: P, b: P) -> f32 {
    let ab = b - a;
    let l2 = ab.x * ab.x + ab.y * ab.y;
    if l2 < 1e-9 {
        return q.dist(a);
    }
    let t = (((q - a).x * ab.x + (q - a).y * ab.y) / l2).clamp(0.0, 1.0);
    q.dist(a + ab * t)
}

pub fn polyline_dist(q: P, pts: &[P], closed: bool) -> f32 {
    if pts.len() == 1 {
        return q.dist(pts[0]);
    }
    let mut d = f32::MAX;
    for w in pts.windows(2) {
        d = d.min(seg_dist(q, w[0], w[1]));
    }
    if closed && pts.len() > 2 {
        d = d.min(seg_dist(q, pts[pts.len() - 1], pts[0]));
    }
    d
}

// --- zoom (magnifier callout) ------------------------------------------------

/// Give a magnifier made before per-part styling explicit values matching how it renders.
pub fn zoom_upgrade(st: &mut Obj) {
    use serde_json::json;
    let copy_col = st
        .get("stroke")
        .filter(|v| !v.is_null())
        .cloned()
        .unwrap_or(json!("#ffff3b30"));
    let copy_w = st.f("width", 3.0);
    let defaults = [
        ("src_color", copy_col.clone()),
        ("src_width", json!(copy_w)),
        ("line_color", copy_col),
        ("line_width", json!((copy_w * 0.7).round().max(1.0))),
        ("radius", json!(16)),
        ("src_dash", json!(false)),
        ("shadow", json!(false)),
        ("smooth", json!(false)),
        ("lines", json!(true)),
    ];
    for (k, v) in defaults {
        st.entry(k).or_insert(v);
    }
}

fn zoom_radius(st: &Obj, rr: R, k: f32) -> f32 {
    (st.f("radius", 16.0) * k).min(rr.w / 2.0).min(rr.h / 2.0)
}

pub fn zoom_shape_poly(rr: R, st: &Obj, k: f32) -> Vec<P> {
    match st.s("shape", "ellipse") {
        "rect" => rounded_poly(rr, 0.0),
        "round" => rounded_poly(rr, zoom_radius(st, rr, k)),
        _ => ellipse_poly(rr),
    }
}

pub fn zoom_shape_path(rr: R, st: &Obj, k: f32) -> Option<Path> {
    match st.s("shape", "ellipse") {
        "rect" => rounded_rect_path(rr, 0.0),
        "round" => rounded_rect_path(rr, zoom_radius(st, rr, k)),
        _ => ellipse_path(rr),
    }
}

/// (source polygon, copy polygon); the copy's corner radius scales with the magnification.
pub fn zoom_polys(layer: &Layer) -> (Vec<P>, Vec<P>) {
    let (src, dst) = (layer.g.src(), layer.g.dst());
    let k = dst.w / src.w.max(1e-6);
    (
        zoom_shape_poly(src, &layer.style, 1.0),
        zoom_shape_poly(dst, &layer.style, k),
    )
}

/// Outer tangent lines between source and copy = convex-hull edges joining the two outlines.
pub fn zoom_connectors(spoly: &[P], dpoly: &[P]) -> Vec<(P, P)> {
    let mut pts: Vec<(f32, f32, u8)> = spoly
        .iter()
        .map(|q| (q.x, q.y, 0))
        .chain(dpoly.iter().map(|q| (q.x, q.y, 1)))
        .collect();
    pts.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    pts.dedup_by(|a, b| (a.0 - b.0).abs() < 1e-3 && (a.1 - b.1).abs() < 1e-3);
    if pts.len() < 3 {
        return vec![];
    }
    let cross =
        |o: (f32, f32, u8), a: (f32, f32, u8), b: (f32, f32, u8)| (a.0 - o.0) * (b.1 - o.1) - (a.1 - o.1) * (b.0 - o.0);
    let mut lower: Vec<(f32, f32, u8)> = Vec::new();
    for &q in &pts {
        while lower.len() >= 2 && cross(lower[lower.len() - 2], lower[lower.len() - 1], q) <= 0.0 {
            lower.pop();
        }
        lower.push(q);
    }
    let mut upper: Vec<(f32, f32, u8)> = Vec::new();
    for &q in pts.iter().rev() {
        while upper.len() >= 2 && cross(upper[upper.len() - 2], upper[upper.len() - 1], q) <= 0.0 {
            upper.pop();
        }
        upper.push(q);
    }
    lower.pop();
    upper.pop();
    let hull: Vec<_> = lower.into_iter().chain(upper).collect();
    let mut lines = Vec::new();
    for i in 0..hull.len() {
        let (a, b) = (hull[i], hull[(i + 1) % hull.len()]);
        if a.2 != b.2 && (a.0 - b.0).hypot(a.1 - b.1) > 1.0 {
            lines.push((p(a.0, a.1), p(b.0, b.1)));
        }
    }
    lines
}

fn paint_zoom(img: &mut Pixmap, region: IR, layer: &Layer, base: &Pixmap) {
    let (st, src, dst) = (&layer.style, layer.g.src(), layer.g.dst());
    if src.w < 1.0 || src.h < 1.0 {
        return;
    }
    let s = src.aligned();
    // pixels currently under the layer: base outside the crop, the running composite inside it
    let mut sub = crop_pixmap(base, s);
    let inter = s.intersect(region);
    if !inter.is_empty() {
        let part = crop_pixmap(img, inter.translated(-region.x, -region.y));
        sub.draw_pixmap(
            inter.x - s.x,
            inter.y - s.y,
            part.as_ref(),
            &PixmapPaint::default(),
            Transform::identity(),
            None,
        );
    }
    let op = layer.opacity.clamp(0.0, 1.0);
    let fade = |c: Color| Color {
        a: (c.a as f32 * op).round() as u8,
        ..c
    };
    let copy_col = st.color("stroke").unwrap_or(RED);
    let copy_w = st.f("width", 3.0);
    let src_col = st.color("src_color").unwrap_or(copy_col);
    let src_w = st.f("src_width", copy_w);
    let line_col = st.color("line_color").unwrap_or(copy_col);
    let line_w = st.f("line_width", (copy_w * 0.7).max(1.0));
    let k = dst.w / src.w.max(1e-6);
    let ts = Transform::from_translate(-region.x as f32, -region.y as f32);
    let (spoly, dpoly) = zoom_polys(layer);
    if st.bd("lines", true) && line_w > 0.0 {
        let paint = solid(fade(line_col));
        let sk = make_stroke(line_w, st.b("dash"), LineCap::Round);
        for (a, b) in zoom_connectors(&spoly, &dpoly) {
            if let Some(path) = line_path(a, b) {
                stroke(img, &path, &paint, &sk, ts);
            }
        }
    }
    if src_w > 0.0
        && let Some(path) = zoom_shape_path(src, st, 1.0)
    {
        stroke(
            img,
            &path,
            &solid(fade(src_col)),
            &make_stroke(src_w, st.b("src_dash"), LineCap::Round),
            ts,
        );
    }
    let Some(dpath) = zoom_shape_path(dst, st, k) else {
        return;
    };
    if st.b("shadow") {
        let off = (dst.h * 0.012).max(2.0);
        let sts = ts.pre_translate(0.0, off);
        for i in (1..=7).rev() {
            stroke(
                img,
                &dpath,
                &solid(fade(Color::rgba(0, 0, 0, 14))),
                &make_stroke(i as f32 * 3.0, false, LineCap::Round),
                sts,
            );
        }
        fill(img, &dpath, &solid(fade(Color::rgba(0, 0, 0, 40))), sts);
    }
    let quality = if st.b("smooth") {
        FilterQuality::Bicubic
    } else {
        FilterQuality::Nearest
    };
    let (ox, oy) = (src.x - s.x as f32, src.y - s.y as f32);
    // sub pixel (u, v) -> dst.x + (u - ox) * k
    let pts = Transform::from_row(k, 0.0, 0.0, k, dst.x - ox * k, dst.y - oy * k);
    let paint = Paint {
        shader: Pattern::new(sub.as_ref(), SpreadMode::Pad, quality, op, pts),
        anti_alias: true,
        ..Default::default()
    };
    fill(img, &dpath, &paint, ts);
    if copy_w > 0.0 {
        stroke(
            img,
            &dpath,
            &solid(fade(copy_col)),
            &make_stroke(copy_w, false, LineCap::Round),
            ts,
        );
    }
}

// --- bounds / hit tests ------------------------------------------------------

fn pts_of(g: &Geom) -> Vec<P> {
    g.pts().iter().map(|a| P::from(*a)).collect()
}

fn pts_bounds(pts: &[P]) -> R {
    let mut it = pts.iter();
    let Some(f) = it.next() else { return R::default() };
    let (mut l, mut t, mut rr, mut b) = (f.x, f.y, f.x, f.y);
    for q in it {
        l = l.min(q.x);
        t = t.min(q.y);
        rr = rr.max(q.x);
        b = b.max(q.y);
    }
    R::ltrb(l, t, rr, b)
}

pub fn bounds(layer: &Layer) -> R {
    let (g, st) = (&layer.g, &layer.style);
    match layer.kind.as_str() {
        "pen" | "highlighter" => pts_bounds(&pts_of(g)).grow(st.f("width", 4.0) / 2.0 + 2.0),
        "line" | "arrow" => {
            let w = if layer.kind == "arrow" {
                head_len(st.f("width", 5.0))
            } else {
                st.f("width", 4.0) / 2.0 + 2.0
            };
            R::pts(g.p1(), g.p2()).grow(w)
        }
        "rect" | "ellipse" | "blur" => g.r().norm().grow(st.f("width", 0.0) / 2.0 + 2.0),
        "text" => text_layout(layer).rect.grow(st.f("size", 24.0) / 6.0 + 2.0),
        "zoom" => g.src().union(g.dst()).grow(st.f("width", 3.0) / 2.0 + 2.0),
        "mark" => {
            let s = st.f("size", 36.0) * 0.7 + st.f("outline_w", 2.0);
            let (c, a) = (g.pos(), g.anchor());
            let mut rr = r(c.x - s, c.y - s, 2.0 * s, 2.0 * s);
            if mark_has_arrow(layer) {
                let h = head_len((st.f("size", 36.0) / 9.0).max(2.0));
                rr = rr.union(r(a.x - h, a.y - h, 2.0 * h, 2.0 * h));
            }
            rr
        }
        _ => R::default(),
    }
}

/// Which part of the layer is under `q` ("body", "arrow", "src"), with tolerance `tol`.
pub fn hit(layer: &Layer, q: P, tol: f32) -> Option<&'static str> {
    let (g, st) = (&layer.g, &layer.style);
    let near = |pts: &[P], closed: bool, w: f32| polyline_dist(q, pts, closed) <= w / 2.0 + tol;
    match layer.kind.as_str() {
        "mark" => {
            if q.dist(g.pos()) <= st.f("size", 36.0) * 0.6 + tol {
                return Some("body");
            }
            if mark_has_arrow(layer) && near(&[g.pos(), g.anchor()], false, (st.f("size", 36.0) / 9.0).max(2.0)) {
                return Some("arrow");
            }
            None
        }
        "text" | "blur" => bounds(layer).contains(q).then_some("body"),
        "zoom" => {
            let (spoly, dpoly) = zoom_polys(layer);
            let w = st.f("width", 3.0);
            for (part, poly) in [("body", &dpoly), ("src", &spoly)] {
                if poly_contains(poly, q) || near(poly, true, w) {
                    return Some(part);
                }
            }
            None
        }
        "rect" | "ellipse" => {
            let rr = g.r().norm();
            let poly = if layer.kind == "rect" {
                rounded_poly(rr, 0.0)
            } else {
                ellipse_poly(rr)
            };
            if st.color("fill").is_some() && poly_contains(&poly, q) {
                return Some("body");
            }
            near(&poly, true, st.f("width", 2.0)).then_some("body")
        }
        "line" | "arrow" => near(&[g.p1(), g.p2()], false, st.f("width", 2.0)).then_some("body"),
        "pen" | "highlighter" => near(&pts_of(g), false, st.f("width", 2.0)).then_some("body"),
        _ => None,
    }
}

// --- editing -----------------------------------------------------------------

pub fn translate(layer: &mut Layer, dx: f32, dy: f32, whole: bool, part: Option<&str>) {
    let g = &mut layer.g;
    let mv = |a: &mut [f32; 2]| {
        a[0] += dx;
        a[1] += dy;
    };
    let mvr = |a: &mut [f32; 4]| {
        a[0] += dx;
        a[1] += dy;
    };
    match layer.kind.as_str() {
        "zoom" => {
            let (s, d) = if whole && part.is_none() {
                (true, true)
            } else {
                (part == Some("src"), part != Some("src"))
            };
            if s && let Some(v) = g.src.as_mut() {
                mvr(v);
            }
            if d && let Some(v) = g.dst.as_mut() {
                mvr(v);
            }
        }
        "pen" | "highlighter" => {
            if let Some(pts) = g.pts.as_mut() {
                pts.iter_mut().for_each(mv);
            }
        }
        "line" | "arrow" => {
            g.p1.as_mut().map(mv);
            g.p2.as_mut().map(mv);
        }
        "rect" | "ellipse" | "blur" => {
            g.r.as_mut().map(mvr);
        }
        "text" => {
            g.pos.as_mut().map(mv);
        }
        "mark" => {
            if g.anchor.is_none() {
                g.anchor = g.pos;
            }
            g.pos.as_mut().map(mv);
            if whole {
                g.anchor.as_mut().map(mv);
            }
        }
        _ => {}
    }
}

pub fn rect_handle_points(rr: R) -> Vec<(&'static str, P)> {
    let c = rr.center();
    vec![
        ("tl", rr.tl()),
        ("t", p(c.x, rr.top())),
        ("tr", p(rr.right(), rr.top())),
        ("r", p(rr.right(), c.y)),
        ("br", rr.br()),
        ("b", p(c.x, rr.bottom())),
        ("bl", p(rr.left(), rr.bottom())),
        ("l", p(rr.left(), c.y)),
    ]
}

fn handle_point(rr: R, key: &str) -> P {
    rect_handle_points(rr)
        .into_iter()
        .find(|(k, _)| *k == key)
        .map(|(_, q)| q)
        .unwrap_or_default()
}

pub fn handles(layer: &Layer) -> Vec<(String, P)> {
    let g = &layer.g;
    match layer.kind.as_str() {
        "line" | "arrow" => vec![("p1".into(), g.p1()), ("p2".into(), g.p2())],
        "rect" | "ellipse" | "blur" => rect_handle_points(g.r())
            .into_iter()
            .map(|(k, q)| (k.to_string(), q))
            .collect(),
        "mark" if mark_has_arrow(layer) => vec![("anchor".into(), g.anchor())],
        "zoom" => {
            let mut out = Vec::new();
            for (prefix, rr) in [("d_", g.dst()), ("s_", g.src())] {
                // copy first: it wins where they overlap
                for (k, q) in rect_handle_points(rr) {
                    if k.len() == 2 {
                        out.push((format!("{prefix}{k}"), q));
                    }
                }
            }
            out
        }
        _ => vec![],
    }
}

pub fn resize_rect(rr: R, key: &str, q: P, square: bool) -> R {
    let (mut l, mut t, mut rt, mut b) = (rr.left(), rr.top(), rr.right(), rr.bottom());
    if key.contains('l') {
        l = q.x;
    }
    if key.contains('r') {
        rt = q.x;
    }
    if key.contains('t') {
        t = q.y;
    }
    if key.contains('b') {
        b = q.y;
    }
    let mut out = R::ltrb(l, t, rt, b);
    if square && key.len() == 2 {
        let s = out.w.abs().max(out.h.abs());
        out.w = s.copysign(if out.w == 0.0 { 1.0 } else { out.w });
        out.h = s.copysign(if out.h == 0.0 { 1.0 } else { out.h });
    }
    out.norm()
}

pub fn snap_angle(origin: P, q: P) -> P {
    let v = q - origin;
    let step = std::f32::consts::PI / 12.0;
    let ang = (v.y.atan2(v.x) / step).round() * step;
    origin + p(ang.cos(), ang.sin()) * v.len()
}

pub fn drag_handle(layer: &mut Layer, key: &str, q: P, orig: &Geom, shift: bool) {
    let kind = layer.kind.clone();
    let g = &mut layer.g;
    match kind.as_str() {
        "line" | "arrow" => {
            let other = if key == "p1" { orig.p2() } else { orig.p1() };
            let q = if shift { snap_angle(other, q) } else { q };
            if key == "p1" {
                g.p1 = Some(q.arr());
            } else {
                g.p2 = Some(q.arr());
            }
        }
        "rect" | "ellipse" | "blur" => g.r = Some(resize_rect(orig.r(), key, q, shift).arr()),
        "mark" if key == "anchor" => g.anchor = Some(q.arr()),
        "zoom" => {
            let (src, dst) = (orig.src(), orig.dst());
            let corner = &key[2..];
            if key.starts_with("d_") {
                // uniform scale of the copy around the opposite corner
                let opp_key = match corner {
                    "tl" => "br",
                    "tr" => "bl",
                    "br" => "tl",
                    _ => "tr",
                };
                let opp = handle_point(dst, opp_key);
                let aspect = src.h / src.w.max(1e-6);
                let w = (q.x - opp.x).abs().max((q.y - opp.y).abs() / aspect).max(4.0);
                let h = w * aspect;
                let x = if q.x < opp.x { opp.x - w } else { opp.x };
                let y = if q.y < opp.y { opp.y - h } else { opp.y };
                g.dst = Some([x, y, w, h]);
            } else {
                // new source area; the copy keeps its width and follows the new aspect
                let rr = resize_rect(src, corner, q, shift);
                if rr.w >= 2.0 && rr.h >= 2.0 {
                    g.src = Some(rr.arr());
                    g.dst = Some([dst.x, dst.y, dst.w, dst.w * rr.h / rr.w]);
                }
            }
        }
        _ => {}
    }
}

// --- compositing -------------------------------------------------------------

/// Copy `region` out of `src` (black where it falls outside).
pub fn crop_pixmap(src: &Pixmap, region: IR) -> Pixmap {
    let mut out = Pixmap::new(region.w.max(1) as u32, region.h.max(1) as u32).unwrap();
    out.fill(tiny_skia::Color::BLACK);
    let sr = IR::new(0, 0, src.width() as i32, src.height() as i32);
    let inter = region.intersect(sr);
    if inter.is_empty() {
        return out;
    }
    let (sw, ow) = (src.width() as usize * 4, out.width() as usize * 4);
    let sd = src.data();
    let od = out.data_mut();
    let n = inter.w as usize * 4;
    for y in inter.y..inter.bottom() {
        let so = y as usize * sw + inter.x as usize * 4;
        let oo = (y - region.y) as usize * ow + (inter.x - region.x) as usize * 4;
        od[oo..oo + n].copy_from_slice(&sd[so..so + n]);
    }
    out
}

fn box_down(src: &Pixmap, w: u32, h: u32) -> Pixmap {
    let mut out = Pixmap::new(w.max(1), h.max(1)).unwrap();
    let (sw, sh) = (src.width() as usize, src.height() as usize);
    let sd = src.data();
    let ow = out.width() as usize;
    let oh = out.height() as usize;
    let od = out.data_mut();
    for oy in 0..oh {
        let y0 = oy * sh / oh;
        let y1 = ((oy + 1) * sh / oh).max(y0 + 1).min(sh);
        for ox in 0..ow {
            let x0 = ox * sw / ow;
            let x1 = ((ox + 1) * sw / ow).max(x0 + 1).min(sw);
            let mut acc = [0u32; 4];
            for y in y0..y1 {
                for x in x0..x1 {
                    let i = (y * sw + x) * 4;
                    for c in 0..4 {
                        acc[c] += sd[i + c] as u32;
                    }
                }
            }
            let n = ((y1 - y0) * (x1 - x0)) as u32;
            let o = (oy * ow + ox) * 4;
            for c in 0..4 {
                od[o + c] = (acc[c] / n) as u8;
            }
        }
    }
    out
}

fn scaled(src: &Pixmap, w: u32, h: u32, quality: FilterQuality) -> Pixmap {
    let mut out = Pixmap::new(w.max(1), h.max(1)).unwrap();
    let t = Transform::from_scale(w as f32 / src.width() as f32, h as f32 / src.height() as f32);
    let paint = PixmapPaint {
        quality,
        ..Default::default()
    };
    out.draw_pixmap(0, 0, src.as_ref(), &paint, t, None);
    out
}

fn apply_effect(img: &mut Pixmap, rect: IR, st: &Obj, opacity: f32) {
    let rr = rect.intersect(IR::new(0, 0, img.width() as i32, img.height() as i32));
    if rr.w < 2 || rr.h < 2 {
        return;
    }
    let src = crop_pixmap(img, rr);
    let k = (st.f("strength", 12.0) as i32).max(2);
    let small = box_down(&src, (rr.w / k).max(1) as u32, (rr.h / k).max(1) as u32);
    let out = if st.s("mode", "pixelate") == "pixelate" {
        scaled(&small, rr.w as u32, rr.h as u32, FilterQuality::Nearest)
    } else {
        let mid = scaled(&small, small.width() * 2, small.height() * 2, FilterQuality::Bilinear);
        scaled(&mid, rr.w as u32, rr.h as u32, FilterQuality::Bicubic)
    };
    let paint = PixmapPaint {
        opacity,
        ..Default::default()
    };
    img.draw_pixmap(rr.x, rr.y, out.as_ref(), &paint, Transform::identity(), None);
}

pub fn blend_mode(key: &str) -> BlendMode {
    match key {
        "multiply" => BlendMode::Multiply,
        "screen" => BlendMode::Screen,
        "overlay" => BlendMode::Overlay,
        "darken" => BlendMode::Darken,
        "lighten" => BlendMode::Lighten,
        "color_dodge" => BlendMode::ColorDodge,
        "color_burn" => BlendMode::ColorBurn,
        "hard_light" => BlendMode::HardLight,
        "soft_light" => BlendMode::SoftLight,
        "difference" => BlendMode::Difference,
        "exclusion" => BlendMode::Exclusion,
        "plus" => BlendMode::Plus,
        _ => BlendMode::SourceOver,
    }
}

/// Render `layers` over `start` (or the base) inside `region` (image px).
pub fn composite(doc: &Document, region: IR, layers: &[Layer], start: Option<&Pixmap>) -> Pixmap {
    let mut img = match start {
        Some(s) => s.clone(),
        None => crop_pixmap(&doc.base, region),
    };
    let numbers: HashMap<String, i64> = doc.mark_numbers();
    let ts = Transform::from_translate(-region.x as f32, -region.y as f32);
    for layer in layers {
        if !layer.visible || layer.opacity <= 0.0 {
            continue;
        }
        match layer.kind.as_str() {
            "blur" => {
                let rr = layer.g.r().norm().aligned().translated(-region.x, -region.y);
                apply_effect(&mut img, rr, &layer.style, layer.opacity);
                continue;
            }
            "zoom" => {
                paint_zoom(&mut img, region, layer, &doc.base);
                continue;
            }
            _ => {}
        }
        let n = numbers.get(&layer.id).copied();
        if layer.blend == "normal" && layer.opacity >= 1.0 {
            paint(&mut img, ts, layer, n);
            continue;
        }
        // isolate the layer so opacity/blend apply to it as a whole (like Photoshop)
        let br = bounds(layer).aligned().intersect(region);
        if br.is_empty() {
            continue;
        }
        let Some(mut tmp) = Pixmap::new(br.w as u32, br.h as u32) else {
            continue;
        };
        paint(
            &mut tmp,
            Transform::from_translate(-br.x as f32, -br.y as f32),
            layer,
            n,
        );
        let paint = PixmapPaint {
            opacity: layer.opacity.clamp(0.0, 1.0),
            blend_mode: blend_mode(&layer.blend),
            ..Default::default()
        };
        img.draw_pixmap(
            br.x - region.x,
            br.y - region.y,
            tmp.as_ref(),
            &paint,
            Transform::identity(),
            None,
        );
    }
    img
}

pub fn render_result(doc: &Document) -> Pixmap {
    let region = doc.crop.unwrap_or_else(|| doc.base_rect());
    composite(doc, region, &doc.layers, None)
}
