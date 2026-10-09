# Kadr

A Wayland screenshot tool (Hyprland; partially sway) with multi-monitor support, annotation layers and an editable history.

```
kadr tray             # tray icon: click — screenshot, middle click — history, right click — menu
kadr                  # select an area (overlay on every screen)
kadr -d 3             # same, after a 3 s delay
kadr screen           # instant shot of the monitor under the cursor -> clipboard + history
kadr history          # history gallery
kadr edit [ID|last]   # open a shot in the layer editor
```

Dependencies: python-pyside6, layer-shell-qt, grim, wl-clipboard, (libnotify).

Data: `~/.local/share/kadr/history/<id>/{base.png,doc.json,result.png,thumb.jpg}`,
settings and remembered tool styles: `~/.config/kadr/config.json`.

## Overlay keys
V pointer · P brush · H highlighter · L line · A arrow · R rectangle · E ellipse ·
T text · M mark · B blur/pixelate · Tab layers · wheel — tool width/size ·
Shift — 15° steps / square · Ctrl+Z / Ctrl+Shift+Z · Del · Ctrl+D duplicate · Ctrl+[ / ] order ·
arrows — nudge (Shift ×10) · Enter/Ctrl+C copy · Ctrl+S save · Ctrl+E editor · Esc cancel.
Marks: click — place, click+drag — place and pull out with an arrow right away; dragging later moves the mark
while the arrow stays pinned to the original spot (Ctrl+drag — move together with the anchor).
