//! Interaction tests on the headless harness (see testkit.rs). Shots land in target/shots.
#![cfg(test)]
use egui::{Key, PointerButton};

use crate::testkit::Harness;
use crate::ui::canvas::{Action, Mode};

#[test]
fn overlay_select_and_draw() {
    let mut h = Harness::new(false, (1280, 800));
    h.frame();
    h.move_to(300.0, 300.0);
    h.snapshot("01-idle-hover-window");
    // drag a selection
    h.drag((200.0, 150.0), (900.0, 600.0));
    assert_eq!(h.canvas.mode, Mode::Edit);
    let crop = h.canvas.doc.crop.unwrap();
    assert_eq!((crop.x, crop.y, crop.w, crop.h), (200, 150, 700, 450));
    h.frame();
    h.snapshot("02-selected");
    // rectangle
    h.key(Key::R);
    h.drag((250.0, 200.0), (420.0, 320.0));
    // arrow
    h.key(Key::A);
    h.drag((700.0, 520.0), (480.0, 300.0));
    // numbered marks: plain + pulled out
    h.key(Key::M);
    h.click(620.0, 230.0);
    h.drag((760.0, 250.0), (820.0, 420.0));
    // text
    h.key(Key::T);
    h.click(300.0, 480.0);
    h.type_text("Hello, Кадр 中文");
    h.key(Key::Escape);
    // blur over part of the window
    h.key(Key::B);
    h.drag((260.0, 360.0), (480.0, 440.0));
    // highlighter
    h.key(Key::H);
    h.drag((520.0, 180.0), (860.0, 180.0));
    // magnifier: select source, then drag the copy aside
    h.key(Key::Z);
    h.drag((300.0, 210.0), (360.0, 250.0));
    h.drag((330.0, 230.0), (560.0, 380.0));
    let kinds: Vec<&str> = h.canvas.doc.layers.iter().map(|l| l.kind.as_str()).collect();
    assert_eq!(
        kinds,
        ["rect", "arrow", "mark", "mark", "text", "blur", "highlighter", "zoom"],
        "{kinds:?}"
    );
    let text = h.canvas.doc.layers.iter().find(|l| l.kind == "text").unwrap();
    assert_eq!(text.g.text(), "Hello, Кадр 中文");
    let zoom = h.canvas.doc.layers.iter().find(|l| l.kind == "zoom").unwrap();
    assert!(zoom.g.dst().x > zoom.g.src().x + 100.0, "copy dragged aside");
    h.key(Key::V);
    h.frame();
    h.snapshot("03-annotated");
    // layers panel
    h.key(Key::Tab);
    h.frame();
    h.frame();
    h.snapshot("04-layers-panel");
    // undo removes the zoom, redo brings it back
    // (the last step moved the copy, the one before created the magnifier)
    h.mods = egui::Modifiers::CTRL;
    h.key(Key::Z);
    h.key(Key::Z);
    assert_eq!(h.canvas.doc.layers.len(), 7);
    h.mods = egui::Modifiers::CTRL | egui::Modifiers::SHIFT;
    h.key(Key::Z);
    h.key(Key::Z);
    assert_eq!(h.canvas.doc.layers.len(), 8);
    h.mods = egui::Modifiers::NONE;
    let img = h.canvas.result();
    assert_eq!((img.width(), img.height()), (700, 450));
    let p = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target/shots/05-result.png");
    crate::imgio::save_png(&img, &p).unwrap();
    h.key(Key::Enter);
    assert_eq!(h.canvas.take_actions(), vec![Action::Copy]);
}

#[test]
fn overlay_click_window_and_double_click() {
    let mut h = Harness::new(false, (1280, 800));
    h.frame();
    h.click(300.0, 300.0);
    let c = h.canvas.doc.crop.expect("window picked");
    assert_eq!((c.x, c.y, c.w, c.h), (80, 60, 520, 380));
    // right click with nothing drawn drops the selection
    h.button(700.0, 700.0, PointerButton::Secondary, true);
    h.button(700.0, 700.0, PointerButton::Secondary, false);
    assert_eq!(h.canvas.mode, Mode::Idle);
    // double click = whole screen
    h.move_to(1000.0, 700.0);
    h.button(1000.0, 700.0, PointerButton::Primary, true);
    h.button(1000.0, 700.0, PointerButton::Primary, false);
    h.button(1000.0, 700.0, PointerButton::Primary, true);
    h.button(1000.0, 700.0, PointerButton::Primary, false);
    let c = h.canvas.doc.crop.unwrap();
    assert_eq!((c.w, c.h), (1280, 800));
}

#[test]
fn editor_roundtrip_json() {
    let mut h = Harness::new(true, (1400, 900));
    h.canvas.doc.crop = Some(crate::geom::IR::new(100, 100, 800, 500));
    h.canvas.enter_edit();
    h.frame();
    h.frame();
    h.key(Key::E);
    let a = h.canvas.to_w(crate::geom::p(200.0, 200.0));
    let b = h.canvas.to_w(crate::geom::p(400.0, 350.0));
    h.drag((a.x, a.y), (b.x, b.y));
    assert_eq!(h.canvas.doc.layers.len(), 1);
    h.frame();
    h.snapshot("06-editor");
    let json = h.canvas.doc.to_json();
    let doc2 = crate::model::Document::from_json(h.canvas.doc.base.clone(), &json);
    assert_eq!(doc2.layers, h.canvas.doc.layers);
    assert_eq!(doc2.crop, h.canvas.doc.crop);
}

