//! Capture overlay: one frozen, dimmed layer-shell surface per output, drawn with egui on
//! EGL. eframe/winit cannot make layer surfaces, so this is a small Wayland backend of its own:
//! sctk for the protocol plumbing, fractional-scale + viewporter for crisp pixels, one egui
//! context per output, keyboard routed by the multi-monitor coordinator.
use std::sync::Arc;
use std::time::{Duration, Instant};

use khronos_egl as egl;
use smithay_client_toolkit::compositor::{CompositorHandler, CompositorState};
use smithay_client_toolkit::output::{OutputHandler, OutputState};
use smithay_client_toolkit::reexports::calloop::EventLoop;
use smithay_client_toolkit::reexports::calloop_wayland_source::WaylandSource;
use smithay_client_toolkit::registry::{ProvidesRegistryState, RegistryState};
use smithay_client_toolkit::seat::keyboard::{
    KeyEvent, KeyboardHandler, Keysym, Modifiers as SModifiers, RawModifiers,
};
use smithay_client_toolkit::seat::pointer::{PointerEvent, PointerEventKind, PointerHandler, ThemeSpec, ThemedPointer};
use smithay_client_toolkit::seat::{Capability, SeatHandler, SeatState};
use smithay_client_toolkit::shell::WaylandSurface;
use smithay_client_toolkit::shell::wlr_layer::{
    Anchor, KeyboardInteractivity, Layer, LayerShell, LayerShellHandler, LayerSurface, LayerSurfaceConfigure,
};
use smithay_client_toolkit::shm::{Shm, ShmHandler};
use smithay_client_toolkit::{
    delegate_compositor, delegate_keyboard, delegate_layer, delegate_output, delegate_pointer, delegate_registry,
    delegate_seat, delegate_shm, registry_handlers,
};
use wayland_client::globals::{GlobalList, registry_queue_init};
use wayland_client::protocol::{wl_keyboard, wl_output, wl_pointer, wl_seat, wl_surface};
use wayland_client::{Connection, Dispatch, Proxy, QueueHandle, delegate_noop};
use wayland_protocols::wp::fractional_scale::v1::client::{wp_fractional_scale_manager_v1, wp_fractional_scale_v1};
use wayland_protocols::wp::viewporter::client::{wp_viewport, wp_viewporter};

use crate::capture::{Capture, Output};
use crate::config::Config;
use crate::geom::R;
use crate::i18n::tr;
use crate::model::Document;
use crate::ui::canvas::{Action, Canvas, Ctl, Mode};
use crate::ui::{panels, style};

struct Gl {
    egl: egl::Instance<egl::Static>,
    display: egl::Display,
    config: egl::Config,
    context: egl::Context,
    gl: Arc<glow::Context>,
}

struct Surf {
    layer: LayerSurface,
    viewport: Option<wp_viewport::WpViewport>,
    _fractional: Option<wp_fractional_scale_v1::WpFractionalScaleV1>,
    egl_window: Option<wayland_egl::WlEglSurface>,
    egl_surface: Option<egl::Surface>,
    size: (u32, u32), // logical
    scale: f64,
    configured: bool,
    frame_pending: bool,
    repaint_at: Option<Instant>,
    ctx: egui::Context,
    painter: Option<egui_glow::Painter>,
    events: Vec<egui::Event>,
    canvas: Canvas,
    cursor: egui::CursorIcon,
}

struct App {
    conn: Connection,
    registry: RegistryState,
    output_state: OutputState,
    seat_state: SeatState,
    compositor: CompositorState,
    shm: Shm,
    gl: Gl,
    surfaces: Vec<Surf>,
    pointer: Option<ThemedPointer>,
    pointer_on: Option<usize>,
    pointer_pos: egui::Pos2,
    keyboard: Option<wl_keyboard::WlKeyboard>,
    mods: egui::Modifiers,
    ctl: Ctl,
    cfg: Config,
    start: Instant,
    done: Option<(Action, usize)>,
    cursor_set: Option<egui::CursorIcon>,
}

impl App {
    fn idx(&self, s: &wl_surface::WlSurface) -> Option<usize> {
        self.surfaces.iter().position(|x| x.layer.wl_surface() == s)
    }

    fn key_target(&self) -> usize {
        // an idle screen under the cursor must not steal Enter/Ctrl+C from the screen with the selection
        match self.ctl.locked {
            Some(l) if l != self.ctl.active && self.surfaces[self.ctl.active].canvas.mode == Mode::Idle => l,
            _ => self.ctl.active,
        }
    }

