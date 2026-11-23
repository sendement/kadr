//! Plain Wayland: output layout (wl_output v4 names + xdg-output logical geometry) and the
//! wlr-screencopy fallback for when the KMS helper is unavailable (no capability, or a
//! driver that does not export scanout buffers).
use std::os::fd::AsFd;

use wayland_client::globals::{GlobalListContents, registry_queue_init};
use wayland_client::protocol::{wl_buffer, wl_output, wl_registry, wl_shm, wl_shm_pool};
use wayland_client::{Connection, Dispatch, QueueHandle, WEnum, delegate_noop};
use wayland_protocols::xdg::xdg_output::zv1::client::{zxdg_output_manager_v1, zxdg_output_v1};
use wayland_protocols_wlr::screencopy::v1::client::{zwlr_screencopy_frame_v1, zwlr_screencopy_manager_v1};

use super::{Output, Rgba};

#[derive(Default)]
struct OutInfo {
    name: String,
    mode: (i32, i32),
    transform: u32,
    scale: i32,
    lpos: Option<(i32, i32)>,
    lsize: Option<(i32, i32)>,
}

#[derive(Default)]
struct State {
    outs: Vec<(wl_output::WlOutput, OutInfo)>,
    frame: FrameState,
}

#[derive(Default)]
struct FrameState {
    format: Option<(u32, u32, u32, u32)>, // format, w, h, stride
    flags: u32,
    done: bool,
    failed: bool,
    buffer_done: bool,
}

impl State {
    fn info(&mut self, o: &wl_output::WlOutput) -> Option<&mut OutInfo> {
        self.outs.iter_mut().find(|(w, _)| w == o).map(|(_, i)| i)
    }
}

