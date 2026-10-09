"""Per-layer-type geometry: painting, bounds, hit tests, handles, compositing."""
import math

from PySide6.QtCore import QLineF, QPointF, QRect, QRectF, Qt
from PySide6.QtGui import (QColor, QFont, QFontMetricsF, QImage, QPainter, QPainterPath,
                           QPainterPathStroker, QPen, QPolygonF)

from .model import BLEND_BY_KEY

RECT_TYPES = ("rect", "ellipse", "blur")
RECT_HANDLES = ("tl", "t", "tr", "r", "br", "b", "bl", "l")


def qcolor(c):
    return QColor(c) if c else None


def P(xy):
    return QPointF(xy[0], xy[1])


def R(r):
    return QRectF(r[0], r[1], r[2], r[3])


def make_pen(color, width, dash=False, cap=Qt.RoundCap):
    pen = QPen(QColor(color), width, Qt.DashLine if dash else Qt.SolidLine, cap, Qt.RoundJoin)
    if dash:
        pen.setDashPattern([2.2, 2.0])
    return pen


# --- geometry helpers ---------------------------------------------------

def stroke_path(pts):
    path = QPainterPath()
    if not pts:
        return path
    path.moveTo(P(pts[0]))
    if len(pts) == 1:
        path.lineTo(pts[0][0] + 0.01, pts[0][1])
        return path
    for i in range(1, len(pts) - 1):
        mid = QPointF((pts[i][0] + pts[i + 1][0]) / 2, (pts[i][1] + pts[i + 1][1]) / 2)
        path.quadTo(P(pts[i]), mid)
    path.lineTo(P(pts[-1]))
    return path


def head_len(width):
    return max(12.0, width * 3.6)


def arrow_head(tip, direction, hl):
    """Slightly notched arrow head polygon pointing along `direction` (unit QPointF)."""
    n = QPointF(-direction.y(), direction.x())
    base = tip - direction * hl
    return QPolygonF([tip, base + n * (hl * 0.5), tip - direction * (hl * 0.72), base - n * (hl * 0.5)])


def draw_arrow(p, a, b, color, width, dash=False, heads="end"):
    v = b - a
    length = math.hypot(v.x(), v.y())
    if length < 0.5:
        return
    u = v / length
    hl = min(head_len(width), length * (0.45 if heads == "both" else 0.8))
    start = a + u * (hl * 0.6) if heads == "both" else a
    end = b - u * (hl * 0.6)
    p.setPen(make_pen(color, width, dash))
    p.drawLine(start, end)
    p.setPen(QPen(QColor(color), 1, Qt.SolidLine, Qt.RoundCap, Qt.RoundJoin))
    p.setBrush(QColor(color))
    p.drawPolygon(arrow_head(b, u, hl))
    if heads == "both":
        p.drawPolygon(arrow_head(a, -u, hl))


def text_font(st):
    f = QFont(st.get("family") or "Sans Serif")
    f.setPixelSize(max(4, int(st.get("size", 24))))
    f.setBold(bool(st.get("bold")))
    f.setItalic(bool(st.get("italic")))
    return f


def text_layout(layer):
    st = layer.style
    f = text_font(st)
    fm = QFontMetricsF(f)
    lines = layer.g.get("text", "").split("\n")
    w = max([fm.horizontalAdvance(l) for l in lines] + [st.get("size", 24) * 0.4])
    pad = st.get("size", 24) * 0.3 if st.get("bg") else 0
    x, y = layer.g["pos"]
    rect = QRectF(x, y, w + 2 * pad, fm.height() * len(lines) + 2 * pad)
    return f, fm, lines, pad, rect


def text_caret(layer):
    f, fm, lines, pad, rect = text_layout(layer)
    x = rect.x() + pad + fm.horizontalAdvance(lines[-1])
    y = rect.y() + pad + fm.height() * (len(lines) - 1)
    return QLineF(x + 1, y, x + 1, y + fm.height())