    fn push_event(&mut self, i: usize, ev: egui::Event) {
        if let Some(s) = self.surfaces.get_mut(i) {
            s.events.push(ev);
            s.repaint_at = Some(Instant::now());
        }
    }

    fn set_active(&mut self, i: usize) {
        self.ctl.active = i;
        for (j, s) in self.surfaces.iter_mut().enumerate() {
            let on = j == i;
            if s.canvas.active != on {
                s.canvas.active = on;
                s.repaint_at = Some(Instant::now());
            }
        }
    }

    fn render(&mut self, i: usize, qh: &QueueHandle<App>) {
        let keyboard = i == self.key_target();
        let s = &mut self.surfaces[i];
        if !s.configured || s.frame_pending {
            return;
        }
        let Some(egl_surface) = s.egl_surface else { return };
        if self
            .gl
            .egl
            .make_current(
                self.gl.display,
                Some(egl_surface),
                Some(egl_surface),
                Some(self.gl.context),
            )
            .is_err()
        {
            return;
        }
        s.repaint_at = None;
        let (w, h) = s.size;
        let ppp = s.scale as f32;
        let mut raw = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(w as f32, h as f32),
            )),
            time: Some(self.start.elapsed().as_secs_f64()),
            events: std::mem::take(&mut s.events),
            focused: true,
            max_texture_side: s.painter.as_ref().map(|p| p.max_texture_side()),
            ..Default::default()
        };
        raw.viewports
            .entry(egui::ViewportId::ROOT)
            .or_default()
            .native_pixels_per_point = Some(ppp);
        let cfg = &mut self.cfg;
        let ctl = &mut self.ctl;
        let canvas = &mut s.canvas;
        canvas.rect = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(w as f32, h as f32));
        let mut out = s.ctx.run_ui(raw, |ui| {
            let ctx = &ui.ctx().clone();
            canvas.input(ctx, cfg, Some(ctl), keyboard);
            canvas.paint(ctx);
            panels::show(canvas, ctx, cfg);
        });
        // a popup closed by a click on the canvas still has to land in the recent colors
        let wants_paste = out
            .platform_output
            .commands
            .iter()
            .any(|c| matches!(c, egui::OutputCommand::CopyText(_)));
        for c in &out.platform_output.commands {
            if let egui::OutputCommand::CopyText(t) = c {
                crate::sys::copy_text(t.as_str());
            }
        }
        let _ = wants_paste;
        s.cursor = out.platform_output.cursor_icon;
        let prims = s.ctx.tessellate(std::mem::take(&mut out.shapes), ppp);
        let px = ((w as f64 * s.scale).round() as u32, (h as f64 * s.scale).round() as u32);
        if let Some(p) = &mut s.painter {
            egui_glow::painter::clear(&self.gl.gl, [px.0, px.1], [0.0, 0.0, 0.0, 1.0]);
            p.paint_and_update_textures([px.0, px.1], ppp, &prims, &mut out.textures_delta);
        }
        let surface = s.layer.wl_surface().clone();
        surface.frame(qh, surface.clone());
        s.frame_pending = true;
        let _ = self.gl.egl.swap_buffers(self.gl.display, egl_surface);
        let delay = out
            .viewport_output
            .get(&egui::ViewportId::ROOT)
            .map_or(Duration::MAX, |v| v.repaint_delay);
        if delay < Duration::from_secs(1000) {
            let at = Instant::now() + delay;
            s.repaint_at = Some(s.repaint_at.map_or(at, |x| x.min(at)));
        }
        for a in s.canvas.take_actions() {
            if self.done.is_none() {
                self.done = Some((a, i));
            }
        }
        // coordinator requests raised during the frame
        if let Some(j) = self.ctl.reset.take()
            && let Some(t) = self.surfaces.get_mut(j)
        {
            t.canvas.reset_selection(None);
            t.repaint_at = Some(Instant::now());
        }
        if let Some(j) = self.ctl.activate.take() {
            self.set_active(j);
        }
        if self.pointer_on == Some(i) {
            self.update_cursor(i);
        }
    }

    fn update_cursor(&mut self, i: usize) {
        let icon = self.surfaces[i].cursor;
        if self.cursor_set == Some(icon) {
            return;
        }
        if let Some(p) = &self.pointer
            && p.set_cursor(&self.conn, to_cursor_icon(icon)).is_ok()
        {
            self.cursor_set = Some(icon);
        }
    }
}

