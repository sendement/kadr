"""Floating panels that dock around the selection."""
from PySide6.QtCore import QItemSelectionModel, QSize, Qt, QTimer
from PySide6.QtGui import QColor, QFont
from PySide6.QtWidgets import (QAbstractItemView, QButtonGroup, QComboBox, QFontComboBox, QFrame,
                               QHBoxLayout, QLabel, QListWidget, QListWidgetItem, QSlider, QToolButton,
                               QVBoxLayout, QWidget)

from . import icons, shapes
from .colorpicker import ColorButton
from .model import BLEND_MODES

TOOLS = [
    ("pointer", "Указатель: выбор и перемещение слоёв, рамка выделения", "V"),
    ("pen", "Кисть", "P"),
    ("highlighter", "Выделитель (маркер)", "H"),
    ("line", "Линия (Shift — шаг 15°)", "L"),
    ("arrow", "Стрелка (Shift — шаг 15°)", "A"),
    ("rect", "Прямоугольник (Shift — квадрат)", "R"),
    ("ellipse", "Эллипс (Shift — круг)", "E"),
    ("text", "Текст", "T"),
    ("mark", "Метка с номером: клик — поставить, тянуть — отвести со стрелкой", "M"),
    ("blur", "Размытие / пикселизация", "B"),
    ("zoom", "Лупа: выделите область, затем вынесите копию в сторону; углы копии — увеличение", "Z"),
]


def stylesheet(accent):
    a = QColor(accent)
    rgb = f"{a.red()},{a.green()},{a.blue()}"
    return f"""
    #panel {{ background: rgba(22,22,28,238); border: 1px solid rgba(255,255,255,30); border-radius: 12px; }}
    #pill {{ background: rgba(22,22,28,225); border: 1px solid rgba(255,255,255,30); border-radius: 9px;
             color: #e6e6ea; padding: 3px 9px; font-size: 12px; }}
    QToolButton {{ background: transparent; border: none; border-radius: 8px; padding: 4px; color: #e6e6ea; }}
    QToolButton:hover {{ background: rgba(255,255,255,24); }}
    QToolButton:checked {{ background: rgb({rgb}); }}
    QToolButton:disabled {{ color: #6a6a72; }}
    QToolButton#primary {{ background: rgb({rgb}); color: #101014; font-weight: 600; padding: 4px 10px; }}
    QToolButton#primary:hover {{ background: rgba({rgb},210); }}
    QToolButton#textbtn {{ padding: 4px 8px; }}
    QLabel {{ color: #b9b9c3; font-size: 11px; background: transparent; }}
    QLabel#title {{ color: #e6e6ea; font-size: 12px; font-weight: 600; }}
    QFrame#sep {{ background: rgba(255,255,255,30); border: none; }}
    QSlider::groove:horizontal {{ height: 4px; background: rgba(255,255,255,40); border-radius: 2px; }}
    QSlider::sub-page:horizontal {{ background: rgb({rgb}); border-radius: 2px; }}
    QSlider::handle:horizontal {{ width: 14px; height: 14px; margin: -5px 0; border-radius: 7px; background: #f2f2f5; }}
    QListWidget {{ background: transparent; border: none; color: #e6e6ea; outline: 0; font-size: 12px; }}
    QListWidget::item {{ padding: 5px 4px; border-radius: 6px; }}
    QListWidget::item:hover {{ background: rgba(255,255,255,14); }}
    QListWidget::item:selected {{ background: rgba({rgb},90); color: #ffffff; }}
    QListWidget QLineEdit {{ padding: 0 4px; }}
    QComboBox, QFontComboBox, QLineEdit {{ background: rgba(255,255,255,16); color: #ececf1;
        border: 1px solid rgba(255,255,255,34); border-radius: 6px; padding: 2px 6px; min-height: 20px;
        font-size: 12px; }}
    QComboBox QAbstractItemView {{ background: #1d1d23; color: #ececf1; border: 1px solid rgba(255,255,255,40);
        selection-background-color: rgba({rgb},140); outline: 0; }}
    QToolTip {{ background: #1d1d23; color: #ececf1; border: 1px solid rgba(255,255,255,40); padding: 4px; }}
    QScrollBar:vertical {{ width: 6px; background: transparent; }}
    QScrollBar::handle:vertical {{ background: rgba(255,255,255,50); border-radius: 3px; }}
    QScrollBar::add-line, QScrollBar::sub-line {{ height: 0; }}
    """


