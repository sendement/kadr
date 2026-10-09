"""Pinned always-on-top screenshot window."""
import os

from PySide6.QtCore import QRectF, QSize, Qt, QTimer
from PySide6.QtGui import QColor, QPainter, QPen
from PySide6.QtWidgets import QFileDialog, QMenu, QWidget

from . import compositor, outputs


class PinWindow(QWidget):
    count = 0

    def __init__(self, img, scale=1.0, pos=None, cfg=None):
        super().__init__(None, Qt.Window | Qt.FramelessWindowHint)
        PinWindow.count += 1
        self.tag = f"kadr-pin-{os.getpid()}-{PinWindow.count}"
        self.setWindowTitle(self.tag)
        self.setAttribute(Qt.WA_DeleteOnClose)
        self.setAttribute(Qt.WA_TranslucentBackground)
        self.img, self.cfg, self.pos0 = img, cfg or {}, pos
        self.base = QSize(round(img.width() / scale), round(img.height() / scale))
        self.zoom, self.alpha = 1.0, 1.0
        self.accent = QColor(self.cfg.get("accent", "#ff4cc2ff"))
        self.setToolTip("Колесо — масштаб, Ctrl+колесо — прозрачность, двойной клик / Esc — закрыть, ПКМ — меню")
        self.resize(self.base)

    def _target(self):
        return QSize(max(40, round(self.base.width() * self.zoom)), max(30, round(self.base.height() * self.zoom)))

    def showEvent(self, e):
        super().showEvent(e)
        if not getattr(self, "_floated", False):
            self._floated = True
            self._setup(20)

    def _setup(self, tries):
        t = self._target()  # not self.size(): by now Qt already holds the tiled size
        if self.isVisible() and not compositor.hypr_float(self.tag, t.width(), t.height(),
                                                          self.pos0, pin=True) and tries:
            QTimer.singleShot(100, lambda: self._setup(tries - 1))

    def paintEvent(self, e):
        p = QPainter(self)
        p.setRenderHint(QPainter.SmoothPixmapTransform)
        p.setOpacity(self.alpha)
        p.drawImage(QRectF(self.rect()), self.img)
        p.setOpacity(1)
        p.setPen(QPen(self.accent, 1))
        p.drawRect(QRectF(self.rect()).adjusted(0.5, 0.5, -0.5, -0.5))

    def mousePressEvent(self, e):
        if e.button() == Qt.LeftButton and self.windowHandle():
            self.windowHandle().startSystemMove()

    def mouseDoubleClickEvent(self, e):
        self.close()

    def keyPressEvent(self, e):
        if e.key() == Qt.Key_Escape:
            self.close()
        elif e.key() == Qt.Key_C and e.modifiers() & Qt.ControlModifier:
            outputs.copy(self.img)

    def wheelEvent(self, e):
        dy = e.angleDelta().y()
        if e.modifiers() & Qt.ControlModifier:
            self.alpha = min(1.0, max(0.15, self.alpha + (0.08 if dy > 0 else -0.08)))
            self.update()
            return
        self.zoom = min(8.0, max(0.1, self.zoom * (1.1 if dy > 0 else 1 / 1.1)))
        self._apply_size()

    def _apply_size(self):
        t = self._target()
        self.resize(t)
        compositor.hypr_resize(self.tag, t.width(), t.height())

    def contextMenuEvent(self, e):
        m = QMenu(self)
        m.addAction("Копировать", lambda: outputs.copy(self.img))
        m.addAction("Сохранить как…", self._save_as)
        m.addAction("Масштаб 100%", lambda: (setattr(self, "zoom", 1.0), self._apply_size()))
        m.addSeparator()
        m.addAction("Закрыть", self.close)
        m.exec(e.globalPos())

    def _save_as(self):
        path, _ = QFileDialog.getSaveFileName(self, "Сохранить снимок", str(outputs.save_dir(self.cfg) / "kadr.png"),
                                              "PNG (*.png);;JPEG (*.jpg);;WebP (*.webp)")
        if path:
            self.img.save(path)