impl Dispatch<wl_registry::WlRegistry, GlobalListContents> for State {
    fn event(
        _: &mut Self,
        _: &wl_registry::WlRegistry,
        _: wl_registry::Event,
        _: &GlobalListContents,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<wl_output::WlOutput, ()> for State {
    fn event(
        s: &mut Self,
        o: &wl_output::WlOutput,
        ev: wl_output::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        let Some(i) = s.info(o) else { return };
        match ev {
            wl_output::Event::Name { name } => i.name = name,
            wl_output::Event::Mode {
                flags, width, height, ..
            } => {
                if let WEnum::Value(f) = flags
                    && f.contains(wl_output::Mode::Current)
                {
                    i.mode = (width, height);
                }
            }
            wl_output::Event::Geometry { transform, .. } => {
                if let WEnum::Value(t) = transform {
                    i.transform = t as u32;
                }
            }
            wl_output::Event::Scale { factor } => i.scale = factor,
            _ => {}
        }
    }
}

impl Dispatch<zxdg_output_v1::ZxdgOutputV1, wl_output::WlOutput> for State {
    fn event(
        s: &mut Self,
        _: &zxdg_output_v1::ZxdgOutputV1,
        ev: zxdg_output_v1::Event,
        o: &wl_output::WlOutput,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        let Some(i) = s.info(o) else { return };
        match ev {
            zxdg_output_v1::Event::LogicalPosition { x, y } => i.lpos = Some((x, y)),
            zxdg_output_v1::Event::LogicalSize { width, height } => i.lsize = Some((width, height)),
            zxdg_output_v1::Event::Name { name } if i.name.is_empty() => i.name = name,
            _ => {}
        }
    }
}

impl Dispatch<zwlr_screencopy_frame_v1::ZwlrScreencopyFrameV1, ()> for State {
    fn event(
        s: &mut Self,
        _: &zwlr_screencopy_frame_v1::ZwlrScreencopyFrameV1,
        ev: zwlr_screencopy_frame_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        use zwlr_screencopy_frame_v1::Event as E;
        match ev {
            E::Buffer {
                format,
                width,
                height,
                stride,
            } => {
                if let WEnum::Value(f) = format
                    && s.frame.format.is_none()
                {
                    s.frame.format = Some((f as u32, width, height, stride));
                }
            }
            E::Flags { flags } => {
                if let WEnum::Value(f) = flags {
                    s.frame.flags = f.bits();
                }
            }
            E::BufferDone => s.frame.buffer_done = true,
            E::Ready { .. } => s.frame.done = true,
            E::Failed => s.frame.failed = true,
            _ => {}
        }
    }
}

delegate_noop!(State: ignore zxdg_output_manager_v1::ZxdgOutputManagerV1);
delegate_noop!(State: ignore zwlr_screencopy_manager_v1::ZwlrScreencopyManagerV1);
delegate_noop!(State: ignore wl_shm::WlShm);
delegate_noop!(State: ignore wl_shm_pool::WlShmPool);
delegate_noop!(State: ignore wl_buffer::WlBuffer);

pub struct Session {
    conn: Connection,
    queue: wayland_client::EventQueue<State>,
    state: State,
    shm: Option<wl_shm::WlShm>,
    copier: Option<zwlr_screencopy_manager_v1::ZwlrScreencopyManagerV1>,
}

impl Session {
    pub fn connect() -> Result<Session, String> {
        let conn = Connection::connect_to_env().map_err(|e| format!("Wayland: {e}"))?;
        let (globals, mut queue) = registry_queue_init::<State>(&conn).map_err(|e| format!("Wayland: {e}"))?;
        let qh = queue.handle();
        let mut state = State::default();
        let xdg: Option<zxdg_output_manager_v1::ZxdgOutputManagerV1> = globals.bind(&qh, 1..=3, ()).ok();
        let outputs: Vec<wl_output::WlOutput> = globals.contents().with_list(|list| {
            list.iter()
                .filter(|g| g.interface == "wl_output")
                .map(|g| {
                    globals
                        .registry()
                        .bind::<wl_output::WlOutput, _, _>(g.name, g.version.min(4), &qh, ())
                })
                .collect()
        });
        for o in outputs {
            if let Some(x) = &xdg {
                x.get_xdg_output(&o, &qh, o.clone());
            }
            state.outs.push((
                o,
                OutInfo {
                    scale: 1,
                    ..Default::default()
                },
            ));
        }
        let shm = globals.bind(&qh, 1..=1, ()).ok();
        let copier = globals.bind(&qh, 1..=3, ()).ok();
        queue.roundtrip(&mut state).map_err(|e| e.to_string())?;
        queue.roundtrip(&mut state).map_err(|e| e.to_string())?;
        Ok(Session {
            conn,
            queue,
            state,
            shm,
            copier,
        })
    }

    pub fn outputs(&self) -> Vec<Output> {
        self.state
            .outs
            .iter()
            .filter(|(_, i)| i.mode.0 > 0)
            .map(|(_, i)| {
                let (mut pw, mut ph) = i.mode;
                if i.transform % 2 == 1 {
                    std::mem::swap(&mut pw, &mut ph);
                }
                let (lw, lh) = i.lsize.unwrap_or((pw / i.scale.max(1), ph / i.scale.max(1)));
                let (x, y) = i.lpos.unwrap_or((0, 0));
                Output {
                    name: i.name.clone(),
                    x,
                    y,
                    w: lw,
                    h: lh,
                    scale: pw as f32 / lw.max(1) as f32,
                    transform: i.transform,
                    workspaces: vec![],
                    image: None,
                }
            })
            .collect()
    }

    /// wlr-screencopy one output into shared memory (oriented as scanned out).
    pub fn screencopy(&mut self, name: &str) -> Result<(Rgba, bool), String> {
        let copier = self.copier.clone().ok_or("compositor has no wlr-screencopy")?;
        let shm = self.shm.clone().ok_or("no wl_shm")?;
        let out = self
            .state
            .outs
            .iter()
            .find(|(_, i)| i.name == name)
            .map(|(o, _)| o.clone())
            .ok_or("no such output")?;
        let qh = self.queue.handle();
        self.state.frame = FrameState::default();
        let frame = copier.capture_output(0, &out, &qh, ());
        while self.state.frame.format.is_none() && !self.state.frame.failed && !self.state.frame.buffer_done {
            self.queue
                .blocking_dispatch(&mut self.state)
                .map_err(|e| e.to_string())?;
        }
        let (format, w, h, stride) = self.state.frame.format.ok_or("screencopy: no shm format offered")?;
        let size = (stride * h) as usize;
        let fd = memfd(size)?;
        let pool = shm.create_pool(fd.as_fd(), size as i32, &qh, ());
        let fmt = wl_shm::Format::try_from(format).map_err(|_| "bad shm format")?;
        let buf = pool.create_buffer(0, w as i32, h as i32, stride as i32, fmt, &qh, ());
        frame.copy(&buf);
        while !self.state.frame.done && !self.state.frame.failed {
            self.queue
                .blocking_dispatch(&mut self.state)
                .map_err(|e| e.to_string())?;
        }
        let ok = self.state.frame.done;
        let y_invert = self.state.frame.flags & 1 != 0;
        let res = if ok {
            let map = unsafe {
                libc::mmap(
                    std::ptr::null_mut(),
                    size,
                    libc::PROT_READ,
                    libc::MAP_SHARED,
                    std::os::fd::AsRawFd::as_raw_fd(&fd),
                    0,
                )
            };
            if map == libc::MAP_FAILED {
                Err("mmap failed".to_string())
            } else {
                let src = unsafe { std::slice::from_raw_parts(map as *const u8, size) };
                let r = convert(src, format, w, h, stride);
                unsafe { libc::munmap(map, size) };
                r.map(|img| (img, y_invert))
            }
        } else {
            Err("screencopy failed".to_string())
        };
        buf.destroy();
        pool.destroy();
        frame.destroy();
        let _ = self.conn.flush();
        res
    }
}

fn memfd(size: usize) -> Result<std::os::fd::OwnedFd, String> {
    use std::os::fd::FromRawFd;
    let fd = unsafe { libc::memfd_create(c"kadr-shm".as_ptr(), libc::MFD_CLOEXEC) };
    if fd < 0 {
        return Err(std::io::Error::last_os_error().to_string());
    }
    let fd = unsafe { std::os::fd::OwnedFd::from_raw_fd(fd) };
    if unsafe { libc::ftruncate(std::os::fd::AsRawFd::as_raw_fd(&fd), size as libc::off_t) } != 0 {
        return Err(std::io::Error::last_os_error().to_string());
    }
    Ok(fd)
}

/// wl_shm formats are little-endian DRM fourccs (argb8888 = B,G,R,A bytes), except 0/1.
fn convert(src: &[u8], format: u32, w: u32, h: u32, stride: u32) -> Result<Rgba, String> {
    const ARGB: u32 = 0;
    const XRGB: u32 = 1;
    const ABGR: u32 = 0x3432_4241;
    const XBGR: u32 = 0x3432_4258;
    const XR30: u32 = 0x3033_5258;
    const XB30: u32 = 0x3033_4258;
    let mut data = vec![0u8; (w * h * 4) as usize];
    for y in 0..h as usize {
        let row = &src[y * stride as usize..];
        for x in 0..w as usize {
            let px = &row[x * 4..x * 4 + 4];
            let (r, g, b) = match format {
                ARGB | XRGB => (px[2], px[1], px[0]),
                ABGR | XBGR => (px[0], px[1], px[2]),
                XR30 | XB30 => {
                    let v = u32::from_le_bytes([px[0], px[1], px[2], px[3]]);
                    let (hi, mid, lo) = ((v >> 20) & 0x3ff, (v >> 10) & 0x3ff, v & 0x3ff);
                    let q = |c: u32| (c >> 2) as u8;
                    if format == XR30 {
                        (q(hi), q(mid), q(lo))
                    } else {
                        (q(lo), q(mid), q(hi))
                    }
                }
                _ => return Err(format!("unsupported shm format {format:#x}")),
            };
            let o = (y * w as usize + x) * 4;
            data[o..o + 4].copy_from_slice(&[r, g, b, 255]);
        }
    }
    Ok(Rgba { w, h, data })
}