def mark_shape(st, c):
    s = float(st.get("size", 36))
    path = QPainterPath()
    shape = st.get("shape", "circle")
    if shape == "square":
        path.addRoundedRect(QRectF(c.x() - s / 2, c.y() - s / 2, s, s), s * 0.18, s * 0.18)
    elif shape == "diamond":
        r = s * 0.62
        path.addPolygon(QPolygonF([QPointF(c.x(), c.y() - r), QPointF(c.x() + r, c.y()),
                                   QPointF(c.x(), c.y() + r), QPointF(c.x() - r, c.y())]))
        path.closeSubpath()
    else:
        path.addEllipse(c, s / 2, s / 2)
    return path


def mark_has_arrow(layer):
    a, c = P(layer.g["anchor"]), P(layer.g["pos"])
    return QLineF(a, c).length() > layer.style.get("size", 36) * 0.55


def mark_arrow_color(st):
    return st.get("fill") or st.get("outline") or "#ffff3b30"


# --- painting -----------------------------------------------------------

def paint(p, layer, mark_no=None):
    t, g, st = layer.type, layer.g, layer.style
    p.save()
    p.setRenderHint(QPainter.Antialiasing)
    if t in ("pen", "highlighter"):
        cap = Qt.SquareCap if t == "highlighter" else Qt.RoundCap
        p.setPen(make_pen(st["color"], st["width"], cap=cap))
        p.setBrush(Qt.NoBrush)
        p.drawPath(stroke_path(g["pts"]))
    elif t == "line":
        p.setPen(make_pen(st["color"], st["width"], st.get("dash")))
        p.drawLine(P(g["p1"]), P(g["p2"]))
    elif t == "arrow":
        draw_arrow(p, P(g["p1"]), P(g["p2"]), st["color"], st["width"], st.get("dash"), st.get("heads", "end"))
    elif t in ("rect", "ellipse"):
        stroke, fill = qcolor(st.get("stroke")), qcolor(st.get("fill"))
        p.setPen(make_pen(stroke, st["width"], st.get("dash")) if stroke and st["width"] > 0 else Qt.NoPen)
        p.setBrush(fill or Qt.NoBrush)
        r = R(g["r"])
        if t == "rect":
            rad = min(st.get("radius", 0), r.width() / 2, r.height() / 2)
            p.drawRoundedRect(r, rad, rad)
        else:
            p.drawEllipse(r)
    elif t == "text":
        f, fm, lines, pad, rect = text_layout(layer)
        bg = qcolor(st.get("bg"))
        if bg:
            p.setPen(Qt.NoPen)
            p.setBrush(bg)
            p.drawRoundedRect(rect, pad * 0.9, pad * 0.9)
        path = QPainterPath()
        for i, line in enumerate(lines):
            path.addText(rect.x() + pad, rect.y() + pad + fm.ascent() + i * fm.height(), f, line)
        outline = qcolor(st.get("outline"))
        if outline:
            p.strokePath(path, QPen(outline, max(2.0, st["size"] / 7), Qt.SolidLine, Qt.RoundCap, Qt.RoundJoin))
        p.fillPath(path, QColor(st["color"]))
    elif t == "mark":
        paint_mark(p, layer, mark_no)
    elif t == "blur":  # preview only; real effect happens in composite()
        p.setPen(QPen(QColor(255, 255, 255, 120), 1, Qt.DashLine))
        p.setBrush(Qt.NoBrush)
        p.drawRect(R(g["r"]))
    p.restore()


def paint_mark(p, layer, n):
    st = layer.style
    s = float(st.get("size", 36))
    c, a = P(layer.g["pos"]), P(layer.g["anchor"])
    if mark_has_arrow(layer):
        v = a - c
        u = v / QLineF(c, a).length()
        draw_arrow(p, c + u * (s * 0.45), a, mark_arrow_color(st), max(2.0, s / 9))
    path = mark_shape(st, c)
    p.setPen(Qt.NoPen)
    p.setBrush(QColor(0, 0, 0, 70))
    p.drawPath(path.translated(s * 0.04, s * 0.07))
    outline, fill = qcolor(st.get("outline")), qcolor(st.get("fill"))
    ow = st.get("outline_w", 2)
    p.setPen(QPen(outline, ow) if outline and ow > 0 else Qt.NoPen)
    p.setBrush(fill or Qt.NoBrush)
    p.drawPath(path)
    text = str(n if n is not None else "?")
    f = QFont()
    f.setBold(True)
    f.setPixelSize(max(4, int(s * (0.52 if len(text) == 1 else 0.44 if len(text) == 2 else 0.34))))
    p.setFont(f)
    p.setPen(QColor(st.get("text") or "#ffffffff"))
    p.drawText(QRectF(c.x() - s / 2, c.y() - s / 2, s, s), Qt.AlignCenter, text)


