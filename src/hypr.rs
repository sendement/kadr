//! Hyprland IPC over its socket (no hyprctl processes): monitors, cursor, clients, window ops.
use std::io::{Read, Write};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::time::Duration;

use serde_json::Value;

use crate::capture::{Output, WindowInfo};

fn socket() -> Option<PathBuf> {
    let sig = std::env::var("HYPRLAND_INSTANCE_SIGNATURE").ok()?;
    let rt = std::env::var("XDG_RUNTIME_DIR").ok()?;
    let p = PathBuf::from(rt).join("hypr").join(sig).join(".socket.sock");
    p.exists().then_some(p)
}

pub fn available() -> bool {
    socket().is_some()
}

pub fn request(cmd: &str) -> Option<String> {
    let mut s = UnixStream::connect(socket()?).ok()?;
    s.set_read_timeout(Some(Duration::from_secs(3))).ok()?;
    s.write_all(cmd.as_bytes()).ok()?;
    let mut out = String::new();
    s.read_to_string(&mut out).ok()?;
    Some(out)
}

pub fn json(what: &str) -> Option<Value> {
    serde_json::from_str(&request(&format!("j/{what}"))?).ok()
}

/// Workspace identity. Hyprland 0.57 dropped "id" for special workspaces: match by name.
fn ws_key(ws: Option<&Value>) -> Option<String> {
    let ws = ws?;
    if let Some(n) = ws.get("name").and_then(Value::as_str)
        && !n.is_empty()
    {
        return Some(n.to_string());
    }
    ws.get("id")
        .and_then(Value::as_i64)
        .filter(|&i| i != 0)
        .map(|i| i.to_string())
}

pub fn add_workspaces(outs: &mut [Output]) {
    let Some(Value::Array(mons)) = json("monitors") else {
        return;
    };
    for m in mons {
        let name = m.get("name").and_then(Value::as_str).unwrap_or("");
        if let Some(o) = outs.iter_mut().find(|o| o.name == name) {
            o.workspaces = [ws_key(m.get("activeWorkspace")), ws_key(m.get("specialWorkspace"))]
                .into_iter()
                .flatten()
                .collect();
        }
    }
}

pub fn cursor_pos() -> Option<(f32, f32)> {
    let c = json("cursorpos")?;
    Some((c.get("x")?.as_f64()? as f32, c.get("y")?.as_f64()? as f32))
}

/// Visible toplevels in hit-test order (topmost first), logical global coords.
pub fn list_windows(outs: &[Output]) -> Vec<WindowInfo> {
    let visible: Vec<&String> = outs.iter().flat_map(|o| o.workspaces.iter()).collect();
    let Some(Value::Array(clients)) = json("clients") else {
        return vec![];
    };
    let mut res = Vec::new();
    for c in clients {
        // unexpected client JSON must not break the capture, only window snapping
        let f = |k: &str| c.get(k).and_then(Value::as_bool).unwrap_or(false);
        if !f("mapped") || f("hidden") {
            continue;
        }
        let Some(ws) = ws_key(c.get("workspace")) else { continue };
        if !visible.iter().any(|v| **v == ws) {
            continue;
        }
        let at = c.get("at").and_then(Value::as_array);
        let size = c.get("size").and_then(Value::as_array);
        let (Some(at), Some(size)) = (at, size) else { continue };
        let n = |a: &Vec<Value>, i: usize| a.get(i).and_then(Value::as_f64).map(|v| v as f32);
        let (Some(x), Some(y), Some(w), Some(h)) = (n(at, 0), n(at, 1), n(size, 0), n(size, 1)) else {
            continue;
        };
        let fh = c.get("focusHistoryID").and_then(Value::as_i64).unwrap_or(99);
        let s = |k: &str| c.get(k).and_then(Value::as_str).unwrap_or("").to_string();
        res.push((
            (!f("fullscreen"), !f("floating"), fh),
            WindowInfo {
                x,
                y,
                w,
                h,
                title: s("title"),
                class: s("class"),
            },
        ));
    }
    res.sort_by_key(|(k, _)| *k);
    res.into_iter().map(|(_, w)| w).collect()
}