#[test]
fn python_history_compat() {
    // a doc.json as written by the Python version
    let json: serde_json::Value = serde_json::from_str(r##"{"version": 1, "meta": {"output": "eDP-1", "scale": 1.0, "created": "2026-10-09T12:00:00", "origin": [0, 0], "id": "x"}, "seq": 3,
      "crop": [10, 20, 300, 200], "layers": [
        {"type": "rect", "g": {"r": [20, 30, 100, 60]}, "style": {"stroke": "#ffff3b30", "fill": null, "width": 4, "radius": 6, "dash": false}, "id": "a1", "name": null, "visible": true, "opacity": 1.0, "blend": "normal", "seq": 1, "locked": false},
        {"type": "mark", "g": {"anchor": [50, 50], "pos": [90, 120]}, "style": {"shape": "circle", "size": 36, "text": "#ffffffff", "fill": "#ffff3b30", "outline": "#ffffffff", "outline_w": 3}, "id": "a2", "name": null, "visible": true, "opacity": 0.5, "blend": "multiply", "seq": 2, "locked": false},
        {"type": "zoom", "g": {"src": [30, 40, 20, 20], "dst": [150, 100, 80, 80]}, "style": {"shape": "ellipse", "stroke": "#ffff3b30", "width": 3, "dash": false}, "id": "a3", "name": "Zoom!", "visible": true, "opacity": 1.0, "blend": "normal", "seq": 3, "locked": false}
      ]}"##).unwrap();
    let base = std::sync::Arc::new(crate::testkit::fake_desktop(400, 300));
    let doc = crate::model::Document::from_json(base, &json);
    assert_eq!(doc.layers.len(), 3);
    assert_eq!(doc.mark_numbers().get("a2"), Some(&1));
    assert_eq!(doc.display_name(&doc.layers[2]), "Zoom!");
    let img = crate::shapes::render_result(&doc);
    assert_eq!((img.width(), img.height()), (300, 200));
}

/// `KADR_FIXTURE=1 XDG_DATA_HOME=… cargo test fixture_history -- --ignored`: synthetic history items.
#[test]
#[ignore]
fn fixture_history() {
    assert!(std::env::var("KADR_FIXTURE").is_ok());
    for n in 0..5 {
        let mut h = Harness::new(false, (1280, 800));
        h.frame();
        h.drag((100.0 + n as f32 * 20.0, 80.0), (900.0, 620.0));
        h.key(Key::R);
        h.drag((250.0, 200.0), (420.0, 320.0));
        h.key(Key::M);
        h.click(620.0, 230.0);
        h.drag((760.0, 250.0), (820.0, 420.0));
        h.key(Key::T);
        h.click(300.0, 480.0);
        h.type_text(&format!("Fixture #{n}"));
        h.key(Key::Escape);
        let img = h.canvas.result();
        h.canvas
            .doc
            .meta
            .insert("created".into(), format!("2026-11-2{n}T1{n}:30:00").into());
        h.canvas.doc.meta.insert("output".into(), "TEST-1".into());
        crate::history::save(&mut h.canvas.doc, &img, Some(&format!("2026112{n}-1{n}3000-abc{n}")), 0).unwrap();
    }
}

/// Real recognition on text drawn with system fonts. Needs the models: set KADR_MODELS to a
/// folder with det.onnx, rec-eslav.onnx, dict-eslav.txt, rec-english.onnx, dict-english.txt
/// (skips otherwise).
#[test]
fn ocr_end_to_end() {
    if !crate::ocr::missing("auto").is_empty() {
        eprintln!("ocr models missing, skipping");
        return;
    }
    let pm = ocr_sample();
    let p = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target/shots/ocr-input.png");
    let _ = crate::imgio::save_png(&pm, &p);
    let t = std::time::Instant::now();
    let res = crate::ocr::recognize(&pm, "auto", 1.0).unwrap();
    eprintln!("recognized {} segments in {:?}", res.lines.len(), t.elapsed());
    let (a, b) = crate::ocr::layout::all(&res.lines);
    let text = crate::ocr::layout::format(&res.lines, a, b);
    eprintln!("---\n{text}\n---");
    assert!(text.starts_with("Скриншотер теперь умеет распознавать текст на выделенной области. Текст можно выделить мышью и скопировать, причём переносы строк"), "{text}");
    assert!(text.contains("\n\nSecond paragraph in English: OCR runs locally with a small neural network, so nothing leaves your computer.\n\n"), "{text}");
    assert!(text.ends_with("fn main() {\n    let words = vec![\"кадр\", \"frame\"];\n    for w in &words {\n        println!(\"{}: {}\", w, w.len());\n    }\n}"), "{text}");
}