# --- bounds / hit tests -------------------------------------------------

def outline_path(layer):
    t, g, _st = layer.type, layer.g, layer.style
    path = QPainterPath()
    if t in ("pen", "highlighter"):
        return stroke_path(g["pts"])
    if t in ("line", "arrow"):
        path.moveTo(P(g["p1"]))
        path.lineTo(P(g["p2"]))
    elif t == "rect":
        path.addRect(R(g["r"]))
    elif t == "ellipse":
        path.addEllipse(R(g["r"]))
    return path


# --- zoom (magnifier callout) --------------------------------------------

def zoom_upgrade(st):
    """Give a magnifier made before per-part styling explicit values matching how it renders."""
    copy_col, copy_w = st.get("stroke") or "#ffff3b30", st.get("width", 3)
    st.setdefault("src_color", copy_col)
    st.setdefault("src_width", copy_w)
    st.setdefault("line_color", copy_col)
    st.setdefault("line_width", max(1, round(copy_w * 0.7)))
    for k, v in (("radius", 16), ("src_dash", False), ("shadow", False), ("smooth", False), ("lines", True)):
        st.setdefault(k, v)
    return st


def zoom_radius(st, r, k=1.0):
    return min(float(st.get("radius", 16)) * k, r.width() / 2, r.height() / 2)


def zoom_paths(layer):
    """(source path, copy path); the copy's corner radius scales with the magnification."""
    st, g = layer.style, layer.g
    src, dst = R(g["src"]), R(g["dst"])
    k = dst.width() / max(src.width(), 1e-6)
    return zoom_path(src, st, 1.0), zoom_path(dst, st, k)


def zoom_path(r, st, k=1.0):
    path = QPainterPath()
    shape = st.get("shape", "ellipse")
    if shape == "rect":
        path.addRect(r)
    elif shape == "round":
        rad = zoom_radius(st, r, k)
        path.addRoundedRect(r, rad, rad)
    else:
        path.addEllipse(r)
    return path


def zoom_connectors(spath, dpath):
    """Outer tangent lines between source and copy = convex-hull edges joining the two outlines."""
    pts = sorted({(round(p.x(), 3), round(p.y(), 3), tag)
                  for tag, path in ((0, spath), (1, dpath)) for p in path.toFillPolygon()})
    if len(pts) < 3:
        return []

    def cross(o, a, b):
        return (a[0] - o[0]) * (b[1] - o[1]) - (a[1] - o[1]) * (b[0] - o[0])

    lower, upper = [], []
    for p in pts:
        while len(lower) >= 2 and cross(lower[-2], lower[-1], p) <= 0:
            lower.pop()
        lower.append(p)
    for p in reversed(pts):
        while len(upper) >= 2 and cross(upper[-2], upper[-1], p) <= 0:
            upper.pop()
        upper.append(p)
    hull = lower[:-1] + upper[:-1]
    lines = []
    for a, b in zip(hull, hull[1:] + hull[:1]):
        if a[2] != b[2] and math.hypot(a[0] - b[0], a[1] - b[1]) > 1:
            lines.append(QLineF(a[0], a[1], b[0], b[1]))
    return lines