fn dispatch(lua: &str, legacy: &str) {
    // Hyprland's Lua config mode (0.55+) takes Lua; older/hyprlang setups the legacy syntax
    let ok = request(&format!("dispatch {lua}")).is_some_and(|r| r.trim() == "ok");
    if !ok {
        let _ = request(&format!("dispatch {legacy}"));
    }
}

fn window_ops(addr: &str, float: bool, size: Option<(i32, i32)>, pos: Option<(i32, i32)>, pin: bool) {
    let win = format!("\"address:{addr}\"");
    if float {
        dispatch(
            &format!("hl.dsp.window.float({{ action = \"on\", window = {win} }})"),
            &format!("setfloating address:{addr}"),
        );
    }
    if let Some((w, h)) = size {
        dispatch(
            &format!("hl.dsp.window.resize({{ x = {w}, y = {h}, window = {win} }})"),
            &format!("resizewindowpixel exact {w} {h},address:{addr}"),
        );
    }
    if let Some((x, y)) = pos {
        dispatch(
            &format!("hl.dsp.window.move({{ x = {x}, y = {y}, relative = false, window = {win} }})"),
            &format!("movewindowpixel exact {x} {y},address:{addr}"),
        );
    }
    if pin {
        dispatch(
            &format!("hl.dsp.window.pin({{ action = \"on\", window = {win} }})"),
            &format!("pin address:{addr}"),
        );
    }
}

fn find_client(title: &str) -> Option<Value> {
    let pid = std::process::id() as i64;
    let Some(Value::Array(clients)) = json("clients") else {
        return None;
    };
    clients.into_iter().find(|c| {
        c.get("pid").and_then(Value::as_i64) == Some(pid) && c.get("title").and_then(Value::as_str) == Some(title)
    })
}

/// Float + size + center (or place) our own toplevel. Returns false until it is mapped.
pub fn float(title: &str, w: i32, h: i32, pos: Option<(i32, i32)>, pin: bool) -> bool {
    if !available() {
        return true;
    }
    let Some(client) = find_client(title) else { return false };
    let addr = client.get("address").and_then(Value::as_str).unwrap_or("").to_string();
    let (mut w, mut h, mut pos) = (w, h, pos);
    if pos.is_none() {
        let mon_id = client.get("monitor").and_then(Value::as_i64);
        if let Some(Value::Array(mons)) = json("monitors")
            && let Some(m) = mons.iter().find(|m| m.get("id").and_then(Value::as_i64) == mon_id)
        {
            let g = |k: &str| m.get(k).and_then(Value::as_f64).unwrap_or(0.0);
            let s = m
                .get("scale")
                .and_then(Value::as_f64)
                .filter(|s| *s > 0.0)
                .unwrap_or(1.0);
            let (mut mw, mut mh) = (g("width") / s, g("height") / s);
            if m.get("transform").and_then(Value::as_i64).unwrap_or(0) % 2 == 1 {
                std::mem::swap(&mut mw, &mut mh);
            }
            w = w.min((mw * 0.92) as i32);
            h = h.min((mh * 0.9) as i32);
            pos = Some((
                (g("x") + (mw - w as f64) / 2.0) as i32,
                (g("y") + (mh - h as f64) / 2.0) as i32,
            ));
        }
    }
    let floating = client.get("floating").and_then(Value::as_bool).unwrap_or(false);
    window_ops(&addr, !floating, Some((w, h)), pos, pin);
    true
}

pub fn resize(title: &str, w: i32, h: i32) {
    if let Some(c) = find_client(title) {
        let addr = c.get("address").and_then(Value::as_str).unwrap_or("").to_string();
        window_ops(&addr, false, Some((w, h)), None, false);
    }
}

pub fn focus(title: &str) {
    if let Some(c) = find_client(title) {
        let addr = c.get("address").and_then(Value::as_str).unwrap_or("");
        dispatch(
            &format!("hl.dsp.focus({{ window = \"address:{addr}\" }})"),
            &format!("focuswindow address:{addr}"),
        );
    }
}
