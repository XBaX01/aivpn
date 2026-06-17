//! JNI entry points for the Android VPN service.
//!
//! Kotlin class: com.aivpn.client.AivpnJni
//!
//! The JNI function names encode class + method:
//!   Java_com_aivpn_client_AivpnJni_<method>

#![allow(non_snake_case)]

mod android_tunnel;

use aivpn_common::client_wire::DEFAULT_MDH_LEN;
use android_tunnel::{
    clear_pending_stop, get_active_download_bytes, get_active_upload_bytes, run_tunnel_android,
    stop_active_tunnel, ACTIVE_QUALITY_SCORE,
};

use std::sync::atomic::Ordering;

use jni::objects::{JByteArray, JClass, JObject, JString};
use jni::sys::{jint, jlong, jstring};
use jni::JNIEnv;

// ──────────────────────────────────────────────────────────
// runTunnel — blocking call; returns when tunnel stops/errors
// ──────────────────────────────────────────────────────────

/// Runs the full VPN tunnel session on the calling thread.
///
/// Parameters (Kotlin):
/// ```kotlin
/// external fun runTunnel(
///     vpnService: VpnService,
///     tunFd: Int,          // borrowed fd from ParcelFileDescriptor; Rust duplicates it
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
    psk_obj: JObject<'local>,       // nullable JByteArray
    mtls_cert_obj: JObject<'local>, // nullable JByteArray
    adaptive_level: jint,
    static_privkey_obj: JObject<'local>, // nullable JByteArray — device binding key
) -> jstring {
    // ── Initialize Android logcat logger once per process lifetime ──
    static LOG_INIT: std::sync::Once = std::sync::Once::new();
    LOG_INIT.call_once(|| {
        android_logger::init_once(
            android_logger::Config::default()
                .with_max_level(log::LevelFilter::Debug)
                .with_tag("aivpn"),
        );
    });

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
        Ok(b) => {
            return make_str(
                &mut env,
                &format!("server_key must be 32 bytes, got {}", b.len()),
            )
        }
        Err(e) => return make_str(&mut env, &format!("bad server_key: {e}")),
    };

    let psk: Option<[u8; 32]> = if psk_obj.is_null() {
        None
    } else {
        // Verify the JObject is a byte array before the unsafe cast.
        // An incorrect caller passing a non-byte-array would produce JVM type
        // confusion; reject early with a clear error instead.
        match env.is_instance_of(&psk_obj, "[B") {
            Ok(true) => {}
            Ok(false) => return make_str(&mut env, "psk must be a byte array (byte[])"),
            Err(e) => return make_str(&mut env, &format!("psk type check failed: {e}")),
        }
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

    let mtls_cert: Option<Vec<u8>> = if mtls_cert_obj.is_null() {
        None
    } else {
        match env.is_instance_of(&mtls_cert_obj, "[B") {
            Ok(true) => {}
            Ok(false) => return make_str(&mut env, "mtls_cert must be a byte array (byte[])"),
            Err(e) => return make_str(&mut env, &format!("mtls_cert type check failed: {e}")),
        }
        let arr: JByteArray<'local> = unsafe { JByteArray::from_raw(mtls_cert_obj.as_raw()) };
        match env.convert_byte_array(&arr) {
            Ok(b) if b.len() == 104 => Some(b),
            Ok(b) => {
                return make_str(
                    &mut env,
                    &format!("mtls_cert must be 104 bytes, got {}", b.len()),
                )
            }
            Err(e) => return make_str(&mut env, &format!("bad mtls_cert: {e}")),
        }
    };

    let static_privkey: Option<[u8; 32]> = if static_privkey_obj.is_null() {
        None
    } else {
        match env.is_instance_of(&static_privkey_obj, "[B") {
            Ok(true) => {}
            Ok(false) => return make_str(&mut env, "static_privkey must be a byte array (byte[])"),
            Err(e) => return make_str(&mut env, &format!("static_privkey type check failed: {e}")),
        }
        let arr: JByteArray<'local> = unsafe { JByteArray::from_raw(static_privkey_obj.as_raw()) };
        match env.convert_byte_array(&arr) {
            Ok(b) if b.len() == 32 => {
                let mut out = [0u8; 32];
                out.copy_from_slice(&b);
                Some(out)
            }
            Ok(b) => {
                return make_str(
                    &mut env,
                    &format!("static_privkey must be 32 bytes, got {}", b.len()),
                )
            }
            Err(e) => return make_str(&mut env, &format!("bad static_privkey: {e}")),
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
        mtls_cert,
        DEFAULT_MDH_LEN,
        adaptive_level.clamp(0, 3) as u8,
        static_privkey,
    ));

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
pub extern "system" fn Java_com_aivpn_client_AivpnJni_stopTunnel(_env: JNIEnv, _class: JClass) {
    stop_active_tunnel();
}

// clearPendingStop — called by the Kotlin restartJob right before launching a
// new intentional connection so the STOP_PENDING flag from the cleanup-phase
// stopTunnel() call does not propagate into the new session.
#[no_mangle]
pub extern "system" fn Java_com_aivpn_client_AivpnJni_clearPendingStop(
    _env: JNIEnv,
    _class: JClass,
) {
    clear_pending_stop();
}

// ──────────────────────────────────────────────────────────
// Traffic counters (polled by Kotlin every ~1 s)
// ──────────────────────────────────────────────────────────

/// Returns the last connection quality score (0–100) computed from KeepaliveAck RTT.
/// Returns 0 if no keepalive round-trip has been observed yet this session.
#[no_mangle]
pub extern "system" fn Java_com_aivpn_client_AivpnJni_getQualityScore(
    _env: JNIEnv,
    _class: JClass,
) -> jint {
    ACTIVE_QUALITY_SCORE.load(Ordering::Relaxed) as jint
}

#[no_mangle]
pub extern "system" fn Java_com_aivpn_client_AivpnJni_getUploadBytes(
    _env: JNIEnv,
    _class: JClass,
) -> jlong {
    get_active_upload_bytes() as jlong
}

#[no_mangle]
pub extern "system" fn Java_com_aivpn_client_AivpnJni_getDownloadBytes(
    _env: JNIEnv,
    _class: JClass,
) -> jlong {
    get_active_download_bytes() as jlong
}

// ──────────────────────────────────────────────────────────
// Helpers
// ──────────────────────────────────────────────────────────

fn make_str(env: &mut JNIEnv, s: &str) -> jstring {
    env.new_string(s)
        .expect("make_str: new_string failed")
        .into_raw()
}
