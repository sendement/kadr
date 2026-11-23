//! kadr-kms — the only privileged part of Kadr.
//!
//! Reading another client's scanout buffer through KMS (DRM_IOCTL_MODE_GETFB2 returning GEM
//! handles) needs CAP_SYS_ADMIN, so this tiny helper is installed with `cap_sys_admin+ep` and
//! does nothing else: it finds the framebuffer on the primary plane of every lit CRTC, exports
//! it as a dmabuf and passes the descriptors to its parent over the Unix socket on stdin
//! (SCM_RIGHTS). It links no GPU libraries, parses no input and drops the capability before
//! writing anything; the unprivileged `kadr` process imports and reads the buffers.
//!
//! Wire format: one sendmsg() with a JSON array (UTF-8, no trailing data) and all dmabuf fds
//! attached in order; `fd` fields index into that fd list.

use std::fs::{self, File, OpenOptions};
use std::io;
use std::os::fd::{AsFd, AsRawFd, BorrowedFd, OwnedFd, RawFd};
use std::path::{Path, PathBuf};

use drm::control::{self, Device as ControlDevice, ResourceHandle};
use drm::{ClientCapability, Device};

struct Card(File);
impl AsFd for Card {
    fn as_fd(&self) -> BorrowedFd<'_> {
        self.0.as_fd()
    }
}
impl Device for Card {}
impl ControlDevice for Card {}

#[derive(serde::Serialize)]
struct PlaneOut {
    fd: usize,
    offset: u32,
    pitch: u32,
}

#[derive(serde::Serialize)]
struct Output {
    name: String,
    card: String,
    render: Option<String>,
    width: u32,
    height: u32,
    format: u32,
    modifier: Option<u64>,
    planes: Vec<PlaneOut>,
    /// plane source rectangle inside the framebuffer (x, y, w, h), whole pixels
    src: [u32; 4],
    /// size of the mode the CRTC scans out
    mode: [u32; 2],
}

fn render_node(card: &Path) -> Option<String> {
    let name = card.file_name()?.to_str()?;
    let dir = PathBuf::from(format!("/sys/class/drm/{name}/device/drm"));
    let mut nodes: Vec<String> = fs::read_dir(dir)
        .ok()?
        .filter_map(|e| e.ok()?.file_name().into_string().ok())
        .filter(|n| n.starts_with("renderD"))
        .collect();
    nodes.sort();
    nodes.first().map(|n| format!("/dev/dri/{n}"))
}

fn prop_values(card: &Card, h: impl ResourceHandle) -> Vec<(String, u64)> {
    let Ok(props) = card.get_properties(h) else {
        return vec![];
    };
    let (ids, vals) = props.as_props_and_values();
    ids.iter()
        .zip(vals)
        .filter_map(|(id, v)| {
            let info = card.get_property(*id).ok()?;
            Some((info.name().to_string_lossy().into_owned(), *v))
        })
        .collect()
}

fn scan_card(path: &Path, outs: &mut Vec<Output>, fds: &mut Vec<OwnedFd>) -> io::Result<()> {
    let card = Card(OpenOptions::new().read(true).write(true).open(path)?);
    let _ = card.set_client_capability(ClientCapability::UniversalPlanes, true);
    let res = card.resource_handles()?;
    // connector name per CRTC (via its current encoder)
    let mut names = Vec::new();
    for &c in res.connectors() {
        let Ok(info) = card.get_connector(c, false) else {
            continue;
        };
        if info.state() != control::connector::State::Connected {
            continue;
        }
        let Some(enc) = info.current_encoder().and_then(|e| card.get_encoder(e).ok()) else {
            continue;
        };
        if let Some(crtc) = enc.crtc() {
            names.push((crtc, format!("{}-{}", info.interface().as_str(), info.interface_id())));
        }
    }
    let render = render_node(path);
    for plane in card.plane_handles()? {
        let Ok(pinfo) = card.get_plane(plane) else { continue };
        let (Some(crtc), Some(fb)) = (pinfo.crtc(), pinfo.framebuffer()) else {
            continue;
        };
        let props = prop_values(&card, plane);
        let prop = |n: &str| props.iter().find(|(k, _)| k == n).map(|(_, v)| *v);
        if prop("type") != Some(1) {
            continue; // 0 overlay, 1 primary, 2 cursor: the compositor's frame is on the primary
        }
        let Some((_, name)) = names.iter().find(|(c, _)| *c == crtc) else {
            continue;
        };
        let Ok(cinfo) = card.get_crtc(crtc) else { continue };
        let Some(mode) = cinfo.mode() else { continue };
        let fbi = card
            .get_planar_framebuffer(fb)
            .map_err(|e| io::Error::other(format!("GETFB2: {e:?}")))?;
        let mut planes = Vec::new();
        let mut opened: Vec<(u32, usize)> = Vec::new();
        let bufs = fbi.buffers();
        for i in 0..4 {
            let Some(b) = bufs[i] else { continue };
            let raw: u32 = b.into();
            if raw == 0 {
                // handles are zeroed for callers without CAP_SYS_ADMIN / DRM master
                return Err(io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    "no GEM handles (CAP_SYS_ADMIN missing)",
                ));
            }
            let idx = match opened.iter().find(|(h, _)| *h == raw) {
                Some(&(_, idx)) => idx,
                None => {
                    let fd = card.buffer_to_prime_fd(b, libc::O_CLOEXEC as u32)?;
                    fds.push(fd);
                    opened.push((raw, fds.len() - 1));
                    fds.len() - 1
                }
            };
            planes.push(PlaneOut {
                fd: idx,
                offset: fbi.offsets()[i],
                pitch: fbi.pitches()[i],
            });
        }
        for b in bufs.iter().flatten() {
            let _ = card.close_buffer(*b);
        }
        if planes.is_empty() {
            // GETFB2 hides the GEM handles from callers without CAP_SYS_ADMIN / DRM master
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "no GEM handles (CAP_SYS_ADMIN missing)",
            ));
        }
        let (fw, fh) = fbi.size();
        let src = match (prop("SRC_X"), prop("SRC_Y"), prop("SRC_W"), prop("SRC_H")) {
            (Some(x), Some(y), Some(w), Some(h)) if w > 0 && h > 0 => {
                [(x >> 16) as u32, (y >> 16) as u32, (w >> 16) as u32, (h >> 16) as u32]
            }
            _ => {
                let (x, y) = cinfo.position();
                [x, y, mode.size().0 as u32, mode.size().1 as u32]
            }
        };
        outs.push(Output {
            name: name.clone(),
            card: path.display().to_string(),
            render: render.clone(),
            width: fw,
            height: fh,
            format: fbi.pixel_format() as u32,
            modifier: fbi.modifier().map(u64::from),
            planes,
            src,
            mode: [mode.size().0 as u32, mode.size().1 as u32],
        });
    }
    Ok(())
}

