//! Desktop glue: clipboard (served by a detached `kadr` child), notifications over D-Bus,
//! the pictures folder, spawning our own subcommands.
use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Stdio};

use crate::config::Config;

fn self_exe() -> PathBuf {
    std::env::current_exe().unwrap_or_else(|_| PathBuf::from("kadr"))
}

/// Run `kadr <args>` detached (own session, no stdio): captures, editors, pins.
pub fn spawn_self(args: &[&str]) {
    let mut cmd = Command::new(self_exe());
    cmd.args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::inherit());
    unsafe {
        use std::os::unix::process::CommandExt;
        cmd.pre_exec(|| {
            libc::setsid();
            Ok(())
        });
    }
    let _ = cmd.spawn();
}

/// Wayland clipboards are served by their owner: hand the data to a detached child that keeps
/// offering it until another client takes the selection over.
fn serve(mime: &str, data: Vec<u8>) {
    let mut cmd = Command::new(self_exe());
    cmd.args(["__clipboard", mime])
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    unsafe {
        use std::os::unix::process::CommandExt;
        cmd.pre_exec(|| {
            libc::setsid();
            Ok(())
        });
    }
    if let Ok(mut child) = cmd.spawn() {
        if let Some(mut stdin) = child.stdin.take() {
            let _ = stdin.write_all(&data);
        }
        // the child exits once it loses the selection; reap it from a thread so it never zombies
        std::thread::spawn(move || {
            let _ = child.wait();
        });
    }
}

pub fn copy_image(img: &tiny_skia::Pixmap) {
    serve("image/png", crate::imgio::png_bytes(img));
}

pub fn copy_text(text: &str) {
    serve("text/plain;charset=utf-8", text.as_bytes().to_vec());
}

/// `kadr __clipboard MIME`: serve stdin as the clipboard selection until replaced.
pub fn clipboard_server(mime: &str) -> i32 {
    use std::io::Read;
    use wl_clipboard_rs::copy::{MimeType, Options, ServeRequests, Source};
    let mut data = Vec::new();
    if std::io::stdin().read_to_end(&mut data).is_err() || data.is_empty() {
        return 1;
    }
    let mut opts = Options::new();
    opts.foreground(true).serve_requests(ServeRequests::Unlimited);
    let mime = if mime.starts_with("text/") {
        MimeType::Text
    } else {
        MimeType::Specific(mime.to_string())
    };
    match opts.copy(Source::Bytes(data.into_boxed_slice()), mime) {
        Ok(()) => 0,
        Err(e) => {
            eprintln!("kadr: clipboard: {e}");
            1
        }
    }
}

pub fn paste_text() -> Option<String> {
    use std::io::Read;
    use wl_clipboard_rs::paste::{ClipboardType, MimeType, Seat, get_contents};
    let (mut pipe, _) = get_contents(ClipboardType::Regular, Seat::Unspecified, MimeType::Text).ok()?;
    let mut s = String::new();
    pipe.read_to_string(&mut s).ok()?;
    Some(s)
}

pub fn notify(cfg: &Config, summary: &str, body: &str, icon: Option<&std::path::Path>) {
    if !cfg.bool("notify", true) {
        return;
    }
    let mut n = notify_rust::Notification::new();
    n.appname("Kadr").summary(summary).body(body);
    match icon {
        Some(p) => n.icon(&p.to_string_lossy()),
        None => n.icon("kadr"),
    };
    let _ = n.show();
}

pub fn pictures_dir() -> PathBuf {
    match dirs::picture_dir() {
        Some(d) if d != crate::config::home() => d,
        _ => crate::config::home().join("Pictures"),
    }
}

pub fn save_dir(cfg: &Config) -> PathBuf {
    let s = cfg.str("save_dir");
    let d = if s.is_empty() {
        pictures_dir().join("Kadr")
    } else if let Some(rest) = s.strip_prefix("~/") {
        crate::config::home().join(rest)
    } else {
        PathBuf::from(s)
    };
    let _ = std::fs::create_dir_all(&d);
    d
}

pub fn save_file(img: &tiny_skia::Pixmap, cfg: &Config) -> std::io::Result<PathBuf> {
    let path = save_dir(cfg).join(chrono::Local::now().format("Kadr_%Y-%m-%d_%H-%M-%S.png").to_string());
    crate::imgio::save_png(img, &path)?;
    Ok(path)
}

pub fn open_path(p: &std::path::Path) {
    let _ = Command::new("xdg-open")
        .arg(p)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn();
}
