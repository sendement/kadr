//! KMS capture: `kadr-kms` (cap_sys_admin) hands over the scanout dmabufs, this side imports
//! them into EGL on the GPU that owns them, detiles/decompresses by drawing into a linear
//! RGBA framebuffer and reads the pixels back. No compositor protocol is involved, so the
//! frame is exactly what is on the glass at that instant (minus the hardware cursor plane).

use std::collections::HashMap;
use std::io;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd};
use std::path::PathBuf;
use std::process::{Command, Stdio};

use glow::HasContext;
use khronos_egl as egl;

use super::Rgba;

#[derive(serde::Deserialize, Debug)]
struct PlaneIn {
    fd: usize,
    offset: u32,
    pitch: u32,
}

#[derive(serde::Deserialize, Debug)]
struct OutputIn {
    name: String,
    render: Option<String>,
    width: u32,
    height: u32,
    format: u32,
    modifier: Option<u64>,
    planes: Vec<PlaneIn>,
    src: [u32; 4],
}

#[derive(serde::Deserialize, Debug)]
struct Reply {
    outputs: Vec<OutputIn>,
    #[serde(default)]
    errors: Vec<String>,
}

/// Path of the helper: next to our own binary first (dev builds, /usr/bin), then $PATH.
pub fn helper_path() -> Option<PathBuf> {
    if let Ok(exe) = std::env::current_exe()
        && let Some(dir) = exe.parent()
    {
        let p = dir.join("kadr-kms");
        if p.exists() {
            return Some(p);
        }
    }
    std::env::var_os("PATH").and_then(|paths| {
        std::env::split_paths(&paths)
            .map(|d| d.join("kadr-kms"))
            .find(|p| p.exists())
    })
}

fn recv(sock: RawFd) -> io::Result<(Vec<u8>, Vec<OwnedFd>)> {
    let mut buf = vec![0u8; 256 * 1024];
    let max_fds = 128;
    let space = unsafe { libc::CMSG_SPACE((max_fds * std::mem::size_of::<RawFd>()) as u32) } as usize;
    let mut cbuf = vec![0u8; space];
    let mut iov = libc::iovec {
        iov_base: buf.as_mut_ptr() as *mut _,
        iov_len: buf.len(),
    };
    let mut msg: libc::msghdr = unsafe { std::mem::zeroed() };
    msg.msg_iov = &mut iov;
    msg.msg_iovlen = 1;
    msg.msg_control = cbuf.as_mut_ptr() as *mut _;
    msg.msg_controllen = space as _;
    let n = unsafe { libc::recvmsg(sock, &mut msg, libc::MSG_CMSG_CLOEXEC) };
    if n < 0 {
        return Err(io::Error::last_os_error());
    }
    let mut fds = Vec::new();
    unsafe {
        let mut c = libc::CMSG_FIRSTHDR(&msg);
        while !c.is_null() {
            if (*c).cmsg_level == libc::SOL_SOCKET && (*c).cmsg_type == libc::SCM_RIGHTS {
                let data = libc::CMSG_DATA(c) as *const RawFd;
                let count = ((*c).cmsg_len as usize - libc::CMSG_LEN(0) as usize) / std::mem::size_of::<RawFd>();
                for i in 0..count {
                    fds.push(OwnedFd::from_raw_fd(*data.add(i)));
                }
            }
            c = libc::CMSG_NXTHDR(&msg, c);
        }
    }
    if msg.msg_flags & libc::MSG_TRUNC != 0 {
        return Err(io::Error::other("kadr-kms reply truncated"));
    }
    buf.truncate(n as usize);
    Ok((buf, fds))
}