def hand(*widgets):
    for w in widgets:
        w.setCursor(Qt.PointingHandCursor)
    return widgets[0]


class Panel(QFrame):
    def __init__(self, canvas, vertical=False):
        super().__init__(canvas)
        self.canvas = canvas
        self.setObjectName("panel")
        self.setAttribute(Qt.WA_StyledBackground)
        # clicks/wheel on the panel background must not fall through to the canvas
        self.setAttribute(Qt.WA_NoMousePropagation)
        self.setCursor(Qt.ArrowCursor)
        self.lay = QVBoxLayout(self) if vertical else QHBoxLayout(self)
        self.lay.setContentsMargins(6, 6, 6, 6)
        self.lay.setSpacing(3)
        self.vertical = vertical

    def button(self, name, tip, cb=None, checkable=False, text=None):
        b = QToolButton()
        b.setIcon(icons.icon(name))
        b.setIconSize(QSize(20, 20))
        b.setToolTip(tip)
        b.setCheckable(checkable)
        b.setFocusPolicy(Qt.NoFocus)
        hand(b)
        if text:
            b.setText(text)
            b.setToolButtonStyle(Qt.ToolButtonTextBesideIcon)
        if cb:
            b.clicked.connect(cb)
        return b

    def sep(self):
        s = QFrame()
        s.setObjectName("sep")
        if self.vertical:
            s.setFixedHeight(1)
        else:
            s.setFixedWidth(1)
        return s

    def fit(self):
        self.adjustSize()
        self.resize(self.sizeHint())


class ToolBar(Panel):
    def __init__(self, canvas):
        super().__init__(canvas, vertical=True)
        self.group = QButtonGroup(self)
        self.buttons = {}
        for name, tip, key in TOOLS:
            b = self.button(name, f"{tip}  [{key}]", checkable=True)
            b.clicked.connect(lambda _=False, n=name: canvas.set_tool(n))
            self.group.addButton(b)
            self.buttons[name] = b
            self.lay.addWidget(b)
        self.fit()

    def sync(self, tool):
        if tool in self.buttons:
            self.buttons[tool].setChecked(True)


class ActionBar(Panel):
    def __init__(self, canvas, editor):
        super().__init__(canvas)
        act = canvas.actionRequested.emit
        self.undo = self.button("undo", "Отменить  [Ctrl+Z]", canvas.undo)
        self.redo = self.button("redo", "Повторить  [Ctrl+Shift+Z]", canvas.redo)
        self.layers = self.button("layers", "Панель слоёв  [Tab]", canvas.toggle_layers, checkable=True)
        for w in (self.undo, self.redo, self.sep(), self.layers, self.sep()):
            self.lay.addWidget(w)
        self.live = self.button("clipboard", "Живой буфер обмена: после каждого изменения\n"
                                "в буфер записывается итоговая картинка", canvas.toggle_live_copy, checkable=True)
        self.lay.addWidget(self.live)
        if editor:
            self.lay.addWidget(self.button("export", "Экспорт в файл…  [Ctrl+Shift+S]", lambda: act("export")))
            self.lay.addWidget(self.button("save", "Сохранить в историю  [Ctrl+S]", lambda: act("save")))
        else:
            self.lay.addWidget(self.button("edit", "Открыть в редакторе  [Ctrl+E]", lambda: act("edit")))
            self.lay.addWidget(self.button("save", "Сохранить в файл  [Ctrl+S]", lambda: act("save")))
        copy = self.button("copy", "Скопировать в буфер  [Enter / Ctrl+C]", lambda: act("copy"), text="Копировать")
        copy.setObjectName("primary")
        copy.setIcon(icons.icon("copy", "#101014"))
        self.lay.addWidget(copy)
        if not editor:
            self.lay.addWidget(self.sep())
            self.lay.addWidget(self.button("close", "Отмена  [Esc]", lambda: act("cancel")))
        self.fit()

    def sync(self):
        self.undo.setEnabled(bool(self.canvas.doc.undo_stack))
        self.redo.setEnabled(bool(self.canvas.doc.redo_stack))
        self.layers.setChecked(self.canvas.layers_visible)
        self.live.setChecked(self.canvas.live_copy)