fn to_cursor_icon(c: egui::CursorIcon) -> cursor_icon::CursorIcon {
    use cursor_icon::CursorIcon as C;
    use egui::CursorIcon as E;
    match c {
        E::Crosshair => C::Crosshair,
        E::PointingHand => C::Pointer,
        E::Text => C::Text,
        E::Move | E::AllScroll => C::Move,
        E::Grab => C::Grab,
        E::Grabbing => C::Grabbing,
        E::ResizeHorizontal | E::ResizeEast | E::ResizeWest => C::EwResize,
        E::ResizeVertical | E::ResizeNorth | E::ResizeSouth => C::NsResize,
        E::ResizeNwSe | E::ResizeNorthWest | E::ResizeSouthEast => C::NwseResize,
        E::ResizeNeSw | E::ResizeNorthEast | E::ResizeSouthWest => C::NeswResize,
        E::NotAllowed | E::NoDrop => C::NotAllowed,
        E::Wait => C::Wait,
        E::Progress => C::Progress,
        E::Help => C::Help,
        _ => C::Default,
    }
}

fn keysym_to_key(sym: Keysym, raw: u32) -> Option<egui::Key> {
    use egui::Key as K;
    let k = match sym {
        Keysym::Escape => K::Escape,
        Keysym::Return | Keysym::KP_Enter => K::Enter,
        Keysym::Tab | Keysym::ISO_Left_Tab => K::Tab,
        Keysym::BackSpace => K::Backspace,
        Keysym::Delete | Keysym::KP_Delete => K::Delete,
        Keysym::Left | Keysym::KP_Left => K::ArrowLeft,
        Keysym::Right | Keysym::KP_Right => K::ArrowRight,
        Keysym::Up | Keysym::KP_Up => K::ArrowUp,
        Keysym::Down | Keysym::KP_Down => K::ArrowDown,
        Keysym::Home => K::Home,
        Keysym::End => K::End,
        Keysym::Page_Up => K::PageUp,
        Keysym::Page_Down => K::PageDown,
        Keysym::Insert => K::Insert,
        Keysym::space => K::Space,
        Keysym::bracketleft | Keysym::braceleft => K::OpenBracket,
        Keysym::bracketright | Keysym::braceright => K::CloseBracket,
        _ => {
            let raw_sym = sym.raw();
            if let Some(c) = char::from_u32(raw_sym).filter(|c| c.is_ascii_alphanumeric()) {
                return egui::Key::from_name(&c.to_ascii_uppercase().to_string());
            }
            // non-Latin layouts (Ctrl+Я is still Ctrl+Z): fall back to the key position
            return evdev_key(raw);
        }
    };
    Some(k)
}

/// US-layout position of an evdev key code, for shortcuts typed on non-Latin layouts.
fn evdev_key(code: u32) -> Option<egui::Key> {
    const ROWS: &[(u32, &str)] = &[
        (2, "1234567890"),
        (16, "QWERTYUIOP"),
        (30, "ASDFGHJKL"),
        (44, "ZXCVBNM"),
    ];
    for (start, keys) in ROWS {
        if code >= *start && code < start + keys.len() as u32 {
            let c = keys.as_bytes()[(code - start) as usize] as char;
            return egui::Key::from_name(&c.to_string());
        }
    }
    match code {
        26 => Some(egui::Key::OpenBracket),
        27 => Some(egui::Key::CloseBracket),
        _ => None,
    }
}

impl CompositorHandler for App {
    fn scale_factor_changed(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        surface: &wl_surface::WlSurface,
        factor: i32,
    ) {
        if let Some(i) = self.idx(surface)
            && self.surfaces[i].viewport.is_none()
        {
            let s = &mut self.surfaces[i];
            s.scale = factor as f64;
            surface.set_buffer_scale(factor);
            resize_egl(s);
            s.repaint_at = Some(Instant::now());
        }
    }
    fn transform_changed(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_surface::WlSurface,
        _: wl_output::Transform,
    ) {
    }
    fn frame(&mut self, _: &Connection, _: &QueueHandle<Self>, surface: &wl_surface::WlSurface, _: u32) {
        if let Some(i) = self.idx(surface) {
            self.surfaces[i].frame_pending = false;
        }
    }
    fn surface_enter(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_surface::WlSurface,
        _: &wl_output::WlOutput,
    ) {
    }
    fn surface_leave(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_surface::WlSurface,
        _: &wl_output::WlOutput,
    ) {
    }
}