/// Grab every lit output. Returns connector name -> physical pixels as scanned out
/// (before undoing the output transform).
pub fn capture() -> Result<HashMap<String, Rgba>, String> {
    let helper = helper_path().ok_or("kadr-kms helper not found")?;
    let mut sv = [0 as RawFd; 2];
    if unsafe {
        libc::socketpair(
            libc::AF_UNIX,
            libc::SOCK_SEQPACKET | libc::SOCK_CLOEXEC,
            0,
            sv.as_mut_ptr(),
        )
    } != 0
    {
        return Err(format!("socketpair: {}", io::Error::last_os_error()));
    }
    let ours = unsafe { OwnedFd::from_raw_fd(sv[0]) };
    let theirs = unsafe { OwnedFd::from_raw_fd(sv[1]) };
    let mut child = Command::new(&helper)
        .stdin(Stdio::from(theirs))
        .stdout(Stdio::null())
        .stderr(Stdio::inherit())
        .spawn()
        .map_err(|e| format!("{}: {e}", helper.display()))?;
    let got = recv(ours.as_raw_fd());
    let _ = child.wait();
    let (json, fds) = got.map_err(|e| format!("kadr-kms: {e}"))?;
    let reply: Reply = serde_json::from_slice(&json).map_err(|e| format!("kadr-kms reply: {e}"))?;
    if reply.outputs.is_empty() {
        return Err(if reply.errors.is_empty() {
            "kadr-kms: no lit outputs".into()
        } else {
            format!("kadr-kms: {}", reply.errors.join("; "))
        });
    }
    let mut by_node: HashMap<String, Vec<&OutputIn>> = HashMap::new();
    for o in &reply.outputs {
        by_node.entry(o.render.clone().unwrap_or_default()).or_default().push(o);
    }
    let mut res = HashMap::new();
    let mut errs = Vec::new();
    for (node, outs) in by_node {
        match Gpu::open(&node) {
            Ok(gpu) => {
                for o in outs {
                    match gpu.read(o, &fds) {
                        Ok(img) => {
                            res.insert(o.name.clone(), img);
                        }
                        Err(e) => errs.push(format!("{}: {e}", o.name)),
                    }
                }
            }
            Err(e) => errs.push(format!("{node}: {e}")),
        }
    }
    if res.is_empty() {
        return Err(errs.join("; "));
    }
    for e in errs {
        eprintln!("kadr: KMS: {e}");
    }
    Ok(res)
}

// --- EGL ------------------------------------------------------------------

const EGL_PLATFORM_GBM_KHR: egl::Enum = 0x31D7;
const EGL_LINUX_DMA_BUF_EXT: egl::Enum = 0x3270;
const EGL_LINUX_DRM_FOURCC_EXT: egl::Attrib = 0x3271;
const PLANE_ATTRS: [[egl::Attrib; 5]; 4] = [
    // fd, offset, pitch, modifier lo, modifier hi
    [0x3272, 0x3273, 0x3274, 0x3443, 0x3444],
    [0x3275, 0x3276, 0x3277, 0x3445, 0x3446],
    [0x3278, 0x3279, 0x327A, 0x3447, 0x3448],
    [0x3440, 0x3441, 0x3442, 0x3449, 0x344A],
];
const GL_TEXTURE_EXTERNAL_OES: u32 = 0x8D65;

type ImageTargetTexture = unsafe extern "system" fn(u32, *mut libc::c_void);

struct Gpu {
    egl: egl::Instance<egl::Static>,
    dpy: egl::Display,
    ctx: egl::Context,
    gl: glow::Context,
    target_tex: ImageTargetTexture,
    gbm: *mut libc::c_void,
    _file: std::fs::File,
}

#[link(name = "gbm")]
unsafe extern "C" {
    fn gbm_create_device(fd: libc::c_int) -> *mut libc::c_void;
    fn gbm_device_destroy(dev: *mut libc::c_void);
}

unsafe fn proc<T>(e: &egl::Instance<egl::Static>, name: &str) -> Option<T> {
    let f = e.get_proc_address(name)?;
    Some(unsafe { std::mem::transmute_copy::<extern "system" fn(), T>(&f) })
}

