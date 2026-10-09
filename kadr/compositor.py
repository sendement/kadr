"""Compositor glue: outputs, cursor, window list, capture, clipboard, layer-shell."""
import ctypes
import json
import os
import shutil
import subprocess
from .i18n import tr
from concurrent.futures import ThreadPoolExecutor
from dataclasses import dataclass, field
from pathlib import Path


@dataclass
class Output:
    name: str
    x: int  # logical global position
    y: int
    w: int  # logical size
    h: int
    scale: float
    pw: int = 0  # physical (captured) size
    ph: int = 0
    workspaces: list = field(default_factory=list)
    image: object = None  # QImage, filled after capture

    def contains(self, x, y):
        return self.x <= x < self.x + self.w and self.y <= y < self.y + self.h


@dataclass
class WindowInfo:
    x: int  # logical global
    y: int
    w: int
    h: int
    title: str
    cls: str


def _json(cmd):
    try:
        out = subprocess.run(cmd, capture_output=True, text=True, timeout=3).stdout
        return json.loads(out)
    except Exception:
        return None


def is_hyprland():
    return bool(os.environ.get("HYPRLAND_INSTANCE_SIGNATURE")) and shutil.which("hyprctl")


def is_sway():
    return bool(os.environ.get("SWAYSOCK")) and shutil.which("swaymsg")


def list_outputs():
    outs = []
    if is_hyprland():
        for m in _json(["hyprctl", "monitors", "-j"]) or []:
            if m.get("disabled"):
                continue
            pw, ph = m["width"], m["height"]
            if m.get("transform", 0) % 2:
                pw, ph = ph, pw
            s = m.get("scale") or 1.0
            ws = [m.get("activeWorkspace", {}).get("id"), m.get("specialWorkspace", {}).get("id")]
            outs.append(Output(m["name"], m["x"], m["y"], round(pw / s), round(ph / s), s, pw, ph,
                               [w for w in ws if w]))
    elif is_sway():
        for m in _json(["swaymsg", "-t", "get_outputs", "-r"]) or []:
            if not m.get("active"):
                continue
            r, s = m["rect"], m.get("scale") or 1.0
            outs.append(Output(m["name"], r["x"], r["y"], r["width"], r["height"], s,
                               round(r["width"] * s), round(r["height"] * s)))
    return outs


def cursor_pos():
    if is_hyprland():
        c = _json(["hyprctl", "cursorpos", "-j"])
        if c:
            return c["x"], c["y"]
    return None


def list_windows(outputs):
    """Visible toplevels in hit-test order (topmost first), logical global coords."""
    if not is_hyprland():
        return []
    visible_ws = {w for o in outputs for w in o.workspaces}
    res = []
    for c in _json(["hyprctl", "clients", "-j"]) or []:
        if not c.get("mapped") or c.get("hidden") or c["workspace"]["id"] not in visible_ws:
            continue
        res.append((not c.get("fullscreen"), not c.get("floating"), c.get("focusHistoryID", 99),
                    WindowInfo(c["at"][0], c["at"][1], c["size"][0], c["size"][1],
                               c.get("title", ""), c.get("class", ""))))
    res.sort(key=lambda t: t[:3])
    return [t[3] for t in res]


def start_grab(outputs):
    """Kick off grim for every output (before Qt starts, to freeze the frame ASAP)."""
    if not shutil.which("grim"):
        raise SystemExit(tr("kadr: grim is required (e.g. pacman -S grim)"))
    names = [o.name for o in outputs] or [None]
    procs = {}
    for n in names:
        cmd = ["grim", "-t", "ppm"] + (["-o", n] if n else []) + ["-"]
        procs[n] = subprocess.Popen(cmd, stdout=subprocess.PIPE, stderr=subprocess.DEVNULL)
    return procs


def finish_grab(procs):
    with ThreadPoolExecutor(max_workers=len(procs)) as ex:
        data = dict(zip(procs, ex.map(lambda p: p.communicate()[0], procs.values())))
    return data


def copy_image(png_bytes):
    if shutil.which("wl-copy"):
        p = subprocess.Popen(["wl-copy", "--type", "image/png"], stdin=subprocess.PIPE,
                             stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, start_new_session=True)
        p.communicate(png_bytes)
        return True
    return False


def copy_text(text):
    if shutil.which("wl-copy"):
        subprocess.run(["wl-copy", text], stderr=subprocess.DEVNULL)


def notify(summary, body="", icon=None):
    if shutil.which("notify-send"):
        cmd = ["notify-send", "-a", "Kadr", summary, body]
        if icon:
            cmd[1:1] = ["-i", str(icon)]
        subprocess.Popen(cmd, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)


def pictures_dir():
    try:
        d = subprocess.run(["xdg-user-dir", "PICTURES"], capture_output=True, text=True).stdout.strip()
        if d and d != str(Path.home()):
            return Path(d)
    except Exception:
        pass
    return Path.home() / "Pictures"


