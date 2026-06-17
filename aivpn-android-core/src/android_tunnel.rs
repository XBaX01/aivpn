//! Android VPN tunnel — runs on top of a TUN fd created by VpnService.Builder and a UDP
//! socket created here and exempted via VpnService.protect(int).
//!
//! Wire protocol is byte-for-byte identical to AivpnCrypto.kt so that both can talk to the
//! same Rust server without any server-side changes.

use std::net::{SocketAddr, SocketAddrV4};
use std::os::fd::OwnedFd;
use std::os::unix::io::{AsRawFd, FromRawFd, RawFd};
use std::sync::atomic::{AtomicBool, AtomicI32, AtomicU16, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use jni::objects::GlobalRef;
use jni::JavaVM;
use tokio::io::unix::AsyncFd;
use tokio::net::UdpSocket;
use tokio::sync::mpsc;
use tokio::time;

use aivpn_common::client_wire::{
    build_inner_packet, build_random_mdh_packet, decode_packet_with_mdh_len,
    obfuscate_client_eph_pub, process_server_hello_with_mdh_len, RecvWindow,
};
use aivpn_common::crypto::{derive_session_keys, KeyPair, SessionKeys};
use aivpn_common::error::{Error, Result};
use aivpn_common::protocol::{ControlPayload, InnerType};
use aivpn_common::upload_pipeline::{self, PacketEncryptor, UploadConfig, ZeroMdhEncryptor};

// ──────────── Constants ────────────

const BUF_SIZE: usize = 2048;
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(10);
const HANDSHAKE_RETRY_INTERVAL: Duration = Duration::from_millis(750);
const KEEPALIVE_INTERVAL: Duration = Duration::from_secs(4); // below typical provider NAT UDP timeout (~10-15s)
const ADAPTIVE_KEEPALIVE_INTERVAL: Duration = Duration::from_secs(2); // aggressive keepalive for restrictive mobile NAT (MTS/Megafon)
const RX_SILENCE: Duration = Duration::from_secs(120); // backup watchdog; network callback already handles real link loss
const RX_CHECK_INTERVAL: Duration = Duration::from_secs(2);
// Mobile networks can briefly stall or batch downstream delivery. Keep this
// detector responsive, but avoid tearing down an otherwise healthy session
// after only a few kilobytes of outbound browser traffic.
const TX_WITHOUT_RX_TIMEOUT: Duration = Duration::from_secs(20);
const TX_WITHOUT_RX_MIN_BYTES: u64 = 64 * 1024;
const CHANNEL_SIZE: usize = 8192;

// ──────────── Session runtime (read by JNI exports in lib.rs) ────────────

pub struct SessionRuntime {
    udp_control_fd: AtomicI32,
    stop_event_fd: AtomicI32,
    upload_bytes: AtomicU64,
    download_bytes: AtomicU64,
    // Set by stop_active_tunnel() before eventfd/socket are ready so that early
    // init phases (DNS, socket creation) can check and bail out immediately.
    stop_requested: AtomicBool,
}

impl SessionRuntime {
    fn new() -> Self {
        Self {
            udp_control_fd: AtomicI32::new(-1),
            stop_event_fd: AtomicI32::new(-1),
            upload_bytes: AtomicU64::new(0),
            download_bytes: AtomicU64::new(0),
            stop_requested: AtomicBool::new(false),
        }
    }
}

static ACTIVE_SESSION: Mutex<Option<Arc<SessionRuntime>>> = Mutex::new(None);

// Last local UDP port used by a tunnel session.  On reconnect we try to bind
// to the same port so CGNAT carriers (MTS et al.) with port-preserving NAT
// don't need to update their inbound routing table — the old mapping already
// points to the right port and downlink arrives immediately.
static LAST_LOCAL_PORT: AtomicU16 = AtomicU16::new(0);

// Set by stop_active_tunnel() when called while no session is active (the gap
// between the old session's ActiveSessionGuard drop and the new session's
// activate_session() call).  activate_session() propagates this to the new
// session so it stops immediately.  clear_pending_stop() resets the flag
// when a new intentional connection is about to start (called from the
// restartJob in Kotlin after cancelAndJoin()).
static STOP_PENDING: AtomicBool = AtomicBool::new(false);

struct ActiveSessionGuard {
    session: Arc<SessionRuntime>,
}

impl Drop for ActiveSessionGuard {
    fn drop(&mut self) {
        let udp_fd = self.session.udp_control_fd.swap(-1, Ordering::SeqCst);
        if udp_fd >= 0 {
            unsafe { libc::close(udp_fd) };
        }

        let stop_fd = self.session.stop_event_fd.swap(-1, Ordering::SeqCst);
        if stop_fd >= 0 {
            unsafe { libc::close(stop_fd) };
        }

        let mut guard = ACTIVE_SESSION.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(current) = guard.as_ref() {
            if Arc::ptr_eq(current, &self.session) {
                *guard = None;
            }
        }
    }
}

fn activate_session(session: Arc<SessionRuntime>) -> Result<ActiveSessionGuard> {
    let mut guard = ACTIVE_SESSION
        .lock()
        .map_err(|_| Error::Session("Active session lock poisoned".into()))?;

    if let Some(existing) = guard.as_ref() {
        if !existing.stop_requested.load(Ordering::SeqCst) {
            return Err(Error::Session(
                "Another Android tunnel session is already active".into(),
            ));
        }
        // Previous session was told to stop but the Rust task has not yet
        // exited (service destroyed before JNI returned).  Evict it so the
        // new connection can proceed; the old ActiveSessionGuard will clear
        // ACTIVE_SESSION only if ptr_eq matches — it won't touch ours.
    }

    // Propagate any stop that arrived while no session was active.
    if STOP_PENDING.swap(false, Ordering::SeqCst) {
        session.stop_requested.store(true, Ordering::SeqCst);
    }

    *guard = Some(session.clone());
    Ok(ActiveSessionGuard { session })
}

pub fn stop_active_tunnel() {
    let (udp_fd, stop_fd) = {
        let guard = ACTIVE_SESSION.lock().unwrap_or_else(|e| e.into_inner());
        guard
            .as_ref()
            .map(|s| {
                // Set the flag FIRST so early init phases (DNS lookup, socket
                // creation) see it before the eventfd/UDP fd are available.
                s.stop_requested.store(true, Ordering::SeqCst);
                (
                    s.udp_control_fd.swap(-1, Ordering::SeqCst),
                    s.stop_event_fd.load(Ordering::SeqCst),
                )
            })
            .unwrap_or_else(|| {
                // No active session in the window between the old session's
                // guard drop and the new session's activate_session() call.
                // Mark the flag so the next session inherits the stop.
                STOP_PENDING.store(true, Ordering::SeqCst);
                (-1, -1)
            })
    };

    if stop_fd >= 0 {
        #[cfg(any(target_os = "android", target_os = "linux"))]
        {
            let value: u64 = 1;
            unsafe {
                let _ = libc::write(
                    stop_fd,
                    &value as *const u64 as *const libc::c_void,
                    std::mem::size_of::<u64>(),
                );
            };
        }
        #[cfg(not(any(target_os = "android", target_os = "linux")))]
        {
            let v: u8 = 1;
            unsafe {
                let _ = libc::write(stop_fd, &v as *const u8 as *const libc::c_void, 1);
            };
        }
    }

    if udp_fd >= 0 {
        unsafe {
            libc::shutdown(udp_fd, libc::SHUT_RDWR);
            libc::close(udp_fd);
        };
    }
}

/// Called by the Kotlin restartJob after cancelAndJoin() — clears any pending
/// stop that was set during the cleanup phase so the intentional new connection
/// is not immediately stopped by a stale flag.
pub fn clear_pending_stop() {
    STOP_PENDING.store(false, Ordering::SeqCst);
}

pub fn get_active_upload_bytes() -> u64 {
    ACTIVE_SESSION
        .lock()
        .ok()
        .and_then(|guard| {
            guard
                .as_ref()
                .map(|s| s.upload_bytes.load(Ordering::Relaxed))
        })
        .unwrap_or(0)
}

pub fn get_active_download_bytes() -> u64 {
    ACTIVE_SESSION
        .lock()
        .ok()
        .and_then(|guard| {
            guard
                .as_ref()
                .map(|s| s.download_bytes.load(Ordering::Relaxed))
        })
        .unwrap_or(0)
}

// ──────────── Entry point ────────────

/// Blocking async function that runs the whole tunnel session.
/// Returns Err on any tunnel failure (causes the Kotlin reconnect loop to kick in).
pub async fn run_tunnel_android(
    vm: JavaVM,
    vpn_service: GlobalRef,
    tun_fd_int: RawFd,
    server_host: String,
    server_port: u16,
    server_key: [u8; 32],
    psk: Option<[u8; 32]>,
    mtls_cert: Option<Vec<u8>>,
    mdh_len: usize,
    adaptive: bool,
    static_privkey: Option<[u8; 32]>,
) -> Result<()> {
    let session = Arc::new(SessionRuntime::new());
    let _active_session_guard = activate_session(session.clone())?;

    // ── 1. Ephemeral keypair + initial session keys ──
    let mut keypair = KeyPair::generate();
    let mut dh = keypair.compute_shared(&server_key)?;
    let mut keys = derive_session_keys(&dh, psk.as_ref(), &keypair.public_key_bytes());

    // ── 2. Create stop signal immediately — before DNS — so a disconnect press
    //    during a slow/hung cellular DNS is handled instantly, not after a 5 s wait.
    let stop_signal = create_stop_signal(&session)?;

    // Resolve host; race against stop signal so disconnect is always responsive.
    let dest_str = format!("{}:{}", server_host, server_port);
    let dest: SocketAddr = tokio::select! {
        biased;
        _ = wait_for_stop_signal(&stop_signal) => {
            return Err(Error::Session("Stop requested".into()));
        }
        result = tokio::time::timeout(
            Duration::from_secs(5),
            tokio::net::lookup_host(&dest_str),
        ) => {
            result
                .map_err(|_| Error::Session("DNS lookup timeout (5 s)".into()))?
                .map_err(Error::Io)?
                .find(|a| a.is_ipv4())
                .ok_or_else(|| Error::Session("Cannot resolve server host to IPv4".into()))?
        }
    };

    if session.stop_requested.load(Ordering::SeqCst) {
        return Err(Error::Session("Stop requested".into()));
    }

    let raw_udp_fd = create_protected_udp_socket(&vm, &vpn_service, dest, &session)?;

    if session.stop_requested.load(Ordering::SeqCst) {
        unsafe { libc::close(raw_udp_fd) };
        return Err(Error::Session("Stop requested".into()));
    }

    // ── 3. Set TUN fd to non-blocking for AsyncFd ──
    let owned_tun_fd = unsafe { libc::dup(tun_fd_int) };
    if owned_tun_fd < 0 {
        return Err(Error::Io(std::io::Error::last_os_error()));
    }

    unsafe { libc::fcntl(owned_tun_fd, libc::F_SETFL, libc::O_NONBLOCK) };
    // SAFETY: this is Rust's private duplicate of the Android-owned TUN fd.
    let owned_tun = unsafe { OwnedFd::from_raw_fd(owned_tun_fd) };
    let tun = AsyncFd::new(owned_tun)?;

    // Convert the raw UDP fd to a tokio UdpSocket (already connected to server).
    let std_udp = unsafe { std::net::UdpSocket::from_raw_fd(raw_udp_fd) };
    std_udp.set_nonblocking(true)?;
    let udp = Arc::new(UdpSocket::from_std(std_udp)?);

    // ── 4. Send init handshake (Control/Keepalive + obfuscated eph_pub) ──
    let mut send_counter: u64 = 0;
    let mut send_seq: u16 = 0;
    let keepalive = ControlPayload::Keepalive { send_ts: 0 }.encode()?;
    {
        let obf_pub = obfuscate_client_eph_pub(&keypair, &server_key);
        let inner = build_inner_packet(InnerType::Control, send_seq, &keepalive);
        let pkt =
            build_random_mdh_packet(&keys, &mut send_counter, &inner, Some(&obf_pub), mdh_len)?;
        send_seq = send_seq.wrapping_add(1);
        udp.send(&pkt).await?;
    }

    // ── 5. Wait for ServerHello with timeout ──
    let mut recv_buf = vec![0u8; BUF_SIZE];
    let handshake_deadline = Instant::now() + HANDSHAKE_TIMEOUT;
    let mut retry_count: u32 = 0;
    let n = loop {
        let now = Instant::now();
        if now >= handshake_deadline {
            return Err(Error::Session("Handshake timeout (10 s)".into()));
        }

        let wait = std::cmp::min(
            HANDSHAKE_RETRY_INTERVAL,
            handshake_deadline.saturating_duration_since(now),
        );
        let retry = time::sleep(wait);
        tokio::pin!(retry);

        tokio::select! {
            _ = wait_for_stop_signal(&stop_signal) => {
                return Err(Error::Session("Tunnel stop requested".into()));
            }

            res = udp.recv(&mut recv_buf) => {
                match res {
                    Ok(n) => break n,
                    Err(e) => return Err(Error::Io(e)),
                }
            }
            _ = &mut retry => {
                if session.stop_requested.load(Ordering::SeqCst) {
                    return Err(Error::Session("Tunnel stop requested".into()));
                }
                retry_count += 1;
                // Rotate keypair only once, on the 2nd retry (~1.5 s after first send).
                // Rotating on every retry created a new server session per 750 ms (~13
                // ghost sessions per 10 s timeout), which caused CGNAT per-IP cap (5)
                // to be hit on the 2nd handshake attempt.  A single rotation at retry 2
                // limits server ghost sessions to 2 max while still forcing a fresh
                // handshake if the server lost the original one.
                if retry_count == 2 {
                    keypair = KeyPair::generate();
                    dh = keypair.compute_shared(&server_key)?;
                    keys = derive_session_keys(&dh, psk.as_ref(), &keypair.public_key_bytes());
                    send_counter = 0;
                    send_seq = 0;
                }
                let obf_pub = obfuscate_client_eph_pub(&keypair, &server_key);
                let inner = build_inner_packet(InnerType::Control, send_seq, &keepalive);
                let pkt = build_random_mdh_packet(&keys, &mut send_counter, &inner, Some(&obf_pub), mdh_len)?;
                send_seq = send_seq.wrapping_add(1);
                udp.send(&pkt).await?;
            }
        }
    };

    let mut recv_win = RecvWindow::new();
    let server_network_cfg = process_server_hello_with_mdh_len(
        &recv_buf[..n],
        &mut keys,
        &keypair,
        &mut recv_win,
        &mut send_counter,
        mdh_len,
    )?;
    let base_keepalive = server_network_cfg
        .as_ref()
        .and_then(|c| c.keepalive_secs)
        .filter(|&s| s > 0)
        .map(|s| Duration::from_secs(s as u64))
        .unwrap_or(KEEPALIVE_INTERVAL);
    let keepalive_interval = if adaptive {
        base_keepalive.min(ADAPTIVE_KEEPALIVE_INTERVAL)
    } else {
        base_keepalive
    };
    let mut transition_recv_keys: Option<SessionKeys> = Some(derive_session_keys(
        &dh,
        psk.as_ref(),
        &keypair.public_key_bytes(),
    ));
    let mut transition_recv_deadline = Some(Instant::now() + Duration::from_secs(2));
    let mut transition_recv_win = std::mem::take(&mut recv_win);
    if let Some(cert) = mtls_cert {
        let cert_len_debug = cert.len();
        let cert_payload = ControlPayload::ClientCert { cert_bytes: cert }.encode()?;
        let inner = build_inner_packet(InnerType::Control, send_seq, &cert_payload);
        let pkt = build_random_mdh_packet(&keys, &mut send_counter, &inner, None, mdh_len)?;
        send_seq = send_seq.wrapping_add(1);
        udp.send(&pkt).await?;
        log::debug!("mTLS: ClientCert sent ({} bytes)", cert_len_debug);
    }
    // Immediately send a keepalive to prevent CGNAT outbound mapping expiry.
    // Megafon/MTS CGNAT can expire the outbound UDP binding in the gap between the
    // last handshake packet and the upload pipeline's first keepalive tick (which is
    // intentionally skipped). One early packet keeps the NAT entry alive.
    {
        let ka = ControlPayload::Keepalive { send_ts: 0 }.encode()?;
        let inner = build_inner_packet(InnerType::Control, send_seq, &ka);
        if let Ok(pkt) = build_random_mdh_packet(&keys, &mut send_counter, &inner, None, mdh_len) {
            send_seq = send_seq.wrapping_add(1);
            let _ = udp.send(&pkt).await;
        }
    }
    notify_tunnel_ready(&vm, &vpn_service, &server_host);
    log::info!("aivpn: handshake + PFS ratchet complete");

    // Warmup: 4 keepalives spaced 100 ms apart after the handshake.
    // Primary fix is local-port reuse (see LAST_LOCAL_PORT above); this is
    // the fallback for carriers that have a brief delay before updating their
    // inbound CGNAT entry even after the outbound mapping was refreshed.
    // Each outbound packet nudges the CGNAT to route subsequent downlink to
    // the current socket rather than the previous (closed) one.
    for _ in 0..4u8 {
        tokio::select! {
            biased;
            _ = wait_for_stop_signal(&stop_signal) => {
                return Err(Error::Session("Tunnel stop requested".into()));
            }
            _ = tokio::time::sleep(Duration::from_millis(100)) => {
                if let Ok(ka) = (ControlPayload::Keepalive { send_ts: 0 }).encode() {
                    let inner = build_inner_packet(InnerType::Control, send_seq, &ka);
                    if let Ok(pkt) = build_random_mdh_packet(&keys, &mut send_counter, &inner, None, mdh_len) {
                        send_seq = send_seq.wrapping_add(1);
                        let _ = udp.send(&pkt).await;
                    }
                }
            }
        }
    }

    // Device enrollment: send static key proof after ratchet (PFS-protected).
    if let Some(priv_bytes) = static_privkey {
        let static_kp = KeyPair::from_private_key(priv_bytes);
        if let Ok(dh_proof) = static_kp.compute_shared(&server_key) {
            let enrollment = ControlPayload::DeviceEnrollment {
                static_pub: static_kp.public_key_bytes(),
                dh_proof,
            };
            if let Ok(encoded) = enrollment.encode() {
                let inner = build_inner_packet(InnerType::Control, send_seq, &encoded);
                if let Ok(pkt) =
                    build_random_mdh_packet(&keys, &mut send_counter, &inner, None, mdh_len)
                {
                    send_seq = send_seq.wrapping_add(1);
                    let _ = udp.send(&pkt).await;
                }
            }
        }
    }

    // ── 6. Main forwarding loop ──
    let mut udp_buf = vec![0u8; BUF_SIZE];
    let mut last_rx = Instant::now();
    let mut upload_at_last_rx = session.upload_bytes.load(Ordering::Relaxed);

    // Split upload into a dedicated pipeline:
    // TUN reader task -> channel -> UDP sender/encrypt task.
    let (tun_tx, mut tun_rx) = mpsc::channel::<Vec<u8>>(CHANNEL_SIZE);
    let (err_tx, mut err_rx) = mpsc::channel::<String>(16);
    let tun_err_tx = err_tx.clone();
    let sender_err_tx = err_tx.clone();

    let read_fd = unsafe { libc::dup(tun.as_raw_fd()) };
    if read_fd < 0 {
        return Err(Error::Io(std::io::Error::last_os_error()));
    }
    let owned_tun_read = unsafe { OwnedFd::from_raw_fd(read_fd) };
    let tun_read = AsyncFd::new(owned_tun_read)?;

    let tun_reader_task = tokio::spawn(async move {
        let mut tun_buf = vec![0u8; BUF_SIZE];
        loop {
            match tun_async_read(&tun_read, &mut tun_buf).await {
                Ok(n) => {
                    if n == 0 {
                        continue;
                    }
                    if tun_buf[0] >> 4 != 4 {
                        continue;
                    }
                    if tun_tx.send(tun_buf[..n].to_vec()).await.is_err() {
                        break;
                    }
                }
                Err(e) => {
                    let _ = tun_err_tx.send(format!("TUN read failed: {e}")).await;
                    break;
                }
            }
        }
    });

    let udp_tx = udp.clone();
    let keys_tx = keys.clone();
    let session_for_upload = session.clone();
    let upload_sender_task = tokio::spawn(async move {
        // Wrap ZeroMdhEncryptor with UPLOAD_BYTES tracking.
        struct AndroidEncryptor {
            inner: ZeroMdhEncryptor,
            session: Arc<SessionRuntime>,
        }

        impl PacketEncryptor for AndroidEncryptor {
            fn encrypt_data(&mut self, payload: &[u8]) -> aivpn_common::error::Result<Vec<u8>> {
                self.inner.encrypt_data(payload)
            }
            fn encrypt_control(
                &mut self,
                payload: &aivpn_common::protocol::ControlPayload,
            ) -> aivpn_common::error::Result<Vec<u8>> {
                self.inner.encrypt_control(payload)
            }
            fn encrypt_keepalive(&mut self) -> aivpn_common::error::Result<Vec<u8>> {
                self.inner.encrypt_keepalive()
            }
            fn on_data_sent(&mut self, payload_len: usize) {
                self.session
                    .upload_bytes
                    .fetch_add(payload_len as u64, Ordering::Relaxed);
            }
        }

        let mut enc = AndroidEncryptor {
            inner: ZeroMdhEncryptor::with_mdh_len(keys_tx, send_counter, send_seq, mdh_len),
            session: session_for_upload,
        };
        let config = UploadConfig {
            keepalive_interval,
            ..Default::default()
        };

        if let Err(e) =
            upload_pipeline::run_upload_loop(&mut tun_rx, None, &udp_tx, &mut enc, &config).await
        {
            let _ = sender_err_tx.send(format!("Upload pipeline: {e}")).await;
        }
    });
    // Periodic check for RX silence — uses a proper Interval so it's not
    // recreated every select! iteration (which would reset the timer).
    let mut rx_check = time::interval(RX_CHECK_INTERVAL);
    rx_check.set_missed_tick_behavior(time::MissedTickBehavior::Delay);

    loop {
        tokio::select! {
            biased;

            _ = wait_for_stop_signal(&stop_signal) => {
                // Send Shutdown 3× (50 ms apart) so the server drops the session
                // immediately even if one UDP packet is lost on the mobile path.
                if let Ok(shutdown_bytes) = (ControlPayload::Shutdown { reason: 0 }).encode() {
                    let inner = build_inner_packet(InnerType::Control, send_seq, &shutdown_bytes);
                    if let Ok(pkt) = build_random_mdh_packet(&keys, &mut send_counter, &inner, None, mdh_len) {
                        for _ in 0..3u8 {
                            let _ = udp.send(&pkt).await;
                            tokio::time::sleep(Duration::from_millis(50)).await;
                        }
                    }
                }
                tun_reader_task.abort();
                upload_sender_task.abort();
                return Err(Error::Session("Tunnel stop requested".into()));
            }

            // ── UDP → TUN (inbound from server) ──
            r = udp.recv(&mut udp_buf) => {
                let n = r?;
                log::debug!("aivpn: udp.recv() → {} bytes", n);
                last_rx = Instant::now();
                upload_at_last_rx = session.upload_bytes.load(Ordering::Relaxed);
                if transition_recv_deadline.is_some_and(|deadline| Instant::now() >= deadline) {
                    transition_recv_keys = None;
                    transition_recv_deadline = None;
                    transition_recv_win.reset();
                }
                let decoded = match decode_packet_with_mdh_len(
                    &udp_buf[..n],
                    &keys,
                    &mut recv_win,
                    mdh_len,
                ) {
                    Ok(decoded) => {
                        Some(decoded)
                    }
                    Err(e) => {
                        log::debug!("aivpn: decode failed (primary keys): {}", e);
                        if let Some(fallback_keys) = transition_recv_keys.as_ref() {
                            let r = decode_packet_with_mdh_len(
                                &udp_buf[..n],
                                fallback_keys,
                                &mut transition_recv_win,
                                mdh_len,
                            );
                            if r.is_err() {
                                log::debug!("aivpn: decode failed (fallback keys) — packet dropped");
                            }
                            r.ok()
                        } else {
                            None
                        }
                    }
                };

                if let Some(decoded) = decoded {
                    log::debug!("aivpn: decoded inner_type={:?} payload={} bytes",
                        decoded.header.inner_type, decoded.payload.len());
                    if decoded.header.inner_type == InnerType::Data && !decoded.payload.is_empty() {
                        tun_async_write(&tun, &decoded.payload).await?;
                        session
                            .download_bytes
                            .fetch_add(decoded.payload.len() as u64, Ordering::Relaxed);
                        log::debug!("aivpn: wrote {} bytes to TUN (rx total={})",
                            decoded.payload.len(),
                            session.download_bytes.load(Ordering::Relaxed));
                    }
                    // Any successfully decoded packet (including keepalive responses)
                    // proves the link is alive.
                    // Handle server-initiated inline rekey (PFS without reconnect).
                    if decoded.header.inner_type == InnerType::Control {
                        if let Ok(ctrl) = ControlPayload::decode(&decoded.payload) {
                            if let ControlPayload::KeyRotate { new_eph_pub } = ctrl {
                                let rekey_kp = KeyPair::generate();
                                if let Ok(dh) = rekey_kp.compute_shared(&new_eph_pub) {
                                    let new_keys = derive_session_keys(
                                        &dh,
                                        Some(&keys.session_key),
                                        &rekey_kp.public_key_bytes(),
                                    );
                                    // Respond with our new ephemeral pub using OLD keys
                                    if let Ok(resp) = (ControlPayload::KeyRotate {
                                        new_eph_pub: rekey_kp.public_key_bytes()
                                    }).encode() {
                                        let inner = build_inner_packet(
                                            InnerType::Control, send_seq, &resp,
                                        );
                                        if let Ok(pkt) = build_random_mdh_packet(
                                            &keys, &mut send_counter, &inner, None, mdh_len,
                                        ) {
                                            let _ = udp.send(&pkt).await;
                                            send_seq = send_seq.wrapping_add(1);
                                        }
                                    }
                                    // Transition: old keys for receive 2 s, new keys for send
                                    transition_recv_keys = Some(keys.clone());
                                    transition_recv_deadline =
                                        Some(Instant::now() + Duration::from_secs(2));
                                    transition_recv_win.reset();
                                    keys = new_keys;
                                    log::info!("aivpn: inline PFS rekey complete");
                                }
                            }
                        }
                    }
                }
            }

            maybe_err = err_rx.recv() => {
                if let Some(msg) = maybe_err {
                    tun_reader_task.abort();
                    upload_sender_task.abort();
                    return Err(Error::Session(msg));
                }
            }

            // ── RX silence detector (proper interval, not recreated each iteration) ──
            _ = rx_check.tick() => {
                let silence = last_rx.elapsed();
                let uploaded_total = session.upload_bytes.load(Ordering::Relaxed);
                let uploaded_since_rx = uploaded_total.saturating_sub(upload_at_last_rx);

                // Half-open path detector: TX is actively flowing, but no RX returns.
                // This catches "connected but dead" states faster after network switches.
                if silence > TX_WITHOUT_RX_TIMEOUT && uploaded_since_rx >= TX_WITHOUT_RX_MIN_BYTES {
                    tun_reader_task.abort();
                    upload_sender_task.abort();
                    return Err(Error::Session(
                        format!(
                            "TX without RX: {} bytes sent in {:?} since last RX — reconnecting",
                            uploaded_since_rx,
                            silence
                        )
                    ));
                }

                if silence > RX_SILENCE {
                    tun_reader_task.abort();
                    upload_sender_task.abort();
                    return Err(Error::Session(
                        format!("No RX for {:?} — reconnecting", silence)
                    ));
                }
            }
        }
    }
}

fn notify_tunnel_ready(vm: &JavaVM, vpn_service: &GlobalRef, host: &str) {
    let mut env = match vm.attach_current_thread() {
        Ok(env) => env,
        Err(e) => {
            log::warn!("aivpn: JNI attach failed for onTunnelReady callback: {e}");
            return;
        }
    };

    let host_j = match env.new_string(host) {
        Ok(s) => s,
        Err(e) => {
            log::warn!("aivpn: JNI new_string failed for onTunnelReady callback: {e}");
            return;
        }
    };

    let host_obj = jni::objects::JObject::from(host_j);

    if let Err(e) = env.call_method(
        vpn_service,
        "onTunnelReady",
        "(Ljava/lang/String;)V",
        &[jni::objects::JValue::Object(&host_obj)],
    ) {
        log::warn!("aivpn: onTunnelReady callback failed: {e}");
        return;
    }

    match env.exception_check() {
        Ok(true) => {
            let _ = env.exception_describe();
            let _ = env.exception_clear();
            log::warn!("aivpn: onTunnelReady callback threw Java exception");
        }
        Ok(false) => {}
        Err(e) => {
            log::warn!("aivpn: exception_check failed after onTunnelReady callback: {e}");
        }
    }
}

// ──────────── Protected UDP socket creation ────────────

fn create_protected_udp_socket(
    vm: &JavaVM,
    vpn_service: &GlobalRef,
    dest: SocketAddr,
    session: &Arc<SessionRuntime>,
) -> Result<RawFd> {
    let fd = unsafe { libc::socket(libc::AF_INET, libc::SOCK_DGRAM, 0) };
    if fd < 0 {
        return Err(Error::Io(std::io::Error::last_os_error()));
    }

    // Call Android VpnService.protect(int) to exempt this socket from the VPN.
    let mut guard = vm
        .attach_current_thread()
        .map_err(|e| Error::Session(format!("JNI attach: {}", e)))?;

    let protected = guard
        .call_method(
            vpn_service,
            "protect",
            "(I)Z",
            &[jni::objects::JValue::Int(fd)],
        )
        .and_then(|v| v.z())
        .unwrap_or(false);

    if !protected {
        // Clear any pending JNI exception from protect() before returning
        // to avoid confusing subsequent JNI calls in the same thread.
        let _ = guard.exception_clear();
        unsafe { libc::close(fd) };
        return Err(Error::Session("VpnService.protect() returned false".into()));
    }

    // Increase OS socket buffers to reduce drops/backpressure on high-throughput links.
    // Ignore errors: kernels may cap/override values.
    let sock_buf: libc::c_int = 4 * 1024 * 1024;
    unsafe {
        let _ = libc::setsockopt(
            fd,
            libc::SOL_SOCKET,
            libc::SO_SNDBUF,
            &sock_buf as *const _ as *const libc::c_void,
            std::mem::size_of_val(&sock_buf) as libc::socklen_t,
        );
        let _ = libc::setsockopt(
            fd,
            libc::SOL_SOCKET,
            libc::SO_RCVBUF,
            &sock_buf as *const _ as *const libc::c_void,
            std::mem::size_of_val(&sock_buf) as libc::socklen_t,
        );
    }

    // Try to reuse the same local port as the previous session.  When a
    // port-preserving CGNAT (MTS, Beeline, etc.) is in use, the carrier's
    // inbound routing table still maps the old external port back to this
    // phone.  Binding to the same internal port means no CGNAT update is
    // needed and downlink arrives immediately without a stale-mapping delay.
    // Falls back to OS-assigned ephemeral port if the saved port is
    // unavailable (first connect, or port taken by another socket).
    let port_hint = LAST_LOCAL_PORT.load(Ordering::Relaxed);
    unsafe {
        let mut any: libc::sockaddr_in = std::mem::zeroed();
        any.sin_family = libc::AF_INET as libc::sa_family_t;
        if port_hint != 0 {
            any.sin_port = port_hint.to_be();
            if libc::bind(
                fd,
                &any as *const libc::sockaddr_in as *const libc::sockaddr,
                std::mem::size_of::<libc::sockaddr_in>() as libc::socklen_t,
            ) < 0
            {
                // Port unavailable — fall back to OS-assigned ephemeral.
                any.sin_port = 0;
                let _ = libc::bind(
                    fd,
                    &any as *const libc::sockaddr_in as *const libc::sockaddr,
                    std::mem::size_of::<libc::sockaddr_in>() as libc::socklen_t,
                );
            }
        } else {
            let _ = libc::bind(
                fd,
                &any as *const libc::sockaddr_in as *const libc::sockaddr,
                std::mem::size_of::<libc::sockaddr_in>() as libc::socklen_t,
            );
        }
    }

    // Connect to server (sets default destination for send/recv, non-blocking for UDP).
    let SocketAddr::V4(v4) = dest else {
        unsafe { libc::close(fd) };
        return Err(Error::Session(
            "Only IPv4 server addresses are supported".into(),
        ));
    };
    let sa = to_sockaddr_in(&v4);
    let rc = unsafe {
        libc::connect(
            fd,
            &sa as *const libc::sockaddr_in as *const libc::sockaddr,
            std::mem::size_of::<libc::sockaddr_in>() as libc::socklen_t,
        )
    };
    if rc < 0 {
        unsafe { libc::close(fd) };
        return Err(Error::Io(std::io::Error::last_os_error()));
    }

    // Persist the local port for the next reconnect attempt.
    unsafe {
        let mut sa: libc::sockaddr_in = std::mem::zeroed();
        let mut len = std::mem::size_of::<libc::sockaddr_in>() as libc::socklen_t;
        if libc::getsockname(
            fd,
            &mut sa as *mut libc::sockaddr_in as *mut libc::sockaddr,
            &mut len,
        ) == 0
        {
            LAST_LOCAL_PORT.store(u16::from_be(sa.sin_port), Ordering::Relaxed);
        }
    }

    let control_fd = unsafe { libc::dup(fd) };
    if control_fd < 0 {
        unsafe { libc::close(fd) };
        return Err(Error::Io(std::io::Error::last_os_error()));
    }

    session.udp_control_fd.store(control_fd, Ordering::SeqCst);

    Ok(fd)
}

#[cfg(any(target_os = "android", target_os = "linux"))]
fn create_stop_signal(session: &Arc<SessionRuntime>) -> Result<AsyncFd<OwnedFd>> {
    let stop_fd = unsafe { libc::eventfd(0, libc::EFD_NONBLOCK | libc::EFD_CLOEXEC) };
    if stop_fd < 0 {
        return Err(Error::Io(std::io::Error::last_os_error()));
    }

    let control_fd = unsafe { libc::dup(stop_fd) };
    if control_fd < 0 {
        unsafe { libc::close(stop_fd) };
        return Err(Error::Io(std::io::Error::last_os_error()));
    }

    session.stop_event_fd.store(control_fd, Ordering::SeqCst);

    // If stop_active_tunnel() fired in the race window between the last
    // stop_requested check and this function, the eventfd was never written
    // to (stop_event_fd was -1 at that point). Arm it now so the main loop
    // exits on its first poll instead of hanging forever.
    if session.stop_requested.load(Ordering::SeqCst) {
        let v: u64 = 1;
        unsafe {
            let _ = libc::write(
                stop_fd,
                &v as *const u64 as *const libc::c_void,
                std::mem::size_of::<u64>(),
            );
        }
    }

    let owned_stop_fd = unsafe { OwnedFd::from_raw_fd(stop_fd) };
    Ok(AsyncFd::new(owned_stop_fd)?)
}

#[cfg(not(any(target_os = "android", target_os = "linux")))]
fn create_stop_signal(session: &Arc<SessionRuntime>) -> Result<AsyncFd<OwnedFd>> {
    let mut fds = [0i32; 2];
    if unsafe { libc::pipe(fds.as_mut_ptr()) } < 0 {
        return Err(Error::Io(std::io::Error::last_os_error()));
    }
    let (read_fd, write_fd) = (fds[0], fds[1]);
    unsafe { libc::fcntl(read_fd, libc::F_SETFL, libc::O_NONBLOCK) };
    let dup_write = unsafe { libc::dup(write_fd) };
    if dup_write < 0 {
        unsafe {
            libc::close(read_fd);
            libc::close(write_fd);
        }
        return Err(Error::Io(std::io::Error::last_os_error()));
    }
    session.stop_event_fd.store(dup_write, Ordering::SeqCst);
    unsafe { libc::close(write_fd) };
    Ok(AsyncFd::new(unsafe { OwnedFd::from_raw_fd(read_fd) })?)
}

#[cfg(any(target_os = "android", target_os = "linux"))]
async fn wait_for_stop_signal(stop_signal: &AsyncFd<OwnedFd>) -> std::io::Result<()> {
    loop {
        let mut guard = stop_signal.readable().await?;
        match guard.try_io(|inner| {
            let mut value: u64 = 0;
            let n = unsafe {
                libc::read(
                    inner.as_raw_fd(),
                    &mut value as *mut u64 as *mut libc::c_void,
                    std::mem::size_of::<u64>(),
                )
            };
            if n < 0 {
                Err(std::io::Error::last_os_error())
            } else {
                Ok(())
            }
        }) {
            Ok(r) => return r,
            Err(_would_block) => continue,
        }
    }
}

#[cfg(not(any(target_os = "android", target_os = "linux")))]
async fn wait_for_stop_signal(stop_signal: &AsyncFd<OwnedFd>) -> std::io::Result<()> {
    loop {
        let mut guard = stop_signal.readable().await?;
        match guard.try_io(|inner| {
            let mut b = [0u8; 1];
            let n =
                unsafe { libc::read(inner.as_raw_fd(), b.as_mut_ptr() as *mut libc::c_void, 1) };
            if n < 0 {
                Err(std::io::Error::last_os_error())
            } else {
                Ok(())
            }
        }) {
            Ok(r) => return r,
            Err(_would_block) => continue,
        }
    }
}

fn to_sockaddr_in(addr: &SocketAddrV4) -> libc::sockaddr_in {
    libc::sockaddr_in {
        #[cfg(any(
            target_os = "macos",
            target_os = "ios",
            target_os = "freebsd",
            target_os = "openbsd",
            target_os = "netbsd",
            target_os = "dragonfly"
        ))]
        sin_len: std::mem::size_of::<libc::sockaddr_in>() as u8,
        sin_family: libc::AF_INET as libc::sa_family_t,
        sin_port: addr.port().to_be(),
        sin_addr: libc::in_addr {
            s_addr: u32::from_ne_bytes(addr.ip().octets()),
        },
        sin_zero: [0; 8],
    }
}

