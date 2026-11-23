//! On-disk history: every shot keeps the full output capture + editable layers.
use std::path::PathBuf;
use std::sync::Arc;

use serde_json::Value;

use crate::config::history_dir;
use crate::imgio;
use crate::model::Document;

pub fn new_id() -> String {
    format!(
        "{}-{:04x}",
        chrono::Local::now().format("%Y%m%d-%H%M%S"),
        fastrand::u16(..)
    )
}

pub fn now_iso() -> String {
    chrono::Local::now().format("%Y-%m-%dT%H:%M:%S").to_string()
}

pub fn item_dir(id: &str) -> PathBuf {
    history_dir().join(id)
}

pub fn save(
    doc: &mut Document,
    result: &tiny_skia::Pixmap,
    id: Option<&str>,
    limit: usize,
) -> std::io::Result<PathBuf> {
    let id = id
        .map(String::from)
        .or_else(|| doc.meta.get("id").and_then(Value::as_str).map(String::from))
        .unwrap_or_else(new_id);
    let d = item_dir(&id);
    std::fs::create_dir_all(&d)?;
    doc.meta.insert("id".into(), id.clone().into());
    doc.meta.entry("created").or_insert_with(|| now_iso().into());
    doc.meta.insert("modified".into(), now_iso().into());
    doc.meta
        .insert("size".into(), serde_json::json!([result.width(), result.height()]));
    if !d.join("base.png").exists() {
        imgio::save_png(&doc.base, &d.join("base.png"))?;
    }
    imgio::save_png(result, &d.join("result.png"))?;
    imgio::thumb_jpg(result, &d.join("thumb.jpg"));
    let tmp = d.join("doc.json.tmp");
    std::fs::write(&tmp, serde_json::to_string(&doc.to_json()).unwrap_or_default())?;
    std::fs::rename(&tmp, d.join("doc.json"))?;
    doc.dirty = false;
    if limit > 0 {
        prune(limit);
    }
    Ok(d)
}

pub fn load(id: &str) -> Result<Document, String> {
    let d = item_dir(id);
    let base =
        imgio::load(&d.join("base.png")).ok_or_else(|| format!("{}: cannot read", d.join("base.png").display()))?;
    let text = std::fs::read_to_string(d.join("doc.json")).map_err(|e| e.to_string())?;
    let data: Value = serde_json::from_str(&text).map_err(|e| e.to_string())?;
    let mut doc = Document::from_json(Arc::new(base), &data);
    doc.meta.insert("id".into(), id.into());
    Ok(doc)
}

pub struct Item {
    pub id: String,
    pub dir: PathBuf,
    pub meta: serde_json::Map<String, Value>,
    pub layers: usize,
}

pub fn list_items() -> Vec<Item> {
    let Ok(rd) = std::fs::read_dir(history_dir()) else {
        return vec![];
    };
    let mut dirs: Vec<PathBuf> = rd.filter_map(|e| e.ok().map(|e| e.path())).collect();
    dirs.sort();
    dirs.reverse();
    let mut items = Vec::new();
    for d in dirs {
        let Ok(text) = std::fs::read_to_string(d.join("doc.json")) else {
            continue;
        };
        let Ok(data) = serde_json::from_str::<Value>(&text) else {
            continue;
        };
        items.push(Item {
            id: d.file_name().unwrap_or_default().to_string_lossy().into_owned(),
            meta: data.get("meta").and_then(Value::as_object).cloned().unwrap_or_default(),
            layers: data.get("layers").and_then(Value::as_array).map_or(0, Vec::len),
            dir: d,
        });
    }
    items
}

pub fn last_id() -> Option<String> {
    list_items().into_iter().next().map(|i| i.id)
}

pub fn delete(id: &str) {
    if !id.is_empty() && !id.contains('/') && id != ".." {
        let _ = std::fs::remove_dir_all(item_dir(id));
    }
}

pub fn prune(limit: usize) {
    for it in list_items().into_iter().skip(limit) {
        delete(&it.id);
    }
}
