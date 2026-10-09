"""Tiny i18n: English source strings, JSON tables per language, system language by default."""
import json
import os
from pathlib import Path

LANGUAGES = {"en": "English", "ru": "Русский", "uk": "Українська", "de": "Deutsch",
             "es": "Español", "fr": "Français", "zh": "中文"}
LOCALES = Path(__file__).parent / "locales"

current = "en"
_table = {}
_qt_translator = None


def detect():
    """First language named by the usual POSIX variables; unsupported ones fall back to English."""
    for var in ("LANGUAGE", "LC_ALL", "LC_MESSAGES", "LANG"):
        for item in (os.environ.get(var) or "").split(":"):
            code = item.split(".")[0].split("@")[0].split("_")[0].lower()
            if not code or code in ("c", "posix"):
                continue
            return code if code in LANGUAGES else "en"
    return "en"


def setup(lang="auto"):
    global current, _table
    code = detect() if lang in (None, "", "auto") else lang
    if code not in LANGUAGES:
        code = "en"
    current = code
    _table = {}
    if code != "en":
        try:
            _table = json.loads((LOCALES / f"{code}.json").read_text(encoding="utf-8"))
        except (OSError, ValueError):
            pass
    return code


def tr(text):
    return _table.get(text, text)


def install_qt(app):
    """Translate Qt's own dialogs (QMessageBox buttons, file dialogs) too."""
    global _qt_translator
    from PySide6.QtCore import QLibraryInfo, QTranslator
    if _qt_translator is not None:
        app.removeTranslator(_qt_translator)
        _qt_translator = None
    if current == "en":
        return
    t = QTranslator(app)
    if t.load(f"qtbase_{current}", QLibraryInfo.path(QLibraryInfo.TranslationsPath)):
        app.installTranslator(t)
        _qt_translator = t