/// Prose in Russian and English over a light background, Rust code on a dark one.
fn ocr_sample() -> tiny_skia::Pixmap {
    let mut pm = tiny_skia::Pixmap::new(900, 560).unwrap();
    pm.fill(tiny_skia::Color::from_rgba8(250, 250, 252, 255));
    let mut paint = tiny_skia::Paint::default();
    paint.set_color_rgba8(30, 30, 35, 255);
    paint.anti_alias = true;
    let sans = crate::text::Font::new("Sans Serif", 15.0, false, false).unwrap();
    let mono = crate::text::Font::new("monospace", 14.0, false, false).unwrap();
    let prose = [
        "Скриншотер теперь умеет распознавать текст на выделенной",
        "области. Текст можно выделить мышью и скопировать, причём",
        "переносы строк внутри абзаца склеиваются автоматически.",
        "",
        "Second paragraph in English: OCR runs locally with a small",
        "neural network, so nothing leaves your computer.",
    ];
    let mut y = 34.0;
    for l in prose {
        if let Some(p) = sans.path(l, 24.0, y) {
            pm.fill_path(
                &p,
                &paint,
                tiny_skia::FillRule::Winding,
                tiny_skia::Transform::identity(),
                None,
            );
        }
        y += 22.0;
    }
    let mut dark = tiny_skia::Paint::default();
    dark.set_color_rgba8(30, 32, 40, 255);
    pm.fill_rect(
        tiny_skia::Rect::from_ltrb(0.0, y, 900.0, 560.0).unwrap(),
        &dark,
        tiny_skia::Transform::identity(),
        None,
    );
    paint.set_color_rgba8(220, 220, 225, 255);
    y += 30.0;
    let code = [
        "fn main() {",
        "    let words = vec![\"кадр\", \"frame\"];",
        "    for w in &words {",
        "        println!(\"{}: {}\", w, w.len());",
        "    }",
        "}",
    ];
    for l in code {
        if let Some(p) = mono.path(l, 24.0, y) {
            pm.fill_path(
                &p,
                &paint,
                tiny_skia::FillRule::Winding,
                tiny_skia::Transform::identity(),
                None,
            );
        }
        y += 20.0;
    }
    pm
}

#[test]
fn ocr_tool_select_and_copy() {
    if !crate::ocr::missing("auto").is_empty() {
        eprintln!("ocr models missing, skipping");
        return;
    }
    let mut h = Harness::with_base(false, ocr_sample());
    h.frame();
    h.drag((10.0, 10.0), (890.0, 550.0));
    h.key(Key::O);
    assert_eq!(h.canvas.tool, "ocr");
    let t = std::time::Instant::now();
    while (h.canvas.ocr_busy() || h.canvas.ocr_lines().is_empty()) && t.elapsed().as_secs() < 30 {
        std::thread::sleep(std::time::Duration::from_millis(50));
        h.frame();
    }
    assert!(!h.canvas.ocr_lines().is_empty(), "nothing recognized");
    h.frame();
    h.snapshot("07-ocr-recognized");
    // drag from the start of "Second" to the end of the first English line
    let lines = h.canvas.ocr_lines().to_vec();
    let second = lines
        .iter()
        .find(|l| l.chars.iter().map(|c| c.c).collect::<String>().starts_with("Second"))
        .unwrap();
    let neural = lines
        .iter()
        .find(|l| l.chars.iter().map(|c| c.c).collect::<String>().starts_with("neural"))
        .unwrap();
    let from = h
        .canvas
        .to_w(crate::geom::p(second.chars[0].x0 + 1.0, second.rect.center().y));
    let to = h.canvas.to_w(crate::geom::p(
        neural.chars.last().unwrap().x1 + 2.0,
        neural.rect.center().y,
    ));
    h.drag((from.x, from.y), (to.x, to.y));
    h.frame();
    h.snapshot("08-ocr-selection");
    h.mods = egui::Modifiers::CTRL;
    h.key(Key::C);
    h.mods = egui::Modifiers::NONE;
    assert_eq!(h.canvas.take_actions(), vec![Action::CopyText]);
    assert_eq!(
        h.canvas.copied_text.take().as_deref(),
        Some(
            "Second paragraph in English: OCR runs locally with a small neural network, so nothing leaves your computer."
        )
    );
    // double click selects a word; Enter copies it
    let w = lines
        .iter()
        .find(|l| l.chars.iter().map(|c| c.c).collect::<String>().starts_with("for w"))
        .unwrap();
    let q = h
        .canvas
        .to_w(crate::geom::p((w.chars[9].x0 + w.chars[9].x1) / 2.0, w.rect.center().y));
    h.move_to(q.x, q.y);
    h.button(q.x, q.y, PointerButton::Primary, true);
    h.button(q.x, q.y, PointerButton::Primary, false);
    h.button(q.x, q.y, PointerButton::Primary, true);
    h.button(q.x, q.y, PointerButton::Primary, false);
    h.key(Key::Enter);
    assert_eq!(h.canvas.take_actions(), vec![Action::CopyText]);
    assert_eq!(h.canvas.copied_text.take().as_deref(), Some("words"));
}
