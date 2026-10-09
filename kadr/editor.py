"""Regular windows: layer editor for a history item and the history gallery."""
from datetime import datetime

from PySide6.QtCore import QSize, Qt, QTimer
from PySide6.QtGui import QDesktopServices, QIcon, QPixmap
from PySide6.QtCore import QUrl
from PySide6.QtWidgets import (QFileDialog, QHBoxLayout, QLabel, QListWidget, QListWidgetItem, QMainWindow,
                               QMessageBox, QToolButton, QVBoxLayout, QWidget)

from . import compositor, history, icons, outputs, shapes
from .canvas import Canvas
from .panels import stylesheet
from .pin import PinWindow
from .i18n import tr

_windows = set()  # keep python refs alive


def _keep(w):
    _windows.add(w)
    w.destroyed.connect(lambda *_: _windows.discard(w))
    return w


def float_window(win, tries=20):
    """Hyprland tiles new toplevels; turn ours into centered floating windows once mapped."""
    def attempt(n):
        w, h = win.want  # not win.size(): by now Qt already holds the tiled size
        if win.isVisible() and not compositor.hypr_float(win.windowTitle(), w, h) and n:
            QTimer.singleShot(100, lambda: attempt(n - 1))
    QTimer.singleShot(60, lambda: attempt(tries))


def fmt_date(iso):
    try:
        dt = datetime.fromisoformat(iso)
    except Exception:
        return iso or ""
    from PySide6.QtCore import QDateTime, QLocale
    from . import i18n
    return QLocale(i18n.current).toString(QDateTime(dt), QLocale.ShortFormat)


class EditorWindow(QMainWindow):
    def __init__(self, item_id, cfg, on_saved=None):
        super().__init__()
        self.setAttribute(Qt.WA_DeleteOnClose)
        self.item_id, self.cfg, self.on_saved = item_id, cfg, on_saved
        self.doc = history.load(item_id)
        self.canvas = Canvas(self.doc, cfg, editor=True)
        self.canvas.actionRequested.connect(self.on_action)
        self.setCentralWidget(self.canvas)
        self.setWindowTitle(f"Kadr — {fmt_date(self.doc.meta.get('created'))}")
        self.setWindowIcon(QIcon(icons.pixmap("mark", "#4cc2ff", 64)))
        self.want = (1400, 900)
        self.resize(*self.want)
        _keep(self)

    def showEvent(self, e):
        super().showEvent(e)
        if not getattr(self, "_floated", False):
            self._floated = True
            float_window(self)

    def on_action(self, a):
        img = shapes.render_result(self.doc)
        if a == "copy":
            outputs.copy(img)
            self.canvas.toast(tr("Copied to clipboard"))
        elif a == "save":
            history.save(self.doc, img, self.item_id)
            self.canvas.toast(tr("Saved to history"))
            if self.on_saved:
                self.on_saved(self.item_id)
        elif a == "export":
            path, _filter = QFileDialog.getSaveFileName(
                self, tr("Export"), str(outputs.save_dir(self.cfg) / f"Kadr_{self.item_id}.png"),
                "PNG (*.png);;JPEG (*.jpg);;WebP (*.webp)")
            if path:
                img.save(path)
                self.canvas.toast(tr("Exported: {path}").format(path=path))
        elif a == "pin":
            scale = float(self.doc.meta.get("scale", 1.0))
            _keep(PinWindow(img, scale, cfg=self.cfg)).show()

    def closeEvent(self, e):
        self.canvas.finish_text()
        if self.doc.dirty:
            r = QMessageBox.question(self, "Kadr", tr("Save changes to history?"),
                                     QMessageBox.Save | QMessageBox.Discard | QMessageBox.Cancel)
            if r == QMessageBox.Cancel:
                e.ignore()
                return
            if r == QMessageBox.Save:
                self.on_action("save")
        self.cfg.save()
        super().closeEvent(e)


