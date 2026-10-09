"""Interactive canvas shared by the capture overlay and the history editor."""
import copy
import math
import time
import zlib

from PySide6.QtCore import QLineF, QPoint, QPointF, QRect, QRectF, Qt, QTimer, Signal
from PySide6.QtGui import (QColor, QCursor, QFont, QFontMetrics, QGuiApplication, QPainter, QPainterPath,
                           QPen, QPixmap, QRegion)
from PySide6.QtWidgets import QApplication, QWidget

from . import shapes
from .colorpicker import ColorPopup
from .panels import ActionBar, LayersPanel, PropsBar, ToolBar, stylesheet
from .i18n import tr

HANDLE_R = 6
TOOL_KEYS = {Qt.Key_V: "pointer", Qt.Key_P: "pen", Qt.Key_H: "highlighter", Qt.Key_L: "line",
             Qt.Key_A: "arrow", Qt.Key_R: "rect", Qt.Key_E: "ellipse", Qt.Key_T: "text",
             Qt.Key_M: "mark", Qt.Key_B: "blur", Qt.Key_Z: "zoom"}
RECT_CURSORS = {"d_tl": Qt.SizeFDiagCursor, "d_br": Qt.SizeFDiagCursor, "d_tr": Qt.SizeBDiagCursor,
                "d_bl": Qt.SizeBDiagCursor, "s_tl": Qt.SizeFDiagCursor, "s_br": Qt.SizeFDiagCursor,
                "s_tr": Qt.SizeBDiagCursor, "s_bl": Qt.SizeBDiagCursor,
                "tl": Qt.SizeFDiagCursor, "br": Qt.SizeFDiagCursor, "tr": Qt.SizeBDiagCursor,
                "bl": Qt.SizeBDiagCursor, "t": Qt.SizeVerCursor, "b": Qt.SizeVerCursor,
                "l": Qt.SizeHorCursor, "r": Qt.SizeHorCursor}


def make_cross_cursor(accent, dpr):
    size = 33
    pm = QPixmap(int(size * dpr), int(size * dpr))
    pm.setDevicePixelRatio(dpr)
    pm.fill(Qt.transparent)
    p = QPainter(pm)
    p.setRenderHint(QPainter.Antialiasing)
    c, gap = 16.5, 4.5
    segs = [QLineF(c, 1, c, c - gap), QLineF(c, c + gap, c, size - 1),
            QLineF(1, c, c - gap, c), QLineF(c + gap, c, size - 1, c)]
    p.setPen(QPen(QColor(0, 0, 0, 170), 3.5, Qt.SolidLine, Qt.RoundCap))
    p.drawLines(segs)
    p.setPen(QPen(accent, 1.6, Qt.SolidLine, Qt.RoundCap))
    p.drawLines(segs)
    p.setPen(QPen(QColor(0, 0, 0, 170), 1))
    p.setBrush(accent)
    p.drawEllipse(QPointF(c, c), 1.8, 1.8)
    p.end()
    return QCursor(pm, 16, 16)


def draw_pill(p, text, pt, align="tl", bounds=None, accent=None):
    f = QFont()
    f.setPixelSize(12)
    p.setFont(f)
    fm = QFontMetrics(f)
    w, h = fm.horizontalAdvance(text) + 18, fm.height() + 8
    x, y = pt.x(), pt.y()
    if "r" in align:
        x -= w
    if "c" in align:
        x -= w / 2
    if "b" in align:
        y -= h
    r = QRectF(x, y, w, h)
    if bounds:
        r.moveLeft(max(bounds.left() + 4, min(r.left(), bounds.right() - w - 4)))
        r.moveTop(max(bounds.top() + 4, min(r.top(), bounds.bottom() - h - 4)))
    p.save()
    p.setRenderHint(QPainter.Antialiasing)
    p.setPen(QPen(QColor(255, 255, 255, 40), 1))
    p.setBrush(QColor(22, 22, 28, 230))
    p.drawRoundedRect(r, h / 2, h / 2)
    p.setPen(accent or QColor("#ececf1"))
    p.drawText(r, Qt.AlignCenter, text)
    p.restore()
    return r


