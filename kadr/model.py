"""Document = captured base image + crop + stack of editable layers."""
import copy
import time
import uuid

from PySide6.QtCore import QRect
from PySide6.QtGui import QPainter

CM = QPainter.CompositionMode
BLEND_MODES = [
    ("normal", "Обычный", CM.CompositionMode_SourceOver),
    ("multiply", "Умножение", CM.CompositionMode_Multiply),
    ("screen", "Экран", CM.CompositionMode_Screen),
    ("overlay", "Перекрытие", CM.CompositionMode_Overlay),
    ("darken", "Затемнение", CM.CompositionMode_Darken),
    ("lighten", "Замена светлым", CM.CompositionMode_Lighten),
    ("color_dodge", "Осветление основы", CM.CompositionMode_ColorDodge),
    ("color_burn", "Затемнение основы", CM.CompositionMode_ColorBurn),
    ("hard_light", "Жёсткий свет", CM.CompositionMode_HardLight),
    ("soft_light", "Мягкий свет", CM.CompositionMode_SoftLight),
    ("difference", "Разница", CM.CompositionMode_Difference),
    ("exclusion", "Исключение", CM.CompositionMode_Exclusion),
    ("plus", "Линейный осветлитель", CM.CompositionMode_Plus),
]
BLEND_BY_KEY = {k: m for k, _, m in BLEND_MODES}

TYPE_NAMES = {
    "pen": "Кисть", "highlighter": "Выделитель", "line": "Линия", "arrow": "Стрелка",
    "rect": "Прямоугольник", "ellipse": "Эллипс", "text": "Текст", "mark": "Метка", "blur": "Размытие",
    "zoom": "Лупа",
}
LAYER_DEFAULTS = {"highlighter": {"blend": "multiply"}}


class Layer:
    def __init__(self, type, g, style, id=None, name=None, visible=True, opacity=1.0,
                 blend="normal", seq=0, locked=False):
        self.type = type
        self.g = g            # geometry, image pixel coordinates
        self.style = style    # tool-specific look
        self.id = id or uuid.uuid4().hex[:10]
        self.name = name      # None -> auto label
        self.visible = visible
        self.opacity = opacity
        self.blend = blend
        self.seq = seq        # creation order (drives mark numbering)
        self.locked = locked

    def to_dict(self):
        return {"type": self.type, "g": copy.deepcopy(self.g), "style": dict(self.style), "id": self.id,
                "name": self.name, "visible": self.visible, "opacity": self.opacity,
                "blend": self.blend, "seq": self.seq, "locked": self.locked}

    @classmethod
    def from_dict(cls, d):
        d = copy.deepcopy(d)
        return cls(d.pop("type"), d.pop("g"), d.pop("style"), **d)


class Document:
    def __init__(self, base, crop=None, layers=None, meta=None):
        self.base = base          # QImage, full output capture
        self.crop = crop          # QRect in image pixels or None
        self.layers = layers or []  # bottom -> top
        self.meta = meta or {}
        self.seq = max([l.seq for l in self.layers], default=0)
        self.undo_stack, self.redo_stack = [], []
        self._last_key, self._last_time = None, 0
        self.dirty = False

    # --- layers -------------------------------------------------------
    def new_layer(self, type, g, style):
        self.seq += 1
        extra = LAYER_DEFAULTS.get(type, {})
        l = Layer(type, g, copy.deepcopy(style), seq=self.seq, **extra)
        self.layers.append(l)
        return l

    def find(self, lid):
        return next((l for l in self.layers if l.id == lid), None)

    def index(self, layer):
        return self.layers.index(layer)

    def mark_numbers(self):
        marks = sorted((l for l in self.layers if l.type == "mark"), key=lambda l: l.seq)
        start = int(self.meta.get("mark_start", 1))
        return {l.id: i + start for i, l in enumerate(marks)}

    def display_name(self, layer, numbers=None):
        if layer.name:
            return layer.name
        if layer.type == "mark":
            n = (numbers or self.mark_numbers()).get(layer.id, "?")
            return f"Метка №{n}"
        same = [l for l in sorted(self.layers, key=lambda l: l.seq) if l.type == layer.type]
        return f"{TYPE_NAMES.get(layer.type, layer.type)} {same.index(layer) + 1}"

    # --- undo ---------------------------------------------------------
    def snapshot(self):
        crop = (self.crop.x(), self.crop.y(), self.crop.width(), self.crop.height()) if self.crop else None
        return crop, [l.to_dict() for l in self.layers], self.seq

    def restore(self, snap):
        crop, layers, self.seq = snap
        self.crop = QRect(*crop) if crop else None
        self.layers = [Layer.from_dict(d) for d in layers]

    def push_undo(self, snap=None, key=None):
        """Record state before a change. Same `key` within 0.8s coalesces (slider drags)."""
        now = time.monotonic()
        self.dirty = True
        if key and key == self._last_key and now - self._last_time < 0.8:
            self._last_time = now
            return
        self._last_key, self._last_time = key, now
        self.undo_stack.append(snap or self.snapshot())
        del self.undo_stack[:-200]
        self.redo_stack.clear()

    def undo(self):
        if not self.undo_stack:
            return False
        self.redo_stack.append(self.snapshot())
        self.restore(self.undo_stack.pop())
        self._last_key = None
        self.dirty = True
        return True

    def redo(self):
        if not self.redo_stack:
            return False
        self.undo_stack.append(self.snapshot())
        self.restore(self.redo_stack.pop())
        self._last_key = None
        self.dirty = True
        return True

    # --- persistence --------------------------------------------------
    def to_json(self):
        c = self.crop
        return {"version": 1, "meta": self.meta, "seq": self.seq,
                "crop": [c.x(), c.y(), c.width(), c.height()] if c else None,
                "layers": [l.to_dict() for l in self.layers]}

    @classmethod
    def from_json(cls, base, d):
        doc = cls(base, QRect(*d["crop"]) if d.get("crop") else None,
                  [Layer.from_dict(x) for x in d.get("layers", [])], d.get("meta", {}))
        doc.seq = max(doc.seq, d.get("seq", 0))
        return doc
