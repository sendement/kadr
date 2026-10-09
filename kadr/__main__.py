"""kadr — multi-monitor screenshot tool with layered annotations and editable history."""
import argparse
import sys
import time

USAGE = """
commands:
  kadr [capture]        select an area (overlay on all screens)
  kadr tray             tray icon (GUI mode; launching it again opens the history)
  kadr screen           instant shot of the screen under the cursor -> clipboard + history
  kadr history          screenshot history
  kadr edit [ID|last]   open a history item in the layer editor

options:
  --lang CODE           interface language: auto (system), {langs}
"""


def main(argv=None):
    from . import i18n
    from .config import Config
    cfg = Config()
    ap = argparse.ArgumentParser(prog="kadr", description=__doc__,
                                 epilog=USAGE.format(langs=", ".join(i18n.LANGUAGES)),
                                 formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("cmd", nargs="?", default="capture", choices=["capture", "screen", "history", "edit", "tray"])
    ap.add_argument("id", nargs="?", default="last", help="history item ID for edit")
    ap.add_argument("-d", "--delay", type=float, default=0, help="delay before the capture, seconds")
    ap.add_argument("--lang", choices=["auto"] + list(i18n.LANGUAGES), help="interface language for this run")
    args = ap.parse_args(argv)
    i18n.setup(args.lang or cfg.get("language", "auto"))

    from . import compositor
    outs, procs, cursor, windows = [], None, None, []
    if args.cmd in ("capture", "screen"):
        if args.delay:
            time.sleep(args.delay)
        outs = compositor.list_outputs()
        procs = compositor.start_grab(outs)  # freeze the frame before Qt even loads
        cursor = compositor.cursor_pos()
        if args.cmd == "capture":
            windows = compositor.list_windows(outs)

    layer_shell = compositor.LayerShell()
    from PySide6.QtCore import QRect
    from PySide6.QtGui import QImage
    from PySide6.QtWidgets import QApplication

    app = QApplication(sys.argv[:1])
    app.setApplicationName("kadr")
    app.setDesktopFileName("kadr")
    i18n.install_qt(app)

    if args.cmd == "tray":
        from .tray import Tray, notify_running
        if notify_running():
            return 0
        app.setQuitOnLastWindowClosed(False)
        tray = Tray(app)  # noqa: F841 (kept alive)
        return app.exec()

    if args.cmd in ("history", "edit"):
        from . import history
        from .editor import EditorWindow, HistoryWindow
        if args.cmd == "history":
            HistoryWindow(cfg).show()
        else:
            item = history.last_id() if args.id == "last" else args.id
            if not item:
                print(i18n.tr("kadr: history is empty"), file=sys.stderr)
                return 1
            EditorWindow(item, cfg).show()
        return app.exec()

    data = compositor.finish_grab(procs)
    if not outs:  # unknown compositor: map Qt screens onto one big grim capture
        full = QImage.fromData(data[None], "PPM")
        geo = QRect()
        for s in app.screens():
            geo = geo.united(s.geometry())
        k = full.width() / max(1, geo.width())
        for s in app.screens():
            g = s.geometry()
            o = compositor.Output(s.name(), g.x(), g.y(), g.width(), g.height(), s.devicePixelRatio())
            o.image = full.copy(QRect(round((g.x() - geo.x()) * k), round((g.y() - geo.y()) * k),
                                      round(g.width() * k), round(g.height() * k)))
            outs.append(o)
    else:
        for o in outs:
            o.image = QImage.fromData(data.get(o.name) or b"", "PPM")

    if args.cmd == "screen":
        from datetime import datetime
        from . import outputs as out
        from .model import Document
        o = next((o for o in outs if cursor and o.contains(*cursor)), outs[0])
        doc = Document(o.image, o.image.rect(), meta={
            "output": o.name, "scale": o.image.width() / o.w, "origin": [o.x, o.y],
            "created": datetime.now().isoformat(timespec="seconds")})
        out.copy(o.image)
        d = out.to_history(doc, o.image, cfg)
        out.notify(cfg, i18n.tr("Screen screenshot copied"), o.name, d / "thumb.jpg" if d else None)
        return 0

    from .overlay import Overlay
    app.setQuitOnLastWindowClosed(False)
    overlay = Overlay(app, outs, windows, cursor, cfg, layer_shell)  # noqa: F841 (kept alive)
    return app.exec()


if __name__ == "__main__":
    sys.exit(main())
