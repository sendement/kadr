"""On-disk history: every shot keeps the full output capture + editable layers."""
import json
import shutil
import uuid
from datetime import datetime

from PySide6.QtCore import QSize, Qt
from PySide6.QtGui import QImage

from .config import HISTORY_DIR
from .model import Document


def new_id():
    return datetime.now().strftime("%Y%m%d-%H%M%S-") + uuid.uuid4().hex[:4]


def item_dir(item_id):
    return HISTORY_DIR / item_id


def save(doc, result, item_id=None, limit=0):
    item_id = item_id or doc.meta.get("id") or new_id()
    d = item_dir(item_id)
    d.mkdir(parents=True, exist_ok=True)
    doc.meta["id"] = item_id
    doc.meta.setdefault("created", datetime.now().isoformat(timespec="seconds"))
    doc.meta["modified"] = datetime.now().isoformat(timespec="seconds")
    doc.meta["size"] = [result.width(), result.height()]
    if not (d / "base.png").exists():
        doc.base.save(str(d / "base.png"), "PNG", 60)
    result.save(str(d / "result.png"), "PNG", 60)
    thumb = result.scaled(QSize(360, 240), Qt.KeepAspectRatio, Qt.SmoothTransformation)
    thumb.save(str(d / "thumb.jpg"), "JPG", 88)
    tmp = d / "doc.json.tmp"
    tmp.write_text(json.dumps(doc.to_json(), ensure_ascii=False))
    tmp.replace(d / "doc.json")
    doc.dirty = False
    if limit:
        prune(limit)
    return d


def load(item_id):
    d = item_dir(item_id)
    base = QImage(str(d / "base.png"))
    if base.isNull():
        raise FileNotFoundError(d / "base.png")
    data = json.loads((d / "doc.json").read_text())
    doc = Document.from_json(base, data)
    doc.meta["id"] = item_id
    return doc


def list_items():
    if not HISTORY_DIR.exists():
        return []
    items = []
    for d in sorted(HISTORY_DIR.iterdir(), reverse=True):
        if not (d / "doc.json").exists():
            continue
        try:
            data = json.loads((d / "doc.json").read_text())
        except Exception:
            continue
        items.append({"id": d.name, "dir": d, "meta": data.get("meta", {}),
                      "layers": len(data.get("layers", []))})
    return items


def last_id():
    items = list_items()
    return items[0]["id"] if items else None


def delete(item_id):
    shutil.rmtree(item_dir(item_id), ignore_errors=True)


def prune(limit):
    items = list_items()
    for it in items[limit:]:
        delete(it["id"])