class Canvas(QWidget):
    actionRequested = Signal(str)

    def __init__(self, doc, cfg, editor=False, scale=1.0, windows=(), controller=None, label=""):
        super().__init__()
        self.doc, self.cfg, self.editor, self.controller = doc, cfg, editor, controller
        self.styles = cfg["tool_styles"]
        self.accent = QColor(cfg["accent"])
        self.out_scale = scale
        self.view_scale = 1.0 / scale
        self.offset = QPointF()
        self.windows = list(windows)  # [(QRectF image px, title)]
        self.label = label
        self.active = editor
        self.state = "edit" if (editor or doc.crop) else "idle"
        self.tool = "pointer"
        self.multi = set()  # ids of all selected layers when more than one is selected
        self.selected = None
        self.drag = None
        self.text_edit = None
        self._text = {}
        self._caret_on = True
        self.hover_win = None
        self.mouse = None
        self._cache, self._dirty, self._below = None, True, None
        self._pick_cb = None
        self.popup = None
        self.layers_visible = bool(cfg.get("layers_visible"))
        self._toast = None
        self.user_view = False
        self.pulse = 0.0

        self.setMouseTracking(True)
        self.setFocusPolicy(Qt.StrongFocus)
        self.setAttribute(Qt.WA_OpaquePaintEvent, not editor)
        self.setStyleSheet(stylesheet(self.accent))
        self.cross = make_cross_cursor(self.accent, scale if not editor else 2.0)

        self.toolbar = ToolBar(self)
        self.actions = ActionBar(self, editor)
        self.props = PropsBar(self)
        self.layers_panel = LayersPanel(self)
        for w in (self.toolbar, self.actions, self.props, self.layers_panel):
            w.hide()

        self.caret_timer = QTimer(self, interval=530, timeout=self._blink)
        # a single click on a window selects it, but only once it is clear no double click follows
        self.click_timer = QTimer(self, singleShot=True, timeout=self._single_click)
        # "live clipboard": re-copy the flattened result shortly after each change
        self.live_copy = bool(cfg.get("live_copy"))
        self.live_timer = QTimer(self, singleShot=True, interval=350, timeout=self._live_copy_now)
        self._live_crc = None
        self._pending_win = None
        if not editor:
            self.glow_timer = QTimer(self, interval=40, timeout=self._glow_tick)
            self.glow_timer.start()
        if self.state == "edit":
            QTimer.singleShot(0, self.enter_edit)
        self.update_cursor()

    @property
    def selected(self):
        return self._selected

    @selected.setter
    def selected(self, layer):
        self._selected = layer
        self.multi = set()

    def sel_layers(self):
        ids = set(self.multi)
        if self._selected:
            ids.add(self._selected.id)
        return [l for l in self.doc.layers if l.id in ids]

    # ------------------------------------------------------------------ view
    def to_img(self, p):
        return QPointF((p.x() - self.offset.x()) / self.view_scale, (p.y() - self.offset.y()) / self.view_scale)

    def to_w(self, p):
        return QPointF(p.x() * self.view_scale + self.offset.x(), p.y() * self.view_scale + self.offset.y())

    def rect_w(self, r):
        return QRectF(self.to_w(r.topLeft()), self.to_w(r.bottomRight()))

    def tol(self):
        return 5 / self.view_scale

    def img_rect(self):
        return QRectF(self.doc.base.rect())

    def clamp_img(self, p):
        r = self.img_rect()
        return QPointF(min(max(p.x(), 0), r.width()), min(max(p.y(), 0), r.height()))

    def fit_view(self):
        if not self.editor or self.width() < 50:
            return
        r = QRectF(self.doc.crop or self.doc.base.rect())
        avail = QRectF(self.rect()).adjusted(64, 58, -20 - (250 if self.layers_visible else 0), -58)
        s = min(avail.width() / r.width(), avail.height() / r.height(), 1.0 / self.devicePixelRatioF())
        self.view_scale = max(0.02, s)
        self.offset = avail.center() - r.center() * self.view_scale
        self.user_view = False
        self.layout_panels()
        self.update()

    def zoom_at(self, factor, pos):
        ip = self.to_img(pos)
        self.view_scale = min(32.0, max(0.02, self.view_scale * factor))
        self.offset = pos - ip * self.view_scale
        self.user_view = True
        self.layout_panels()
        self.update()

    # ----------------------------------------------------------------- state
    def enter_edit(self):
        self.state = "edit"
        self.toolbar.sync(self.tool)
        self.toolbar.show()
        self.actions.show()
        self.layers_panel.setVisible(self.layers_visible)
        self.props.rebuild(force=True)
        self.refresh_panels()
        if self.editor:
            self.fit_view()
        self.layout_panels()
        self.invalidate()
        self.update_cursor()
        self.setFocus()

    def reset_selection(self):
        self.finish_text()
        self.doc.crop = None
        self.doc.layers.clear()
        self.doc.undo_stack.clear()
        self.doc.redo_stack.clear()
        self.selected = None
        self.state = "idle"
        self.close_popup()
        for w in (self.toolbar, self.actions, self.props, self.layers_panel):
            w.hide()
        if self.controller:
            self.controller.unlock()
        self.update_cursor()
        self.update()

    def set_active(self, on):
        if self.active != on:
            self.active = on
            self.update_cursor()
            self.update()

    def invalidate(self):
        self._dirty = True
        self.update()
        if self.live_copy and self.state == "edit" and self.doc.crop:
            self.live_timer.start()

    def toggle_live_copy(self):
        self.live_copy = not self.live_copy
        self.cfg["live_copy"] = self.live_copy
        self.actions.sync()
        if self.live_copy:
            self._live_crc = None
            self._live_copy_now()
            self.toast(tr("Live clipboard: the final image is updated on every change"))
        else:
            self.live_timer.stop()
            self.toast(tr("Live clipboard off"))

    def _live_copy_now(self):
        if not self.live_copy or self.state != "edit" or not self.doc.crop:
            return
        if self.drag or self.text_edit:  # wait until the edit settles
            self.live_timer.start()
            return
        self.ensure_cache()
        img = self._cache
        crc = (img.width(), img.height(), zlib.crc32(bytes(img.constBits())))
        if crc != self._live_crc:
            self._live_crc = crc
            from .outputs import copy
            copy(img)

    def toast(self, text):
        self._toast = (text, time.monotonic())
        self.update()
        QTimer.singleShot(1600, self.update)

    # ---------------------------------------------------------------- panels
    def layout_panels(self):
        if self.state != "edit" or not self.doc.crop:
            return
        W, H, g = self.width(), self.height(), 8
        cw = self.rect_w(QRectF(self.doc.crop))
        tb = self.toolbar.sizeHint()
        self.toolbar.resize(tb)
        side_used = None
        if W - cw.right() >= tb.width() + 2 * g:
            x, side_used = cw.right() + g, "right"
        elif cw.left() >= tb.width() + 2 * g:
            x, side_used = cw.left() - g - tb.width(), "left"
        else:
            x = cw.right() - g - tb.width()
        x = max(g, min(x, W - tb.width() - g))
        y = max(g, min(cw.top(), H - tb.height() - g))
        self.toolbar.move(int(x), int(y))

        slots = {"below": cw.bottom() + g, "above": cw.top() - g,
                 "in_bottom": cw.bottom() - g, "in_top": cw.top() + g}

        def place_h(w, prefs, align_right):
            if not w.isVisible():
                return
            sz = w.sizeHint()
            w.resize(sz)
            h = sz.height()
            for side in prefs:
                if side == "below" and slots["below"] + h + g <= H:
                    y = slots["below"]
                    slots["below"] += h + g
                    break
                if side == "above" and slots["above"] - h >= g:
                    y = slots["above"] - h
                    slots["above"] -= h + g
                    break
            else:
                if prefs[0] == "below":
                    y = slots["in_bottom"] - h
                    slots["in_bottom"] -= h + g
                else:
                    y = slots["in_top"]
                    slots["in_top"] += h + g
            x = cw.right() - sz.width() if align_right else cw.left()
            x = max(g, min(x, W - sz.width() - g))
            y = max(g, min(y, H - h - g))
            w.move(int(x), int(y))

        place_h(self.actions, ["below", "above"], True)
        place_h(self.props, ["above", "below"], False)

        if self.layers_panel.isVisible():
            lp = self.layers_panel.sizeHint()
            self.layers_panel.resize(lp)
            if side_used != "left" and cw.left() >= lp.width() + 2 * g:
                x = cw.left() - g - lp.width()
            elif side_used == "right" and W - (cw.right() + tb.width() + g) >= lp.width() + 2 * g:
                x = cw.right() + tb.width() + 2 * g
            elif side_used == "left" and W - cw.right() >= lp.width() + 2 * g:
                x = cw.right() + g
            else:
                x = cw.left() + g if side_used != "left" else cw.right() - lp.width() - g - (
                    tb.width() + g if side_used is None else 0)
            x = max(g, min(x, W - lp.width() - g))
            y = max(g, min(cw.top(), H - lp.height() - g))
            self.layers_panel.move(int(x), int(y))
        # inside-placement fallback can stack panels; push vertical ones clear of horizontal bars
        for v in (self.toolbar, self.layers_panel):
            if not v.isVisible():
                continue
            for h in (self.props, self.actions):
                if h.isVisible() and v.geometry().intersects(h.geometry()):
                    if h.geometry().center().y() < v.geometry().center().y() or h is self.props:
                        y = h.geometry().bottom() + g
                    else:
                        y = h.geometry().top() - g - v.height()
                    v.move(v.x(), int(max(g, min(y, H - v.height() - g))))
        if self.popup and self.popup.isVisible():
            self.popup.raise_()

    def refresh_panels(self, layers=True):
        self.actions.sync()
        self.props.rebuild()
        if layers and self.layers_panel.isVisible():
            self.layers_panel.refresh()
            self.layout_panels()

    def toggle_layers(self):
        self.layers_visible = not self.layers_visible
        self.cfg["layers_visible"] = self.layers_visible
        self.layers_panel.setVisible(self.layers_visible)
        if self.layers_visible:
            self.layers_panel.refresh()
        self.actions.sync()
        if self.editor and not self.user_view:
            self.fit_view()
        self.layout_panels()

    def open_color_popup(self, button):
        self.close_popup()
        self.popup = ColorPopup(self, button)
        self.popup.adjustSize()
        sz = self.popup.sizeHint()
        self.popup.resize(sz)
        pos = button.mapTo(self, QPoint(0, button.height() + 6))
        if pos.y() + sz.height() > self.height() - 8:
            pos = button.mapTo(self, QPoint(0, -sz.height() - 6))
        pos.setX(max(8, min(pos.x(), self.width() - sz.width() - 8)))
        pos.setY(max(8, pos.y()))
        self.popup.move(pos)
        self.popup.show()
        self.popup.raise_()

    def close_popup(self):
        if self.popup:
            self.popup.hide()
            self.popup.deleteLater()
            self.popup = None

    def start_pick(self, cb):
        self._pick_cb = cb
        self.update_cursor()
        self.toast(tr("Eyedropper: click on the screenshot (Esc — cancel)"))

    def color_at(self, ip):
        crop = self.doc.crop
        x, y = int(ip.x()), int(ip.y())
        if crop and crop.contains(x, y) and self._cache is not None:
            return self._cache.pixelColor(x - crop.x(), y - crop.y())
        if self.doc.base.rect().contains(x, y):
            return self.doc.base.pixelColor(x, y)
        return QColor(Qt.black)

    # ----------------------------------------------------------------- tools
    def set_tool(self, name):
        self.finish_text()
        self.tool = name
        if self.selected and name != "pointer" and self.selected.type != name:
            self.selected = None
            self.layers_panel.refresh()
        self.toolbar.sync(name)
        self.props.rebuild()
        self.update_cursor()
        self.update()

    def props_target(self):
        sel = self.selected
        if sel and sel in self.doc.layers and (self.tool == "pointer" or self.tool == sel.type):
            return sel.type, sel.style, sel
        if self.tool in self.styles:
            return self.tool, self.styles[self.tool], None
        return None

    def apply_style(self, key, value):
        target = self.props_target()
        if not target:
            return
        kind, st, layer = target
        self.styles[kind][key] = value
        if layer:
            targets = [l for l in self.sel_layers() if key in l.style] or [layer]
            self.doc.push_undo(key=("style", tuple(l.id for l in targets), key))
            for l in targets:
                l.style[key] = value
            self.invalidate()
            self.actions.sync()

    def set_layer_prop(self, prop, value, layer=None):
        targets = [layer] if layer else self.sel_layers()
        if not targets:
            return
        self.doc.push_undo(key=("prop", tuple(l.id for l in targets), prop))
        for l in targets:
            setattr(l, prop, value)
        self.invalidate()
        self.actions.sync()
        if prop in ("name", "visible"):
            self.props.rebuild(force=True)

    def select(self, layer, from_panel=False, add=False):
        if add and layer:
            ids = {l.id for l in self.sel_layers()}
            ids ^= {layer.id}
            self.select_many(ids, layer.id if layer.id in ids else None)
            self.layers_panel.refresh()
            return
        if layer is self.selected and not self.multi:
            return
        if self.text_edit and layer is not self.text_edit:
            self.finish_text()
        self.selected = layer
        if layer and self.tool not in ("pointer", layer.type):
            self.tool = "pointer"
            self.toolbar.sync("pointer")
        self.props.rebuild()
        if not from_panel and self.layers_panel.isVisible():
            self.layers_panel.refresh()
        elif from_panel:
            self.layers_panel.refresh()
        self.update()

    def select_many(self, ids, current=None):
        self.finish_text()
        layers = [l for l in self.doc.layers if l.id in ids]
        cur = self.doc.find(current) if current in ids else (layers[-1] if layers else None)
        self.selected = cur
        self.multi = {l.id for l in layers} if len(layers) > 1 else set()
        if cur and self.tool not in ("pointer", cur.type):
            self.tool = "pointer"
            self.toolbar.sync("pointer")
        self.props.rebuild()
        self.update()

    def layer_at(self, ip, types=None):
        # a generous grab zone: anything within ~12px on screen, plus the box around selected layers
        tol = 12 / self.view_scale
        pad = 8 / self.view_scale
        for layer in reversed(self.sel_layers()):
            # zoom/mark bounds span empty space between their parts: those need a precise hit
            if layer.visible and not layer.locked and (not types or layer.type in types) \
                    and layer.type not in ("zoom", "mark") \
                    and shapes.bounds(layer).adjusted(-pad, -pad, pad, pad).contains(ip):
                return layer
        for layer in reversed(self.doc.layers):
            if not layer.visible or layer.locked or (types and layer.type not in types):
                continue
            if shapes.hit(layer, ip, tol):
                return layer
        return None

    def handle_at(self, pos):
        if not self.selected or self.selected not in self.doc.layers or self.text_edit or self.multi:
            return None
        for key, pt in shapes.handles(self.selected).items():
            if QLineF(self.to_w(pt), pos).length() <= HANDLE_R + 3:
                return key
        return None

    def crop_handle_at(self, pos):
        if self.state != "edit" or not self.doc.crop:
            return None
        cw = self.rect_w(QRectF(self.doc.crop))
        for key, pt in shapes.rect_handle_points(cw).items():
            if abs(pt.x() - pos.x()) <= 7 and abs(pt.y() - pos.y()) <= 7:
                return key
        if self.tool == "pointer" and cw.adjusted(-5, -5, 5, 5).contains(pos) \
                and not cw.adjusted(5, 5, -5, -5).contains(pos):
            key = ""
            if abs(pos.y() - cw.top()) <= 5:
                key += "t"
            elif abs(pos.y() - cw.bottom()) <= 5:
                key += "b"
            if abs(pos.x() - cw.left()) <= 5:
                key += "l"
            elif abs(pos.x() - cw.right()) <= 5:
                key += "r"
            return key or None
        return None

    def new_layer(self, kind, g, push=True):
        snap = self.doc.snapshot()
        layer = self.doc.new_layer(kind, g, self.styles[kind])
        if push:
            self.doc.push_undo(snap)
        self.selected = layer
        return layer, snap

    def discard_last(self):
        if self.doc.undo_stack:
            self.doc.restore(self.doc.undo_stack.pop())
        self.selected = None

    # ------------------------------------------------------------------ text
    def start_text(self, layer, snap, new):
        self.text_edit = layer
        self._text = {"snap": snap, "new": new, "orig": layer.g.get("text", "")}
        self.selected = layer
        self._caret_on = True
        self.caret_timer.start()
        self.props.rebuild()
        self.invalidate()

    def finish_text(self):
        layer = self.text_edit
        if not layer:
            return
        self.text_edit = None
        self.caret_timer.stop()
        t = self._text
        if not layer.g["text"].strip():
            if layer in self.doc.layers:
                self.doc.layers.remove(layer)
            if not t["new"]:
                self.doc.push_undo(t["snap"])
            self.selected = None
        elif t["new"] or layer.g["text"] != t["orig"]:
            self.doc.push_undo(t["snap"])
        self.invalidate()
        self.refresh_panels()

    def _blink(self):
        self._caret_on = not self._caret_on
        self.update()

    # --------------------------------------------------------------- editing
    def begin_drag(self, d, layer=None, pushed=False):
        d.setdefault("snap", None if pushed else self.doc.snapshot())
        d["pushed"] = pushed
        d["press"] = self.mouse
        self.drag = d
        self._below = None
        if layer is not None and self.doc.crop and layer in self.doc.layers:
            idx = self.doc.index(layer)
            region = QRect(self.doc.crop)
            self._below = (region, idx, shapes.composite(self.doc, region, self.doc.layers[:idx]))

    def changed(self):
        d = self.drag
        if d and not d["pushed"]:
            self.doc.push_undo(d["snap"])
            d["pushed"] = True
        self.invalidate()

    def undo(self):
        self.finish_text()
        if self.doc.undo():
            self._after_history()

    def redo(self):
        self.finish_text()
        if self.doc.redo():
            self._after_history()

    def _after_history(self):
        sel, multi = self.selected, self.multi
        self.selected = self.doc.find(sel.id) if sel else None
        self.multi = {i for i in multi if self.doc.find(i)}
        self._below = None
        if not self.doc.crop and not self.editor:
            self.reset_selection()
            return
        self.invalidate()
        self.props.rebuild(force=True)
        self.refresh_panels()
        self.layout_panels()

    def delete_selected(self):
        layers = self.sel_layers()
        if not layers:
            return
        self.doc.push_undo()
        for l in layers:
            self.doc.layers.remove(l)
        self.selected = None
        self.invalidate()
        self.refresh_panels()

    def duplicate_selected(self):
        from .model import Layer
        src = self.sel_layers()
        if not src:
            return
        self.doc.push_undo()
        copies = []
        for l in reversed(src):
            d = l.to_dict()
            d.pop("id")
            self.doc.seq += 1
            d["seq"] = self.doc.seq
            if d["name"]:
                d["name"] += tr(" (copy)")
            layer = Layer.from_dict(d)
            shapes.translate(layer, 16, 16)
            self.doc.layers.insert(self.doc.index(l) + 1, layer)
            copies.append(layer)
        self.select_many({l.id for l in copies}, copies[0].id)
        self.invalidate()
        self.refresh_panels()

    def move_selected(self, direction):
        ids = {l.id for l in self.sel_layers()}
        L = self.doc.layers
        if not ids:
            return
        snap = self.doc.snapshot()
        moved = False
        order = range(len(L) - 1, -1, -1) if direction > 0 else range(len(L))
        for i in order:
            j = i + direction
            if L[i].id in ids and 0 <= j < len(L) and L[j].id not in ids:
                L[i], L[j] = L[j], L[i]
                moved = True
        if moved:
            self.doc.push_undo(snap)
            self.invalidate()
            self.refresh_panels()

    def reorder(self, ids):
        layers = [self.doc.find(i) for i in ids]
        if None in layers or len(layers) != len(self.doc.layers):
            return
        self.doc.push_undo()
        self.doc.layers = layers
        self.invalidate()
        self.refresh_panels()

    def nudge(self, dx, dy):
        layers = self.sel_layers()
        if layers:
            self.doc.push_undo(key=("nudge", tuple(l.id for l in layers)))
            for l in layers:
                shapes.translate(l, dx, dy)
        elif self.doc.crop:
            self.doc.push_undo(key=("nudge", "crop"))
            r = self.doc.crop.translated(dx, dy)
            if self.img_rect().toRect().contains(r):
                self.doc.crop = r
            self.layout_panels()
        self.invalidate()
        self.actions.sync()

    # --------------------------------------------------------------- mouse
    def enterEvent(self, e):
        if self.controller:
            self.controller.hover(self)

    def mousePressEvent(self, e):
        pos = e.position()
        self.mouse = pos
        self.setFocus()
        if self.controller:
            self.controller.hover(self)
        ip = self.to_img(pos)
        if self._pick_cb:
            cb, self._pick_cb = self._pick_cb, None
            if e.button() == Qt.LeftButton:
                cb(self.color_at(ip))
            self.update_cursor()
            return
        if self.popup:
            self.close_popup()
        if e.button() == Qt.MiddleButton and self.editor:
            self.drag = {"kind": "pan", "last": pos}
            self.setCursor(Qt.ClosedHandCursor)
            return
        if e.button() == Qt.RightButton:
            if self.text_edit:
                self.finish_text()
            elif self.state == "edit" and not self.editor and not self.doc.layers:
                self.reset_selection()
            elif self.tool != "pointer":
                self.set_tool("pointer")
            else:
                self.select(None)
            return
        if e.button() != Qt.LeftButton:
            return
        mods = e.modifiers()
        if self.state == "idle":
            if self.controller and not self.controller.can_start(self):
                self.toast(tr("Another screen already has drawings — finish or cancel them (Esc)"))
                return
            self.drag = {"kind": "select", "start": self.clamp_img(ip), "end": self.clamp_img(ip), "moved": False}
            self.state = "selecting"
            if self.controller:
                self.controller.lock(self)
            self.update()
            return
        if self.state != "edit":
            return

        if self.text_edit:
            if shapes.hit(self.text_edit, ip, self.tol()):
                return
            self.finish_text()
            if self.tool == "text":
                return

        key = self.handle_at(pos)
        if key:
            sel = self.selected
            self.begin_drag({"kind": "handle", "layer": sel, "key": key, "orig": copy.deepcopy(sel.g)}, sel)
            return
        ck = self.crop_handle_at(pos)
        if ck:
            self.begin_drag({"kind": "crop", "key": ck, "orig": QRectF(self.doc.crop)})
            return

        if not self.editor and not QRectF(self.doc.crop).contains(ip):
            self.select(None)
            self.drag = {"kind": "select", "start": self.clamp_img(ip), "end": self.clamp_img(ip),
                         "moved": False, "snap": self.doc.snapshot()}
            self.state = "selecting"
            self.update()
            return

        t = self.tool
        x, y = ip.x(), ip.y()
        if t == "pointer":
            layer = self.layer_at(ip)
            if layer and mods & Qt.ShiftModifier:
                self.select(layer, add=True)
                return
            if layer:
                group = self.sel_layers()
                if layer not in group or len(group) < 2:
                    self.select(layer)
                    group = [layer]
                partial = layer.type in ("mark", "zoom")
                whole = len(group) > 1 or not partial or bool(mods & Qt.ControlModifier)
                part = None if whole else shapes.hit(layer, ip, self.tol())
                self.begin_drag({"kind": "move", "layer": layer, "layers": group, "last": ip, "whole": whole,
                                 "part": part}, group[0])
                return
            self.select(None)
            if QRectF(self.doc.crop).contains(ip):
                self.begin_drag({"kind": "cropmove", "start": ip, "orig": QRectF(self.doc.crop)})
            self.update()
            return
        if t == "mark":
            layer = self.layer_at(ip, ("mark",))
            if layer:
                self.select(layer)
                self.begin_drag({"kind": "move", "layer": layer, "last": ip,
                                 "whole": bool(mods & Qt.ControlModifier)}, layer)
                return
            layer, _ = self.new_layer("mark", {"anchor": [x, y], "pos": [x, y]})
            self.begin_drag({"kind": "mark_new", "layer": layer}, layer, pushed=True)
            self.refresh_panels()
            return
        if t == "zoom":
            layer = self.layer_at(ip, ("zoom",))
            if layer:
                self.select(layer)
                whole = bool(mods & Qt.ControlModifier)
                part = None if whole else shapes.hit(layer, ip, self.tol())
                self.begin_drag({"kind": "move", "layer": layer, "last": ip, "whole": whole, "part": part}, layer)
                return
            layer, _ = self.new_layer("zoom", {"src": [x, y, 0, 0], "dst": [x, y, 0, 0]})
            self.begin_drag({"kind": "draw", "layer": layer, "start": ip}, layer, pushed=True)
            self.props.rebuild()
            return
        if t == "text":
            layer = self.layer_at(ip, ("text",))
            if layer:
                self.select(layer)
                self.start_text(layer, self.doc.snapshot(), False)
                return
            size = self.styles["text"]["size"]
            layer, snap = self.new_layer("text", {"pos": [x, y - size * 0.65], "text": ""}, push=False)
            self.start_text(layer, snap, True)
            return
        if t in ("pen", "highlighter"):
            g = {"pts": [[x, y]]}
        elif t in ("line", "arrow"):
            g = {"p1": [x, y], "p2": [x, y]}
        else:
            g = {"r": [x, y, 0, 0]}
        layer, _ = self.new_layer(t, g)
        self.begin_drag({"kind": "draw", "layer": layer, "start": ip}, layer, pushed=True)
        self.props.rebuild()

    def mouseMoveEvent(self, e):
        pos = e.position()
        self.mouse = pos
        ip = self.to_img(pos)
        d = self.drag
        if not d:
            if self.state == "idle":
                self.update_hover(ip)
            self.update_cursor(pos, ip)
            if not self.editor and (self.state != "edit" or self._pick_cb):
                self.update()
            return
        shift = bool(e.modifiers() & Qt.ShiftModifier)
        kind = d["kind"]
        if kind == "pan":
            self.offset += pos - d["last"]
            d["last"] = pos
            self.user_view = True
            self.layout_panels()
            self.update()
            return
        if kind == "select":
            d["end"] = self.clamp_img(ip)
            if not d["moved"] and QLineF(d["start"], d["end"]).length() * self.view_scale > 4:
                d["moved"] = True
                for w in (self.toolbar, self.actions, self.props, self.layers_panel):
                    w.hide()
                self.close_popup()
            self.update()
            return
        layer = d.get("layer")
        if kind == "draw":
            t = layer.type
            if t in ("pen", "highlighter"):
                last = layer.g["pts"][-1]
                if math.hypot(ip.x() - last[0], ip.y() - last[1]) * self.view_scale >= 1.5:
                    layer.g["pts"].append([ip.x(), ip.y()])
            elif t in ("line", "arrow"):
                p2 = shapes.snap_angle(d["start"], ip) if shift else ip
                layer.g["p2"] = [p2.x(), p2.y()]
            else:
                r = shapes.resize_rect(QRectF(d["start"], d["start"]), "br", ip, shift)
                if ip.x() < d["start"].x() or ip.y() < d["start"].y():
                    r = QRectF(d["start"], ip).normalized()
                    if shift:
                        s = max(r.width(), r.height())
                        r = QRectF(d["start"], d["start"] + QPointF(
                            math.copysign(s, ip.x() - d["start"].x()),
                            math.copysign(s, ip.y() - d["start"].y()))).normalized()
                if t == "zoom":
                    layer.g["src"] = layer.g["dst"] = [r.x(), r.y(), r.width(), r.height()]
                else:
                    layer.g["r"] = [r.x(), r.y(), r.width(), r.height()]
            self.changed()
        elif kind == "mark_new":
            layer.g["pos"] = [ip.x(), ip.y()]
            self.changed()
        elif kind == "move":
            dx, dy = ip.x() - d["last"].x(), ip.y() - d["last"].y()
            if dx or dy:
                for l in d.get("layers") or [layer]:
                    shapes.translate(l, dx, dy, d["whole"], d.get("part"))
                d["last"] = ip
                self.changed()
        elif kind == "handle":
            shapes.drag_handle(layer, d["key"], ip, d["orig"], shift)
            self.changed()
        elif kind in ("crop", "cropmove"):
            if kind == "crop":
                r = shapes.resize_rect(d["orig"], d["key"], self.clamp_img(ip))
            else:
                r = d["orig"].translated(ip - d["start"])
                b = self.img_rect()
                r.moveLeft(max(0, min(r.left(), b.width() - r.width())))
                r.moveTop(max(0, min(r.top(), b.height() - r.height())))
            rr = QRect(round(r.x()), round(r.y()), max(1, round(r.width())), max(1, round(r.height())))
            if rr != self.doc.crop:
                self.doc.crop = rr
                self.changed()
                self.layout_panels()
                self.actions.sync()

    def mouseReleaseEvent(self, e):
        d = self.drag
        if not d:
            return
        self.drag = None
        self._below = None
        kind = d["kind"]
        if kind == "pan":
            self.update_cursor()
            return
        if kind == "select":
            if not d["moved"]:
                if d.get("snap"):  # plain click outside an existing selection: keep it
                    self.state = "edit"
                    self.update()
                    return
                # click with no selection yet: wait to see whether it is a double click
                self.state = "idle"
                if self.controller:
                    self.controller.unlock()
                self._pending_win = self.hover_win
                self.click_timer.start(QApplication.doubleClickInterval())
                self.update()
                return
            r = QRectF(d["start"], d["end"]).normalized().toAlignedRect()
            r = r & self.doc.base.rect()
            if r.width() < 2 or r.height() < 2:
                r = self.doc.base.rect()
            if d.get("snap") and self.doc.crop:
                self.doc.push_undo(d["snap"])
            self.doc.crop = r
            self.hover_win = None
            self.enter_edit()
            return
        layer = d.get("layer")
        if kind == "draw":
            t = layer.type
            degenerate = False
            if t in ("line", "arrow"):
                degenerate = QLineF(shapes.P(layer.g["p1"]), shapes.P(layer.g["p2"])).length() < 3
            elif t in shapes.RECT_TYPES:
                degenerate = layer.g["r"][2] < 3 or layer.g["r"][3] < 3
            elif t == "zoom":
                degenerate = layer.g["src"][2] < 3 or layer.g["src"][3] < 3
                if not degenerate:
                    layer.g["src"] = list(layer.g["src"])  # src and dst must not share one list
                    layer.g["dst"] = list(layer.g["src"])
                    self.toast(tr("Drag the copy aside · copy corners — magnification · Ctrl — move as a whole"))
            if degenerate:
                self.discard_last()
        self.invalidate()
        self.refresh_panels()
        if layer is not None and layer.type == "zoom":
            self.props.rebuild(force=True)  # refresh the "×N" magnification readout
        self.update_cursor()

    def _single_click(self):
        win, self._pending_win = self._pending_win, None
        if not win or self.state != "idle":
            return
        if self.controller:
            if not self.controller.can_start(self):
                return
            self.controller.lock(self)
        self.doc.crop = win[0].toAlignedRect() & self.doc.base.rect()
        self.hover_win = None
        self.enter_edit()

    def select_full_screen(self):
        if self.state == "edit":
            self.doc.push_undo()
        self.doc.crop = self.doc.base.rect()
        self.hover_win = None
        self.enter_edit()

    def mouseDoubleClickEvent(self, e):
        ip = self.to_img(e.position())
        if not self.editor and e.button() == Qt.LeftButton:
            if self.state == "idle":
                self.click_timer.stop()
                self._pending_win = None
                if self.controller:
                    if not self.controller.can_start(self):
                        return
                    self.controller.lock(self)
                self.select_full_screen()
                return
            if self.state == "edit" and not QRectF(self.doc.crop).contains(ip) \
                    and not self.crop_handle_at(e.position()):
                self.select_full_screen()
                return
        if self.state == "edit" and self.tool in ("pointer", "text"):
            layer = self.layer_at(ip, ("text",))
            if layer:
                self.select(layer)
                self.start_text(layer, self.doc.snapshot(), False)
                return
        if self.state == "edit" and not self.editor and self.tool == "pointer" and not self.layer_at(ip) \
                and QRectF(self.doc.crop).contains(ip):
            self.actionRequested.emit("copy")
            return
        # Qt delivers a quick second press as a double click: with no special meaning here it is a press,
        # otherwise a fast re-press (twitchy click, touchpad drag hiccup) would be silently swallowed
        self.mousePressEvent(e)

    def wheelEvent(self, e):
        dy = e.angleDelta().y()
        if not dy:
            return
        if self.editor and e.modifiers() & Qt.ControlModifier:
            self.zoom_at(1.15 if dy > 0 else 1 / 1.15, e.position())
            return
        if self.state != "edit":
            return
        target = self.props_target()
        key = next((k for k in ("width", "size", "strength") if k in target[1]), None) if target else None
        if key:
            st = target[1]
            step = (1 if dy > 0 else -1) * (2 if key == "size" else 1)
            self.apply_style(key, max(1, st[key] + step))
            self.props.rebuild(force=True)
            self.toast(f"{tr({'width': 'Width', 'size': 'Size', 'strength': 'Strength'}[key])}: {st[key]}")
        elif self.editor:
            self.offset += QPointF(e.angleDelta().x(), dy) / 2
            self.user_view = True
            self.layout_panels()
            self.update()

    def update_hover(self, ip):
        hw = None
        for r, title in self.windows:
            if r.contains(ip):
                hw = (r, title)
                break
        if hw != self.hover_win:
            self.hover_win = hw

    def update_cursor(self, pos=None, ip=None):
        if self._pick_cb:
            self.setCursor(self.cross)
            return
        if not self.editor and not self.active:
            self.setCursor(Qt.ArrowCursor)
            return
        if self.state in ("idle", "selecting"):
            self.setCursor(self.cross)
            return
        pos = pos or self.mouse
        if pos is None:
            self.setCursor(self.cross if self.tool != "pointer" else Qt.ArrowCursor)
            return
        ip = ip or self.to_img(pos)
        key = self.handle_at(pos)
        if key:
            self.setCursor(RECT_CURSORS.get(key, Qt.SizeAllCursor))
            return
        ck = self.crop_handle_at(pos)
        if ck:
            self.setCursor(RECT_CURSORS.get(ck, Qt.SizeAllCursor))
            return
        t = self.tool
        if t == "pointer":
            if self.layer_at(ip):
                self.setCursor(Qt.SizeAllCursor)
            elif self.doc.crop and QRectF(self.doc.crop).contains(ip):
                self.setCursor(Qt.OpenHandCursor)
            else:
                self.setCursor(self.cross if not self.editor else Qt.ArrowCursor)
        elif t == "text":
            self.setCursor(Qt.IBeamCursor)
        elif t == "mark" and self.layer_at(ip, ("mark",)):
            self.setCursor(Qt.SizeAllCursor)
        else:
            self.setCursor(self.cross)

    # ------------------------------------------------------------- keyboard
    def keyPressEvent(self, e):
        if self.controller:
            target = self.controller.key_target()
            if target is not self:
                target.keyPressEvent(e)
                return
        k, mods = e.key(), e.modifiers()
        ctrl, shift = bool(mods & Qt.ControlModifier), bool(mods & Qt.ShiftModifier)
        if self.text_edit:
            self.text_key(e)
            return
        if k == Qt.Key_Escape:
            if self._pick_cb:
                self._pick_cb = None
                self.update_cursor()
            elif self.popup:
                self.close_popup()
            elif self.editor:
                self.select(None)
            else:
                self.actionRequested.emit("cancel")
            return
        if self.state != "edit":
            if k == Qt.Key_C and not ctrl and self.mouse is not None:
                c = self.color_at(self.to_img(self.mouse)).name()
                QGuiApplication.clipboard().setText(c)
                from .compositor import copy_text
                copy_text(c)
                self.toast(tr("Color {c} copied").format(c=c))
            elif (k in (Qt.Key_Return, Qt.Key_Enter, Qt.Key_Space)) or (ctrl and k == Qt.Key_A):
                if self.controller and self.controller.can_start(self):
                    self.controller.lock(self)
                    self.doc.crop = self.doc.base.rect()
                    self.enter_edit()
            return
        if ctrl and k == Qt.Key_Z:
            self.redo() if shift else self.undo()
        elif ctrl and k == Qt.Key_Y:
            self.redo()
        elif (ctrl and k == Qt.Key_C) or (not ctrl and k in (Qt.Key_Return, Qt.Key_Enter)):
            self.actionRequested.emit("copy")
        elif ctrl and k == Qt.Key_S:
            self.actionRequested.emit("export" if shift and self.editor else "save")
        elif ctrl and k == Qt.Key_E and not self.editor:
            self.actionRequested.emit("edit")
        elif ctrl and k == Qt.Key_D:
            self.duplicate_selected()
        elif ctrl and k == Qt.Key_A:
            self.select_many({l.id for l in self.doc.layers})
            self.layers_panel.refresh()
        elif ctrl and k == Qt.Key_BracketRight:
            self.move_selected(1)
        elif ctrl and k == Qt.Key_BracketLeft:
            self.move_selected(-1)
        elif ctrl and k == Qt.Key_0 and self.editor:
            self.fit_view()
        elif k in (Qt.Key_Delete, Qt.Key_Backspace):
            self.delete_selected()
        elif k == Qt.Key_Tab:
            self.toggle_layers()
        elif k in (Qt.Key_Left, Qt.Key_Right, Qt.Key_Up, Qt.Key_Down):
            step = 10 if shift else 1
            dx = {Qt.Key_Left: -step, Qt.Key_Right: step}.get(k, 0)
            dy = {Qt.Key_Up: -step, Qt.Key_Down: step}.get(k, 0)
            self.nudge(dx, dy)
        elif not ctrl and k in TOOL_KEYS:
            self.set_tool(TOOL_KEYS[k])

    def focusNextPrevChild(self, nxt):
        return False  # keep Tab for the layers panel toggle

    def text_key(self, e):
        layer = self.text_edit
        k, mods = e.key(), e.modifiers()
        ctrl = bool(mods & Qt.ControlModifier)
        txt = layer.g["text"]
        if k == Qt.Key_Escape or (ctrl and k in (Qt.Key_Return, Qt.Key_Enter)):
            self.finish_text()
            return
        if k in (Qt.Key_Return, Qt.Key_Enter):
            txt += "\n"
        elif k == Qt.Key_Backspace:
            if ctrl:
                txt = txt.rstrip()
                cut = max(txt.rfind(" "), txt.rfind("\n"))
                txt = txt[:cut + 1] if cut >= 0 else ""
            else:
                txt = txt[:-1]
        elif ctrl and k == Qt.Key_V:
            txt += QGuiApplication.clipboard().text()
        elif ctrl and k == Qt.Key_Z:
            self.finish_text()
            self.undo()
            return
        elif e.text() and e.text().isprintable() and not ctrl:
            txt += e.text()
        else:
            return
        layer.g["text"] = txt
        self._caret_on = True
        self.invalidate()

    # --------------------------------------------------------------- paint
    def current_crop(self):
        d = self.drag
        if d and d["kind"] == "select":
            return QRectF(d["start"], d["end"]).normalized() if d["moved"] else None
        if self.state == "edit" and self.doc.crop:
            return QRectF(self.doc.crop)
        return None

    def ensure_cache(self):
        if not self._dirty and self._cache is not None:
            return
        region = QRect(self.doc.crop)
        b = self._below
        if b and b[0] == region and b[1] <= len(self.doc.layers):
            self._cache = shapes.composite(self.doc, region, self.doc.layers[b[1]:], start=b[2])
        else:
            self._cache = shapes.composite(self.doc, region, self.doc.layers)
        self._dirty = False

    def _glow_tick(self):
        if not self.active or not self.isVisible():
            return
        self.pulse = (self.pulse + 0.09) % (2 * math.pi)
        r = self.rect()
        self.update(QRegion(r).subtracted(QRegion(r.adjusted(18, 18, -18, -18))))

    def paintEvent(self, e):
        p = QPainter(self)
        s = self.view_scale
        if self.editor:
            p.fillRect(self.rect(), QColor("#141418"))
        p.save()
        p.translate(self.offset)
        p.scale(s, s)
        if self.editor:
            p.setRenderHint(QPainter.SmoothPixmapTransform, s < 1)
        p.drawImage(QPointF(0, 0), self.doc.base)
        p.restore()

        crop = self.current_crop()
        dim = QColor(0, 0, 0, 150 if self.editor else (115 if self.active else 170))
        path = QPainterPath()
        path.addRect(self.rect_w(self.img_rect()))
        cw = self.rect_w(crop) if crop else None
        if cw:
            path.addRect(cw)
        p.fillPath(path, dim)

        if self.state == "edit" and self.doc.crop:
            self.ensure_cache()
            p.save()
            p.setRenderHint(QPainter.SmoothPixmapTransform, self.editor and s < 1)
            p.drawImage(cw, self._cache)
            p.restore()

        if self.state == "idle" and self.active and not self.editor:
            if self.hover_win:
                wr = self.rect_w(self.hover_win[0])
                c = QColor(self.accent)
                c.setAlpha(28)
                p.fillRect(wr, c)
                p.setPen(QPen(self.accent, 2))
                p.drawRect(wr.adjusted(1, 1, -1, -1))
                draw_pill(p, self.hover_win[1] or tr("Window"), wr.topLeft() + QPointF(8, 8), "tl", QRectF(self.rect()))

        if cw:
            p.setRenderHint(QPainter.Antialiasing, False)
            p.setPen(QPen(self.accent, 1.5))
            p.setBrush(Qt.NoBrush)
            p.drawRect(cw)
            if self.state == "edit" and not (self.drag and self.drag["kind"] in ("draw", "move", "mark_new")):
                p.setRenderHint(QPainter.Antialiasing)
                p.setPen(QPen(self.accent, 1.5))
                p.setBrush(QColor("#f5f5f7"))
                for pt in shapes.rect_handle_points(cw).values():
                    p.drawRoundedRect(QRectF(pt.x() - 4, pt.y() - 4, 8, 8), 2, 2)
            if self.state == "selecting" or (self.drag and self.drag["kind"] in ("crop", "cropmove")):
                r = crop.toAlignedRect()
                txt = f"{r.width()} × {r.height()}"
                draw_pill(p, txt, cw.topLeft() - QPointF(0, 6), "bl", QRectF(self.rect()))

        if self.state == "edit":
            self.paint_selection(p)

        if not self.editor and self.active:
            self.paint_glow(p)
            if self.state == "idle":
                draw_pill(p, tr("Select an area  ·  click — window  ·  double click / Enter — full screen  ·  "
                                "C — copy color  ·  Esc — cancel"),
                          QPointF(self.width() / 2, 22), "tc")
        if self.mouse is not None and not self.editor and self.active and (
                self.state in ("idle", "selecting") or self._pick_cb):
            self.paint_loupe(p)
        elif self.mouse is not None and self.editor and self._pick_cb:
            self.paint_loupe(p)
        if self._toast and time.monotonic() - self._toast[1] < 1.5:
            draw_pill(p, self._toast[0], QPointF(self.width() / 2, self.height() - 40), "bc", accent=self.accent)

    def paint_selection(self, p):
        sel = self.selected
        if sel and sel in self.doc.layers and not (self.drag and self.drag["kind"] == "draw"):
            p.save()
            p.setRenderHint(QPainter.Antialiasing)
            p.setBrush(Qt.NoBrush)
            for l in self.sel_layers():
                br = self.rect_w(shapes.bounds(l)).adjusted(-3, -3, 3, 3)
                p.setPen(QPen(QColor(255, 255, 255, 200), 1, Qt.DashLine))
                p.drawRect(br)
                pen = QPen(self.accent, 1, Qt.DashLine)
                pen.setDashOffset(4)
                p.setPen(pen)
                p.drawRect(br)
            if not self.text_edit and not self.multi:
                p.setPen(QPen(self.accent, 1.5))
                p.setBrush(QColor("#ffffff"))
                for key, pt in shapes.handles(sel).items():
                    w = self.to_w(pt)
                    if key == "anchor":
                        p.setBrush(self.accent)
                    p.drawEllipse(w, HANDLE_R - 1, HANDLE_R - 1)
            p.restore()
        if self.text_edit and self._caret_on and self.text_edit in self.doc.layers:
            line = shapes.text_caret(self.text_edit)
            p.save()
            p.setPen(QPen(self.accent, 2))
            p.drawLine(QLineF(self.to_w(line.p1()), self.to_w(line.p2())))
            p.restore()

    def paint_glow(self, p):
        r = QRectF(self.rect())
        a = 0.7 + 0.3 * math.sin(self.pulse)
        p.save()
        p.setRenderHint(QPainter.Antialiasing, False)
        p.setBrush(Qt.NoBrush)
        n = 16
        for i in range(n):
            c = QColor(self.accent)
            c.setAlphaF(max(0.0, min(1.0, a * 0.85 * (1 - i / n) ** 2.2)))
            p.setPen(QPen(c, 1))
            p.drawRect(r.adjusted(i + 0.5, i + 0.5, -i - 0.5, -i - 0.5))
        p.setPen(QPen(self.accent, 2))
        p.drawRect(r.adjusted(1, 1, -1, -1))
        p.restore()

    def paint_loupe(self, p):
        ip = self.to_img(self.mouse)
        ix, iy = int(ip.x()), int(ip.y())
        n, z = 15, 9
        size = n * z
        src_img = self.doc.base
        sub = src_img.copy(QRect(ix - n // 2, iy - n // 2, n, n))
        x, y = self.mouse.x() + 26, self.mouse.y() + 26
        if x + size > self.width() - 10:
            x = self.mouse.x() - 26 - size
        if y + size + 30 > self.height() - 10:
            y = self.mouse.y() - 26 - size - 30
        r = QRectF(x, y, size, size)
        p.save()
        p.setRenderHint(QPainter.Antialiasing)
        clip = QPainterPath()
        clip.addRoundedRect(r, 12, 12)
        p.setClipPath(clip)
        p.fillRect(r, Qt.black)
        p.drawImage(r, sub)
        p.setPen(QPen(QColor(255, 255, 255, 22), 1))
        for i in range(1, n):
            p.drawLine(QPointF(r.left() + i * z, r.top()), QPointF(r.left() + i * z, r.bottom()))
            p.drawLine(QPointF(r.left(), r.top() + i * z), QPointF(r.right(), r.top() + i * z))
        cx = r.left() + (n // 2) * z
        cy = r.top() + (n // 2) * z
        p.setPen(QPen(Qt.black, 3))
        p.drawRect(QRectF(cx, cy, z, z))
        p.setPen(QPen(Qt.white, 1.5))
        p.drawRect(QRectF(cx, cy, z, z))
        p.setClipping(False)
        p.setPen(QPen(self.accent, 2))
        p.setBrush(Qt.NoBrush)
        p.drawRoundedRect(r, 12, 12)
        p.restore()
        c = src_img.pixelColor(ix, iy) if src_img.rect().contains(ix, iy) else QColor(Qt.black)
        draw_pill(p, f"{ix}, {iy}   {c.name().upper()}", QPointF(r.center().x(), r.bottom() + 6), "tc")

    # --------------------------------------------------------------- misc
    def resizeEvent(self, e):
        if self.editor and not self.user_view:
            self.fit_view()
        self.layout_panels()