def paint_zoom(img, region, layer, base):
    st, g = layer.style, layer.g
    src, dst = R(g["src"]), R(g["dst"])
    if src.width() < 1 or src.height() < 1:
        return
    s = src.toAlignedRect()
    # pixels currently under the layer: base outside the crop, the running composite inside it
    sub = QImage(s.size(), QImage.Format_ARGB32_Premultiplied)
    sub.fill(Qt.black)
    sp = QPainter(sub)
    sp.drawImage(QPointF(0, 0), base, QRectF(s))
    inter = s & region
    if not inter.isEmpty():
        sp.drawImage(QPointF(inter.topLeft() - s.topLeft()), img, QRectF(inter.translated(-region.topLeft())))
    sp.end()

    # older layers only have stroke/width/dash: fall back to the look they were made with
    copy_col, copy_w = st.get("stroke") or "#ffff3b30", float(st.get("width", 3))
    src_col, src_w = st.get("src_color") or copy_col, float(st.get("src_width", copy_w))
    line_col = st.get("line_color") or copy_col
    line_w = float(st.get("line_width", max(1.0, copy_w * 0.7)))
    spath, dpath = zoom_paths(layer)

    p = QPainter(img)
    p.translate(-region.x(), -region.y())
    p.setOpacity(layer.opacity)
    p.setRenderHint(QPainter.Antialiasing)
    p.setBrush(Qt.NoBrush)
    if st.get("lines", True) and line_w > 0:
        p.setPen(make_pen(line_col, line_w, st.get("dash")))
        p.drawLines(zoom_connectors(spath, dpath))
    if src_w > 0:
        p.setPen(make_pen(src_col, src_w, st.get("src_dash")))
        p.drawPath(spath)
    if st.get("shadow"):
        sh = dpath.translated(0, max(2.0, dst.height() * 0.012))
        for i in range(7, 0, -1):
            p.setPen(QPen(QColor(0, 0, 0, 14), i * 3.0, Qt.SolidLine, Qt.RoundCap, Qt.RoundJoin))
            p.drawPath(sh)
        p.fillPath(sh, QColor(0, 0, 0, 40))
    p.save()
    p.setClipPath(dpath)
    p.setRenderHint(QPainter.SmoothPixmapTransform, bool(st.get("smooth")))
    p.drawImage(dst, sub, QRectF(src.x() - s.x(), src.y() - s.y(), src.width(), src.height()))
    p.restore()
    if copy_w > 0:
        p.setPen(make_pen(copy_col, copy_w))
        p.drawPath(dpath)
    p.end()


def bounds(layer):
    t, g, st = layer.type, layer.g, layer.style
    if t in ("pen", "highlighter"):
        w = st["width"] / 2 + 2
        return stroke_path(g["pts"]).boundingRect().adjusted(-w, -w, w, w)
    if t in ("line", "arrow"):
        w = head_len(st["width"]) if t == "arrow" else st["width"] / 2 + 2
        return QRectF(P(g["p1"]), P(g["p2"])).normalized().adjusted(-w, -w, w, w)
    if t in RECT_TYPES:
        w = st.get("width", 0) / 2 + 2
        return R(g["r"]).adjusted(-w, -w, w, w)
    if t == "text":
        r = text_layout(layer)[4]
        o = st["size"] / 6 + 2
        return r.adjusted(-o, -o, o, o)
    if t == "zoom":
        w = st.get("width", 3) / 2 + 2
        return R(g["src"]).united(R(g["dst"])).adjusted(-w, -w, w, w)
    if t == "mark":
        s = st.get("size", 36) * 0.7 + st.get("outline_w", 2)
        c, a = P(g["pos"]), P(g["anchor"])
        r = QRectF(c.x() - s, c.y() - s, 2 * s, 2 * s)
        if mark_has_arrow(layer):
            h = head_len(max(2.0, st.get("size", 36) / 9))
            r = r.united(QRectF(a.x() - h, a.y() - h, 2 * h, 2 * h))
        return r
    return QRectF()


def hit(layer, pt, tol):
    t, g, st = layer.type, layer.g, layer.style
    if t == "mark":
        if QLineF(pt, P(g["pos"])).length() <= st.get("size", 36) * 0.6 + tol:
            return "body"
        if mark_has_arrow(layer):
            path = QPainterPath(P(g["pos"]))
            path.lineTo(P(g["anchor"]))
            return "arrow" if _stroke_hit(path, max(2.0, st["size"] / 9), pt, tol) else None
        return None
    if t in ("text", "blur"):
        return "body" if bounds(layer).contains(pt) else None
    if t == "zoom":
        spath, dpath = zoom_paths(layer)
        for part, path in (("body", dpath), ("src", spath)):
            if path.contains(pt) or _stroke_hit(path, st.get("width", 3), pt, tol):
                return part
        return None
    if t in ("rect", "ellipse") and st.get("fill"):
        if outline_path(layer).contains(pt):
            return "body"
    width = st.get("width", 2)
    return "body" if _stroke_hit(outline_path(layer), width, pt, tol) else None