class Slider(QWidget):
    def __init__(self, value, lo, hi, cb, suffix="", width=90):
        super().__init__()
        lay = QHBoxLayout(self)
        lay.setContentsMargins(0, 0, 0, 0)
        lay.setSpacing(4)
        self.s = QSlider(Qt.Horizontal)
        self.s.setRange(lo, hi)
        self.s.setValue(int(value))
        self.s.setFixedWidth(width)
        self.s.setFocusPolicy(Qt.NoFocus)
        hand(self.s)
        self.l = QLabel()
        self.l.setFixedWidth(34)
        self.suffix = suffix
        self.l.setText(f"{int(value)}{suffix}")
        self.s.valueChanged.connect(lambda v: (self.l.setText(f"{v}{suffix}"), cb(v)))
        lay.addWidget(self.s)
        lay.addWidget(self.l)


class PropsBar(Panel):
    """Context properties of the active tool or the selected layer."""

    def __init__(self, canvas):
        super().__init__(canvas)
        self.key = None

    def rebuild(self, force=False):
        target = self.canvas.props_target()
        key = (target[0], target[2].id if target[2] else None) if target else None
        if key == self.key and not force:
            return
        self.key = key
        # repopulate while hidden: children added to a visible widget would stay hidden
        self.hide()
        while self.lay.count():
            w = self.lay.takeAt(0).widget()
            if w:
                w.hide()
                w.setParent(None)
                w.deleteLater()
        if not target:
            self.hide()
            return
        kind, st, layer = target
        add = self.lay.addWidget
        cv = self.canvas

        def label(t):
            add(QLabel(t))

        def color(k, tip, allow_none=False):
            b = ColorButton(cv, st.get(k), allow_none, tip)
            b.colorChanged.connect(lambda c: cv.apply_style(k, c))
            add(b)

        def slider(k, lo, hi, tip, suffix="", width=90):
            if tip:
                label(tip)
            add(Slider(st.get(k, lo), lo, hi, lambda v: cv.apply_style(k, v), suffix, width))

        def toggle(k, ic, tip):
            b = self.button(ic, tip, checkable=True)
            b.setChecked(bool(st.get(k)))
            b.toggled.connect(lambda v: cv.apply_style(k, v))
            add(b)

        def choice(k, opts, relayout=False):
            grp = QButtonGroup(self)
            for value, ic, tip in opts:
                b = self.button(ic, tip, checkable=True)
                b.setChecked(st.get(k) == value)
                b.clicked.connect(lambda _=False, v=value: (
                    cv.apply_style(k, v),
                    relayout and QTimer.singleShot(0, lambda: self.rebuild(force=True))))
                grp.addButton(b)
                add(b)

        title = QLabel(cv.doc.display_name(layer) if layer else {
            "pen": "Кисть", "highlighter": "Выделитель", "line": "Линия", "arrow": "Стрелка",
            "rect": "Прямоугольник", "ellipse": "Эллипс", "text": "Текст", "mark": "Метка",
            "blur": "Размытие", "zoom": "Лупа"}.get(kind, kind))
        title.setObjectName("title")
        add(title)
        add(self.sep())

        if kind in ("pen", "highlighter"):
            color("color", "Цвет")
            slider("width", 1, 80, "Толщина")
        elif kind in ("line", "arrow"):
            color("color", "Цвет")
            slider("width", 1, 40, "Толщина")
            toggle("dash", "dash", "Пунктир")
            if kind == "arrow":
                choice("heads", [("end", "arrow", "Наконечник с одной стороны"),
                                 ("both", "arrow_both", "Наконечники с двух сторон")])
        elif kind in ("rect", "ellipse"):
            label("Контур")
            color("stroke", "Цвет контура", True)
            label("Заливка")
            color("fill", "Цвет заливки", True)
            slider("width", 0, 40, "Толщина")
            if kind == "rect":
                slider("radius", 0, 60, "Скругление")
            toggle("dash", "dash", "Пунктир")
        elif kind == "text":
            color("color", "Цвет текста")
            fc = QFontComboBox()
            fc.setCurrentFont(QFont(st.get("family", "Sans Serif")))
            fc.setFixedWidth(150)
            fc.setFocusPolicy(Qt.ClickFocus)
            hand(fc, fc.view())
            fc.currentFontChanged.connect(lambda f: cv.apply_style("family", f.family()))
            add(fc)
            slider("size", 8, 160, "Размер")
            toggle("bold", "bold", "Жирный")
            toggle("italic", "italic", "Курсив")
            label("Обводка")
            color("outline", "Цвет обводки", True)
            label("Фон")
            color("bg", "Цвет подложки", True)
        elif kind == "mark":
            choice("shape", [("circle", "circle", "Круг"), ("square", "square", "Квадрат"),
                             ("diamond", "diamond", "Ромб")])
            slider("size", 16, 120, "Размер")
            label("Цифра")
            color("text", "Цвет номера")
            label("Фон")
            color("fill", "Цвет фона (и стрелки)", True)
            label("Контур")
            color("outline", "Цвет контура", True)
            slider("outline_w", 0, 10, "Толщина")
            nxt = QLabel(f"след. №{len(cv.doc.mark_numbers()) + int(cv.doc.meta.get('mark_start', 1))}")
            add(nxt)
        elif kind == "zoom":
            shapes.zoom_upgrade(st)
            choice("shape", [("ellipse", "ellipse", "Эллипс"), ("rect", "rect_sharp", "Прямоугольник"),
                             ("round", "round", "Скруглённый прямоугольник")], relayout=True)
            if st.get("shape") == "round":
                slider("radius", 2, 80, "Скругл.", width=60)
            add(self.sep())
            label("Область")
            color("src_color", "Цвет рамки выделяемой области")
            slider("src_width", 0, 12, "", width=60)
            toggle("src_dash", "dash", "Пунктирная рамка области")
            add(self.sep())
            label("Копия")
            color("stroke", "Цвет рамки копии")
            slider("width", 0, 12, "", width=60)
            toggle("shadow", "shadow", "Тень под копией")
            toggle("smooth", "smooth", "Сглаживание при увеличении (иначе чёткие пиксели)")
            add(self.sep())
            toggle("lines", "lines", "Линии-выноски от области к копии")
            color("line_color", "Цвет линий-выносок")
            slider("line_width", 1, 10, "", width=60)
            toggle("dash", "dash", "Пунктирные линии-выноски")
            if layer:
                k = layer.g["dst"][2] / max(layer.g["src"][2], 1e-6)
                label(f"увеличение ×{k:.1f}")
        elif kind == "blur":
            choice("mode", [("pixelate", "blur", "Пикселизация"), ("blur", "drop", "Размытие")])
            slider("strength", 2, 60, "Сила")
        self.fit()
        self.show()
        cv.layout_panels()


