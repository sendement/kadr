//! Resident tray icon (StatusNotifierItem over D-Bus): the GUI entry point. Captures, the
//! history and editors run as their own processes, so the tray never blocks.
use std::io::Read;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::PathBuf;

use ksni::blocking::TrayMethods;
use ksni::menu::{RadioGroup, RadioItem, StandardItem, SubMenu};
use ksni::{MenuItem, ToolTip};

use crate::config::Config;
use crate::i18n::{self, tr, trf};

fn socket() -> PathBuf {
    let rt = std::env::var_os("XDG_RUNTIME_DIR").map_or_else(std::env::temp_dir, PathBuf::from);
    rt.join(format!("kadr-tray-{}.sock", unsafe { libc::getuid() }))
}

/// A themed icon, so the SNI item carries an IconName: bars like quickshell ignore raw pixmaps.
fn install_icon(accent: &str) {
    let path = crate::config::home().join(".local/share/icons/hicolor/scalable/apps/kadr.svg");
    let svg = crate::icons::svg("kadr", accent, 2.2);
    if std::fs::read_to_string(&path).ok().as_deref() != Some(svg.as_str()) {
        if let Some(d) = path.parent() {
            let _ = std::fs::create_dir_all(d);
        }
        let _ = std::fs::write(&path, svg);
    }
}

struct Tray;

fn capture(delay: &str, cmd: &str) {
    crate::sys::spawn_self(&[cmd, "--delay", delay]);
}

fn open_history() {
    if !crate::windows::poke_history("raise") {
        crate::sys::spawn_self(&["history"]);
    }
}

impl ksni::Tray for Tray {
    fn id(&self) -> String {
        "kadr".into()
    }
    fn title(&self) -> String {
        "Kadr".into()
    }
    fn icon_name(&self) -> String {
        "kadr".into()
    }
    fn tool_tip(&self) -> ToolTip {
        ToolTip {
            title: "Kadr".into(),
            description: tr("Kadr — click: screenshot, middle click: history"),
            ..Default::default()
        }
    }
    fn activate(&mut self, _x: i32, _y: i32) {
        // let the tray popup vanish before the frame is grabbed
        capture("0.25", "capture");
    }
    fn secondary_activate(&mut self, _x: i32, _y: i32) {
        open_history();
    }
    fn menu(&self) -> Vec<MenuItem<Self>> {
        let chosen = Config::load().str("language");
        let mut codes = vec!["auto".to_string()];
        codes.extend(i18n::LANGUAGES.iter().map(|(c, _)| c.to_string()));
        let mut options = vec![RadioItem {
            label: trf(
                "Automatic (system: {lang})",
                &[("lang", &i18n::name_of(i18n::detect()))],
            ),
            ..Default::default()
        }];
        options.extend(i18n::LANGUAGES.iter().map(|(_, n)| RadioItem {
            label: n.to_string(),
            ..Default::default()
        }));
        let selected = codes.iter().position(|c| *c == chosen).unwrap_or(0);
        vec![
            StandardItem {
                label: tr("Take screenshot"),
                icon_name: "kadr".into(),
                activate: Box::new(|_| capture("0.25", "capture")),
                ..Default::default()
            }
            .into(),
            StandardItem {
                label: tr("Screenshot of the screen under the cursor"),
                activate: Box::new(|_| capture("0.3", "screen")),
                ..Default::default()
            }
            .into(),
            StandardItem {
                label: tr("Screenshot in 3 seconds"),
                activate: Box::new(|_| capture("3", "capture")),
                ..Default::default()
            }
            .into(),
            MenuItem::Separator,
            StandardItem {
                label: tr("Screenshot history"),
                activate: Box::new(|_| open_history()),
                ..Default::default()
            }
            .into(),
            SubMenu {
                label: tr("Language"),
                submenu: vec![
                    RadioGroup {
                        selected,
                        select: Box::new(move |_: &mut Tray, i| {
                            let code = codes.get(i).cloned().unwrap_or_else(|| "auto".into());
                            let mut cfg = Config::load(); // fresh: other processes may have saved tool styles meanwhile
                            cfg.set("language", code.clone());
                            cfg.save();
                            i18n::setup(&code);
                            crate::windows::poke_history("lang");
                        }),
                        options,
                    }
                    .into(),
                ],
                ..Default::default()
            }
            .into(),
            MenuItem::Separator,
            StandardItem {
                label: tr("Quit"),
                icon_name: "application-exit".into(),
                activate: Box::new(|_| std::process::exit(0)),
                ..Default::default()
            }
            .into(),
        ]
    }
}

pub fn run() -> i32 {
    let path = socket();
    // a second `kadr tray` opens the history of the running one
    if let Ok(mut s) = UnixStream::connect(&path) {
        use std::io::Write;
        let _ = s.write_all(b"history");
        return 0;
    }
    let _ = std::fs::remove_file(&path);
    let listener = match UnixListener::bind(&path) {
        Ok(l) => l,
        Err(e) => {
            eprintln!("kadr: {}: {e}", path.display());
            return 1;
        }
    };
    install_icon(&Config::load().accent().rgb_name());
    let handle = match Tray.spawn() {
        Ok(h) => h,
        Err(e) => {
            eprintln!("kadr: tray: {e}");
            return 1;
        }
    };
    for s in listener.incoming().flatten() {
        let mut s = s;
        let mut msg = String::new();
        let _ = s.read_to_string(&mut msg);
        if msg.trim() == "history" {
            open_history();
        }
        handle.update(|_| {});
    }
    0
}
