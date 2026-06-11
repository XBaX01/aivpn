// SPDX-License-Identifier: GPL-2.0
//! dev.rs — misc device registration and ioctl dispatch for aivpn.ko

use kernel::prelude::*;
use kernel::io_buffer::{IoBufferReader, IoBufferWriter};
use kernel::file_operations::{FileOperations, IoctlCommand, IoctlHandler};
use kernel::miscdev::Registration;

// ── UAPI ioctl numbers (mirrored from include/uapi/aivpn.h) ──────────────────
// _IOW(0xAE, nr, size) = (2<<30)|(magic<<8)|nr|(size<<16)
// _IOR(0xAE, nr, size) = (1<<30)|(magic<<8)|nr|(size<<16)
// _IOWR = (3<<30); _IO = magic<<8|nr

const M: u32 = 0xAE;
const fn iow(nr: u32, sz: u32) -> u32  { (2 << 30) | (M << 8) | nr | (sz << 16) }
const fn ior(nr: u32, sz: u32) -> u32  { (1 << 30) | (M << 8) | nr | (sz << 16) }
const fn iowr(nr: u32, sz: u32) -> u32 { (3 << 30) | (M << 8) | nr | (sz << 16) }
const fn io(nr: u32) -> u32            { (M << 8) | nr }

// Packed struct sizes matching C definitions (see include/uapi/aivpn.h)
//   aivpn_session_add:  16+32+32+32+8+4+28+8 = 160 bytes
//   aivpn_session_del:  16
//   aivpn_session_stat: 16+4+8+8+8+8 = 52 (with __attribute__((packed)))
//   aivpn_set_tun:      4
//   aivpn_set_udp_sock: 4
const IOC_SESSION_ADD:  u32 = iow(1,  160);
const IOC_SESSION_DEL:  u32 = iow(2,  16);
const IOC_SESSION_STAT: u32 = iowr(3, 52);
const IOC_SET_TUN:      u32 = iow(4,  4);
const IOC_SET_UDP_SOCK: u32 = iow(5,  4);
const IOC_FLUSH:        u32 = io(6);
const IOC_GET_VERSION:  u32 = ior(7,  4);
const API_VERSION:      u32 = 1;

// ── C helper declarations ─────────────────────────────────────────────────────

extern "C" {
    fn aivpn_session_insert(add: *const u8) -> i32;
    fn aivpn_session_remove(session_id: *const u8) -> i32;
    fn aivpn_session_stat(stat: *mut u8) -> i32;
    fn aivpn_session_flush();
    fn aivpn_tun_set_device(ifindex: u32) -> i32;
    fn aivpn_tun_clear();
    fn aivpn_udp_hook_install_by_fd(fd: i32) -> i32;
}

// ── Device ────────────────────────────────────────────────────────────────────

pub struct AivpnDev {
    _reg: Pin<Box<Registration<AivpnDev>>>,
}

impl AivpnDev {
    pub fn new() -> Result<Pin<Box<Self>>> {
        let reg = Registration::new_pinned::<AivpnDev>(
            kernel::c_str!("aivpn"),
            None,
            &kernel::THIS_MODULE,
        )?;
        Ok(Box::try_pin_init(try_pin_init!(AivpnDev { _reg: reg }))?)
    }
}

#[vtable]
impl FileOperations for AivpnDev {
    type Data = ();
    type OpenData = ();
    fn open(_: &(), _: &kernel::file::File) -> Result<()> { Ok(()) }
}

impl IoctlHandler for AivpnDev {
    type Target<'a> = ();

    fn ioctl((): &(), _file: &kernel::file::File, cmd: &mut IoctlCommand) -> Result<i32> {
        match cmd.raw_cmd() {
            n if n == IOC_SESSION_ADD => {
                let mut buf = [0u8; 160];
                cmd.user_slice_reader()?.read_slice(&mut buf)?;
                to_result(unsafe { aivpn_session_insert(buf.as_ptr()) })?;
                Ok(0)
            }
            n if n == IOC_SESSION_DEL => {
                let mut id = [0u8; 16];
                cmd.user_slice_reader()?.read_slice(&mut id)?;
                to_result(unsafe { aivpn_session_remove(id.as_ptr()) })?;
                Ok(0)
            }
            n if n == IOC_SESSION_STAT => {
                let mut buf = [0u8; 52];
                cmd.user_slice_reader()?.read_slice(&mut buf[..16])?;
                to_result(unsafe { aivpn_session_stat(buf.as_mut_ptr()) })?;
                cmd.user_slice_writer()?.write_slice(&buf)?;
                Ok(0)
            }
            n if n == IOC_SET_TUN => {
                let mut b = [0u8; 4];
                cmd.user_slice_reader()?.read_slice(&mut b)?;
                to_result(unsafe { aivpn_tun_set_device(u32::from_ne_bytes(b)) })?;
                Ok(0)
            }
            n if n == IOC_SET_UDP_SOCK => {
                let mut b = [0u8; 4];
                cmd.user_slice_reader()?.read_slice(&mut b)?;
                to_result(unsafe { aivpn_udp_hook_install_by_fd(i32::from_ne_bytes(b)) })?;
                Ok(0)
            }
            n if n == IOC_FLUSH => {
                unsafe { aivpn_session_flush() };
                Ok(0)
            }
            n if n == IOC_GET_VERSION => {
                cmd.user_slice_writer()?.write_slice(&API_VERSION.to_ne_bytes())?;
                Ok(0)
            }
            _ => Err(EINVAL),
        }
    }
}
