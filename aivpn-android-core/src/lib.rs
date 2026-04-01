//! JNI entry points for the Android VPN service.
//!
//! Kotlin class: com.aivpn.client.AivpnJni
//!
//! The JNI function names encode class + method:
//!   Java_com_aivpn_client_AivpnJni_<method>

#![allow(non_snake_case)]

mod android_tunnel;

use android_tunnel::{run_tunnel_android, DOWNLOAD_BYTES, TUNNEL_UDP_FD, UPLOAD_BYTES};

use jni::objects::{JByteArray, JClass, JObject, JString};
use jni::sys::{jint, jlong, jstring};
use jni::JNIEnv;
use std::sync::atomic::Ordering;

// ──────────────────────────────────────────────────────────
// runTunnel — blocking call; returns when tunnel stops/errors
// ──────────────────────────────────────────────────────────

/// Runs the full VPN tunnel session on the calling thread.
///
/// Parameters (Kotlin):
/// ```kotlin
/// external fun runTunnel(
///     vpnService: VpnService,
///     tunFd: Int,          // from ParcelFileDescriptor.detachFd()
///     serverHost: String,
///     serverPort: Int,
///     serverKey: ByteArray, // 32 bytes
///     psk: ByteArray?,      // 32 bytes or null
/// ): String               // "" on clean exit, error message otherwise
/// ```
#[no_mangle]
pub extern "system" fn Java_com_aivpn_client_AivpnJni_runTunnel<'local>(
    mut env: JNIEnv<'local>,
    _class: JClass<'local>,
    vpn_service: JObject<'local>,
    tun_fd: jint,
    server_host: JString<'local>,
    server_port: jint,
    server_key_arr: JByteArray<'local>,
    psk_obj: JObject<'local>, // nullable JByteArray
) -> jstring {
    // ── Unpack arguments ──
    let host = match env.get_string(&server_host) {
        Ok(s) => String::from(s),
        Err(e) => return make_str(&mut env, &format!("bad server_host: {e}")),
    };

    let key_bytes = match env.convert_byte_array(&server_key_arr) {
        Ok(b) if b.len() == 32 => {
            let mut arr = [0u8; 32];
            arr.copy_from_slice(&b);
            arr
        }
        Ok(b) => return make_str(&mut env, &format!("server_key must be 32 bytes, got {}", b.len())),
        Err(e) => return make_str(&mut env, &format!("bad server_key: {e}")),
    };

    let psk: Option<[u8; 32]> = if psk_obj.is_null() {
        None
    } else {
        let arr: JByteArray<'local> = unsafe { JByteArray::from_raw(psk_obj.as_raw()) };
        match env.convert_byte_array(&arr) {
            Ok(b) if b.len() == 32 => {
                let mut out = [0u8; 32];
                out.copy_from_slice(&b);
                Some(out)
            }
            Ok(b) => return make_str(&mut env, &format!("psk must be 32 bytes, got {}", b.len())),
            Err(e) => return make_str(&mut env, &format!("bad psk: {e}")),
        }
    };

    // ── Get JavaVM for use inside the tokio runtime ──
    let vm = match env.get_java_vm() {
        Ok(vm) => vm,
        Err(e) => return make_str(&mut env, &format!("get_java_vm: {e}")),
    };
    let vpn_global = match env.new_global_ref(&vpn_service) {
        Ok(g) => g,
        Err(e) => return make_str(&mut env, &format!("global_ref: {e}")),
    };

    // ── Run on a current-thread tokio runtime (we ARE an IO thread already) ──
    let rt = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(rt) => rt,
        Err(e) => return make_str(&mut env, &format!("tokio runtime: {e}")),
    };

    let result = rt.block_on(run_tunnel_android(
        vm,
        vpn_global,
        tun_fd,
        host,
        server_port as u16,
        key_bytes,
        psk,
    ));

    // Clean up global fd reference so stopTunnel() doesn't close a stale fd.
    TUNNEL_UDP_FD.store(-1, Ordering::SeqCst);

    match result {
        Ok(()) => make_str(&mut env, ""),
        Err(e) => make_str(&mut env, &e.to_string()),
    }
}

// ──────────────────────────────────────────────────────────
// stopTunnel — closes the protected UDP socket so recv() fails
// and the tunnel loop exits immediately.
// ──────────────────────────────────────────────────────────

#[no_mangle]
pub extern "system" fn Java_com_aivpn_client_AivpnJni_stopTunnel(
    _env: JNIEnv,
    _class: JClass,
) {
    let fd = TUNNEL_UDP_FD.swap(-1, Ordering::SeqCst);
    if fd >= 0 {
        unsafe { libc::close(fd) };
    }
}

// ──────────────────────────────────────────────────────────
// Traffic counters (polled by Kotlin every ~1 s)
// ──────────────────────────────────────────────────────────

#[no_mangle]
pub extern "system" fn Java_com_aivpn_client_AivpnJni_getUploadBytes(
    _env: JNIEnv,
    _class: JClass,
) -> jlong {
    UPLOAD_BYTES.load(Ordering::Relaxed) as jlong
}

#[no_mangle]
pub extern "system" fn Java_com_aivpn_client_AivpnJni_getDownloadBytes(
    _env: JNIEnv,
    _class: JClass,
) -> jlong {
    DOWNLOAD_BYTES.load(Ordering::Relaxed) as jlong
}

// ──────────────────────────────────────────────────────────
// Helpers
// ──────────────────────────────────────────────────────────

fn make_str(env: &mut JNIEnv, s: &str) -> jstring {
    env.new_string(s)
        .expect("make_str: new_string failed")
        .into_raw()
}