def _stroke_hit(path, width, pt, tol):
    s = QPainterPathStroker()
    s.setWidth(width + 2 * tol)
    s.setCapStyle(Qt.RoundCap)
    s.setJoinStyle(Qt.RoundJoin)
    return s.createStroke(path).contains(pt)


# --- editing ------------------------------------------------------------

def translate(layer, dx, dy, whole=True, part=None):
    g = layer.g
    mv = lambda xy: [xy[0] + dx, xy[1] + dy]
    if layer.type == "zoom":
        keys = ("src", "dst") if whole and part is None else (("src",) if part == "src" else ("dst",))
        for k in keys:
            r = g[k]
            g[k] = [r[0] + dx, r[1] + dy, r[2], r[3]]
        return
    if layer.type in ("pen", "highlighter"):
        g["pts"] = [mv(p) for p in g["pts"]]
    elif layer.type in ("line", "arrow"):
        g["p1"], g["p2"] = mv(g["p1"]), mv(g["p2"])
    elif layer.type in RECT_TYPES:
        r = g["r"]
        g["r"] = [r[0] + dx, r[1] + dy, r[2], r[3]]
    elif layer.type == "text":
        g["pos"] = mv(g["pos"])
    elif layer.type == "mark":
        g["pos"] = mv(g["pos"])
        if whole:
            g["anchor"] = mv(g["anchor"])


def rect_handle_points(r):
    return {"tl": r.topLeft(), "t": QPointF(r.center().x(), r.top()), "tr": r.topRight(),
            "r": QPointF(r.right(), r.center().y()), "br": r.bottomRight(),
            "b": QPointF(r.center().x(), r.bottom()), "bl": r.bottomLeft(),
            "l": QPointF(r.left(), r.center().y())}


def handles(layer):
    g = layer.g
    if layer.type in ("line", "arrow"):
        return {"p1": P(g["p1"]), "p2": P(g["p2"])}
    if layer.type in RECT_TYPES:
        return rect_handle_points(R(g["r"]))
    if layer.type == "mark" and mark_has_arrow(layer):
        return {"anchor": P(g["anchor"])}
    if layer.type == "zoom":
        out = {}
        for prefix, key in (("d_", "dst"), ("s_", "src")):  # copy first: it wins where they overlap
            r = R(g[key])
            out.update({prefix + k: v for k, v in rect_handle_points(r).items() if len(k) == 2})
        return out
    return {}


def resize_rect(r, key, pt, square=False):
    l, t, rr, b = r.left(), r.top(), r.right(), r.bottom()
    if "l" in key:
        l = pt.x()
    if "r" in key:
        rr = pt.x()
    if "t" in key:
        t = pt.y()
    if "b" in key:
        b = pt.y()
    out = QRectF(QPointF(l, t), QPointF(rr, b))
    if square and len(key) == 2:
        s = max(abs(out.width()), abs(out.height()))
        out.setWidth(math.copysign(s, out.width() or 1))
        out.setHeight(math.copysign(s, out.height() or 1))
    return out.normalized()


def snap_angle(origin, pt):
    v = pt - origin
    ang = math.atan2(v.y(), v.x())
    step = math.pi / 12
    ang = round(ang / step) * step
    length = math.hypot(v.x(), v.y())
    return origin + QPointF(math.cos(ang) * length, math.sin(ang) * length)