fn drop_caps() {
    // capset() with an empty set: everything after the scan runs unprivileged
    #[repr(C)]
    struct Header {
        version: u32,
        pid: i32,
    }
    #[repr(C)]
    #[derive(Default, Clone, Copy)]
    struct Data {
        effective: u32,
        permitted: u32,
        inheritable: u32,
    }
    let mut h = Header {
        version: 0x2008_0522,
        pid: 0,
    };
    let d = [Data::default(); 2];
    unsafe {
        libc::syscall(libc::SYS_capset, &mut h as *mut Header, d.as_ptr());
    }
}

fn send(sock: RawFd, json: &[u8], fds: &[OwnedFd]) -> io::Result<()> {
    let raw: Vec<RawFd> = fds.iter().map(|f| f.as_raw_fd()).collect();
    let space = unsafe { libc::CMSG_SPACE(std::mem::size_of_val(raw.as_slice()) as u32) } as usize;
    let mut cbuf = vec![0u8; space.max(1)];
    let mut iov = libc::iovec {
        iov_base: json.as_ptr() as *mut _,
        iov_len: json.len(),
    };
    let mut msg: libc::msghdr = unsafe { std::mem::zeroed() };
    msg.msg_iov = &mut iov;
    msg.msg_iovlen = 1;
    if !raw.is_empty() {
        msg.msg_control = cbuf.as_mut_ptr() as *mut _;
        msg.msg_controllen = space as _;
        unsafe {
            let c = libc::CMSG_FIRSTHDR(&msg);
            (*c).cmsg_level = libc::SOL_SOCKET;
            (*c).cmsg_type = libc::SCM_RIGHTS;
            (*c).cmsg_len = libc::CMSG_LEN(std::mem::size_of_val(raw.as_slice()) as u32) as _;
            std::ptr::copy_nonoverlapping(raw.as_ptr(), libc::CMSG_DATA(c) as *mut RawFd, raw.len());
        }
    }
    if unsafe { libc::sendmsg(sock, &msg, 0) } < 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

fn main() {
    let mut cards: Vec<PathBuf> = fs::read_dir("/dev/dri")
        .map(|d| {
            d.filter_map(|e| e.ok().map(|e| e.path()))
                .filter(|p| {
                    p.file_name()
                        .and_then(|n| n.to_str())
                        .is_some_and(|n| n.starts_with("card"))
                })
                .collect()
        })
        .unwrap_or_default();
    cards.sort();
    let mut outs = Vec::new();
    let mut fds = Vec::new();
    let mut errors = Vec::new();
    for c in &cards {
        if let Err(e) = scan_card(c, &mut outs, &mut fds) {
            errors.push(format!("{}: {e}", c.display()));
        }
    }
    drop_caps();
    let body = serde_json::json!({ "outputs": outs, "errors": errors });
    let json = serde_json::to_vec(&body).unwrap();
    // stdin must be the parent's Unix socket; on a terminal just print the scan for debugging
    let sock = 0;
    let mut st: libc::stat = unsafe { std::mem::zeroed() };
    let is_sock = unsafe { libc::fstat(sock, &mut st) } == 0 && (st.st_mode & libc::S_IFMT) == libc::S_IFSOCK;
    if !is_sock {
        println!("{}", String::from_utf8_lossy(&json));
        return;
    }
    if let Err(e) = send(sock, &json, &fds) {
        eprintln!("kadr-kms: {e}");
        std::process::exit(1);
    }
}
