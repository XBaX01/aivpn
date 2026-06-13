//! C FFI entry points for the iOS Network Extension (NEPacketTunnelProvider).
//!
//! The AivpnTunnel extension links against libaivpn_core.a (this crate compiled for
//! aarch64-apple-ios / x86_64-apple-ios-simulator) and calls these functions directly.

#![allow(non_snake_case)]

mod ios_tunnel;

use ios_tunnel::{
    get_active_download_bytes, get_active_upload_bytes, run_tunnel_ios, stop_active_tunnel,
    OnReadyFn, SendCtx,
};

/// Runs the full VPN tunnel session on the calling thread.
/// Returns 0 on clean rekey-triggered exit, -1 on error.
#[no_mangle]
pub extern "C" fn aivpn_run_tunnel(
    tun_fd: libc::c_int,
    server_host: *const libc::c_char,
    server_port: libc::c_int,
    server_key: *const u8,
    psk: *const u8,
    cert_bytes: *const u8,
    cert_len: libc::c_int,
    on_ready: Option<OnReadyFn>,
    ctx: *mut libc::c_void,
) -> libc::c_int {
    // SAFETY: server_host is a NUL-terminated C string from Swift.
    let host = unsafe {
        match std::ffi::CStr::from_ptr(server_host).to_str() {
            Ok(s) => s.to_owned(),
            Err(_) => return -1,
        }
    };

    // SAFETY: server_key points to 32 bytes passed by Swift.
    let key_bytes = unsafe {
        let mut arr = [0u8; 32];
        arr.copy_from_slice(std::slice::from_raw_parts(server_key, 32));
        arr
    };

    let psk_opt: Option<[u8; 32]> = if psk.is_null() {
        None
    } else {
        // SAFETY: psk points to 32 bytes passed by Swift.
        let mut arr = [0u8; 32];
        unsafe { arr.copy_from_slice(std::slice::from_raw_parts(psk, 32)) };
        Some(arr)
    };

    let mtls_cert: Option<Vec<u8>> = if cert_bytes.is_null() || cert_len <= 0 {
        None
    } else {
        // SAFETY: cert_bytes points to cert_len bytes passed by Swift.
        Some(unsafe { std::slice::from_raw_parts(cert_bytes, cert_len as usize).to_vec() })
    };

    let rt = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(rt) => rt,
        Err(_) => return -1,
    };

    match rt.block_on(run_tunnel_ios(
        tun_fd,
        host,
        server_port as u16,
        key_bytes,
        psk_opt,
        mtls_cert,
        on_ready,
        SendCtx(ctx),
    )) {
        Ok(()) => 0,
        Err(_) => -1,
    }
}

/// Close the active UDP socket so the tunnel loop exits immediately.
#[no_mangle]
pub extern "C" fn aivpn_stop_tunnel() {
    stop_active_tunnel();
}

/// Total bytes sent to the server in the current session.
#[no_mangle]
pub extern "C" fn aivpn_get_upload_bytes() -> i64 {
    get_active_upload_bytes() as i64
}

/// Total bytes received from the server in the current session.
#[no_mangle]
pub extern "C" fn aivpn_get_download_bytes() -> i64 {
    get_active_download_bytes() as i64
}