class LayersPanel(Panel):
    def __init__(self, canvas):
        super().__init__(canvas, vertical=True)
        self._updating = False
        head = QHBoxLayout()
        head.setSpacing(2)
        t = QLabel("Слои")
        t.setObjectName("title")
        head.addWidget(t)
        head.addStretch()
        for name, tip, cb in (("duplicate", "Дублировать выбранные  [Ctrl+D]", canvas.duplicate_selected),
                              ("up", "Выше  [Ctrl+]]", lambda: canvas.move_selected(1)),
                              ("down", "Ниже  [Ctrl+[]", lambda: canvas.move_selected(-1)),
                              ("trash", "Удалить выбранные  [Del]", canvas.delete_selected)):
            b = self.button(name, tip, cb)
            b.setIconSize(QSize(16, 16))
            head.addWidget(b)
        self.lay.addLayout(head)

        self.list = QListWidget()
        self.list.setFixedWidth(230)
        self.list.setSelectionMode(QAbstractItemView.ExtendedSelection)
        self.list.setToolTip("Ctrl/Shift+клик — выбрать несколько")
        hand(self.list.viewport())
        self.list.setDragDropMode(QAbstractItemView.InternalMove)
        self.list.setEditTriggers(QAbstractItemView.DoubleClicked)
        self.list.setFocusPolicy(Qt.ClickFocus)
        self.list.setIconSize(QSize(16, 16))
        self.list.itemChanged.connect(self._item_changed)
        self.list.itemSelectionChanged.connect(self._selection_changed)
        self.list.model().rowsMoved.connect(self._rows_moved)
        self.lay.addWidget(self.list)

        row = QHBoxLayout()
        row.addWidget(QLabel("Наложение"))
        self.blend = QComboBox()
        for k, name, _ in BLEND_MODES:
            self.blend.addItem(name, k)
        self.blend.setFocusPolicy(Qt.NoFocus)
        hand(self.blend, self.blend.view())
        self.blend.activated.connect(lambda i: canvas.set_layer_prop("blend", self.blend.itemData(i)))
        row.addWidget(self.blend, 1)
        self.lay.addLayout(row)
        row = QHBoxLayout()
        row.addWidget(QLabel("Непрозрачн."))
        self.opacity = Slider(100, 0, 100, lambda v: self._updating or canvas.set_layer_prop("opacity", v / 100),
                              "%", 110)
        row.addWidget(self.opacity, 1)
        self.lay.addLayout(row)
        self.refresh()

    def refresh(self):
        cv = self.canvas
        self._updating = True
        self.list.clear()
        numbers = cv.doc.mark_numbers()
        selected = {l.id for l in cv.sel_layers()}
        for layer in reversed(cv.doc.layers):
            it = QListWidgetItem(icons.icon(layer.type if layer.type != "blur" else "blur"),
                                 cv.doc.display_name(layer, numbers))
            it.setData(Qt.UserRole, layer.id)
            it.setFlags(it.flags() | Qt.ItemIsUserCheckable | Qt.ItemIsEditable | Qt.ItemIsDragEnabled)
            it.setFlags(it.flags() & ~Qt.ItemIsDropEnabled)
            it.setCheckState(Qt.Checked if layer.visible else Qt.Unchecked)
            it.setToolTip("Галочка — видимость, двойной клик — переименовать, перетаскивание — порядок")
            self.list.addItem(it)
            if layer.id in selected:
                it.setSelected(True)
            if cv.selected is layer:
                self.list.setCurrentItem(it, QItemSelectionModel.NoUpdate)
        base = QListWidgetItem(icons.icon("rect"), "Снимок (фон)")
        base.setFlags(Qt.NoItemFlags)
        self.list.addItem(base)
        h = min(440, max(80, self.list.count() * 30 + 6))
        self.list.setFixedHeight(h)
        sel = cv.selected
        self.blend.setEnabled(bool(sel))
        self.opacity.setEnabled(bool(sel))
        if sel:
            self.blend.setCurrentIndex(max(0, self.blend.findData(sel.blend)))
            self.opacity.s.setValue(round(sel.opacity * 100))
        self._updating = False
        self.fit()

    def _id(self, it):
        return it.data(Qt.UserRole) if it else None

    def _item_changed(self, it):
        if self._updating:
            return
        layer = self.canvas.doc.find(self._id(it))
        if not layer:
            return
        vis = it.checkState() == Qt.Checked
        name = it.text().strip()
        if vis != layer.visible:
            self.canvas.set_layer_prop("visible", vis, layer)
        elif name and name != self.canvas.doc.display_name(layer):
            self.canvas.set_layer_prop("name", name, layer)

    def _selection_changed(self):
        if self._updating:
            return
        ids = {self._id(it) for it in self.list.selectedItems()} - {None}
        cur = self._id(self.list.currentItem())
        self.canvas.select_many(ids, cur)
        sel = self.canvas.selected
        self._updating = True
        self.blend.setEnabled(bool(sel))
        self.opacity.setEnabled(bool(sel))
        if sel:
            self.blend.setCurrentIndex(max(0, self.blend.findData(sel.blend)))
            self.opacity.s.setValue(round(sel.opacity * 100))
        self._updating = False

    def _rows_moved(self, *a):
        if self._updating:
            return
        ids = [self._id(self.list.item(i)) for i in range(self.list.count())]
        self.canvas.reorder([i for i in reversed(ids) if i])
