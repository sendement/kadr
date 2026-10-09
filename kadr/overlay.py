"""Capture overlay: one frozen, dimmed layer-shell surface per output."""
from datetime import datetime

from PySide6.QtCore import QObject, QPointF, QRectF, Qt, QTimer

from . import outputs as out
from .canvas import Canvas
from .editor import EditorWindow, _keep
from .model import Document
from .pin import PinWindow
from .shapes import render_result


class Overlay(QObject):
    def __init__(self, app, outs, windows, cursor, cfg, layer_shell):
        super().__init__()
        self.app, self.cfg = app, cfg
        self.locked = None
        self.canvases = []
        screens = {s.name(): s for s in app.screens()}
        created = datetime.now().isoformat(timespec="seconds")
        active = None
        for o in outs:
            scr = screens.get(o.name)
            if scr is None or o.image is None or o.image.isNull():
                continue
            sx, sy = o.image.width() / o.w, o.image.height() / o.h
            wins = []
            for w in windows:
                r = QRectF((w.x - o.x) * sx, (w.y - o.y) * sy, w.w * sx, w.h * sy)
                r = r & QRectF(o.image.rect())
                if r.width() > 20 and r.height() > 20:
                    wins.append((r, f"{w.cls} — {w.title}" if w.cls else w.title))
            doc = Document(o.image, meta={"output": o.name, "scale": sx, "created": created,
                                          "origin": [o.x, o.y]})
            c = Canvas(doc, cfg, scale=sx, windows=wins, controller=self, label=o.name)
            c.setWindowTitle("kadr")
            c.actionRequested.connect(lambda a, c=c: self.on_action(c, a))
            if not layer_shell.apply(c, scr, "kadr"):
                c.setWindowFlags(Qt.FramelessWindowHint | Qt.WindowStaysOnTopHint)
                c.setScreen(scr)
                c.setGeometry(scr.geometry())
                c.showFullScreen()
            else:
                c.resize(scr.geometry().size())
                c.show()
            self.canvases.append(c)
            if cursor and o.contains(*cursor):
                active = c
        if not self.canvases:
            raise SystemExit("kadr: не удалось сопоставить экраны Qt с выходами композитора")
        self.set_active(active or self.canvases[0])
        if cursor:
            c = self.active
            o = next(o for o in outs if o.name == c.label)
            c.mouse = QPointF(cursor[0] - o.x, cursor[1] - o.y)
            c.update_hover(c.to_img(c.mouse))

    # --- multi-monitor coordination --------------------------------------
    def set_active(self, canvas):
        self.active = canvas
        for c in self.canvases:
            c.set_active(c is canvas)
        canvas.activateWindow()
        canvas.setFocus()

    def _movable(self):
        """The locked screen may be abandoned while nothing is drawn on it yet."""
        c = self.locked
        return c is None or (not c.doc.layers and c.drag is None and not c.text_edit)

    def hover(self, canvas):
        if canvas is not self.active and (self.locked is canvas or self._movable()):
            self.set_active(canvas)

    def can_start(self, canvas):
        if self.locked in (None, canvas):
            return True
        if self._movable():
            self.locked.reset_selection()  # also unlocks
            return True
        return False

    def lock(self, canvas):
        self.locked = canvas
        if canvas is not self.active:
            self.set_active(canvas)

    def unlock(self):
        self.locked = None

    def key_target(self):
        # an idle screen under the cursor must not steal Enter/Ctrl+C from the screen with the selection
        if self.locked and self.active is not self.locked and self.active.state == "idle":
            return self.locked
        return self.active

    # --- finishing -------------------------------------------------------
    def close_all(self):
        for c in self.canvases:
            c.hide()
        self.app.processEvents()

    def on_action(self, canvas, action):
        cfg = self.cfg
        if action == "cancel":
            self.close_all()
            cfg.save()
            self.app.quit()
            return
        canvas.finish_text()
        doc = canvas.doc
        img = render_result(doc)
        self.close_all()
        size = f"{img.width()}×{img.height()}"
        if action == "copy":
            out.copy(img)
            d = out.to_history(doc, img, cfg)
            out.notify(cfg, "Скриншот скопирован", size, d / "thumb.jpg" if d else None)
        elif action == "save":
            path = out.save_file(img, cfg)
            if cfg.get("copy_on_save", True):
                out.copy(img)
            out.to_history(doc, img, cfg)
            out.notify(cfg, "Скриншот сохранён", str(path), path)
        elif action in ("pin", "edit"):
            d = out.to_history(doc, img, cfg)
            if action == "pin":
                ox, oy = doc.meta.get("origin", [0, 0])
                s = doc.meta.get("scale", 1.0)
                pos = (ox + doc.crop.x() / s, oy + doc.crop.y() / s) if doc.crop else None
                _keep(PinWindow(img, s, pos, cfg)).show()
            elif d:
                EditorWindow(d.name, cfg).show()
            cfg.save()
            QTimer.singleShot(0, self._close_overlays)
            return
        cfg.save()
        self.app.quit()

    def _close_overlays(self):
        for c in self.canvases:
            c.close()
            c.deleteLater()
        self.canvases.clear()
        self.app.setQuitOnLastWindowClosed(True)