def drag_handle(layer, key, pt, orig, shift=False):
    g = layer.g
    if layer.type in ("line", "arrow"):
        other = P(orig["p2" if key == "p1" else "p1"])
        pt = snap_angle(other, pt) if shift else pt
        g[key] = [pt.x(), pt.y()]
    elif layer.type in RECT_TYPES:
        r = resize_rect(R(orig["r"]), key, pt, shift)
        g["r"] = [r.x(), r.y(), r.width(), r.height()]
    elif layer.type == "mark" and key == "anchor":
        g["anchor"] = [pt.x(), pt.y()]
    elif layer.type == "zoom":
        src, dst, corner = R(orig["src"]), R(orig["dst"]), key[2:]
        if key.startswith("d_"):  # uniform scale of the copy around the opposite corner
            opp = rect_handle_points(dst)[{"tl": "br", "tr": "bl", "br": "tl", "bl": "tr"}[corner]]
            aspect = src.height() / max(src.width(), 1e-6)
            w = max(abs(pt.x() - opp.x()), abs(pt.y() - opp.y()) / aspect, 4)
            h = w * aspect
            x = opp.x() - w if pt.x() < opp.x() else opp.x()
            y = opp.y() - h if pt.y() < opp.y() else opp.y()
            g["dst"] = [x, y, w, h]
        else:  # new source area; the copy keeps its width and follows the new aspect
            r = resize_rect(src, corner, pt, shift)
            if r.width() >= 2 and r.height() >= 2:
                g["src"] = [r.x(), r.y(), r.width(), r.height()]
                g["dst"] = [dst.x(), dst.y(), dst.width(), dst.width() * r.height() / r.width()]


# --- compositing --------------------------------------------------------

def apply_effect(img, rect, st, opacity):
    r = rect & img.rect()
    if r.width() < 2 or r.height() < 2:
        return
    src = img.copy(r)
    k = max(2, int(st.get("strength", 12)))
    small = src.scaled(max(1, r.width() // k), max(1, r.height() // k), Qt.IgnoreAspectRatio,
                       Qt.FastTransformation if st.get("mode") == "pixelate" else Qt.SmoothTransformation)
    if st.get("mode") == "pixelate":
        out = small.scaled(r.width(), r.height(), Qt.IgnoreAspectRatio, Qt.FastTransformation)
    else:
        mid = small.scaled(max(1, small.width() * 2), max(1, small.height() * 2), Qt.IgnoreAspectRatio,
                           Qt.SmoothTransformation)
        out = mid.scaled(r.width(), r.height(), Qt.IgnoreAspectRatio, Qt.SmoothTransformation)
    p = QPainter(img)
    p.setOpacity(opacity)
    p.drawImage(r.topLeft(), out)
    p.end()


def composite(doc, region, layers, start=None):
    """Render `layers` over `start` (or the base) inside `region` (QRect, image px)."""
    if start is not None:
        img = start.copy()
    else:
        img = QImage(region.size(), QImage.Format_ARGB32_Premultiplied)
        img.fill(Qt.black)
        p = QPainter(img)
        p.drawImage(QPointF(0, 0), doc.base, QRectF(region))
        p.end()
    numbers = doc.mark_numbers()
    p = QPainter(img)
    p.translate(-region.x(), -region.y())
    for layer in layers:
        if not layer.visible or layer.opacity <= 0:
            continue
        if layer.type in ("blur", "zoom"):
            p.end()
            if layer.type == "blur":
                rr = R(layer.g["r"]).toAlignedRect().translated(-region.x(), -region.y())
                apply_effect(img, rr, layer.style, layer.opacity)
            else:
                paint_zoom(img, region, layer, doc.base)
            p = QPainter(img)
            p.translate(-region.x(), -region.y())
            continue
        n = numbers.get(layer.id)
        if layer.blend == "normal" and layer.opacity >= 1:
            paint(p, layer, n)
            continue
        # isolate the layer so opacity/blend apply to it as a whole (like Photoshop)
        br = bounds(layer).toAlignedRect() & region
        if br.isEmpty():
            continue
        tmp = QImage(br.size(), QImage.Format_ARGB32_Premultiplied)
        tmp.fill(Qt.transparent)
        tp = QPainter(tmp)
        tp.translate(-br.x(), -br.y())
        paint(tp, layer, n)
        tp.end()
        p.save()
        p.setOpacity(layer.opacity)
        p.setCompositionMode(BLEND_BY_KEY.get(layer.blend, QPainter.CompositionMode_SourceOver))
        p.drawImage(br.topLeft(), tmp)
        p.restore()
    p.end()
    return img


def render_result(doc):
    region = doc.crop or doc.base.rect()
    return composite(doc, QRect(region), doc.layers)
