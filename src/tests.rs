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
