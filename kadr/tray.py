"""Resident tray icon: the GUI entry point (CLI commands stay standalone)."""
import os
import subprocess
import sys
from pathlib import Path

from PySide6.QtCore import QObject
from PySide6.QtGui import QAction, QColor, QIcon
from PySide6.QtNetwork import QLocalServer, QLocalSocket
from PySide6.QtWidgets import QMenu, QSystemTrayIcon

from . import i18n, icons
from .config import Config
from .i18n import tr

SERVER = f"kadr-tray-{os.getuid()}"


def notify_running(message=b"history"):
    """If a tray is already running, hand it the request and return True."""
    sock = QLocalSocket()
    sock.connectToServer(SERVER)
    if not sock.waitForConnected(300):
        return False
    sock.write(message)
    sock.flush()
    sock.waitForBytesWritten(300)
    sock.disconnectFromServer()
    return True


ICON_FILE = Path.home() / ".local/share/icons/hicolor/scalable/apps/kadr.svg"


def tray_icon(accent):
    """A themed icon, so the SNI item carries an IconName: bars like quickshell ignore raw pixmaps."""
    try:
        svg = icons.svg("kadr", QColor(accent).name()).replace('stroke-width="2"', 'stroke-width="2.2"')
        if not ICON_FILE.exists() or ICON_FILE.read_text() != svg:
            ICON_FILE.parent.mkdir(parents=True, exist_ok=True)
            ICON_FILE.write_text(svg)
    except OSError:
        pass
    fallback = QIcon()
    for size in (16, 22, 24, 32, 48, 64):
        fallback.addPixmap(icons.pixmap("kadr", accent, size))
    return QIcon.fromTheme("kadr", fallback)


class Tray(QObject):
    def __init__(self, app):
        super().__init__()
        self.app = app
        self.history = None
        cfg = Config()
        self.server = QLocalServer(self)
        QLocalServer.removeServer(SERVER)
        self.server.listen(SERVER)
        self.server.newConnection.connect(self._on_connection)

        self.menu = QMenu()
        self.icon = QSystemTrayIcon(tray_icon(cfg["accent"]), self)
        self.build_menu()
        self.icon.setContextMenu(self.menu)
        self.icon.activated.connect(self._on_activated)
        self.icon.show()

    def build_menu(self):
        self.menu.clear()
        for text, icon, cb in ((tr("Take screenshot"), "kadr", self.capture),
                               (tr("Screenshot of the screen under the cursor"), "rect",
                                lambda: self.spawn("screen", delay=0.3)),
                               (tr("Screenshot in 3 seconds"), "history", lambda: self.spawn("capture", delay=3)),
                               (None, None, None),
                               (tr("Screenshot history"), "layers", self.open_history)):
            if text is None:
                self.menu.addSeparator()
                continue
            a = QAction(icons.icon(icon), text, self.menu)
            a.triggered.connect(cb)
            self.menu.addAction(a)

        lang_menu = self.menu.addMenu(tr("Language"))
        chosen = Config().get("language", "auto")
        options = [("auto", tr("Automatic (system: {lang})").format(lang=i18n.LANGUAGES[i18n.detect()]))]
        options += list(i18n.LANGUAGES.items())
        for code, name in options:
            a = QAction(name, lang_menu, checkable=True, checked=code == chosen)
            a.triggered.connect(lambda _=False, c=code: self.set_language(c))
            lang_menu.addAction(a)

        self.menu.addSeparator()
        a = QAction(icons.icon("close"), tr("Quit"), self.menu)
        a.triggered.connect(self.app.quit)
        self.menu.addAction(a)
        self.icon.setToolTip(tr("Kadr — click: screenshot, middle click: history"))

    def set_language(self, code):
        cfg = Config()  # fresh: other processes may have saved tool styles meanwhile
        cfg["language"] = code
        cfg.save()
        i18n.setup(code)
        i18n.install_qt(self.app)
        self.build_menu()  # captures are separate processes and read the setting on start
        if self.history is not None:
            try:
                self.history.close()
            except RuntimeError:
                pass
            self.history = None
            self.open_history()

    def _on_activated(self, reason):
        if reason == QSystemTrayIcon.Trigger:
            self.capture()
        elif reason == QSystemTrayIcon.MiddleClick:
            self.open_history()

    def _on_connection(self):
        conn = self.server.nextPendingConnection()
        conn.readyRead.connect(lambda: self._handle(bytes(conn.readAll()).decode(errors="ignore")))

    def _handle(self, msg):
        if msg.strip() == "history":
            self.open_history()

    def spawn(self, cmd, delay=0.0):
        """Captures run as their own process: a fresh overlay, and the tray never blocks."""
        env = dict(os.environ)
        root = str(Path(__file__).resolve().parent.parent)
        env["PYTHONPATH"] = root + (os.pathsep + env["PYTHONPATH"] if env.get("PYTHONPATH") else "")
        args = [sys.executable, "-m", "kadr", cmd] + (["--delay", str(delay)] if delay else [])
        subprocess.Popen(args, env=env, start_new_session=True,
                         stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)

    def capture(self):
        self.spawn("capture", delay=0.25)  # let the tray menu/popup vanish before grim grabs

    def open_history(self):
        from .editor import HistoryWindow
        if self.history is not None:
            try:
                self.history.reload()
                self.history.raise_()
                self.history.activateWindow()
                return
            except RuntimeError:  # window was closed and deleted
                self.history = None
        self.history = HistoryWindow(Config())  # fresh config: captures run in other processes
        self.history.destroyed.connect(lambda *_: setattr(self, "history", None))
        self.history.show()
