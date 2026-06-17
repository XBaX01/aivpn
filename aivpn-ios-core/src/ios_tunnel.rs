//! iOS VPN tunnel — runs on top of an AF_UNIX SOCK_DGRAM socketpair fd passed from
//! the NEPacketTunnelProvider extension. The protocol is byte-for-byte identical to the
//! Android and macOS clients; only the TUN I/O and stop-signal mechanisms differ.
//!
//! Key differences from android_tunnel.rs:
//!  - No JNI: protect() is unnecessary (NEPacketTunnelProvider is automatically outside VPN)
//!  - Stop signal uses pipe() instead of eventfd() (not available on iOS/macOS)
//!  - on_ready notification via C callback instead of JNI method call

#![allow(clippy::too_many_arguments)]

use std::ffi::CString;
use std::net::{SocketAddr, SocketAddrV4};
use std::os::fd::OwnedFd;
use std::os::unix::io::{AsRawFd, FromRawFd, RawFd};
use std::sync::atomic::{AtomicI32, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use tokio::io::unix::AsyncFd;
use tokio::net::UdpSocket;
use tokio::sync::mpsc;
use tokio::time;

use aivpn_common::client_wire::{
    build_inner_packet, build_random_mdh_packet, decode_packet_with_mdh_len,
    obfuscate_client_eph_pub, process_server_hello_with_mdh_len, RecvWindow, DEFAULT_MDH_LEN,
};
use aivpn_common::crypto::{derive_session_keys, KeyPair, SessionKeys};
use aivpn_common::error::{Error, Result};
use aivpn_common::protocol::{ControlPayload, InnerType};
use aivpn_common::upload_pipeline::{self, PacketEncryptor, UploadConfig, ZeroMdhEncryptor};

// ──────────── Constants (identical to android_tunnel.rs) ────────────

const BUF_SIZE: usize = 1500;
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(10);
const HANDSHAKE_RETRY_INTERVAL: Duration = Duration::from_millis(750);
const KEEPALIVE_INTERVAL: Duration = Duration::from_secs(15);
const RX_SILENCE: Duration = Duration::from_secs(120);
const RX_CHECK_INTERVAL: Duration = Duration::from_secs(2);
const TX_WITHOUT_RX_TIMEOUT: Duration = Duration::from_secs(20);
const TX_WITHOUT_RX_MIN_BYTES: u64 = 64 * 1024;
const REKEY_INTERVAL: Duration = Duration::from_secs(1800);
const CHANNEL_SIZE: usize = 8192;

// ──────────── Session runtime ────────────

pub struct SessionRuntime {
    udp_control_fd: AtomicI32,
    stop_pipe_write: AtomicI32,
    upload_bytes: AtomicU64,
    download_bytes: AtomicU64,
}

impl SessionRuntime {
    fn new() -> Self {
        Self {
            udp_control_fd: AtomicI32::new(-1),
            stop_pipe_write: AtomicI32::new(-1),
            upload_bytes: AtomicU64::new(0),
            download_bytes: AtomicU64::new(0),
        }
    }
}

static ACTIVE_SESSION: Mutex<Option<Arc<SessionRuntime>>> = Mutex::new(None);

struct ActiveSessionGuard {
    session: Arc<SessionRuntime>,
}

impl Drop for ActiveSessionGuard {
    fn drop(&mut self) {
        let udp_fd = self.session.udp_control_fd.swap(-1, Ordering::SeqCst);
        if udp_fd >= 0 {
            unsafe { libc::close(udp_fd) };
        }
        let pipe_write = self.session.stop_pipe_write.swap(-1, Ordering::SeqCst);
        if pipe_write >= 0 {
            unsafe { libc::close(pipe_write) };
        }
        if let Ok(mut guard) = ACTIVE_SESSION.lock() {
            if let Some(current) = guard.as_ref() {
                if Arc::ptr_eq(current, &self.session) {
                    *guard = None;
                }
            }
        }
    }
}

fn activate_session(session: Arc<SessionRuntime>) -> Result<ActiveSessionGuard> {
    let mut guard = ACTIVE_SESSION
        .lock()
        .map_err(|_| Error::Session("Session lock poisoned".into()))?;
    if guard.is_some() {
        return Err(Error::Session(
            "Another iOS tunnel session is already active".into(),
        ));
    }
    *guard = Some(session.clone());
    Ok(ActiveSessionGuard { session })
}

pub fn stop_active_tunnel() {
    let (udp_fd, pipe_write) = ACTIVE_SESSION
        .lock()
        .ok()
        .and_then(|guard| {
            guard.as_ref().map(|s| {
                (
                    s.udp_control_fd.swap(-1, Ordering::SeqCst),
                    s.stop_pipe_write.load(Ordering::SeqCst),
                )
            })
        })
        .unwrap_or((-1, -1));

    if pipe_write >= 0 {
        let v: u8 = 1;
        unsafe { libc::write(pipe_write, &v as *const u8 as *const libc::c_void, 1) };
    }
    if udp_fd >= 0 {
        unsafe {
            libc::shutdown(udp_fd, libc::SHUT_RDWR);
            libc::close(udp_fd);
        }
    }
}

pub fn get_active_upload_bytes() -> u64 {
    ACTIVE_SESSION
        .lock()
        .ok()
        .and_then(|g| g.as_ref().map(|s| s.upload_bytes.load(Ordering::Relaxed)))
        .unwrap_or(0)
}

pub fn get_active_download_bytes() -> u64 {
    ACTIVE_SESSION
        .lock()
        .ok()
        .and_then(|g| g.as_ref().map(|s| s.download_bytes.load(Ordering::Relaxed)))
        .unwrap_or(0)
}

// ──────────── C callback type ────────────

pub type OnReadyFn = unsafe extern "C" fn(host: *const libc::c_char, ctx: *mut libc::c_void);

// Wrap the raw ctx pointer so the Future can be Send.
pub struct SendCtx(pub *mut libc::c_void);
unsafe impl Send for SendCtx {}

// ──────────── Entry point ────────────

pub async fn run_tunnel_ios(
    tun_fd: RawFd,
    server_host: String,
    server_port: u16,
    server_key: [u8; 32],
    psk: Option<[u8; 32]>,
    mtls_cert: Option<Vec<u8>>,
    on_ready: Option<OnReadyFn>,
    ctx: SendCtx,
) -> Result<()> {
    let session = Arc::new(SessionRuntime::new());
    let _guard = activate_session(session.clone())?;

    // 1. Ephemeral keypair + Zero-RTT session keys
    let mut keypair = KeyPair::generate();
    let mut dh = keypair.compute_shared(&server_key)?;
    let mut keys = derive_session_keys(&dh, psk.as_ref(), &keypair.public_key_bytes());

    // 2. UDP socket — no protect() needed: extension runs outside VPN routing
    let dest_str = format!("{}:{}", server_host, server_port);
    let dest: SocketAddr = tokio::net::lookup_host(&dest_str)
        .await
        .map_err(Error::Io)?
        .find(|a| a.is_ipv4())
        .ok_or_else(|| Error::Session("Cannot resolve server host to IPv4".into()))?;

    let raw_udp_fd = create_udp_socket(dest, &session)?;
    let stop_signal = create_stop_signal(&session)?;

    // 3. TUN fd (socketpair end; Swift bridges packetFlow <-> this fd)
    let owned_tun_fd = unsafe { libc::dup(tun_fd) };
    if owned_tun_fd < 0 {
        return Err(Error::Io(std::io::Error::last_os_error()));
    }
    unsafe { libc::fcntl(owned_tun_fd, libc::F_SETFL, libc::O_NONBLOCK) };
    let owned_tun = unsafe { OwnedFd::from_raw_fd(owned_tun_fd) };
    let tun = AsyncFd::new(owned_tun)?;

    let std_udp = unsafe { std::net::UdpSocket::from_raw_fd(raw_udp_fd) };
    std_udp.set_nonblocking(true)?;
    let udp = Arc::new(UdpSocket::from_std(std_udp)?);

    // 4. Send init handshake
    let mdh_len = DEFAULT_MDH_LEN;
    let mut send_counter: u64 = 0;
    let mut send_seq: u16 = 0;
    let keepalive = ControlPayload::Keepalive.encode()?;
    {
        let obf_pub = obfuscate_client_eph_pub(&keypair, &server_key);
        let inner = build_inner_packet(InnerType::Control, send_seq, &keepalive);
        let pkt =
            build_random_mdh_packet(&keys, &mut send_counter, &inner, Some(&obf_pub), mdh_len)?;
        send_seq = send_seq.wrapping_add(1);
        udp.send(&pkt).await?;
    }

    // 5. Wait for ServerHello
    let mut recv_buf = vec![0u8; BUF_SIZE];
    let deadline = Instant::now() + HANDSHAKE_TIMEOUT;
    let n = loop {
        let now = Instant::now();
        if now >= deadline {
            return Err(Error::Session("Handshake timeout (10 s)".into()));
        }
        let wait = std::cmp::min(
            HANDSHAKE_RETRY_INTERVAL,
            deadline.saturating_duration_since(now),
        );
        let retry = time::sleep(wait);
        tokio::pin!(retry);
        tokio::select! {
            _ = wait_for_stop(&stop_signal) => {
                return Err(Error::Session("Tunnel stop requested".into()));
            }
            r = udp.recv(&mut recv_buf) => { break r?; }
            _ = &mut retry => {
                // Fresh keypair on retry: reusing the same keypair causes retry
                // packets to match an existing server session and be treated as
                // keepalives — the server never sends a new ServerHello.
                keypair = KeyPair::generate();
                dh = keypair.compute_shared(&server_key)?;
                keys = derive_session_keys(&dh, psk.as_ref(), &keypair.public_key_bytes());
                send_counter = 0;
                send_seq = 0;
                let obf_pub = obfuscate_client_eph_pub(&keypair, &server_key);
                let inner = build_inner_packet(InnerType::Control, send_seq, &keepalive);
                let pkt = build_random_mdh_packet(&keys, &mut send_counter, &inner, Some(&obf_pub), mdh_len)?;
                send_seq = send_seq.wrapping_add(1);
                udp.send(&pkt).await?;
            }
        }
    };

    let mut recv_win = RecvWindow::new();
    process_server_hello_with_mdh_len(
        &recv_buf[..n],
        &mut keys,
        &keypair,
        &mut recv_win,
        &mut send_counter,
        mdh_len,
    )?;
    let mut tr_keys: Option<SessionKeys> = Some(derive_session_keys(
        &dh,
        psk.as_ref(),
        &keypair.public_key_bytes(),
    ));
    let mut tr_deadline = Some(Instant::now() + Duration::from_secs(2));
    let mut tr_win = std::mem::take(&mut recv_win);

    if let Some(cert) = mtls_cert {
        let cert_len_debug = cert.len();
        let cert_payload = ControlPayload::ClientCert { cert_bytes: cert }.encode()?;
        let inner = build_inner_packet(InnerType::Control, send_seq, &cert_payload);
        let pkt = build_random_mdh_packet(&keys, &mut send_counter, &inner, None, mdh_len)?;
        send_seq = send_seq.wrapping_add(1);
        udp.send(&pkt).await?;
        log::debug!("mTLS: ClientCert sent ({} bytes)", cert_len_debug);
    }

    // Notify tunnel ready via C callback (after ClientCert so app UI opens after auth)
    if let Some(cb) = on_ready {
        if let Ok(c_host) = CString::new(server_host.as_str()) {
            unsafe { cb(c_host.as_ptr(), ctx.0) };
        }
    }

    // 6. Main forwarding loop
    let mut udp_buf = vec![0u8; BUF_SIZE];
    let mut last_rx = Instant::now();
    let mut upload_at_last_rx = 0u64;

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

    let tun_reader = tokio::spawn(async move {
        let mut buf = vec![0u8; BUF_SIZE];
        loop {
            match tun_async_read(&tun_read, &mut buf).await {
                Ok(0) => continue,
                Ok(n) => {
                    if buf[0] >> 4 != 4 {
                        continue;
                    } // IPv4 only
                    if tun_tx.send(buf[..n].to_vec()).await.is_err() {
                        break;
                    }
                }
                Err(e) => {
                    let _ = tun_err_tx.send(format!("TUN read: {e}")).await;
                    break;
                }
            }
        }
    });

    let udp_tx = udp.clone();
    let keys_tx = keys.clone();
    let session_up = session.clone();
    let upload_task = tokio::spawn(async move {
        struct IosEncryptor {
            inner: ZeroMdhEncryptor,
            session: Arc<SessionRuntime>,
        }
        impl PacketEncryptor for IosEncryptor {
            fn encrypt_data(&mut self, p: &[u8]) -> aivpn_common::error::Result<Vec<u8>> {
                self.inner.encrypt_data(p)
            }
            fn encrypt_control(
                &mut self,
                p: &ControlPayload,
            ) -> aivpn_common::error::Result<Vec<u8>> {
                self.inner.encrypt_control(p)
            }
            fn encrypt_keepalive(&mut self) -> aivpn_common::error::Result<Vec<u8>> {
                self.inner.encrypt_keepalive()
            }
            fn on_data_sent(&mut self, len: usize) {
                self.session
                    .upload_bytes
                    .fetch_add(len as u64, Ordering::Relaxed);
            }
        }
        let mut enc = IosEncryptor {
            inner: ZeroMdhEncryptor::with_mdh_len(keys_tx, send_counter, send_seq, mdh_len),
            session: session_up,
        };
        let cfg = UploadConfig {
            keepalive_interval: KEEPALIVE_INTERVAL,
            ..Default::default()
        };
        if let Err(e) =
            upload_pipeline::run_upload_loop(&mut tun_rx, None, &udp_tx, &mut enc, &cfg).await
        {
            let _ = sender_err_tx.send(format!("Upload: {e}")).await;
        }
    });

    let rekey = time::sleep(REKEY_INTERVAL);
    tokio::pin!(rekey);
    let mut rx_check = time::interval(RX_CHECK_INTERVAL);
    rx_check.set_missed_tick_behavior(time::MissedTickBehavior::Delay);

    loop {
        tokio::select! {
            biased;

            _ = wait_for_stop(&stop_signal) => {
                // Send Shutdown before closing so the server drops the session
                // immediately instead of waiting for the 30-second ghost timeout.
                if let Ok(shutdown_bytes) = ControlPayload::Shutdown { reason: 0 }.encode() {
                    let inner = build_inner_packet(InnerType::Control, send_seq, &shutdown_bytes);
                    if let Ok(pkt) = build_random_mdh_packet(&keys, &mut send_counter, &inner, None, mdh_len) {
                        let _ = udp.send(&pkt).await;
                    }
                }
                tun_reader.abort(); upload_task.abort();
                return Err(Error::Session("Stop requested".into()));
            }

            _ = &mut rekey => {
                tun_reader.abort(); upload_task.abort();
                return Ok(());
            }

            r = udp.recv(&mut udp_buf) => {
                let n = r?;
                last_rx = Instant::now();
                upload_at_last_rx = session.upload_bytes.load(Ordering::Relaxed);
                if tr_deadline.is_some_and(|d| Instant::now() >= d) {
                    tr_keys = None;
                    tr_deadline = None;
                    tr_win.reset();
                }
                let decoded = decode_packet_with_mdh_len(&udp_buf[..n], &keys, &mut recv_win, mdh_len).ok()
                    .or_else(|| tr_keys.as_ref().and_then(|tk|
                        decode_packet_with_mdh_len(&udp_buf[..n], tk, &mut tr_win, mdh_len).ok()));
                if let Some(d) = decoded {
                    if d.header.inner_type == InnerType::Data && !d.payload.is_empty() {
                        tun_async_write(&tun, &d.payload).await?;
                        session.download_bytes.fetch_add(d.payload.len() as u64, Ordering::Relaxed);
                    }
                }
            }

            maybe_err = err_rx.recv() => {
                if let Some(msg) = maybe_err {
                    tun_reader.abort(); upload_task.abort();
                    return Err(Error::Session(msg));
                }
            }

            _ = rx_check.tick() => {
                let silence = last_rx.elapsed();
                let uploaded = session.upload_bytes.load(Ordering::Relaxed);
                let since_rx = uploaded.saturating_sub(upload_at_last_rx);
                if silence > TX_WITHOUT_RX_TIMEOUT && since_rx >= TX_WITHOUT_RX_MIN_BYTES {
                    tun_reader.abort(); upload_task.abort();
                    return Err(Error::Session(
                        format!("TX without RX: {since_rx} bytes in {silence:?} — reconnecting")
                    ));
                }
                if silence > RX_SILENCE {
                    tun_reader.abort(); upload_task.abort();
                    return Err(Error::Session(format!("No RX for {silence:?} — reconnecting")));
                }
            }
        }
    }
}

// ──────────── Helpers ────────────

fn create_udp_socket(dest: SocketAddr, session: &Arc<SessionRuntime>) -> Result<RawFd> {
    let fd = unsafe { libc::socket(libc::AF_INET, libc::SOCK_DGRAM, 0) };
    if fd < 0 {
        return Err(Error::Io(std::io::Error::last_os_error()));
    }
    let buf: libc::c_int = 4 * 1024 * 1024;
    unsafe {
        libc::setsockopt(
            fd,
            libc::SOL_SOCKET,
            libc::SO_SNDBUF,
            &buf as *const _ as *const libc::c_void,
            std::mem::size_of_val(&buf) as libc::socklen_t,
        );
        libc::setsockopt(
            fd,
            libc::SOL_SOCKET,
            libc::SO_RCVBUF,
            &buf as *const _ as *const libc::c_void,
            std::mem::size_of_val(&buf) as libc::socklen_t,
        );
    }
    let SocketAddr::V4(v4) = dest else {
        unsafe { libc::close(fd) };
        return Err(Error::Session(
            "Only IPv4 server addresses are supported".into(),
        ));
    };
    let sa = to_sockaddr_in(&v4);
    if unsafe {
        libc::connect(
            fd,
            &sa as *const libc::sockaddr_in as *const libc::sockaddr,
            std::mem::size_of::<libc::sockaddr_in>() as libc::socklen_t,
        )
    } < 0
    {
        unsafe { libc::close(fd) };
        return Err(Error::Io(std::io::Error::last_os_error()));
    }
    let dup_fd = unsafe { libc::dup(fd) };
    if dup_fd < 0 {
        unsafe { libc::close(fd) };
        return Err(Error::Io(std::io::Error::last_os_error()));
    }
    session.udp_control_fd.store(dup_fd, Ordering::SeqCst);
    Ok(fd)
}

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
            libc::close(write_fd)
        };
        return Err(Error::Io(std::io::Error::last_os_error()));
    }
    session.stop_pipe_write.store(dup_write, Ordering::SeqCst);
    unsafe { libc::close(write_fd) };
    Ok(AsyncFd::new(unsafe { OwnedFd::from_raw_fd(read_fd) })?)
}