// ──────────── Async TUN I/O ────────────

async fn tun_async_read(tun: &AsyncFd<OwnedFd>, buf: &mut [u8]) -> std::io::Result<usize> {
    loop {
        let mut guard = tun.readable().await?;
        match guard.try_io(|inner| {
            let n = unsafe {
                libc::read(
                    inner.as_raw_fd(),
                    buf.as_mut_ptr() as *mut libc::c_void,
                    buf.len(),
                )
            };
            if n < 0 {
                Err(std::io::Error::last_os_error())
            } else {
                Ok(n as usize)
            }
        }) {
            Ok(r) => return r,
            Err(_would_block) => continue,
        }
    }
}

async fn tun_async_write(tun: &AsyncFd<OwnedFd>, data: &[u8]) -> std::io::Result<()> {
    let mut written = 0usize;
    while written < data.len() {
        let mut guard = tun.writable().await?;
        match guard.try_io(|inner| {
            let n = unsafe {
                libc::write(
                    inner.as_raw_fd(),
                    data[written..].as_ptr() as *const libc::c_void,
                    data.len() - written,
                )
            };

            if n < 0 {
                let err = std::io::Error::last_os_error();
                if err.kind() == std::io::ErrorKind::WouldBlock {
                    Err(std::io::Error::from(std::io::ErrorKind::WouldBlock))
                } else {
                    Err(err)
                }
            } else {
                Ok(n as usize)
            }
        }) {
            Ok(Ok(0)) => {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::WriteZero,
                    "TUN write returned 0",
                ));
            }
            Ok(Ok(n)) => {
                written += n;
            }
            Ok(Err(e)) => {
                return Err(e);
            }
            Err(_would_block) => continue,
        }
    }
    Ok(())
}
