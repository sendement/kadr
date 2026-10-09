import copy
import json
import os
from pathlib import Path

CONF_DIR = Path(os.environ.get("XDG_CONFIG_HOME") or Path.home() / ".config") / "kadr"
DATA_DIR = Path(os.environ.get("XDG_DATA_HOME") or Path.home() / ".local/share") / "kadr"
HISTORY_DIR = DATA_DIR / "history"

TOOL_DEFAULTS = {
    "pen": {"color": "#ffff3b30", "width": 4},
    "highlighter": {"color": "#ffffd60a", "width": 22},
    "line": {"color": "#ffff3b30", "width": 4, "dash": False},
    "arrow": {"color": "#ffff3b30", "width": 5, "dash": False, "heads": "end"},
    "rect": {"stroke": "#ffff3b30", "fill": None, "width": 4, "radius": 6, "dash": False},
    "ellipse": {"stroke": "#ffff3b30", "fill": None, "width": 4, "dash": False},
    "text": {"color": "#ffff3b30", "family": "Sans Serif", "size": 26, "bold": True, "italic": False,
             "bg": None, "outline": "#ffffffff"},
    "mark": {"shape": "circle", "size": 36, "text": "#ffffffff", "fill": "#ffff3b30",
             "outline": "#ffffffff", "outline_w": 3},
    "blur": {"mode": "pixelate", "strength": 12},
    "zoom": {"shape": "ellipse", "radius": 16,
             "src_color": "#ffff3b30", "src_width": 2, "src_dash": False,           # source area
             "stroke": "#ffff3b30", "width": 3, "shadow": True, "smooth": False,   # magnified copy
             "lines": True, "line_color": "#ffff3b30", "line_width": 2, "dash": False},  # connectors
}

DEFAULTS = {
    "accent": "#ff4cc2ff",
    "save_dir": "",            # empty -> <XDG Pictures>/Kadr
    "copy_on_save": True,
    "notify": True,
    "history_limit": 500,      # 0 = unlimited
    "recent_colors": [],
    "tool_styles": {},
}


class Config(dict):
    path = CONF_DIR / "config.json"

    def __init__(self):
        super().__init__(copy.deepcopy(DEFAULTS))
        try:
            self.update(json.loads(self.path.read_text()))
        except Exception:
            pass
        styles = copy.deepcopy(TOOL_DEFAULTS)
        for k, v in (self.get("tool_styles") or {}).items():
            if k in styles:
                styles[k].update(v)
        self["tool_styles"] = styles

    def save(self):
        try:
            CONF_DIR.mkdir(parents=True, exist_ok=True)
            tmp = self.path.with_suffix(".tmp")
            tmp.write_text(json.dumps(self, indent=2, ensure_ascii=False))
            tmp.replace(self.path)
        except Exception:
            pass

    def add_recent_color(self, c):
        if not c:
            return
        rc = [x for x in self["recent_colors"] if x != c]
        self["recent_colors"] = [c] + rc[:9]
