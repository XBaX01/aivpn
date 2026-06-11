//! kernel_accel.rs — optional /dev/aivpn kernel-module acceleration.
//!
//! Call `KernelAccel::try_open()` at startup. Returns `None` if the module
//! is not loaded (`ENODEV`/`ENOENT`), so the caller can fall back to the
//! user-space TUN path transparently.

use std::fs::OpenOptions;
use std::os::unix::io::{AsRawFd, RawFd};
use std::io;

// ── UAPI ioctl numbers (must match include/uapi/aivpn.h) ─────────────────────

const MAGIC: u64 = 0xAE;
const fn iow(nr: u64, sz: u64) -> u64  { (2u64 << 30) | (MAGIC << 8) | nr | (sz << 16) }
const fn ior(nr: u64, sz: u64) -> u64  { (1u64 << 30) | (MAGIC << 8) | nr | (sz << 16) }
const fn iowr(nr: u64, sz: u64) -> u64 { (3u64 << 30) | (MAGIC << 8) | nr | (sz << 16) }
const fn io_(nr: u64) -> u64           { (MAGIC << 8) | nr }

const IOC_SESSION_ADD:  u64 = iow(1,  160);
const IOC_SESSION_DEL:  u64 = iow(2,   16);
#[allow(dead_code)]
const IOC_SESSION_STAT: u64 = iowr(3,  52);
const IOC_SET_TUN:      u64 = iow(4,    4);
const IOC_SET_UDP_SOCK: u64 = iow(5,    4);
const IOC_FLUSH:        u64 = io_(6);
const IOC_GET_VERSION:  u64 = ior(7,    4);

pub const API_VERSION: u32 = 1;

// ── Wire structs (packed, matching C structs in include/uapi/aivpn.h) ─────────

#[repr(C, packed)]
pub struct SessionAdd {
    pub session_id:   [u8; 16],
    pub session_key:  [u8; 32],
    pub tag_secret:   [u8; 32],
    pub prng_seed:    [u8; 32],
    pub counter_base: u64,
    pub client_ip:    u32,
    pub client_addr:  [u8; 28],
    pub window_ms:    u64,
}

// ── KernelAccel handle ────────────────────────────────────────────────────────

pub struct KernelAccel {
    file: std::fs::File,
}

impl KernelAccel {
    /// Returns `None` if `/dev/aivpn` is absent (module not loaded).
    pub fn try_open() -> Option<Self> {
        match OpenOptions::new().read(true).write(true).open("/dev/aivpn") {
            Ok(f) => {
                let ka = KernelAccel { file: f };
                match ka.api_version() {
                    Ok(v) if v == API_VERSION => Some(ka),
                    Ok(v) => {
                        tracing::warn!("aivpn: kernel module API version mismatch (got {v}, want {API_VERSION}) — using user-space path");
                        None
                    }
                    Err(e) => {
                        tracing::warn!("aivpn: GET_VERSION ioctl failed: {e} — using user-space path");
                        None
                    }
                }
            }
            Err(e) if e.kind() == io::ErrorKind::NotFound => None,
            Err(e) if e.raw_os_error() == Some(libc::ENODEV) => None,
            Err(e) => {
                tracing::warn!("aivpn: open /dev/aivpn failed: {e} — using user-space path");
                None
            }
        }
    }

    fn fd(&self) -> RawFd { self.file.as_raw_fd() }

    pub fn api_version(&self) -> io::Result<u32> {
        let mut v: u32 = 0;
        ioctl_ref(self.fd(), IOC_GET_VERSION, &mut v)?;
        Ok(v)
    }

    /// Install a session into the kernel accelerator.
    pub fn session_add(&self, add: &SessionAdd) -> io::Result<()> {
        ioctl_ref(self.fd(), IOC_SESSION_ADD, add)?;
        Ok(())
    }

    /// Remove a session by its 16-byte session_id.
    pub fn session_remove(&self, session_id: &[u8; 16]) -> io::Result<()> {
        ioctl_ref(self.fd(), IOC_SESSION_DEL, session_id)?;
        Ok(())
    }

    /// Point the kernel accelerator at a TUN interface by its ifindex.
    pub fn set_tun(&self, ifindex: u32) -> io::Result<()> {
        ioctl_ref(self.fd(), IOC_SET_TUN, &ifindex)?;
        Ok(())
    }

    /// Point the kernel accelerator at an existing UDP socket by fd.
    pub fn set_udp_sock(&self, udp_fd: RawFd) -> io::Result<()> {
        let fd_as_u32 = udp_fd as u32;
        ioctl_ref(self.fd(), IOC_SET_UDP_SOCK, &fd_as_u32)?;
        Ok(())
    }

    /// Flush all sessions from the kernel table.
    pub fn flush(&self) -> io::Result<()> {
        ioctl_void(self.fd(), IOC_FLUSH)?;
        Ok(())
    }
}

impl Drop for KernelAccel {
    fn drop(&mut self) {
        let _ = self.flush();
    }
}

// ── ioctl helpers ─────────────────────────────────────────────────────────────

fn ioctl_ref<T>(fd: RawFd, cmd: u64, arg: &T) -> io::Result<i32> {
    let ret = unsafe { libc::ioctl(fd, cmd, arg as *const T) };
    if ret < 0 { Err(io::Error::last_os_error()) } else { Ok(ret) }
}

fn ioctl_void(fd: RawFd, cmd: u64) -> io::Result<i32> {
    let ret = unsafe { libc::ioctl(fd, cmd, 0usize) };
    if ret < 0 { Err(io::Error::last_os_error()) } else { Ok(ret) }
}
