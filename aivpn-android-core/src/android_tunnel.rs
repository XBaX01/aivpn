//! Android VPN tunnel — runs on top of a TUN fd created by VpnService.Builder and a UDP
//! socket created here and exempted via VpnService.protect(int).
//!
//! Wire protocol is byte-for-byte identical to AivpnCrypto.kt so that both can talk to the
//! same Rust server without any server-side changes.

use std::net::{SocketAddr, SocketAddrV4};
use std::os::fd::OwnedFd;
use std::os::unix::io::{AsRawFd, FromRawFd, RawFd};
use std::sync::atomic::{AtomicI32, AtomicU64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use jni::objects::GlobalRef;
use jni::JavaVM;
use tokio::io::unix::AsyncFd;
use tokio::net::UdpSocket;
use tokio::time;

use aivpn_common::client_wire::{
    build_inner_packet, build_zero_mdh_packet, decode_packet_with_mdh_len,
    obfuscate_client_eph_pub, process_server_hello_with_mdh_len, RecvWindow, DEFAULT_ZERO_MDH,
};
use aivpn_common::crypto::{
    derive_session_keys, KeyPair,
};
use aivpn_common::error::{Error, Result};
use aivpn_common::protocol::{ControlPayload, InnerType};

// ──────────── Constants ────────────

const BUF_SIZE: usize = 1500;
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(10);
const KEEPALIVE_INTERVAL: Duration = Duration::from_secs(25);
const RX_SILENCE_MS: u64 = 120_000; // 2 min: detect dead NAT before keepalive masks it
const REKEY_INTERVAL: Duration = Duration::from_secs(1800); // 30 min

// ──────────── Public globals (read by JNI exports in lib.rs) ────────────

pub static TUNNEL_UDP_FD: AtomicI32 = AtomicI32::new(-1);
pub static UPLOAD_BYTES: AtomicU64 = AtomicU64::new(0);
pub static DOWNLOAD_BYTES: AtomicU64 = AtomicU64::new(0);

// ──────────── Entry point ────────────

/// Blocking async function that runs the whole tunnel session.
/// Returns Ok(()) only on REKEY_INTERVAL expiry (clean reconnect trigger).
/// All errors cause the Kotlin reconnect loop to kick in.
pub async fn run_tunnel_android(
    vm: JavaVM,
    vpn_service: GlobalRef,
    tun_fd_int: RawFd,
    server_host: String,
    server_port: u16,
    server_key: [u8; 32],
    psk: Option<[u8; 32]>,
) -> Result<()> {
    // Reset per-session counters.
    UPLOAD_BYTES.store(0, Ordering::Relaxed);
    DOWNLOAD_BYTES.store(0, Ordering::Relaxed);

    // ── 1. Ephemeral keypair + initial session keys (Zero-RTT like existing Kotlin) ──
    let keypair = KeyPair::generate();
    let dh = keypair.compute_shared(&server_key)?;
    let mut keys = derive_session_keys(&dh, psk.as_ref(), &keypair.public_key_bytes());

    // ── 2. Create and protect UDP socket ──
    // Resolve host (async DNS so we don't block the tokio thread).
    let dest_str = format!("{}:{}", server_host, server_port);
    let dest: SocketAddr = tokio::net::lookup_host(&dest_str)
        .await
        .map_err(|e| Error::Io(e))?
        .find(|a| a.is_ipv4())
        .ok_or_else(|| Error::Session("Cannot resolve server host to IPv4".into()))?;

    let raw_udp_fd = create_protected_udp_socket(&vm, &vpn_service, dest)?;
    TUNNEL_UDP_FD.store(raw_udp_fd, Ordering::SeqCst);

    // ── 3. Set TUN fd to non-blocking for AsyncFd ──
    unsafe { libc::fcntl(tun_fd_int, libc::F_SETFL, libc::O_NONBLOCK) };
    // SAFETY: we own this fd (Kotlin called detachFd()).
    let owned_tun = unsafe { OwnedFd::from_raw_fd(tun_fd_int) };
    let tun = AsyncFd::new(owned_tun)?;

    // Convert the raw UDP fd to a tokio UdpSocket (already connected to server).
    let std_udp = unsafe { std::net::UdpSocket::from_raw_fd(raw_udp_fd) };
    std_udp.set_nonblocking(true)?;
    let udp = UdpSocket::from_std(std_udp)?;

    // ── 4. Send init handshake (Control/Keepalive + obfuscated eph_pub) ──
    let mut send_counter: u64 = 0;
    let mut send_seq: u16 = 0;
    {
        let keepalive = ControlPayload::Keepalive.encode()?;
        let inner = build_inner_packet(InnerType::Control, send_seq, &keepalive);
        send_seq = send_seq.wrapping_add(1);
        let obf_pub = obfuscate_client_eph_pub(&keypair, &server_key);
        let pkt = build_zero_mdh_packet(&keys, &mut send_counter, &inner, Some(&obf_pub))?;
        udp.send(&pkt).await?;
    }

    // ── 5. Wait for ServerHello with timeout ──
    let mut recv_buf = vec![0u8; BUF_SIZE];
    let n = time::timeout(HANDSHAKE_TIMEOUT, udp.recv(&mut recv_buf))
        .await
        .map_err(|_| Error::Session("Handshake timeout (10 s)".into()))??;

    let mut recv_win = RecvWindow::new();
    process_server_hello_with_mdh_len(
        &recv_buf[..n],
        &mut keys,
        &keypair,
        &mut recv_win,
        &mut send_counter,
        DEFAULT_ZERO_MDH.len(),
    )?;
    log::info!("aivpn: handshake + PFS ratchet complete");

    // ── 6. Main forwarding loop ──
    let mut tun_buf = vec![0u8; BUF_SIZE];
    let mut udp_buf = vec![0u8; BUF_SIZE];
    let mut last_rx_ms = monotonic_ms();
    let rekey_sleep = time::sleep(REKEY_INTERVAL);
    tokio::pin!(rekey_sleep);
    let mut ka_interval = time::interval(KEEPALIVE_INTERVAL);
    ka_interval.tick().await; // discard immediate first tick

    loop {
        tokio::select! {
            biased;

            // ── Rekey (triggers fresh reconnect in Kotlin) ──
            _ = &mut rekey_sleep => {
                log::info!("aivpn: rekey interval — signalling reconnect");
                return Ok(());
            }

            // ── TUN → UDP (outbound IP packets) ──
            r = tun_async_read(&tun, &mut tun_buf) => {
                let n = r?;
                if n == 0 { continue; }
                // Drop non-IPv4 packets (IPv6 version nibble = 6, first byte 0x60-0x6F).
                // Android routes ::/0 into TUN to prevent IPv6 leaks; we must discard
                // those packets here because the server only speaks IPv4.
                if tun_buf[0] >> 4 != 4 { continue; }
                let inner = build_inner_packet(InnerType::Data, send_seq, &tun_buf[..n]);
                send_seq = send_seq.wrapping_add(1);
                let pkt = build_zero_mdh_packet(&keys, &mut send_counter, &inner, None)?;
                udp.send(&pkt).await?;
                UPLOAD_BYTES.fetch_add(n as u64, Ordering::Relaxed);
            }

            // ── UDP → TUN (inbound from server) ──
            r = udp.recv(&mut udp_buf) => {
                let n = r?;
                last_rx_ms = monotonic_ms();
                if let Ok(decoded) = decode_packet_with_mdh_len(
                    &udp_buf[..n],
                    &keys,
                    &mut recv_win,
                    DEFAULT_ZERO_MDH.len(),
                ) {
                    if decoded.header.inner_type == InnerType::Data && !decoded.payload.is_empty() {
                        tun_write(&tun, &decoded.payload)?;
                        DOWNLOAD_BYTES.fetch_add(decoded.payload.len() as u64, Ordering::Relaxed);
                    }
                }
            }

            // ── Keepalive + RX-silence check ──
            _ = ka_interval.tick() => {
                let silence = monotonic_ms().saturating_sub(last_rx_ms);
                if silence > RX_SILENCE_MS {
                    return Err(Error::Session(
                        format!("No RX for {}ms — reconnecting", silence)
                    ));
                }
                let keepalive = ControlPayload::Keepalive.encode()?;
                let inner = build_inner_packet(InnerType::Control, send_seq, &keepalive);
                send_seq = send_seq.wrapping_add(1);
                let pkt = build_zero_mdh_packet(&keys, &mut send_counter, &inner, None)?;
                udp.send(&pkt).await?;
            }
        }
    }
}

// ──────────── Protected UDP socket creation ────────────

fn create_protected_udp_socket(
    vm: &JavaVM,
    vpn_service: &GlobalRef,
    dest: SocketAddr,
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
        unsafe { libc::close(fd) };
        return Err(Error::Session("VpnService.protect() returned false".into()));
    }

    // Connect to server (sets default destination for send/recv, non-blocking for UDP).
    let SocketAddr::V4(v4) = dest else {
        unsafe { libc::close(fd) };
        return Err(Error::Session("Only IPv4 server addresses are supported".into()));
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

    Ok(fd)
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

fn tun_write(tun: &AsyncFd<OwnedFd>, data: &[u8]) -> std::io::Result<()> {
    // TUN writes are rare and small; a blocking write is fine here.
    let n = unsafe {
        libc::write(
            tun.as_raw_fd(),
            data.as_ptr() as *const libc::c_void,
            data.len(),
        )
    };
    if n < 0 {
        Err(std::io::Error::last_os_error())
    } else {
        Ok(())
    }
}

fn monotonic_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}