impl Gpu {
    fn open(node: &str) -> Result<Gpu, String> {
        let e = egl::Instance::new(egl::Static);
        // GBM on the render node: enumerating EGL devices instead would initialize every
        // vendor's driver (seconds with NVIDIA's, and it wakes a sleeping dGPU)
        let file = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(node)
            .map_err(|err| format!("{node}: {err}"))?;
        let gbm = unsafe { gbm_create_device(file.as_raw_fd()) };
        if gbm.is_null() {
            return Err("gbm_create_device failed".into());
        }
        let dpy = unsafe { e.get_platform_display(EGL_PLATFORM_GBM_KHR, gbm, &[egl::ATTRIB_NONE]) }
            .map_err(|err| format!("eglGetPlatformDisplay: {err}"))?;
        e.initialize(dpy).map_err(|err| format!("eglInitialize: {err}"))?;
        e.bind_api(egl::OPENGL_ES_API)
            .map_err(|err| format!("eglBindAPI: {err}"))?;
        let cfg_attrs = [
            egl::SURFACE_TYPE,
            0,
            egl::RENDERABLE_TYPE,
            egl::OPENGL_ES2_BIT,
            egl::NONE,
        ];
        let cfg = e.choose_first_config(dpy, &cfg_attrs).ok().flatten();
        let ctx = e
            .create_context(
                dpy,
                cfg.unwrap_or_else(|| unsafe { egl::Config::from_ptr(std::ptr::null_mut()) }),
                None,
                &[egl::CONTEXT_CLIENT_VERSION, 2, egl::NONE],
            )
            .map_err(|err| format!("eglCreateContext: {err}"))?;
        e.make_current(dpy, None, None, Some(ctx))
            .map_err(|err| format!("eglMakeCurrent: {err}"))?;
        let gl = unsafe {
            glow::Context::from_loader_function(|s| e.get_proc_address(s).map_or(std::ptr::null(), |f| f as *const _))
        };
        let target_tex: ImageTargetTexture =
            unsafe { proc(&e, "glEGLImageTargetTexture2DOES") }.ok_or("no glEGLImageTargetTexture2DOES")?;
        Ok(Gpu {
            egl: e,
            dpy,
            ctx,
            gl,
            target_tex,
            gbm,
            _file: file,
        })
    }

    fn read(&self, o: &OutputIn, fds: &[OwnedFd]) -> Result<Rgba, String> {
        let mut attrs: Vec<egl::Attrib> = vec![
            egl::WIDTH as _,
            o.width as _,
            egl::HEIGHT as _,
            o.height as _,
            EGL_LINUX_DRM_FOURCC_EXT,
            o.format as _,
        ];
        for (i, p) in o.planes.iter().enumerate().take(4) {
            let fd = fds.get(p.fd).ok_or("missing dmabuf fd")?;
            let a = PLANE_ATTRS[i];
            attrs.extend([a[0], fd.as_raw_fd() as _, a[1], p.offset as _, a[2], p.pitch as _]);
            if let Some(m) = o.modifier {
                attrs.extend([a[3], (m & 0xffff_ffff) as _, a[4], (m >> 32) as _]);
            }
        }
        attrs.push(egl::ATTRIB_NONE);
        let image = unsafe {
            self.egl.create_image(
                self.dpy,
                egl::Context::from_ptr(egl::NO_CONTEXT),
                EGL_LINUX_DMA_BUF_EXT,
                egl::ClientBuffer::from_ptr(std::ptr::null_mut()),
                &attrs,
            )
        }
        .map_err(|err| {
            format!(
                "dmabuf import (format {:#x}, modifier {:#x?}): {err}",
                o.format, o.modifier
            )
        })?;
        let res = unsafe { self.blit(o, image.as_ptr()) };
        let _ = self.egl.destroy_image(self.dpy, image);
        res
    }