def _hypr_window_ops(addr, float_=False, size=None, pos=None, pin=False):
    """Apply window ops; Hyprland's Lua config mode (0.55+) and the legacy syntax are both handled."""
    win = f'"address:{addr}"'
    lua, legacy = [], []
    if float_:
        lua.append(f'hl.dsp.window.float({{ action = "on", window = {win} }})')
        legacy.append(f"setfloating address:{addr}")
    if size:
        lua.append(f"hl.dsp.window.resize({{ x = {int(size[0])}, y = {int(size[1])}, window = {win} }})")
        legacy.append(f"resizewindowpixel exact {int(size[0])} {int(size[1])},address:{addr}")
    if pos:
        lua.append(f"hl.dsp.window.move({{ x = {int(pos[0])}, y = {int(pos[1])}, relative = false, window = {win} }})")
        legacy.append(f"movewindowpixel exact {int(pos[0])} {int(pos[1])},address:{addr}")
    if pin:
        lua.append(f'hl.dsp.window.pin({{ action = "on", window = {win} }})')
        legacy.append(f"pin address:{addr}")
    for l, g in zip(lua, legacy):
        out = subprocess.run(["hyprctl", "dispatch", l], capture_output=True, text=True).stdout.strip()
        if out != "ok":
            subprocess.run(["hyprctl", "dispatch", *g.split(" ", 1)], capture_output=True)


def _find_client(title):
    pid = os.getpid()
    return next((c for c in _json(["hyprctl", "clients", "-j"]) or []
                 if c.get("pid") == pid and c.get("title") == title), None)


def hypr_float(title, w, h, pos=None, pin=False):
    """Float + size + center (or place) our own toplevel. Returns False until it is mapped."""
    if not is_hyprland():
        return True
    client = _find_client(title)
    if not client:
        return False
    if pos is None:
        mon = next((m for m in _json(["hyprctl", "monitors", "-j"]) or []
                    if m.get("id") == client.get("monitor")), None)
        if mon:
            s = mon.get("scale") or 1.0
            mw, mh = mon["width"] / s, mon["height"] / s
            if mon.get("transform", 0) % 2:
                mw, mh = mh, mw
            w, h = min(w, int(mw * 0.92)), min(h, int(mh * 0.9))
            pos = (mon["x"] + (mw - w) / 2, mon["y"] + (mh - h) / 2)
    _hypr_window_ops(client["address"], float_=not client.get("floating"), size=(w, h), pos=pos, pin=pin)
    return True


def hypr_resize(title, w, h):
    if is_hyprland():
        client = _find_client(title)
        if client:
            _hypr_window_ops(client["address"], size=(w, h))


class LayerShell:
    """Minimal ctypes bridge to LayerShellQt::Window (PySide6 has no bindings)."""
    LAYER_OVERLAY = 3
    ANCHOR_ALL = 1 | 2 | 4 | 8
    KEYBOARD_EXCLUSIVE = 1

    def __init__(self):
        self.ok = False
        try:
            lib = ctypes.CDLL("libLayerShellQtInterface.so.6")
            vp, i = ctypes.c_void_p, ctypes.c_int

            def fn(name, *args, res=None):
                f = getattr(lib, name)
                f.argtypes, f.restype = list(args), res
                return f

            self._get = fn("_ZN12LayerShellQt6Window3getEP7QWindow", vp, res=vp)
            self._layer = fn("_ZN12LayerShellQt6Window8setLayerENS0_5LayerE", vp, i)
            self._anchors = fn("_ZN12LayerShellQt6Window10setAnchorsE6QFlagsINS0_6AnchorEE", vp, i)
            self._kbd = fn("_ZN12LayerShellQt6Window24setKeyboardInteractivityENS0_21KeyboardInteractivityE", vp, i)
            self._zone = fn("_ZN12LayerShellQt6Window16setExclusiveZoneEi", vp, i)
            self._screen = fn("_ZN12LayerShellQt6Window9setScreenEP7QScreen", vp, vp)
            self._scope = fn("_ZN12LayerShellQt6Window8setScopeERK7QString", vp, vp)
            core = ctypes.CDLL("libQt6Core.so.6")
            # static QString QString::fromUtf8(QByteArrayView) -> returned via hidden sret pointer
            self._from_utf8 = core._ZN7QString8fromUtf8E14QByteArrayView
            self._from_utf8.argtypes = [vp, ctypes.c_ssize_t, ctypes.c_char_p]
            self._from_utf8.restype = None
            self._keep = []
            self.ok = True
        except Exception:
            pass

    def apply(self, widget, screen, scope="kadr"):
        if not self.ok:
            return False
        import shiboken6
        widget.winId()
        lw = self._get(shiboken6.getCppPointer(widget.windowHandle())[0])
        if not lw:
            return False
        self._layer(lw, self.LAYER_OVERLAY)
        self._anchors(lw, self.ANCHOR_ALL)
        self._kbd(lw, self.KEYBOARD_EXCLUSIVE)
        self._zone(lw, -1)
        self._screen(lw, shiboken6.getCppPointer(screen)[0])
        try:
            raw = scope.encode()
            qs = ctypes.create_string_buffer(64)  # sizeof(QString) == 24, leaked on purpose
            self._from_utf8(qs, len(raw), raw)
            self._keep.append(qs)
            self._scope(lw, qs)
        except Exception:
            pass
        return True
