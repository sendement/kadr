"""Inline stroke icons (24x24, lucide-like), tinted at runtime."""
from PySide6.QtCore import QByteArray, QRectF, Qt
from PySide6.QtGui import QIcon, QImage, QPainter, QPixmap
from PySide6.QtSvg import QSvgRenderer

_SVG = {
    "pointer": '<path d="M5 3l6.5 17 2.3-7.2L21 10.5z"/>',
    "pen": '<path d="M12 20h9"/><path d="M16.5 3.5a2.1 2.1 0 0 1 3 3L7 19l-4 1 1-4z"/>',
    "highlighter": '<path d="M9 11l-6 6v3h9l3-3"/><path d="M22 12l-4.6 4.6a2 2 0 0 1-2.8 0l-5.2-5.2a2 2 0 0 1 0-2.8L14 4"/>',
    "line": '<path d="M5 19L19 5"/>',
    "arrow": '<path d="M5 19L19 5"/><path d="M10 5h9v9"/>',
    "arrow_both": '<path d="M5 19L19 5"/><path d="M12 5h7v7"/><path d="M12 19H5v-7"/>',
    "rect": '<rect x="3" y="5" width="18" height="14" rx="2"/>',
    "ellipse": '<ellipse cx="12" cy="12" rx="9.5" ry="7"/>',
    "text": '<path d="M4 7V4h16v3"/><path d="M9 20h6"/><path d="M12 4v16"/>',
    "mark": '<circle cx="12" cy="12" r="9.5"/><path d="M10 9l2.5-1.8V17"/>',
    "blur": '<rect x="3" y="3" width="6" height="6" fill="currentColor"/><rect x="15" y="3" width="6" height="6" fill="currentColor"/>'
            '<rect x="9" y="9" width="6" height="6" fill="currentColor"/><rect x="3" y="15" width="6" height="6" fill="currentColor"/>'
            '<rect x="15" y="15" width="6" height="6" fill="currentColor"/><rect x="3" y="3" width="18" height="18" rx="1"/>',
    "drop": '<path d="M12 2.7l5.7 5.7a8 8 0 1 1-11.3 0z"/>',
    "undo": '<path d="M9 14L4 9l5-5"/><path d="M4 9h10.5a5.5 5.5 0 0 1 0 11H11"/>',
    "redo": '<path d="M15 14l5-5-5-5"/><path d="M20 9H9.5a5.5 5.5 0 0 0 0 11H13"/>',
    "layers": '<path d="M12 2l10 5-10 5L2 7z"/><path d="M2 17l10 5 10-5"/><path d="M2 12l10 5 10-5"/>',
    "copy": '<rect x="9" y="9" width="13" height="13" rx="2"/><path d="M5 15H4a2 2 0 0 1-2-2V4a2 2 0 0 1 2-2h9a2 2 0 0 1 2 2v1"/>',
    "save": '<path d="M19 21H5a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2h11l5 5v11a2 2 0 0 1-2 2z"/><path d="M17 21v-8H7v8"/><path d="M7 3v5h8"/>',
    "export": '<path d="M21 15v4a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2v-4"/><path d="M7 10l5 5 5-5"/><path d="M12 15V3"/>',
    "kadr": '<path d="M3 8V5a2 2 0 0 1 2-2h3M16 3h3a2 2 0 0 1 2 2v3M21 16v3a2 2 0 0 1-2 2h-3M8 21H5a2 2 0 0 1-2-2v-3"/><circle cx="12" cy="12" r="3.5"/>',
    "zoom": '<circle cx="10.5" cy="10.5" r="7"/><path d="M21 21l-5.5-5.5"/><path d="M10.5 7.5v6M7.5 10.5h6"/>',
    "rect_sharp": '<rect x="3" y="5" width="18" height="14"/>',
    "round": '<rect x="3" y="5" width="18" height="14" rx="5"/>',
    "shadow": '<rect x="3" y="3" width="14" height="14" rx="2"/><path d="M21 7v12a2 2 0 0 1-2 2H7"/>',
    "smooth": '<path d="M2 14c2.5-6 5-6 7.5 0s5 6 7.5 0 3.5-5 5-3"/>',
    "lines": '<rect x="2" y="9" width="5" height="5" rx="1"/><rect x="14" y="4" width="8" height="15" rx="1.5"/><path d="M7 9l7-5M7 14l7 5"/>',
    "clipboard": '<rect x="8" y="2" width="8" height="4" rx="1"/><path d="M16 4h2a2 2 0 0 1 2 2v14a2 2 0 0 1-2 2H6a2 2 0 0 1-2-2V6a2 2 0 0 1 2-2h2"/><path d="M9 14l2 2 4-4"/>',
    "pin": '<path d="M12 17v5"/><path d="M8 3h8"/><path d="M9 3v7l-3 4v3h12v-3l-3-4V3"/>',
    "edit": '<path d="M15 3h6v6"/><path d="M10 14L21 3"/><path d="M18 13v6a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2V8a2 2 0 0 1 2-2h6"/>',
    "close": '<path d="M18 6L6 18M6 6l12 12"/>',
    "trash": '<path d="M3 6h18"/><path d="M19 6l-1 14a2 2 0 0 1-2 2H8a2 2 0 0 1-2-2L5 6"/><path d="M10 11v6M14 11v6"/><path d="M9 6V4h6v2"/>',
    "up": '<path d="M12 19V5M5 12l7-7 7 7"/>',
    "down": '<path d="M12 5v14M19 12l-7 7-7-7"/>',
    "duplicate": '<rect x="8" y="8" width="13" height="13" rx="2"/><path d="M4 16V5a1 1 0 0 1 1-1h11"/>',
    "eyedropper": '<path d="M2 22l1-1h3l9-9"/><path d="M3 21v-3l9-9"/><path d="M15 6l3.4-3.4a2.1 2.1 0 1 1 3 3L18 9l.4.4a2.1 2.1 0 1 1-3 3l-3.8-3.8a2.1 2.1 0 1 1 3-3z"/>',
    "bold": '<path d="M6 4h8a4 4 0 0 1 0 8H6z"/><path d="M6 12h9a4 4 0 0 1 0 8H6z"/>',
    "italic": '<path d="M19 4h-9M14 20H5M15 4L9 20"/>',
    "dash": '<path d="M2 12h4M10 12h4M18 12h4"/>',
    "circle": '<circle cx="12" cy="12" r="8.5"/>',
    "square": '<rect x="4" y="4" width="16" height="16" rx="3"/>',
    "diamond": '<path d="M12 2.5l9.5 9.5-9.5 9.5L2.5 12z"/>',
    "history": '<path d="M3 12a9 9 0 1 0 3-6.7L3 8"/><path d="M3 3v5h5"/><path d="M12 7v5l4 2"/>',
    "folder": '<path d="M4 20h16a2 2 0 0 0 2-2V8a2 2 0 0 0-2-2h-7.9a2 2 0 0 1-1.7-.9l-.8-1.2A2 2 0 0 0 7.9 3H4a2 2 0 0 0-2 2v13a2 2 0 0 0 2 2z"/>',
    "fit": '<path d="M3 8V3h5M21 8V3h-5M3 16v5h5M21 16v5h-5"/>',
    "check": '<path d="M20 6L9 17l-5-5"/>',
    "ocr": '<path d="M3 7V5a2 2 0 0 1 2-2h2M17 3h2a2 2 0 0 1 2 2v2M21 17v2a2 2 0 0 1-2 2h-2M7 21H5a2 2 0 0 1-2-2v-2"/><path d="M7 8h10M7 12h10M7 16h6"/>',
}

_cache = {}


def svg(name, color="#e6e6ea"):
    body = _SVG[name].replace("currentColor", color)
    return (f'<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="{color}" '
            f'stroke-width="2" stroke-linecap="round" stroke-linejoin="round">{body}</svg>')


def pixmap(name, color="#e6e6ea", size=24, dpr=2.0):
    key = (name, color, size, dpr)
    if key not in _cache:
        img = QImage(int(size * dpr), int(size * dpr), QImage.Format_ARGB32_Premultiplied)
        img.fill(Qt.transparent)
        p = QPainter(img)
        QSvgRenderer(QByteArray(svg(name, color).encode())).render(p, QRectF(0, 0, img.width(), img.height()))
        p.end()
        pm = QPixmap.fromImage(img)
        pm.setDevicePixelRatio(dpr)
        _cache[key] = pm
    return _cache[key]


def icon(name, color="#e6e6ea", active="#101014"):
    ic = QIcon()
    ic.addPixmap(pixmap(name, color), QIcon.Normal, QIcon.Off)
    ic.addPixmap(pixmap(name, active), QIcon.Normal, QIcon.On)
    ic.addPixmap(pixmap(name, "#6a6a72"), QIcon.Disabled, QIcon.Off)
    return ic