async fn wait_for_stop(sig: &AsyncFd<OwnedFd>) -> std::io::Result<()> {
    loop {
        let mut guard = sig.readable().await?;
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
            Err(_) => continue,
        }
    }
}

fn to_sockaddr_in(addr: &SocketAddrV4) -> libc::sockaddr_in {
    libc::sockaddr_in {
        sin_len: std::mem::size_of::<libc::sockaddr_in>() as u8,
        sin_family: libc::AF_INET as libc::sa_family_t,
        sin_port: addr.port().to_be(),
        sin_addr: libc::in_addr {
            s_addr: u32::from_ne_bytes(addr.ip().octets()),
        },
        sin_zero: [0; 8],
    }
}

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
            Err(_) => continue,
        }
    }
}

async fn tun_async_write(tun: &AsyncFd<OwnedFd>, data: &[u8]) -> std::io::Result<()> {
    let mut written = 0;
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
                let e = std::io::Error::last_os_error();
                if e.kind() == std::io::ErrorKind::WouldBlock {
                    Err(std::io::Error::from(std::io::ErrorKind::WouldBlock))
                } else {
                    Err(e)
                }
            } else {
                Ok(n as usize)
            }
        }) {
            Ok(Ok(0)) => {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::WriteZero,
                    "write 0",
                ))
            }
            Ok(Ok(n)) => written += n,
            Ok(Err(e)) => return Err(e),
            Err(_) => continue,
        }
    }
    Ok(())
}
