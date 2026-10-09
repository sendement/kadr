"""Inline color picker (no separate toplevel windows — those would hide under the overlay)."""
from PySide6.QtCore import QPointF, QRectF, Qt, Signal
from PySide6.QtGui import QBrush, QColor, QLinearGradient, QPainter, QPen
from PySide6.QtWidgets import (QFrame, QGridLayout, QHBoxLayout, QLabel, QLineEdit, QToolButton,
                               QVBoxLayout, QWidget)

from . import icons
from .i18n import tr

PALETTE = ["#ffff3b30", "#ffff9500", "#ffffcc00", "#ff34c759", "#ff00c7be", "#ff30b0ff", "#ff007aff",
           "#ff5856d6", "#ffaf52de", "#ffff2d55", "#ffffffff", "#ff8e8e93", "#ff3a3a3c", "#ff000000"]


def checker(p, r, size=5):
    p.save()
    p.setClipRect(r)
    p.fillRect(r, QColor("#cfcfcf"))
    y = int(r.top())
    row = 0
    while y < r.bottom():
        x = int(r.left()) + (size if row % 2 else 0)
        while x < r.right():
            p.fillRect(QRectF(x, y, size, size), QColor("#8f8f8f"))
            x += size * 2
        y += size
        row += 1
    p.restore()


def paint_swatch(p, r, color, radius=6):
    p.save()
    p.setRenderHint(QPainter.Antialiasing)
    path_r = QRectF(r)
    p.setPen(Qt.NoPen)
    if color is None:
        p.setBrush(QColor(255, 255, 255, 20))
        p.drawRoundedRect(path_r, radius, radius)
        p.setPen(QPen(QColor("#ff453a"), 2))
        p.drawLine(path_r.bottomLeft() + QPointF(4, -4), path_r.topRight() + QPointF(-4, 4))
    else:
        c = QColor(color)
        if c.alpha() < 255:
            p.setClipRect(path_r)
            checker(p, path_r)
        p.setBrush(c)
        p.drawRoundedRect(path_r, radius, radius)
    p.setClipping(False)
    p.setBrush(Qt.NoBrush)
    p.setPen(QPen(QColor(255, 255, 255, 70), 1))
    p.drawRoundedRect(path_r.adjusted(0.5, 0.5, -0.5, -0.5), radius, radius)
    p.restore()


class ColorButton(QToolButton):
    colorChanged = Signal(object)  # "#aarrggbb" or None

    def __init__(self, canvas, color, allow_none=False, tip=""):
        super().__init__()
        self.canvas, self.color, self.allow_none = canvas, color, allow_none
        self.setFixedSize(28, 28)
        self.setToolTip(tip)
        self.setCursor(Qt.PointingHandCursor)
        self.clicked.connect(lambda: canvas.open_color_popup(self))

    def set_color(self, c, emit=True):
        self.color = c
        self.update()
        if emit:
            self.colorChanged.emit(c)

    def paintEvent(self, e):
        p = QPainter(self)
        paint_swatch(p, QRectF(self.rect()).adjusted(3, 3, -3, -3), self.color)
        if self.underMouse():
            p.setRenderHint(QPainter.Antialiasing)
            p.setPen(QPen(QColor(255, 255, 255, 140), 1.5))
            p.drawRoundedRect(QRectF(self.rect()).adjusted(1, 1, -1, -1), 7, 7)


class SVArea(QWidget):
    changed = Signal()

    def __init__(self):
        super().__init__()
        self.h, self.s, self.v = 0.0, 1.0, 1.0
        self.setFixedSize(200, 130)
        self.setCursor(Qt.CrossCursor)

    def paintEvent(self, e):
        p = QPainter(self)
        r = QRectF(self.rect())
        p.fillRect(r, QColor.fromHsvF(self.h, 1, 1))
        g = QLinearGradient(r.topLeft(), r.topRight())
        g.setColorAt(0, QColor(255, 255, 255))
        g.setColorAt(1, QColor(255, 255, 255, 0))
        p.fillRect(r, g)
        g = QLinearGradient(r.topLeft(), r.bottomLeft())
        g.setColorAt(0, QColor(0, 0, 0, 0))
        g.setColorAt(1, QColor(0, 0, 0))
        p.fillRect(r, g)
        p.setRenderHint(QPainter.Antialiasing)
        c = QPointF(self.s * r.width(), (1 - self.v) * r.height())
        p.setPen(QPen(Qt.black, 3))
        p.drawEllipse(c, 6, 6)
        p.setPen(QPen(Qt.white, 1.5))
        p.drawEllipse(c, 6, 6)

    def _set(self, pos):
        self.s = min(1, max(0, pos.x() / self.width()))
        self.v = 1 - min(1, max(0, pos.y() / self.height()))
        self.update()
        self.changed.emit()

    def mousePressEvent(self, e):
        self._set(e.position())

    def mouseMoveEvent(self, e):
        self._set(e.position())