class HistoryWindow(QMainWindow):
    def __init__(self, cfg):
        super().__init__()
        self.setAttribute(Qt.WA_DeleteOnClose)
        self.cfg = cfg
        self.setWindowTitle("Kadr — " + tr("Screenshot history"))
        self.setWindowIcon(QIcon(icons.pixmap("history", "#4cc2ff", 64)))
        self.want = (1180, 760)
        self.resize(*self.want)
        root = QWidget()
        root.setObjectName("root")
        root.setStyleSheet(stylesheet(cfg["accent"]) + """
            #root { background: #141418; }
            QListWidget { font-size: 12px; }
            QListWidget::item { padding: 8px; margin: 4px; border-radius: 10px; background: rgba(255,255,255,8); }
            QLabel#count { color: #8a8a94; }
        """)
        lay = QVBoxLayout(root)
        lay.setContentsMargins(12, 10, 12, 12)

        bar = QHBoxLayout()
        t = QLabel(tr("Screenshot history"))
        t.setObjectName("title")
        t.setStyleSheet("font-size: 15px;")
        bar.addWidget(t)
        self.count = QLabel()
        self.count.setObjectName("count")
        bar.addWidget(self.count)
        bar.addStretch()
        for name, text, cb in (("edit", tr("Edit"), self.open_selected),
                               ("copy", tr("Copy"), self.copy_selected),
                               ("export", tr("Export"), self.export_selected),
                               ("folder", tr("Folder"), self.open_folder),
                               ("trash", tr("Delete"), self.delete_selected)):
            b = QToolButton()
            b.setIcon(icons.icon(name))
            b.setText(text)
            b.setToolButtonStyle(Qt.ToolButtonTextBesideIcon)
            b.setObjectName("textbtn")
            b.setCursor(Qt.PointingHandCursor)
            b.clicked.connect(cb)
            bar.addWidget(b)
        lay.addLayout(bar)

        self.list = QListWidget()
        self.list.setViewMode(QListWidget.IconMode)
        self.list.setIconSize(QSize(320, 200))
        self.list.setGridSize(QSize(350, 268))
        self.list.setResizeMode(QListWidget.Adjust)
        self.list.setMovement(QListWidget.Static)
        self.list.setSelectionMode(QListWidget.ExtendedSelection)
        self.list.setWordWrap(True)
        self.list.viewport().setCursor(Qt.PointingHandCursor)
        self.list.itemDoubleClicked.connect(lambda it: self.open_item(it.data(Qt.UserRole)))
        lay.addWidget(self.list)
        self.setCentralWidget(root)
        self.reload()
        _keep(self)

    def showEvent(self, e):
        super().showEvent(e)
        if not getattr(self, "_floated", False):
            self._floated = True
            float_window(self)

    def reload(self, *_):
        self.list.clear()
        items = history.list_items()
        self.count.setText(tr("Total: {n}").format(n=len(items)))
        for it in items:
            m = it["meta"]
            size = m.get("size") or ["?", "?"]
            layers = it["layers"]
            text = f"{fmt_date(m.get('created'))}\n{size[0]}×{size[1]} · " + tr("layers: {n}").format(n=layers)
            if m.get("output"):
                text += f" · {m['output']}"
            li = QListWidgetItem(QIcon(QPixmap(str(it["dir"] / "thumb.jpg"))), text)
            li.setData(Qt.UserRole, it["id"])
            li.setTextAlignment(Qt.AlignHCenter | Qt.AlignTop)
            self.list.addItem(li)

    def selected_ids(self):
        return [it.data(Qt.UserRole) for it in self.list.selectedItems()]

    def open_item(self, item_id):
        EditorWindow(item_id, self.cfg, on_saved=self.reload).show()

    def open_selected(self):
        for i in self.selected_ids()[:4]:
            self.open_item(i)

    def copy_selected(self):
        ids = self.selected_ids()
        if ids:
            outputs.copy(shapes.render_result(history.load(ids[0])))

    def pin_selected(self):
        for i in self.selected_ids()[:4]:
            doc = history.load(i)
            _keep(PinWindow(shapes.render_result(doc), float(doc.meta.get("scale", 1.0)), cfg=self.cfg)).show()

    def export_selected(self):
        ids = self.selected_ids()
        if not ids:
            return
        if len(ids) == 1:
            path, _filter = QFileDialog.getSaveFileName(self, tr("Export"), str(outputs.save_dir(self.cfg) / f"Kadr_{ids[0]}.png"),
                                                  "PNG (*.png);;JPEG (*.jpg);;WebP (*.webp)")
            if path:
                shapes.render_result(history.load(ids[0])).save(path)
            return
        d = QFileDialog.getExistingDirectory(self, tr("Export folder"), str(outputs.save_dir(self.cfg)))
        if d:
            for i in ids:
                shapes.render_result(history.load(i)).save(f"{d}/Kadr_{i}.png")

    def open_folder(self):
        ids = self.selected_ids()
        path = history.item_dir(ids[0]) if ids else history.HISTORY_DIR
        QDesktopServices.openUrl(QUrl.fromLocalFile(str(path)))

    def delete_selected(self):
        ids = self.selected_ids()
        if not ids:
            return
        if QMessageBox.question(self, "Kadr", tr("Delete screenshots: {n}? This cannot be undone.").format(n=len(ids))) != QMessageBox.Yes:
            return
        for i in ids:
            history.delete(i)
        self.reload()

    def keyPressEvent(self, e):
        if e.key() == Qt.Key_Delete:
            self.delete_selected()
        elif e.key() in (Qt.Key_Return, Qt.Key_Enter):
            self.open_selected()
        elif e.key() == Qt.Key_C and e.modifiers() & Qt.ControlModifier:
            self.copy_selected()
        else:
            super().keyPressEvent(e)
