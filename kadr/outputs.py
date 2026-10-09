"""What happens to a finished shot: clipboard, file, history, pin."""
from datetime import datetime
from pathlib import Path

from PySide6.QtCore import QBuffer, QByteArray, QIODevice

from . import compositor, history


def png_bytes(img):
    ba = QByteArray()
    buf = QBuffer(ba)
    buf.open(QIODevice.WriteOnly)
    img.save(buf, "PNG", 80)
    return bytes(ba)


def save_dir(cfg):
    d = Path(cfg["save_dir"]).expanduser() if cfg.get("save_dir") else compositor.pictures_dir() / "Kadr"
    d.mkdir(parents=True, exist_ok=True)
    return d


def copy(img):
    return compositor.copy_image(png_bytes(img))


def save_file(img, cfg):
    path = save_dir(cfg) / datetime.now().strftime("Kadr_%Y-%m-%d_%H-%M-%S.png")
    img.save(str(path), "PNG", 60)
    return path


def to_history(doc, img, cfg):
    try:
        return history.save(doc, img, limit=int(cfg.get("history_limit") or 0))
    except Exception as e:
        compositor.notify("Kadr: не удалось сохранить в историю", str(e))
        return None


def notify(cfg, title, body="", icon=None):
    if cfg.get("notify", True):
        compositor.notify(title, body, icon)