class Bar(QWidget):
    """Horizontal hue or alpha strip."""
    changed = Signal()

    def __init__(self, kind):
        super().__init__()
        self.kind, self.value, self.base = kind, 0.0, QColor(Qt.red)
        self.setFixedSize(200, 14)
        self.setCursor(Qt.PointingHandCursor)

    def paintEvent(self, e):
        p = QPainter(self)
        p.setRenderHint(QPainter.Antialiasing)
        r = QRectF(self.rect()).adjusted(0, 2, 0, -2)
        g = QLinearGradient(r.topLeft(), r.topRight())
        if self.kind == "hue":
            for i in range(7):
                g.setColorAt(i / 6, QColor.fromHsvF(min(i / 6, 0.9999), 1, 1))
        else:
            checker(p, r, 4)
            c0 = QColor(self.base)
            c0.setAlpha(0)
            c1 = QColor(self.base)
            c1.setAlpha(255)
            g.setColorAt(0, c0)
            g.setColorAt(1, c1)
        p.setPen(Qt.NoPen)
        p.setBrush(QBrush(g))
        p.drawRoundedRect(r, 4, 4)
        x = self.value * (self.width() - 1)
        p.setPen(QPen(Qt.black, 3))
        p.drawRoundedRect(QRectF(x - 3, 0, 6, self.height()), 2, 2)
        p.setPen(QPen(Qt.white, 1.5))
        p.drawRoundedRect(QRectF(x - 3, 0, 6, self.height()), 2, 2)

    def _set(self, pos):
        self.value = min(1, max(0, pos.x() / self.width()))
        self.update()
        self.changed.emit()

    def mousePressEvent(self, e):
        self._set(e.position())

    def mouseMoveEvent(self, e):
        self._set(e.position())


class Swatch(QToolButton):
    def __init__(self, color, cb):
        super().__init__()
        self.color = color
        self.setFixedSize(22, 22)
        self.setToolTip(QColor(color).name(QColor.HexArgb) if color else tr("None"))
        self.setCursor(Qt.PointingHandCursor)
        self.clicked.connect(lambda: cb(self.color))

    def paintEvent(self, e):
        p = QPainter(self)
        paint_swatch(p, QRectF(self.rect()).adjusted(2, 2, -2, -2), self.color, 5)


class ColorPopup(QFrame):
    def __init__(self, canvas, button):
        super().__init__(canvas)
        self.setObjectName("panel")
        self.setAttribute(Qt.WA_StyledBackground)
        # clicks/wheel on the panel background must not fall through to the canvas
        self.setAttribute(Qt.WA_NoMousePropagation)
        self.setCursor(Qt.ArrowCursor)
        self.canvas, self.button = canvas, button
        lay = QVBoxLayout(self)
        lay.setContentsMargins(10, 10, 10, 10)
        lay.setSpacing(8)

        grid = QGridLayout()
        grid.setSpacing(3)
        for i, c in enumerate(PALETTE):
            grid.addWidget(Swatch(c, self.pick), i // 7, i % 7)
        lay.addLayout(grid)
        recent = canvas.cfg.get("recent_colors") or []
        if recent:
            row = QHBoxLayout()
            row.setSpacing(3)
            lbl = QLabel(tr("Recent"))
            row.addWidget(lbl)
            for c in recent[:6]:
                row.addWidget(Swatch(c, self.pick))
            row.addStretch()
            lay.addLayout(row)

        self.sv = SVArea()
        self.hue = Bar("hue")
        self.alpha = Bar("alpha")
        lay.addWidget(self.sv)
        lay.addWidget(self.hue)
        lay.addWidget(self.alpha)

        row = QHBoxLayout()
        row.setSpacing(4)
        self.hex = QLineEdit()
        self.hex.setFixedWidth(96)
        self.hex.returnPressed.connect(self._from_hex)
        row.addWidget(self.hex)
        dropper = QToolButton()
        dropper.setIcon(icons.icon("eyedropper"))
        dropper.setToolTip(tr("Eyedropper: pick a color from the screenshot"))
        dropper.clicked.connect(self._eyedropper)
        dropper.setCursor(Qt.PointingHandCursor)
        row.addWidget(dropper)
        if button.allow_none:
            none = QToolButton()
            none.setText(tr("None"))
            none.setToolTip(tr("No color"))
            none.clicked.connect(lambda: self.pick(None))
            none.setCursor(Qt.PointingHandCursor)
            row.addWidget(none)
        row.addStretch()
        lay.addLayout(row)

        self.sv.changed.connect(self._from_ui)
        self.hue.changed.connect(self._from_ui)
        self.alpha.changed.connect(self._from_ui)
        self._load(button.color or "#ffff3b30")

    def _load(self, c):
        c = QColor(c)
        h, s, v, a = c.getHsvF()
        if h >= 0:
            self.sv.h = self.hue.value = h
        self.sv.s, self.sv.v = s, v
        self.alpha.value = a
        self._refresh(c)

    def _refresh(self, c):
        base = QColor(c)
        base.setAlpha(255)
        self.alpha.base = base
        self.hex.setText(c.name(QColor.HexArgb) if c.alpha() < 255 else c.name())
        for w in (self.sv, self.hue, self.alpha):
            w.update()

    def _from_ui(self):
        self.sv.h = self.hue.value
        c = QColor.fromHsvF(min(self.hue.value, 0.9999), self.sv.s, self.sv.v, self.alpha.value)
        self._refresh(c)
        self.button.set_color(c.name(QColor.HexArgb))

    def _from_hex(self):
        c = QColor(self.hex.text().strip())
        if c.isValid():
            self.pick(c.name(QColor.HexArgb))

    def _eyedropper(self):
        self.canvas.start_pick(lambda c: self.pick(c.name(QColor.HexArgb)))

    def pick(self, c):
        if c is not None:
            self._load(c)
        self.button.set_color(c)

    def hideEvent(self, e):
        self.canvas.cfg.add_recent_color(self.button.color)
        super().hideEvent(e)