    unsafe fn blit(&self, o: &OutputIn, image: *mut libc::c_void) -> Result<Rgba, String> {
        let gl = &self.gl;
        let [sx, sy, sw, sh] = o.src;
        let (sw, sh) = (
            sw.min(o.width.saturating_sub(sx)).max(1),
            sh.min(o.height.saturating_sub(sy)).max(1),
        );
        unsafe {
            let tex = gl.create_texture()?;
            gl.bind_texture(GL_TEXTURE_EXTERNAL_OES, Some(tex));
            gl.tex_parameter_i32(GL_TEXTURE_EXTERNAL_OES, glow::TEXTURE_MIN_FILTER, glow::NEAREST as i32);
            gl.tex_parameter_i32(GL_TEXTURE_EXTERNAL_OES, glow::TEXTURE_MAG_FILTER, glow::NEAREST as i32);
            (self.target_tex)(GL_TEXTURE_EXTERNAL_OES, image);
            if gl.get_error() != glow::NO_ERROR {
                gl.delete_texture(tex);
                return Err("glEGLImageTargetTexture2DOES failed".into());
            }
            let rb = gl.create_renderbuffer()?;
            gl.bind_renderbuffer(glow::RENDERBUFFER, Some(rb));
            gl.renderbuffer_storage(glow::RENDERBUFFER, glow::RGBA8, sw as i32, sh as i32);
            let fbo = gl.create_framebuffer()?;
            gl.bind_framebuffer(glow::FRAMEBUFFER, Some(fbo));
            gl.framebuffer_renderbuffer(glow::FRAMEBUFFER, glow::COLOR_ATTACHMENT0, glow::RENDERBUFFER, Some(rb));
            let prog = self.program()?;
            gl.use_program(Some(prog));
            let loc = gl.get_uniform_location(prog, "u_rect");
            gl.uniform_4_f32(
                loc.as_ref(),
                sx as f32 / o.width as f32,
                sy as f32 / o.height as f32,
                sw as f32 / o.width as f32,
                sh as f32 / o.height as f32,
            );
            gl.viewport(0, 0, sw as i32, sh as i32);
            gl.disable(glow::BLEND);
            gl.draw_arrays(glow::TRIANGLE_STRIP, 0, 4);
            let mut data = vec![0u8; (sw * sh * 4) as usize];
            gl.pixel_store_i32(glow::PACK_ALIGNMENT, 1);
            gl.read_pixels(
                0,
                0,
                sw as i32,
                sh as i32,
                glow::RGBA,
                glow::UNSIGNED_BYTE,
                glow::PixelPackData::Slice(Some(&mut data)),
            );
            let err = gl.get_error();
            gl.bind_framebuffer(glow::FRAMEBUFFER, None);
            gl.delete_framebuffer(fbo);
            gl.delete_renderbuffer(rb);
            gl.delete_texture(tex);
            gl.delete_program(prog);
            if err != glow::NO_ERROR {
                return Err(format!("GL error {err:#x} while reading back"));
            }
            Ok(Rgba { w: sw, h: sh, data })
        }
    }

    unsafe fn program(&self) -> Result<glow::Program, String> {
        let gl = &self.gl;
        // row 0 of the dmabuf (top) is drawn at the bottom of the viewport, which is where
        // glReadPixels starts: the read-back rows come out top-down
        let vs = "attribute vec2 a_pos;\nuniform vec4 u_rect;\nvarying vec2 v_uv;\nvoid main() {\n\
                  v_uv = u_rect.xy + a_pos * u_rect.zw;\n  gl_Position = vec4(a_pos * 2.0 - 1.0, 0.0, 1.0);\n}\n";
        let fs = "#extension GL_OES_EGL_image_external : require\nprecision mediump float;\n\
                  uniform samplerExternalOES u_tex;\nvarying vec2 v_uv;\nvoid main() {\n\
                  gl_FragColor = vec4(texture2D(u_tex, v_uv).rgb, 1.0);\n}\n";
        unsafe {
            let prog = gl.create_program()?;
            for (kind, src) in [(glow::VERTEX_SHADER, vs), (glow::FRAGMENT_SHADER, fs)] {
                let sh = gl.create_shader(kind)?;
                gl.shader_source(sh, src);
                gl.compile_shader(sh);
                if !gl.get_shader_compile_status(sh) {
                    return Err(format!("shader: {}", gl.get_shader_info_log(sh)));
                }
                gl.attach_shader(prog, sh);
                gl.delete_shader(sh);
            }
            gl.bind_attrib_location(prog, 0, "a_pos");
            gl.link_program(prog);
            if !gl.get_program_link_status(prog) {
                return Err(format!("link: {}", gl.get_program_info_log(prog)));
            }
            let verts: [f32; 8] = [0.0, 0.0, 1.0, 0.0, 0.0, 1.0, 1.0, 1.0];
            let buf = gl.create_buffer()?;
            gl.bind_buffer(glow::ARRAY_BUFFER, Some(buf));
            gl.buffer_data_u8_slice(glow::ARRAY_BUFFER, bytemuck_f32(&verts), glow::STATIC_DRAW);
            gl.enable_vertex_attrib_array(0);
            gl.vertex_attrib_pointer_f32(0, 2, glow::FLOAT, false, 0, 0);
            Ok(prog)
        }
    }
}

fn bytemuck_f32(v: &[f32]) -> &[u8] {
    unsafe { std::slice::from_raw_parts(v.as_ptr() as *const u8, std::mem::size_of_val(v)) }
}

impl Drop for Gpu {
    fn drop(&mut self) {
        let _ = self.egl.make_current(self.dpy, None, None, None);
        let _ = self.egl.destroy_context(self.dpy, self.ctx);
        let _ = self.egl.terminate(self.dpy);
        unsafe { gbm_device_destroy(self.gbm) };
    }
}