impl OutputHandler for App {
    fn output_state(&mut self) -> &mut OutputState {
        &mut self.output_state
    }
    fn new_output(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {}
    fn update_output(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {}
    fn output_destroyed(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {}
}

fn resize_egl(s: &mut Surf) {
    let (w, h) = s.size;
    let pw = (w as f64 * s.scale).round() as i32;
    let ph = (h as f64 * s.scale).round() as i32;
    if let Some(win) = &s.egl_window {
        win.resize(pw, ph, 0, 0);
    }
    if let Some(vp) = &s.viewport {
        vp.set_destination(w as i32, h as i32);
    }
}

impl LayerShellHandler for App {
    fn closed(&mut self, _: &Connection, _: &QueueHandle<Self>, layer: &LayerSurface) {
        if let Some(i) = self.idx(layer.wl_surface())
            && self.done.is_none()
        {
            self.done = Some((Action::Cancel, i));
        }
    }

    fn configure(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        layer: &LayerSurface,
        cfg: LayerSurfaceConfigure,
        _: u32,
    ) {
        let Some(i) = self.idx(layer.wl_surface()) else { return };
        let gl = &self.gl;
        let s = &mut self.surfaces[i];
        let (w, h) = (cfg.new_size.0.max(1), cfg.new_size.1.max(1));
        s.size = (w, h);
        if s.egl_window.is_none() {
            let pw = (w as f64 * s.scale).round() as i32;
            let ph = (h as f64 * s.scale).round() as i32;
            let Ok(win) = wayland_egl::WlEglSurface::new(s.layer.wl_surface().id(), pw, ph) else {
                return;
            };
            let surf = unsafe {
                gl.egl
                    .create_window_surface(gl.display, gl.config, win.ptr() as egl::NativeWindowType, None)
            };
            let Ok(surf) = surf else { return };
            s.egl_window = Some(win);
            s.egl_surface = Some(surf);
            let _ = gl
                .egl
                .make_current(gl.display, Some(surf), Some(surf), Some(gl.context));
            let _ = gl.egl.swap_interval(gl.display, 0);
            s.painter = egui_glow::Painter::new(gl.gl.clone(), "", None, false).ok();
            if let Some(vp) = &s.viewport {
                vp.set_destination(w as i32, h as i32);
            } else {
                s.layer.wl_surface().set_buffer_scale(s.scale.round() as i32);
            }
        } else {
            resize_egl(s);
        }
        s.configured = true;
        s.repaint_at = Some(Instant::now());
    }
}

impl SeatHandler for App {
    fn seat_state(&mut self) -> &mut SeatState {
        &mut self.seat_state
    }
    fn new_seat(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_seat::WlSeat) {}
    fn new_capability(&mut self, _: &Connection, qh: &QueueHandle<Self>, seat: wl_seat::WlSeat, cap: Capability) {
        if cap == Capability::Keyboard && self.keyboard.is_none() {
            self.keyboard = self.seat_state.get_keyboard(qh, &seat, None).ok();
        }
        if cap == Capability::Pointer && self.pointer.is_none() {
            let surface = self.compositor.create_surface(qh);
            self.pointer = self
                .seat_state
                .get_pointer_with_theme(qh, &seat, self.shm.wl_shm(), surface, ThemeSpec::default())
                .ok();
        }
    }
    fn remove_capability(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_seat::WlSeat, cap: Capability) {
        if cap == Capability::Keyboard
            && let Some(k) = self.keyboard.take()
        {
            k.release();
        }
        if cap == Capability::Pointer
            && let Some(p) = self.pointer.take()
        {
            p.pointer().release();
        }
    }
    fn remove_seat(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_seat::WlSeat) {}
}

impl KeyboardHandler for App {
    fn enter(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_keyboard::WlKeyboard,
        _: &wl_surface::WlSurface,
        _: u32,
        _: &[u32],
        _: &[Keysym],
    ) {
    }
    fn leave(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_keyboard::WlKeyboard,
        _: &wl_surface::WlSurface,
        _: u32,
    ) {
    }
    fn press_key(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_keyboard::WlKeyboard, _: u32, ev: KeyEvent) {
        self.key(ev, true, false);
    }
    fn repeat_key(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_keyboard::WlKeyboard, _: u32, ev: KeyEvent) {
        self.key(ev, true, true);
    }
    fn release_key(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_keyboard::WlKeyboard,
        _: u32,
        ev: KeyEvent,
    ) {
        self.key(ev, false, false);
    }
    fn update_modifiers(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_keyboard::WlKeyboard,
        _: u32,
        m: SModifiers,
        _: RawModifiers,
        _: u32,
    ) {
        self.mods = egui::Modifiers {
            alt: m.alt,
            ctrl: m.ctrl,
            shift: m.shift,
            mac_cmd: false,
            command: m.ctrl,
        };
        let ev = egui::Event::ModifiersChanged(self.mods);
        for i in 0..self.surfaces.len() {
            self.surfaces[i].events.push(ev.clone());
        }
    }
}

impl App {
    fn key(&mut self, ev: KeyEvent, pressed: bool, repeat: bool) {
        let target = self.key_target();
        let key = keysym_to_key(ev.keysym, ev.raw_code);
        if pressed && self.mods.ctrl && key == Some(egui::Key::V) {
            if let Some(t) = crate::sys::paste_text() {
                self.push_event(target, egui::Event::Paste(t));
            }
            return;
        }
        if let Some(k) = key {
            self.push_event(
                target,
                egui::Event::Key {
                    key: k,
                    physical_key: None,
                    pressed,
                    repeat,
                    modifiers: self.mods,
                },
            );
        }
        if pressed
            && !self.mods.ctrl
            && !self.mods.alt
            && let Some(t) = ev.utf8.filter(|t| !t.is_empty() && t.chars().all(|c| !c.is_control()))
        {
            self.push_event(target, egui::Event::Text(t));
        }
    }
}

impl PointerHandler for App {
    fn pointer_frame(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_pointer::WlPointer,
        events: &[PointerEvent],
    ) {
        for e in events {
            let Some(i) = self.idx(&e.surface) else { continue };
            let pos = egui::pos2(e.position.0 as f32, e.position.1 as f32);
            match e.kind {
                PointerEventKind::Enter { .. } => {
                    self.pointer_on = Some(i);
                    self.cursor_set = None;
                    self.pointer_pos = pos;
                    self.push_event(i, egui::Event::PointerMoved(pos));
                    self.update_cursor(i);
                }
                PointerEventKind::Leave { .. } => {
                    if self.pointer_on == Some(i) {
                        self.pointer_on = None;
                    }
                    self.push_event(i, egui::Event::PointerGone);
                }
                PointerEventKind::Motion { .. } => {
                    self.pointer_pos = pos;
                    self.push_event(i, egui::Event::PointerMoved(pos));
                }
                PointerEventKind::Press { button, .. } | PointerEventKind::Release { button, .. } => {
                    let pressed = matches!(e.kind, PointerEventKind::Press { .. });
                    let b = match button {
                        0x110 => egui::PointerButton::Primary,
                        0x111 => egui::PointerButton::Secondary,
                        0x112 => egui::PointerButton::Middle,
                        0x113 => egui::PointerButton::Extra1,
                        0x114 => egui::PointerButton::Extra2,
                        _ => continue,
                    };
                    self.push_event(
                        i,
                        egui::Event::PointerButton {
                            pos,
                            button: b,
                            pressed,
                            modifiers: self.mods,
                        },
                    );
                }
                PointerEventKind::Axis {
                    horizontal, vertical, ..
                } => {
                    let (unit, delta) = if vertical.value120 != 0 || horizontal.value120 != 0 {
                        (
                            egui::MouseWheelUnit::Line,
                            egui::vec2(-horizontal.value120 as f32 / 120.0, -vertical.value120 as f32 / 120.0),
                        )
                    } else if vertical.discrete != 0 || horizontal.discrete != 0 {
                        (
                            egui::MouseWheelUnit::Line,
                            egui::vec2(-horizontal.discrete as f32, -vertical.discrete as f32),
                        )
                    } else {
                        (
                            egui::MouseWheelUnit::Point,
                            egui::vec2(-horizontal.absolute as f32, -vertical.absolute as f32),
                        )
                    };
                    if delta != egui::Vec2::ZERO {
                        self.push_event(
                            i,
                            egui::Event::MouseWheel {
                                unit,
                                delta,
                                modifiers: self.mods,
                                phase: egui::TouchPhase::Move,
                            },
                        );
                    }
                }
            }
        }
    }
}

impl ShmHandler for App {
    fn shm_state(&mut self) -> &mut Shm {
        &mut self.shm
    }
}

impl ProvidesRegistryState for App {
    fn registry(&mut self) -> &mut RegistryState {
        &mut self.registry
    }
    registry_handlers![OutputState, SeatState];
}

impl Dispatch<wp_fractional_scale_v1::WpFractionalScaleV1, wl_surface::WlSurface> for App {
    fn event(
        app: &mut Self,
        _: &wp_fractional_scale_v1::WpFractionalScaleV1,
        ev: wp_fractional_scale_v1::Event,
        surface: &wl_surface::WlSurface,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let wp_fractional_scale_v1::Event::PreferredScale { scale } = ev
            && let Some(i) = app.idx(surface)
        {
            let s = &mut app.surfaces[i];
            let new = scale as f64 / 120.0;
            if (new - s.scale).abs() > 1e-6 {
                s.scale = new;
                resize_egl(s);
                s.repaint_at = Some(Instant::now());
            }
        }
    }
}

delegate_noop!(App: ignore wp_fractional_scale_manager_v1::WpFractionalScaleManagerV1);
delegate_noop!(App: ignore wp_viewporter::WpViewporter);
delegate_noop!(App: ignore wp_viewport::WpViewport);
delegate_compositor!(App);
delegate_output!(App);
delegate_shm!(App);
delegate_seat!(App);
delegate_keyboard!(App);
delegate_pointer!(App);
delegate_layer!(App);
delegate_registry!(App);

fn init_egl(conn: &Connection) -> Result<Gl, String> {
    let e = egl::Instance::new(egl::Static);
    const PLATFORM_WAYLAND: egl::Enum = 0x31D8;
    let display = unsafe {
        e.get_platform_display(
            PLATFORM_WAYLAND,
            conn.backend().display_ptr() as *mut _,
            &[egl::ATTRIB_NONE],
        )
    }
    .map_err(|err| format!("EGL display: {err}"))?;
    e.initialize(display).map_err(|err| format!("eglInitialize: {err}"))?;
    e.bind_api(egl::OPENGL_ES_API).map_err(|err| err.to_string())?;
    let attrs = [
        egl::SURFACE_TYPE,
        egl::WINDOW_BIT,
        egl::RENDERABLE_TYPE,
        egl::OPENGL_ES3_BIT,
        egl::RED_SIZE,
        8,
        egl::GREEN_SIZE,
        8,
        egl::BLUE_SIZE,
        8,
        egl::ALPHA_SIZE,
        8,
        egl::NONE,
    ];
    let config = e
        .choose_first_config(display, &attrs)
        .map_err(|err| err.to_string())?
        .ok_or("no EGL config")?;
    let context = e
        .create_context(display, config, None, &[egl::CONTEXT_MAJOR_VERSION, 3, egl::NONE])
        .map_err(|err| format!("eglCreateContext: {err}"))?;
    e.make_current(display, None, None, Some(context))
        .map_err(|err| err.to_string())?;
    let gl = unsafe {
        glow::Context::from_loader_function(|s| e.get_proc_address(s).map_or(std::ptr::null(), |f| f as *const _))
    };
    Ok(Gl {
        egl: e,
        display,
        config,
        context,
        gl: Arc::new(gl),
    })
}

/// Show the overlay over every captured output; returns the process exit code.
pub fn run(cap: Capture, mut cfg: Config) -> i32 {
    let conn = match Connection::connect_to_env() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("kadr: Wayland: {e}");
            return 1;
        }
    };
    let (globals, queue): (GlobalList, _) = match registry_queue_init::<App>(&conn) {
        Ok(x) => x,
        Err(e) => {
            eprintln!("kadr: Wayland: {e}");
            return 1;
        }
    };
    let qh = queue.handle();
    let gl = match init_egl(&conn) {
        Ok(g) => g,
        Err(e) => {
            eprintln!("kadr: {e}");
            return 1;
        }
    };
    let (Ok(compositor), Ok(layer_shell), Ok(shm)) = (
        CompositorState::bind(&globals, &qh),
        LayerShell::bind(&globals, &qh),
        Shm::bind(&globals, &qh),
    ) else {
        eprintln!("kadr: the compositor has no wlr-layer-shell");
        return 1;
    };
    let fractional: Option<wp_fractional_scale_manager_v1::WpFractionalScaleManagerV1> =
        globals.bind(&qh, 1..=1, ()).ok();
    let viewporter: Option<wp_viewporter::WpViewporter> = globals.bind(&qh, 1..=1, ()).ok();
    let mut event_loop: EventLoop<App> = match EventLoop::try_new() {
        Ok(l) => l,
        Err(e) => {
            eprintln!("kadr: {e}");
            return 1;
        }
    };
    let mut app = App {
        conn: conn.clone(),
        registry: RegistryState::new(&globals),
        output_state: OutputState::new(&globals, &qh),
        seat_state: SeatState::new(&globals, &qh),
        compositor,
        shm,
        gl,
        surfaces: vec![],
        pointer: None,
        pointer_on: None,
        pointer_pos: egui::Pos2::ZERO,
        keyboard: None,
        mods: egui::Modifiers::NONE,
        ctl: Ctl::default(),
        cfg: cfg.clone(),
        start: Instant::now(),
        done: None,
        cursor_set: None,
    };
    if WaylandSource::new(conn.clone(), queue)
        .insert(event_loop.handle())
        .is_err()
    {
        return 1;
    }
    // let the output/seat globals arrive
    let _ = event_loop.dispatch(Duration::from_millis(50), &mut app);
    let _ = conn.roundtrip();
    let _ = event_loop.dispatch(Duration::ZERO, &mut app);

    let created = crate::history::now_iso();
    let accent = cfg.accent();
    let mut active = None;
    for o in &cap.outputs {
        let Some(wl_out) = app
            .output_state
            .outputs()
            .find(|w| app.output_state.info(w).and_then(|i| i.name).as_deref() == Some(&o.name))
        else {
            continue;
        };
        let Some(img) = &o.image else { continue };
        let surface = app.compositor.create_surface(&qh);
        let layer = layer_shell.create_layer_surface(&qh, surface.clone(), Layer::Overlay, Some("kadr"), Some(&wl_out));
        layer.set_anchor(Anchor::all());
        layer.set_keyboard_interactivity(KeyboardInteractivity::Exclusive);
        layer.set_exclusive_zone(-1);
        let fractional_obj = fractional
            .as_ref()
            .map(|f| f.get_fractional_scale(&surface, &qh, surface.clone()));
        let viewport = if fractional.is_some() {
            viewporter.as_ref().map(|v| v.get_viewport(&surface, &qh, ()))
        } else {
            None
        };
        layer.commit();
        let doc = make_doc(o, img, &created);
        let sx = img.width() as f32 / o.w.max(1) as f32;
        let sy = img.height() as f32 / o.h.max(1) as f32;
        let mut wins = Vec::new();
        for w in &cap.windows {
            let r = R {
                x: (w.x - o.x as f32) * sx,
                y: (w.y - o.y as f32) * sy,
                w: w.w * sx,
                h: w.h * sy,
            }
            .intersect(doc.base_rect().f());
            if r.w > 20.0 && r.h > 20.0 {
                wins.push((
                    r,
                    if w.class.is_empty() {
                        w.title.clone()
                    } else {
                        format!("{} — {}", w.class, w.title)
                    },
                ));
            }
        }
        let mut canvas = Canvas::new(doc, &cfg, false, sx, wins);
        canvas.index = app.surfaces.len();
        if let Some((cx, cy)) = cap.cursor
            && o.contains(cx, cy)
        {
            active = Some(app.surfaces.len());
            canvas.mouse = Some(egui::pos2(cx - o.x as f32, cy - o.y as f32));
        }
        let ctx = egui::Context::default();
        style::setup_fonts(&ctx);
        style::setup(&ctx, accent);
        app.surfaces.push(Surf {
            layer,
            viewport,
            _fractional: fractional_obj,
            egl_window: None,
            egl_surface: None,
            size: (o.w as u32, o.h as u32),
            scale: if fractional.is_some() {
                o.scale as f64
            } else {
                o.scale.ceil() as f64
            },
            configured: false,
            frame_pending: false,
            repaint_at: Some(Instant::now()),
            ctx,
            painter: None,
            events: vec![],
            canvas,
            cursor: egui::CursorIcon::Crosshair,
        });
    }
    if app.surfaces.is_empty() {
        eprintln!("{}", tr("kadr: could not show the overlay on any output"));
        return 1;
    }
    let a = active.unwrap_or(0);
    app.set_active(a);
    if let Some(m) = app.surfaces[a].canvas.mouse {
        // seed the window hover so a click without moving still picks the window under the cursor
        app.surfaces[a].events.push(egui::Event::PointerMoved(m));
    }

    while app.done.is_none() {
        let now = Instant::now();
        let next = app.surfaces.iter().filter_map(|s| s.repaint_at).min();
        let timeout = next.map_or(Duration::from_millis(500), |t| t.saturating_duration_since(now));
        if event_loop.dispatch(Some(timeout), &mut app).is_err() {
            break;
        }
        let now = Instant::now();
        for i in 0..app.surfaces.len() {
            if app.surfaces[i].repaint_at.is_some_and(|t| t <= now) {
                app.render(i, &qh);
            }
        }
        let _ = conn.flush();
    }
    let (action, idx) = app.done.unwrap_or((Action::Cancel, 0));
    let mut canvas = None;
    // tear the overlay down first so the screen is back before slower work (PNG, history)
    let surfaces = std::mem::take(&mut app.surfaces);
    for (i, mut s) in surfaces.into_iter().enumerate() {
        if let Some(p) = &mut s.painter {
            p.destroy();
        }
        if let Some(es) = s.egl_surface.take() {
            let _ = app.gl.egl.destroy_surface(app.gl.display, es);
        }
        drop(s.egl_window.take());
        if i == idx {
            canvas = Some(s.canvas);
        }
        drop(s.layer);
    }
    let _ = conn.roundtrip();
    cfg = app.cfg.clone();
    let code = match canvas {
        Some(c) => finish(action, c, &mut cfg),
        None => 0,
    };
    cfg.save();
    code
}

fn make_doc(o: &Output, img: &Arc<tiny_skia::Pixmap>, created: &str) -> Document {
    let mut meta = serde_json::Map::new();
    meta.insert("output".into(), o.name.clone().into());
    meta.insert(
        "scale".into(),
        serde_json::json!(img.width() as f64 / o.w.max(1) as f64),
    );
    meta.insert("created".into(), created.into());
    meta.insert("origin".into(), serde_json::json!([o.x, o.y]));
    Document::new(img.clone(), None, vec![], meta)
}

fn finish(action: Action, mut cv: Canvas, cfg: &mut Config) -> i32 {
    if action == Action::Cancel {
        return 0;
    }
    if action == Action::CopyText {
        // OCR: the clipboard gets the text, not the picture
        let text = cv.copied_text.take().unwrap_or_default();
        crate::sys::copy_text(&text);
        let preview: String = text.chars().take(120).collect();
        crate::sys::notify(cfg, &tr("Text copied"), &preview, None);
        return 0;
    }
    let img = cv.result();
    let size = format!("{}×{}", img.width(), img.height());
    let limit = cfg.int("history_limit", 500).max(0) as usize;
    let to_history = |cv: &mut Canvas| match crate::history::save(&mut cv.doc, &img, None, limit) {
        Ok(d) => Some(d),
        Err(e) => {
            crate::sys::notify(cfg, &tr("Kadr: could not save to history"), &e.to_string(), None);
            None
        }
    };
    match action {
        Action::Copy => {
            crate::sys::copy_image(&img);
            let d = to_history(&mut cv);
            crate::sys::notify(
                cfg,
                &tr("Screenshot copied"),
                &size,
                d.map(|d| d.join("thumb.jpg")).as_deref(),
            );
        }
        Action::Save => {
            let path = crate::sys::save_file(&img, cfg);
            if cfg.bool("copy_on_save", true) {
                crate::sys::copy_image(&img);
            }
            to_history(&mut cv);
            match path {
                Ok(p) => crate::sys::notify(cfg, &tr("Screenshot saved"), &p.display().to_string(), Some(&p)),
                Err(e) => crate::sys::notify(cfg, &tr("Kadr: could not save to history"), &e.to_string(), None),
            }
        }
        Action::Edit | Action::Pin | Action::Export => {
            if let Some(d) = to_history(&mut cv) {
                let id = d.file_name().unwrap_or_default().to_string_lossy().into_owned();
                if action == Action::Pin {
                    let [ox, oy] = cv
                        .doc
                        .meta
                        .get("origin")
                        .and_then(|v| serde_json::from_value::<[f32; 2]>(v.clone()).ok())
                        .unwrap_or([0.0, 0.0]);
                    let s = cv.doc.meta_f("scale", 1.0);
                    let c = cv.doc.crop.unwrap_or_default();
                    let at = format!("{},{}", (ox + c.x as f32 / s).round(), (oy + c.y as f32 / s).round());
                    crate::sys::spawn_self(&["pin", &id, "--at", &at]);
                } else {
                    crate::sys::spawn_self(&["edit", &id]);
                }
            }
        }
        Action::Cancel | Action::CopyText => {}
    }
    0
}
