//! ~/.config/kadr/config.json — same keys as the Python version, so settings carry over.
use std::path::PathBuf;

use serde_json::{Map, Value, json};

pub type Obj = Map<String, Value>;

pub fn conf_dir() -> PathBuf {
    dirs::config_dir()
        .unwrap_or_else(|| home().join(".config"))
        .join("kadr")
}

pub fn data_dir() -> PathBuf {
    dirs::data_dir()
        .unwrap_or_else(|| home().join(".local/share"))
        .join("kadr")
}

pub fn history_dir() -> PathBuf {
    data_dir().join("history")
}

pub fn home() -> PathBuf {
    dirs::home_dir().unwrap_or_else(|| PathBuf::from("/"))
}

pub fn tool_defaults() -> Value {
    json!({
        "pen": {"color": "#ffff3b30", "width": 4},
        "highlighter": {"color": "#ffffd60a", "width": 22},
        "line": {"color": "#ffff3b30", "width": 4, "dash": false},
        "arrow": {"color": "#ffff3b30", "width": 5, "dash": false, "heads": "end"},
        "rect": {"stroke": "#ffff3b30", "fill": null, "width": 4, "radius": 6, "dash": false},
        "ellipse": {"stroke": "#ffff3b30", "fill": null, "width": 4, "dash": false},
        "text": {"color": "#ffff3b30", "family": "Sans Serif", "size": 26, "bold": true, "italic": false,
                 "bg": null, "outline": "#ffffffff"},
        "mark": {"shape": "circle", "size": 36, "text": "#ffffffff", "fill": "#ffff3b30",
                 "outline": "#ffffffff", "outline_w": 3},
        "blur": {"mode": "pixelate", "strength": 12},
        "zoom": {"shape": "ellipse", "radius": 16,
                 "src_color": "#ffff3b30", "src_width": 2, "src_dash": false,
                 "stroke": "#ffff3b30", "width": 3, "shadow": true, "smooth": false,
                 "lines": true, "line_color": "#ffff3b30", "line_width": 2, "dash": false},
    })
}

fn defaults() -> Value {
    json!({
        "accent": "#ff4cc2ff",
        "language": "auto",
        "save_dir": "",
        "copy_on_save": true,
        "notify": true,
        "history_limit": 500,
        "recent_colors": [],
        "tool_styles": {},
    })
}

#[derive(Clone)]
pub struct Config {
    pub v: Obj,
}

impl Config {
    pub fn path() -> PathBuf {
        conf_dir().join("config.json")
    }

    pub fn load() -> Config {
        let saved = std::fs::read_to_string(Self::path())
            .ok()
            .and_then(|t| serde_json::from_str::<Value>(&t).ok());
        Self::from_saved(saved)
    }

    #[cfg(test)]
    pub fn default_for_tests() -> Obj {
        Self::from_saved(None).v
    }

    fn from_saved(saved: Option<Value>) -> Config {
        let mut v = match defaults() {
            Value::Object(m) => m,
            _ => unreachable!(),
        };
        if let Some(Value::Object(m)) = saved {
            v.extend(m);
        }
        let mut styles = match tool_defaults() {
            Value::Object(m) => m,
            _ => unreachable!(),
        };
        if let Some(Value::Object(saved)) = v.get("tool_styles") {
            for (k, s) in saved {
                if let (Some(Value::Object(dst)), Value::Object(src)) = (styles.get_mut(k), s) {
                    for (kk, vv) in src {
                        dst.insert(kk.clone(), vv.clone());
                    }
                }
            }
        }
        v.insert("tool_styles".into(), Value::Object(styles));
        Config { v }
    }

    pub fn save(&self) {
        let dir = conf_dir();
        let _ = std::fs::create_dir_all(&dir);
        let tmp = dir.join("config.tmp");
        if let Ok(text) = serde_json::to_string_pretty(&self.v)
            && std::fs::write(&tmp, text).is_ok()
        {
            let _ = std::fs::rename(&tmp, Self::path());
        }
    }

    pub fn str(&self, k: &str) -> String {
        self.v.get(k).and_then(Value::as_str).unwrap_or("").to_string()
    }

    pub fn bool(&self, k: &str, def: bool) -> bool {
        self.v.get(k).and_then(Value::as_bool).unwrap_or(def)
    }

    pub fn int(&self, k: &str, def: i64) -> i64 {
        self.v.get(k).and_then(Value::as_i64).unwrap_or(def)
    }

    pub fn set(&mut self, k: &str, v: impl Into<Value>) {
        self.v.insert(k.into(), v.into());
    }

    pub fn accent(&self) -> crate::color::Color {
        crate::color::Color::parse(&self.str("accent")).unwrap_or(crate::color::Color::rgb(0x4c, 0xc2, 0xff))
    }

    pub fn style(&self, kind: &str) -> Obj {
        self.v
            .get("tool_styles")
            .and_then(|s| s.get(kind))
            .and_then(Value::as_object)
            .cloned()
            .unwrap_or_default()
    }

    pub fn style_mut(&mut self, kind: &str) -> &mut Obj {
        let styles = self.v.entry("tool_styles").or_insert_with(|| Value::Object(Obj::new()));
        if !styles.is_object() {
            *styles = Value::Object(Obj::new());
        }
        let s = styles
            .as_object_mut()
            .unwrap()
            .entry(kind)
            .or_insert_with(|| Value::Object(Obj::new()));
        if !s.is_object() {
            *s = Value::Object(Obj::new());
        }
        s.as_object_mut().unwrap()
    }

    pub fn has_style(&self, kind: &str) -> bool {
        self.v.get("tool_styles").and_then(|s| s.get(kind)).is_some()
    }

    pub fn recent_colors(&self) -> Vec<String> {
        self.v
            .get("recent_colors")
            .and_then(Value::as_array)
            .map(|a| a.iter().filter_map(|c| c.as_str().map(String::from)).collect())
            .unwrap_or_default()
    }

    pub fn add_recent_color(&mut self, c: &str) {
        let mut rc: Vec<String> = self.recent_colors().into_iter().filter(|x| x != c).collect();
        rc.insert(0, c.to_string());
        rc.truncate(10);
        self.set("recent_colors", rc);
    }
}
