//! kadr — multi-monitor screenshot tool with layered annotations and editable history.
mod capture;
mod color;
mod config;
mod geom;
mod history;
mod hypr;
mod i18n;
mod icons;
mod imgio;
mod model;
mod overlay;
mod shapes;
mod sys;
mod testkit;
#[cfg(test)]
mod tests;
mod text;
mod tray;
mod ui;
mod windows;

use config::Config;
use i18n::tr;

const USAGE: &str = "kadr — multi-monitor screenshot tool with layered annotations and editable history

commands:
  kadr [capture]        select an area (overlay on all screens)
  kadr tray             tray icon (GUI mode; launching it again opens the history)
  kadr screen           instant shot of the screen under the cursor -> clipboard + history
  kadr history          screenshot history
  kadr edit [ID|last]   open a history item in the layer editor
  kadr pin [ID|last]    pin a history item on top of other windows

options:
  -d, --delay SECONDS   delay before the capture
  --lang CODE           interface language: auto (system), {langs}
  -h, --help            this help

environment:
  KADR_CAPTURE=kms|screencopy   force a capture backend (default: KMS, screencopy fallback)
";

struct Args {
    cmd: String,
    id: Option<String>,
    delay: f64,
    lang: Option<String>,
    at: Option<(i32, i32)>,
}

fn parse_args() -> Result<Args, String> {
    let mut a = Args {
        cmd: "capture".into(),
        id: None,
        delay: 0.0,
        lang: None,
        at: None,
    };
    let mut it = std::env::args().skip(1);
    let mut positional = Vec::new();
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "-h" | "--help" => {
                let langs: Vec<&str> = i18n::LANGUAGES.iter().map(|(c, _)| *c).collect();
                print!("{}", USAGE.replace("{langs}", &langs.join(", ")));
                std::process::exit(0);
            }
            "-V" | "--version" => {
                println!("kadr {}", env!("CARGO_PKG_VERSION"));
                std::process::exit(0);
            }
            "-d" | "--delay" => a.delay = it.next().and_then(|v| v.parse().ok()).ok_or("--delay needs a number")?,
            "--lang" => a.lang = Some(it.next().ok_or("--lang needs a code")?),
            "--at" => {
                let v = it.next().ok_or("--at needs X,Y")?;
                let (x, y) = v.split_once(',').ok_or("--at needs X,Y")?;
                a.at = Some((
                    x.trim().parse().map_err(|_| "bad --at")?,
                    y.trim().parse().map_err(|_| "bad --at")?,
                ));
            }
            s if s.starts_with("--delay=") => a.delay = s[8..].parse().map_err(|_| "--delay needs a number")?,
            s if s.starts_with("--lang=") => a.lang = Some(s[7..].to_string()),
            s if s.starts_with('-') && s.len() > 1 && !s.starts_with("--") && s[1..].parse::<f64>().is_err() => {
                return Err(format!("unknown option {s}"));
            }
            _ => positional.push(arg),
        }
    }
    let mut pos = positional.into_iter();
    if let Some(c) = pos.next() {
        a.cmd = c;
    }
    a.id = pos.next();
    Ok(a)
}

fn main() {
    let args = std::env::args().collect::<Vec<_>>();
    if args.get(1).map(String::as_str) == Some("__grab") {
        // debugging aid: save what the capture backends see, one PNG per output
        let dir = std::path::PathBuf::from(args.get(2).map_or(".", |s| s.as_str()));
        match capture::grab(false) {
            Ok(c) => {
                for o in &c.outputs {
                    let p = dir.join(format!("{}.png", o.name));
                    let _ = imgio::save_png(o.image.as_ref().unwrap(), &p);
                    eprintln!("{} via {} -> {}", o.name, c.backend, p.display());
                }
                std::process::exit(0);
            }
            Err(e) => {
                eprintln!("kadr: {e}");
                std::process::exit(1);
            }
        }
    }
    if args.get(1).map(String::as_str) == Some("__clipboard") {
        std::process::exit(sys::clipboard_server(args.get(2).map_or("image/png", |s| s.as_str())));
    }
    let args = match parse_args() {
        Ok(a) => a,
        Err(e) => {
            eprintln!("kadr: {e} (see kadr --help)");
            std::process::exit(2);
        }
    };
    let cfg = Config::load();
    let lang = args.lang.clone().unwrap_or_else(|| cfg.str("language"));
    i18n::setup(&lang);
    // the font database takes ~100 ms to scan: warm it up while the capture runs
    let fonts = std::thread::spawn(|| {
        text::db();
    });
    let code = match args.cmd.as_str() {
        "capture" | "screen" => {
            if args.delay > 0.0 {
                std::thread::sleep(std::time::Duration::from_secs_f64(args.delay));
            }
            let cap = match capture::grab(args.cmd == "capture") {
                Ok(c) => c,
                Err(e) => {
                    eprintln!("kadr: {e}");
                    std::process::exit(1);
                }
            };
            let _ = fonts.join();
            if args.cmd == "screen" {
                screen(cap, &cfg)
            } else {
                overlay::run(cap, cfg)
            }
        }
        "tray" => tray::run(),
        "history" => windows::history(cfg),
        "edit" | "pin" => {
            let id = match args.id.as_deref() {
                None | Some("last") => history::last_id(),
                Some(i) => Some(i.to_string()),
            };
            match id {
                None => {
                    eprintln!("{}", tr("kadr: history is empty"));
                    1
                }
                Some(id) if args.cmd == "edit" => windows::editor(cfg, id),
                Some(id) => windows::pin(cfg, id, args.at),
            }
        }
        other => {
            eprintln!("kadr: unknown command {other} (see kadr --help)");
            2
        }
    };
    std::process::exit(code);
}

/// `kadr screen`: the output under the cursor straight to the clipboard + history.
fn screen(cap: capture::Capture, cfg: &Config) -> i32 {
    let o = cap
        .cursor
        .and_then(|(x, y)| cap.outputs.iter().find(|o| o.contains(x, y)))
        .unwrap_or(&cap.outputs[0]);
    let Some(img) = o.image.clone() else { return 1 };
    let mut meta = serde_json::Map::new();
    meta.insert("output".into(), o.name.clone().into());
    meta.insert(
        "scale".into(),
        serde_json::json!(img.width() as f64 / o.w.max(1) as f64),
    );
    meta.insert("origin".into(), serde_json::json!([o.x, o.y]));
    meta.insert("created".into(), history::now_iso().into());
    let crop = Some(geom::IR::new(0, 0, img.width() as i32, img.height() as i32));
    let mut doc = model::Document::new(img.clone(), crop, vec![], meta);
    sys::copy_image(&img);
    let limit = cfg.int("history_limit", 500).max(0) as usize;
    let d = history::save(&mut doc, &img, None, limit).ok();
    sys::notify(
        cfg,
        &tr("Screen screenshot copied"),
        &o.name,
        d.map(|d| d.join("thumb.jpg")).as_deref(),
    );
    0
}
