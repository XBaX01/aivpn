//! AIVPN Client - Full Implementation
//!
//! Complete VPN client with:
//! - Real TUN device integration
//! - Mimicry Engine for traffic shaping
//! - Key exchange and session management
//! - Control plane handling

use bytes::Bytes;
use portable_atomic::AtomicU64;
use std::collections::VecDeque;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::sync::Mutex;
use std::time::{Duration, Instant};
use tokio::io::AsyncReadExt;
use tokio::net::UdpSocket;
use tokio::sync::mpsc;
use tracing::{debug, error, info, warn};

use aivpn_common::client_wire::{
    build_inner_packet, decode_packet_with_mdh_len, obfuscate_client_eph_pub, DecodedPacket,
    RecvWindow,
};
use aivpn_common::crypto::{self, KeyPair, SessionKeys, X25519_PUBLIC_KEY_SIZE};
use aivpn_common::error::{Error, Result};
use aivpn_common::mask::{BootstrapDescriptor, MaskProfile};
use aivpn_common::network_config::{ClientNetworkConfig, DEFAULT_KEEPALIVE_SECS};
use aivpn_common::protocol::{ControlPayload, InnerType, MAX_PACKET_SIZE};
use aivpn_common::quality::{AdaptiveLevel, QualityTracker};
use aivpn_common::upload_pipeline::{self, PacketEncryptor, UploadConfig};

use crate::bootstrap_cache;
use crate::tunnel::{Tunnel, TunnelConfig};
#[cfg(target_os = "linux")]
use aivpn_common::kernel_accel::{xdp_attach, xdp_default_iface, xdp_detach, KernelAccel};
use aivpn_common::mimicry::MimicryEngine;
#[cfg(target_os = "linux")]
use libc;

/// RAII guard that aborts a spawned task when dropped.
/// Used to ensure the admin IPC socket task is cancelled when run() returns,
/// so the next reconnect iteration can bind 127.0.0.1:44301 without
/// "Address already in use".
struct AbortOnDrop(tokio::task::JoinHandle<()>);
impl Drop for AbortOnDrop {
    fn drop(&mut self) {
        self.0.abort();
    }
}

fn packet_mdh_len_for_mask(mask: &MaskProfile) -> usize {
    mask.header_spec
        .as_ref()
        .map(|spec| spec.min_length())
        .unwrap_or_else(|| mask.header_template.len())
}

/// Client configuration
#[derive(Debug, Clone)]
pub struct ClientConfig {
    pub server_addr: String,
    pub server_public_key: [u8; X25519_PUBLIC_KEY_SIZE],
    /// Ed25519 signing public key for verifying ServerHello signatures and mask updates.
    /// When `Some`, the client rejects unsigned or incorrectly signed messages from
    /// the server, preventing MITM attacks.
    pub server_signing_key: Option<[u8; 32]>,
    pub preshared_key: Option<[u8; 32]>,
    pub initial_mask: MaskProfile,
    pub tun_config: TunnelConfig,
    /// When set, run as SOCKS5 proxy on this address instead of a TUN device.
    pub proxy_listen: Option<std::net::SocketAddr>,
    /// Optional 104-byte mTLS certificate sent to the server after session setup.
    /// Required when the server is configured with `mtls.required = true`.
    pub mtls_cert: Option<Vec<u8>>,
}

/// Client state
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClientState {
    Unprovisioned,
    Provisioned,
    Connecting,
    Connected,
    Reconnecting,
    Disconnected,
}

struct UploadCryptoState {
    keys: SessionKeys,
    counter: u64,
    seq: u16,
}

/// AIVPN Client instance
pub struct AivpnClient {
    config: ClientConfig,
    state: ClientState,
    tunnel: Tunnel,
    udp_socket: Option<Arc<UdpSocket>>,
    mimicry_engine: Option<MimicryEngine>,
    pub control_tx: Option<mpsc::Sender<ControlPayload>>,
    pending_mask: Arc<Mutex<Option<aivpn_common::mask::MaskProfile>>>,
    session_keys: Option<SessionKeys>,
    upload_state: Option<Arc<Mutex<UploadCryptoState>>>,
    transition_recv_keys: Option<SessionKeys>,
    transition_recv_deadline: Option<Instant>,
    keypair: KeyPair,
    counter: u64,
    send_seq: u32,
    _recv_seq: u32,
    recv_window: RecvWindow,
    transition_recv_window: RecvWindow,
    recv_mdh_len: usize,
    prev_recv_mdh_len: Option<usize>,
    // Traffic counters
    bytes_sent: Arc<AtomicU64>,
    bytes_received: Arc<AtomicU64>,
    // Pre-allocated buffers for zero-copy I/O (OPTIMIZATION)
    _send_buf: Vec<u8>,
    _recv_buf: Vec<u8>,
    proxy_rx_queue: Option<Arc<Mutex<VecDeque<Vec<u8>>>>>,
    // Recording tracking
    active_recording_session: Option<[u8; 16]>,
    keepalive_interval: Duration,
    /// Local UDP port used on last successful connect — reused on reconnect to
    /// preserve CGNAT inbound mapping (port-preserving carriers like MTS).
    last_local_port: Option<u16>,
    /// Static X25519 keypair — persisted across reconnects for device binding (0.9.0+).
    static_keypair: Option<KeyPair>,
    /// Connection quality tracker — RTT, jitter, loss → 0–100 score (0.9.0+).
    quality_tracker: QualityTracker,
    /// Current adaptive mode level — adjusted from quality score (0.9.0+).
    adaptive_level: AdaptiveLevel,
    /// Epoch-ms timestamp of last outbound keepalive — shared with upload task for RTT.
    keepalive_sent_ms: Arc<AtomicU64>,
    /// Kernel-module accelerator (Linux only, auto-detected via /dev/aivpn).
    #[cfg(target_os = "linux")]
    kernel_accel: Option<Arc<KernelAccel>>,
    /// Interface on which the XDP early-filter was attached (Linux only).
    #[cfg(target_os = "linux")]
    xdp_iface: Option<String>,
}

impl AivpnClient {
    /// Create new client
    pub fn new(config: ClientConfig) -> Result<Self> {
        let keypair = KeyPair::generate();
        let tunnel = Tunnel::new(config.tun_config.clone());
        let recv_mdh_len = packet_mdh_len_for_mask(&config.initial_mask);
        let bytes_sent = Arc::new(AtomicU64::new(0));
        let bytes_received = Arc::new(AtomicU64::new(0));

        let static_keypair = load_or_generate_static_keypair();

        Ok(Self {
            config,
            state: ClientState::Provisioned,
            tunnel,
            udp_socket: None,
            mimicry_engine: None,
            control_tx: None,
            pending_mask: Arc::new(Mutex::new(None)),
            session_keys: None,
            #[cfg(target_os = "linux")]
            kernel_accel: None,
            #[cfg(target_os = "linux")]
            xdp_iface: None,
            upload_state: None,
            transition_recv_keys: None,
            transition_recv_deadline: None,
            keypair,
            counter: 0,
            send_seq: 0,
            _recv_seq: 0,
            recv_window: RecvWindow::new(),
            transition_recv_window: RecvWindow::new(),
            recv_mdh_len,
            prev_recv_mdh_len: None,
            bytes_sent: bytes_sent.clone(),
            bytes_received: bytes_received.clone(),
            // Pre-allocate buffers to MAX_PACKET_SIZE to avoid reallocations
            _send_buf: Vec::with_capacity(MAX_PACKET_SIZE),
            _recv_buf: Vec::with_capacity(MAX_PACKET_SIZE),
            proxy_rx_queue: None,
            active_recording_session: None,
            keepalive_interval: Duration::from_secs(DEFAULT_KEEPALIVE_SECS as u64),
            last_local_port: None,
            static_keypair,
            quality_tracker: QualityTracker::new(),
            adaptive_level: AdaptiveLevel::Off,
            keepalive_sent_ms: Arc::new(AtomicU64::new(0)),
        })
    }

    /// Connect to server
    pub async fn connect(&mut self) -> Result<()> {
        info!("Connecting to AIVPN server...");
        self.state = ClientState::Connecting;

        // Create TUN device first (skipped in proxy mode)
        if self.config.proxy_listen.is_none() {
            self.tunnel.create()?;
        }

        // Resolve the server address. Docker/local test setups often use a
        // hostname rather than a literal IP:port string.
        let server_addr = tokio::net::lookup_host(&self.config.server_addr)
            .await
            .map_err(Error::Io)?
            .next()
            .ok_or_else(|| {
                Error::Io(std::io::Error::new(
                    std::io::ErrorKind::InvalidInput,
                    format!(
                        "failed to resolve server address: {}",
                        self.config.server_addr
                    ),
                ))
            })?;

        // Create UDP socket with 4MB OS buffers (OPTIMIZATION)
        let domain = if server_addr.is_ipv4() {
            socket2::Domain::IPV4
        } else {
            socket2::Domain::IPV6
        };
        let socket2_sock =
            socket2::Socket::new(domain, socket2::Type::DGRAM, Some(socket2::Protocol::UDP))
                .map_err(Error::Io)?;

        socket2_sock.set_nonblocking(true).map_err(Error::Io)?;
        let _ = socket2_sock.set_recv_buffer_size(4 * 1024 * 1024);
        let _ = socket2_sock.set_send_buffer_size(4 * 1024 * 1024);

        // Try to reuse the previous local port so port-preserving CGNAT carriers
        // (MTS, Beeline) don't need to update their inbound routing table on
        // reconnect — the old mapping already points to the right port.
        let hint_port = self.last_local_port.unwrap_or(0);
        let bind_addr: SocketAddr = if server_addr.is_ipv4() {
            format!("0.0.0.0:{}", hint_port).parse().unwrap()
        } else {
            format!("[::]:{}", hint_port).parse().unwrap()
        };
        if socket2_sock.bind(&bind_addr.into()).is_err() && hint_port != 0 {
            // Saved port unavailable — fall back to OS-assigned ephemeral.
            let fallback: SocketAddr = if server_addr.is_ipv4() {
                "0.0.0.0:0".parse().unwrap()
            } else {
                "[::]:0".parse().unwrap()
            };
            socket2_sock.bind(&fallback.into()).map_err(Error::Io)?;
        }

        // Connect UDP socket
        socket2_sock
            .connect(&server_addr.into())
            .map_err(Error::Io)?;

        // Persist the local port for the next reconnect.
        self.last_local_port = socket2_sock
            .local_addr()
            .ok()
            .and_then(|a| a.as_socket())
            .map(|a| a.port());

        let std_sock: std::net::UdpSocket = socket2_sock.into();
        let socket = UdpSocket::from_std(std_sock).map_err(Error::Io)?;

        self.udp_socket = Some(Arc::new(socket));

        // Auto-detect kernel acceleration (Linux only).
        #[cfg(target_os = "linux")]
        {
            let ka = KernelAccel::try_open();
            if ka.is_some() {
                info!("Kernel acceleration: active (aivpn.ko loaded — /dev/aivpn ready)");
            } else {
                info!("Kernel acceleration: not available — using built-in user-space data path");
            }
            if let Some(ref ka) = ka {
                // Wire UDP socket
                use std::os::unix::io::AsRawFd;
                let udp_fd = self.udp_socket.as_ref().unwrap().as_raw_fd();
                if let Err(e) = ka.set_udp_sock(udp_fd) {
                    warn!("kernel set_udp_sock failed: {e}");
                }
                // Wire TUN device (skipped in proxy mode)
                if self.config.proxy_listen.is_none() {
                    let tun_name = self.tunnel.name();
                    if let Ok(cname) = std::ffi::CString::new(tun_name) {
                        let ifindex = unsafe { libc::if_nametoindex(cname.as_ptr()) };
                        if ifindex > 0 {
                            if let Err(e) = ka.set_tun(ifindex) {
                                warn!("kernel set_tun failed: {e}");
                            } else {
                                info!("Kernel acceleration wired to TUN (ifindex={ifindex})");
                            }
                        }
                    }
                }
            }
            self.kernel_accel = ka.map(Arc::new);

            // Attach XDP early-filter to the physical NIC (best-effort).
            // XDP drops malformed/expired packets at NIC level before socket buffer
            // allocation, providing DDoS protection independent of aivpn.ko.
            if let Some(iface) = xdp_default_iface() {
                match xdp_attach(&iface, server_addr.port(), 10_000) {
                    Ok(()) => self.xdp_iface = Some(iface),
                    Err(e) => info!("XDP early-filter not available: {e}"),
                }
            }
        }

        if self.config.proxy_listen.is_none() {
            self.tunnel.set_server_ip(server_addr.ip().to_string());
            // Enable full tunnel only after the server UDP path is established.
            if self.config.tun_config.full_tunnel {
                self.tunnel.enable_full_tunnel()?;
            }
            if !self.config.tun_config.include_routes.is_empty()
                || !self.config.tun_config.exclude_routes.is_empty()
            {
                self.tunnel.apply_split_routes()?;
            }
            if self.config.tun_config.kill_switch {
                self.tunnel.activate_kill_switch()?;
            }
        }

        // Initialize mimicry engine
        self.mimicry_engine = Some(MimicryEngine::new(self.config.initial_mask.clone()));

        // Derive session keys (Zero-RTT)
        let dh_result = self
            .keypair
            .compute_shared(&self.config.server_public_key)?;
        debug!("Client DH result: {}", hex::encode(&dh_result));
        debug!(
            "Client eph_pub: {}",
            hex::encode(self.keypair.public_key_bytes())
        );
        debug!(
            "Client PSK: {:?}",
            self.config.preshared_key.as_ref().map(hex::encode)
        );
        self.session_keys = Some(crypto::derive_session_keys(
            &dh_result,
            self.config.preshared_key.as_ref(),
            &self.keypair.public_key_bytes(),
        ));
        let keys = self
            .session_keys
            .as_ref()
            .ok_or(Error::Session("session_keys not set after derive".into()))?;
        debug!("Client tag_secret: {}", hex::encode(&keys.tag_secret));

        self.state = ClientState::Connected;
        info!("Connected to server at {}", self.config.server_addr);
        info!("TUN device: {}", self.tunnel.name());

        Ok(())
    }

    fn apply_server_network_override(&mut self, network_config: ClientNetworkConfig) -> Result<()> {
        let current_config = self.config.tun_config.client_network_config()?;
        if current_config == network_config {
            return Ok(());
        }

        info!(
            "Applying server-confirmed network override: client {} gateway {} /{} mtu {}",
            network_config.client_ip,
            network_config.server_vpn_ip,
            network_config.prefix_len,
            network_config.mtu,
        );

        let tun_name = self.config.tun_config.tun_name.clone();
        let full_tunnel = self.config.tun_config.full_tunnel;
        if self.config.proxy_listen.is_none() {
            self.tunnel.apply_network_config(network_config.clone())?;
        }
        self.config.tun_config =
            TunnelConfig::from_network_config(tun_name, network_config, full_tunnel);
        Ok(())
    }

    /// Disconnect from server
    pub async fn disconnect(&mut self) {
        info!("Disconnecting...");

        // Send shutdown message if connected
        if self.state == ClientState::Connected {
            if self.session_keys.is_some() {
                let shutdown = ControlPayload::Shutdown { reason: 0 };
                let _ = self.send_control(&shutdown).await;
            }
        }

        self.state = ClientState::Disconnected;
        self.udp_socket = None;

        // Detach XDP filter (Linux only, best-effort)
        #[cfg(target_os = "linux")]
        if let Some(ref iface) = self.xdp_iface.take() {
            xdp_detach(iface);
        }

        // Zeroize keys
        self.session_keys = None;
        self.upload_state = None;
        self.transition_recv_keys = None;
        self.transition_recv_deadline = None;
    }

    /// Run the client main loop
    pub async fn run(&mut self, shutdown: Arc<AtomicBool>) -> Result<()> {
        self.connect().await?;

        // Send initial handshake packet with eph_pub to establish session
        self.send_init().await?;

        info!("Starting client main loop");
        info!("Routing traffic through AIVPN tunnel...");

        // Create channels for TUN -> upload pipeline and UDP -> main loop
        let (tun_to_udp_tx, tun_to_udp_rx) = mpsc::channel::<Vec<u8>>(8192);
        let (udp_to_tun_tx, mut udp_to_tun_rx) = mpsc::channel::<Bytes>(8192);
        let (admin_tx, mut admin_rx) = mpsc::channel::<String>(16);
        let (control_tx, control_rx) = mpsc::channel::<ControlPayload>(32);
        self.control_tx = Some(control_tx.clone());

        // mTLS ClientCert is sent inside the ServerHello handler, after the PFS
        // ratchet completes, so it is protected by the ratcheted session keys.

        // Spawn local IPC listener for CLI commands. Stored in AbortOnDrop so the task
        // (and its bound UDP socket) is cancelled when run() returns. Without this,
        // the orphaned task keeps 127.0.0.1:44301 bound across reconnect iterations,
        // causing the next run() call to fail with "Address already in use".
        let _admin_task = AbortOnDrop(tokio::spawn(async move {
            match tokio::net::UdpSocket::bind("127.0.0.1:44301").await {
                Ok(socket) => {
                    let mut buf = [0u8; 1024];
                    loop {
                        if let Ok((len, _addr)) = socket.recv_from(&mut buf).await {
                            if let Ok(msg) = String::from_utf8(buf[..len].to_vec()) {
                                let _ = admin_tx.send(msg).await;
                            }
                        }
                    }
                }
                Err(e) => {
                    error!(
                        "Failed to bind local admin UDP socket 127.0.0.1:44301: {}",
                        e
                    );
                }
            }
        }));

        // Proxy mode: start smoltcp + SOCKS5 instead of creating a TUN device
        if let Some(listen_addr) = self.config.proxy_listen {
            let vpn_ip = self
                .config
                .tun_config
                .tun_addr
                .parse::<std::net::Ipv4Addr>()
                .map_err(|e| Error::Io(std::io::Error::new(std::io::ErrorKind::InvalidInput, e)))?;
            let gateway_ip = self
                .config
                .tun_config
                .server_vpn_ip
                .parse::<std::net::Ipv4Addr>()
                .map_err(|e| Error::Io(std::io::Error::new(std::io::ErrorKind::InvalidInput, e)))?;
            let proxy_cfg = crate::proxy::ProxyConfig {
                listen_addr,
                vpn_ip,
                gateway_ip,
                prefix_len: self.config.tun_config.prefix_len,
            };
            let handle = crate::proxy::spawn_proxy(proxy_cfg, tun_to_udp_tx.clone())
                .await
                .map_err(Error::Io)?;
            self.proxy_rx_queue = Some(Arc::clone(&handle.rx_queue));
        }

        // Take the TUN reader for the spawned task (skipped in proxy mode)
        let tun_task = if self.config.proxy_listen.is_none() {
            let mut tun_reader = self
                .tunnel
                .take_reader()
                .ok_or(Error::Session("TUN reader not available".into()))?;
            let tun_to_udp_tx_clone = tun_to_udp_tx.clone();
            let shutdown_for_tasks = shutdown.clone();
            tokio::spawn(async move {
                let mut buf = vec![0u8; MAX_PACKET_SIZE];
                loop {
                    if shutdown_for_tasks.load(Ordering::SeqCst) {
                        break;
                    }

                    match tun_reader.read(&mut buf).await {
                        Ok(n) => {
                            if n > 0 {
                                debug!("TUN read {} bytes", n);

                                #[cfg(target_os = "macos")]
                                let payload: Vec<u8> = if n > 4 && buf[0] == 0 && buf[1] == 0 {
                                    buf[4..n].to_vec()
                                } else {
                                    buf[..n].to_vec()
                                };

                                #[cfg(not(target_os = "macos"))]
                                let payload: Vec<u8> = buf[..n].to_vec();

                                let _ = tun_to_udp_tx_clone.send(payload).await;
                            }
                        }
                        Err(e) => {
                            error!("TUN read error: {}", e);
                            tokio::time::sleep(Duration::from_millis(10)).await;
                        }
                    }
                }
            })
        } else {
            tokio::spawn(std::future::pending::<()>())
        };

        // Spawn UDP reader task
        let udp_socket = self
            .udp_socket
            .as_ref()
            .ok_or(Error::Session(
                "UDP socket not initialized before run()".into(),
            ))?
            .clone();
        let udp_to_tun_tx_clone = udp_to_tun_tx.clone();
        let shutdown_for_tasks = shutdown.clone();
        let udp_task = tokio::spawn(async move {
            let mut buf = vec![0u8; MAX_PACKET_SIZE];
            let mut consecutive_errors: u32 = 0;

            loop {
                if shutdown_for_tasks.load(Ordering::SeqCst) {
                    break;
                }

                match udp_socket.recv(&mut buf).await {
                    Ok(n) => {
                        consecutive_errors = 0;
                        if n > 0 {
                            let _ = udp_to_tun_tx_clone
                                .send(Bytes::copy_from_slice(&buf[..n]))
                                .await;
                        }
                    }
                    Err(e) => {
                        consecutive_errors += 1;
                        error!("UDP recv error: {}", e);
                        if consecutive_errors >= 20 {
                            // Socket is likely dead; let the main loop handle reconnect.
                            break;
                        }
                        tokio::time::sleep(Duration::from_millis(10)).await;
                    }
                }
            }
        });

        // Spawn stats writer task
        let stats_shutdown = shutdown.clone();
        let stats_bytes_sent = self.bytes_sent.clone();
        let stats_bytes_received = self.bytes_received.clone();
        let stats_task = tokio::spawn(async move {
            // Determine platform-appropriate stats paths
            #[cfg(target_os = "windows")]
            let stats_paths: Vec<std::path::PathBuf> = {
                let mut paths = Vec::new();
                if let Some(local_app) = std::env::var_os("LOCALAPPDATA") {
                    let dir = std::path::PathBuf::from(local_app).join("AIVPN");
                    let _ = tokio::fs::create_dir_all(&dir).await;
                    paths.push(dir.join("traffic.stats"));
                }
                let tmp = std::env::temp_dir().join("aivpn-traffic.stats");
                paths.push(tmp);
                paths
            };
            #[cfg(not(target_os = "windows"))]
            let stats_paths: Vec<std::path::PathBuf> = vec![
                std::path::PathBuf::from("/var/run/aivpn/traffic.stats"),
                std::path::PathBuf::from("/tmp/aivpn-traffic.stats"),
            ];

            // Write initial stats
            for path in &stats_paths {
                let _ = tokio::fs::write(path, "sent:0,received:0").await;
            }
            info!("Initial stats written");

            let mut interval = tokio::time::interval(Duration::from_secs(1));
            loop {
                interval.tick().await;
                if stats_shutdown.load(Ordering::SeqCst) {
                    break;
                }
                let sent = stats_bytes_sent.load(Ordering::Relaxed);
                let received = stats_bytes_received.load(Ordering::Relaxed);
                let stats = format!("sent:{},received:{}", sent, received);
                for path in &stats_paths {
                    let _ = tokio::fs::write(path, &stats).await;
                }
            }
        });

        // ── Spawn upload task using the shared pipeline ──
        let upload_udp = self
            .udp_socket
            .as_ref()
            .ok_or(Error::Session(
                "UDP socket not initialized before upload task".into(),
            ))?
            .clone();
        let upload_keys = self
            .session_keys
            .clone()
            .ok_or(Error::Session("No session keys".into()))?;
        let upload_engine = self
            .mimicry_engine
            .take()
            .ok_or(Error::Session("No mimicry engine".into()))?;
        let upload_seq = self.send_seq as u16;
        let upload_counter = self.counter;
        let upload_bytes_sent = self.bytes_sent.clone();
        let upload_state = Arc::new(Mutex::new(UploadCryptoState {
            keys: upload_keys,
            counter: upload_counter,
            seq: upload_seq,
        }));
        self.upload_state = Some(upload_state.clone());

        let upload_pending_mask = self.pending_mask.clone();

        let mut upload_task = tokio::spawn(Self::spawn_upload(
            tun_to_udp_rx,
            control_rx,
            upload_udp,
            upload_engine,
            upload_state,
            upload_bytes_sent,
            upload_pending_mask,
            self.keepalive_interval,
            self.keepalive_sent_ms.clone(),
            self.adaptive_level.fec_n(),
        ));

        // Main loop: download + shutdown + upload health
        let mut shutdown_tick = tokio::time::interval(Duration::from_secs(1));
        shutdown_tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

        // RX silence watchdog: detect silent path failure (NAT rebind, carrier drop).
        // The UDP socket stays open and recv() blocks indefinitely when the path dies,
        // so we track the last received packet and reconnect after 45 s of silence.
        let mut rx_watchdog = tokio::time::interval(Duration::from_secs(15));
        rx_watchdog.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        let mut last_rx = std::time::Instant::now();

        let run_res: Result<()> = loop {
            tokio::select! {
                // Allow fast shutdown.
                _ = shutdown_tick.tick() => {
                    if shutdown.load(Ordering::SeqCst) {
                        info!("Shutdown requested");
                        stats_task.abort();
                        break Ok(());
                    }
                }

                _ = rx_watchdog.tick() => {
                    const RX_SILENCE: Duration = Duration::from_secs(45);
                    if last_rx.elapsed() > RX_SILENCE {
                        warn!("No server traffic for {:?} — reconnecting", last_rx.elapsed());
                        break Err(Error::Session("RX silence timeout".into()));
                    }
                }

                // Upload task completed (error or channel closed).
                join_res = &mut upload_task => {
                    break match join_res {
                        Ok(Ok(())) => Err(Error::Channel("Upload loop ended unexpectedly".into())),
                        Ok(Err(e)) => Err(e),
                        Err(e) => Err(Error::Session(format!("Upload task panicked: {e}"))),
                    };
                }

                cmd = admin_rx.recv() => {
                    if let Some(cmd) = cmd {
                        if let Some(service) = cmd.strip_prefix("record_start:") {
                            crate::record_cmd::handle_recording_status(true, Some(service));
                            let payload = ControlPayload::RecordingStart { service: service.to_string() };
                            if let Err(e) = control_tx.send(payload).await {
                                error!("Failed to send RecordingStart to upload task: {}", e);
                            } else {
                                info!("Sent RecordingStart for {}", service);
                            }
                        } else if cmd == "record_stop" {
                            if let Some(session_id) = self.active_recording_session {
                                let current_service = crate::record_cmd::read_local_status().and_then(|status| status.service);
                                crate::record_cmd::mark_recording_stop_requested(current_service.as_deref());
                                let payload = ControlPayload::RecordingStop { session_id };
                                if let Err(e) = control_tx.send(payload).await {
                                    error!("Failed to send RecordingStop to upload task: {}", e);
                                } else {
                                    info!("Sent RecordingStop");
                                }
                            } else {
                                warn!("No active recording session to stop");
                                crate::record_cmd::handle_recording_failed("No active recording session to stop");
                            }
                        } else if cmd == "record_status" {
                            let payload = ControlPayload::RecordingStatusRequest;
                            if let Err(e) = control_tx.send(payload).await {
                                error!("Failed to send RecordingStatusRequest to upload task: {}", e);
                            }
                        }
                    }
                }

                // UDP -> TUN (inbound traffic)
                res = udp_to_tun_rx.recv() => {
                    let packet = match res {
                        Some(p) => p,
                        None => break Err(Error::Channel("UDP->TUN channel closed".into())),
                    };
                    last_rx = std::time::Instant::now();

                    if let Err(e) = self.receive_and_write_packet(&packet).await {
                        match &e {
                            Error::InvalidPacket(_) => warn!("Receive invalid packet: {}", e),
                            Error::Crypto(_) => warn!("Receive error (crypto): {}", e),
                            _ => {
                                warn!("Receive error: {}", e);
                                break Err(e);
                            }
                        }
                    }
                }
            }
        };

        // Stop background tasks before disconnecting.
        tun_task.abort();
        udp_task.abort();
        let _ = tun_task.await;
        let _ = udp_task.await;

        self.disconnect().await;

        run_res
    }

    /// Spawn the upload task using the shared pipeline.
    async fn spawn_upload(
        mut rx: mpsc::Receiver<Vec<u8>>,
        mut control_rx: mpsc::Receiver<ControlPayload>,
        udp: Arc<UdpSocket>,
        engine: MimicryEngine,
        upload_state: Arc<Mutex<UploadCryptoState>>,
        bytes_sent: Arc<AtomicU64>,
        pending_mask: Arc<Mutex<Option<aivpn_common::mask::MaskProfile>>>,
        keepalive_interval: Duration,
        keepalive_sent_ms: Arc<AtomicU64>,
        fec_n: u8,
    ) -> Result<()> {
        /// Wraps MimicryEngine to implement the shared PacketEncryptor trait.
        struct MimicryEncryptor {
            engine: MimicryEngine,
            upload_state: Arc<Mutex<UploadCryptoState>>,
            bytes_sent: Arc<AtomicU64>,
            pending_mask: Arc<Mutex<Option<aivpn_common::mask::MaskProfile>>>,
            keepalive_sent_ms: Arc<AtomicU64>,
            fec_encoder: Option<aivpn_common::fec::FecEncoder>,
            pending_fec: Option<Vec<u8>>,
        }

        impl MimicryEncryptor {
            fn check_mask(&mut self) {
                if let Some(mask) = self.pending_mask.lock().unwrap().take() {
                    self.engine.update_mask(mask);
                }
            }
        }

        impl PacketEncryptor for MimicryEncryptor {
            fn encrypt_data(&mut self, payload: &[u8]) -> Result<Vec<u8>> {
                self.check_mask();
                let mut state = self.upload_state.lock().unwrap_or_else(|e| e.into_inner());
                let inner = build_inner_packet(InnerType::Data, state.seq, payload);
                state.seq = state.seq.wrapping_add(1);
                let keys = state.keys.clone();
                let pkt = self
                    .engine
                    .build_packet(&inner, &keys, &mut state.counter, None)?;
                self.engine.update_fsm();

                // FEC: feed payload; if group complete, pre-encrypt repair datagram
                if let Some(fec) = self.fec_encoder.as_mut() {
                    if let Some(repair) = fec.feed(payload) {
                        let repair_payload = repair.encode();
                        let repair_inner =
                            build_inner_packet(InnerType::FecRepair, state.seq, &repair_payload);
                        state.seq = state.seq.wrapping_add(1);
                        if let Ok(enc_repair) =
                            self.engine
                                .build_packet(&repair_inner, &keys, &mut state.counter, None)
                        {
                            self.pending_fec = Some(enc_repair);
                        }
                    }
                }

                Ok(pkt)
            }

            fn take_fec_repair(&mut self) -> Option<Vec<u8>> {
                self.pending_fec.take()
            }

            fn encrypt_control(&mut self, payload: &ControlPayload) -> Result<Vec<u8>> {
                self.check_mask();
                let mut state = self.upload_state.lock().unwrap_or_else(|e| e.into_inner());
                let bytes = payload.encode()?;
                let inner = build_inner_packet(InnerType::Control, state.seq, &bytes);
                state.seq = state.seq.wrapping_add(1);
                let keys = state.keys.clone();
                self.engine
                    .build_packet(&inner, &keys, &mut state.counter, None)
            }

            fn encrypt_keepalive(&mut self) -> Result<Vec<u8>> {
                self.check_mask();
                // Record send time for RTT measurement via KeepaliveAck.
                let now_ms = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_millis() as u64;
                self.keepalive_sent_ms.store(now_ms, Ordering::Relaxed);
                let mut state = self.upload_state.lock().unwrap_or_else(|e| e.into_inner());
                let keepalive = ControlPayload::Keepalive { send_ts: now_ms }.encode()?;
                let inner = build_inner_packet(InnerType::Control, state.seq, &keepalive);
                state.seq = state.seq.wrapping_add(1);
                let keys = state.keys.clone();
                self.engine
                    .build_packet(&inner, &keys, &mut state.counter, None)
            }

            fn on_data_sent(&mut self, payload_len: usize) {
                self.bytes_sent
                    .fetch_add(payload_len as u64, Ordering::Relaxed);
            }
        }

        let mut enc = MimicryEncryptor {
            engine,
            upload_state,
            bytes_sent,
            pending_mask,
            keepalive_sent_ms,
            fec_encoder: if fec_n > 0 {
                Some(aivpn_common::fec::FecEncoder::new(fec_n, 1500))
            } else {
                None
            },
            pending_fec: None,
        };
        let config = UploadConfig {
            keepalive_interval,
            ..Default::default()
        };
        upload_pipeline::run_upload_loop(&mut rx, Some(&mut control_rx), &udp, &mut enc, &config)
            .await
    }

    /// Receive packet from server and write to TUN (using pre-computed mdh_len)
    async fn receive_and_write_packet(&mut self, packet: &[u8]) -> Result<()> {
        if self
            .transition_recv_deadline
            .is_some_and(|deadline| Instant::now() >= deadline)
        {
            self.transition_recv_keys = None;
            self.transition_recv_deadline = None;
            self.transition_recv_window.reset();
        }

        let mdh_len = self.recv_mdh_len;

        let keys = self
            .session_keys
            .as_ref()
            .ok_or(Error::Session("No session keys".into()))?;

        let decoded = match decode_packet_with_mdh_len(packet, keys, &mut self.recv_window, mdh_len)
        {
            Ok(decoded) => {
                // Clear prev_mdh_len after successful decode with current — transition complete
                self.prev_recv_mdh_len = None;
                decoded
            }
            Err(primary_err) => {
                // Fallback 1: try transition keys (PFS ratchet)
                if let Some(fallback_keys) = self.transition_recv_keys.as_ref() {
                    if let Ok(decoded) = decode_packet_with_mdh_len(
                        packet,
                        fallback_keys,
                        &mut self.transition_recv_window,
                        mdh_len,
                    ) {
                        return self.process_decoded(decoded).await;
                    }
                }

                // Fallback 2: try previous MDH length (mask rotation in-flight)
                if let Some(prev_mdh) = self.prev_recv_mdh_len {
                    if prev_mdh != mdh_len {
                        if let Ok(decoded) = decode_packet_with_mdh_len(
                            packet,
                            keys,
                            &mut self.recv_window,
                            prev_mdh,
                        ) {
                            debug!(
                                "Decoded with prev_mdh_len={} (transition in-flight)",
                                prev_mdh
                            );
                            return self.process_decoded(decoded).await;
                        }
                    }
                }

                return Err(primary_err);
            }
        };
        self.process_decoded(decoded).await
    }

    /// Process a successfully decoded packet (shared by primary and fallback paths)
    async fn process_decoded(&mut self, decoded: DecodedPacket) -> Result<()> {
        let inner_header = decoded.header;
        let ip_payload = decoded.payload;

        match inner_header.inner_type {
            InnerType::Data => {
                if ip_payload.is_empty() || (ip_payload[0] >> 4 != 4 && ip_payload[0] >> 4 != 6) {
                    return Err(Error::InvalidPacket("Invalid IP version in payload"));
                }
                if let Some(q) = &self.proxy_rx_queue {
                    q.lock().unwrap().push_back(ip_payload.to_vec());
                } else {
                    self.tunnel.write_packet_async(&ip_payload).await?;
                }
                self.bytes_received
                    .fetch_add(ip_payload.len() as u64, Ordering::Relaxed);
                debug!(
                    "Received {} bytes from server, wrote to TUN",
                    ip_payload.len()
                );
            }
            InnerType::Control => {
                let control = ControlPayload::decode(&ip_payload)?;
                self.handle_server_control(control).await?;
            }
            _ => {
                debug!(
                    "Received non-data packet type: {:?}",
                    inner_header.inner_type
                );
            }
        }

        Ok(())
    }

    /// Handle control messages from server
    async fn handle_server_control(&mut self, control: ControlPayload) -> Result<()> {
        match control {
            ControlPayload::MaskUpdate {
                mask_data,
                signature,
            } => {
                // The server signs the raw mask_data bytes (sign_mask() in session.rs).
                // Verify before deserialising so a bad signature is caught immediately.
                if let Some(signing_key) = &self.config.server_signing_key {
                    use ed25519_dalek::{Signature, Verifier, VerifyingKey};
                    match VerifyingKey::from_bytes(signing_key) {
                        Ok(vk) => {
                            let sig = Signature::from_bytes(&signature);
                            if vk.verify(&mask_data, &sig).is_err() {
                                warn!("MaskUpdate rejected: invalid ed25519 signature");
                                return Ok(());
                            }
                        }
                        Err(e) => {
                            warn!("MaskUpdate rejected: bad signing key in config: {}", e);
                            return Ok(());
                        }
                    }
                }
                match rmp_serde::from_slice::<MaskProfile>(&mask_data) {
                    Ok(new_mask) => self.update_mask(new_mask),
                    Err(e) => warn!("Failed to parse mask update: {}", e),
                }
            }
            ControlPayload::BootstrapDescriptorUpdate { descriptor_data } => {
                if descriptor_data.len() > 512 * 1024 {
                    warn!(
                        "BootstrapDescriptorUpdate rejected: payload too large ({} bytes)",
                        descriptor_data.len()
                    );
                    return Ok(());
                }
                match rmp_serde::from_slice::<BootstrapDescriptor>(&descriptor_data) {
                    Ok(descriptor) => {
                        let trusted = self.config.server_signing_key.as_ref();
                        if let Err(e) =
                            bootstrap_cache::store_verified_descriptor(descriptor, trusted)
                        {
                            warn!("Failed to store bootstrap descriptor: {}", e);
                        }
                    }
                    Err(e) => warn!("Failed to parse bootstrap descriptor update: {}", e),
                }
            }
            ControlPayload::KeyRotate { new_eph_pub } => {
                let client_rekey_kp = crypto::KeyPair::generate();
                let dh_rekey = match client_rekey_kp.compute_shared(&new_eph_pub) {
                    Ok(dh) => dh,
                    Err(e) => {
                        warn!("Inline rekey: DH failed: {}", e);
                        return Ok(());
                    }
                };
                let current_sk = match self.session_keys.as_ref() {
                    Some(k) => k.session_key,
                    None => {
                        warn!("Inline rekey: no session keys");
                        return Ok(());
                    }
                };
                let new_keys = crypto::derive_session_keys(
                    &dh_rekey,
                    Some(&current_sk),
                    &client_rekey_kp.public_key_bytes(),
                );
                // Send response with OLD keys before switching
                let response = ControlPayload::KeyRotate {
                    new_eph_pub: client_rekey_kp.public_key_bytes(),
                };
                if let Err(e) = self.send_control(&response).await {
                    warn!("Inline rekey: failed to send response: {}", e);
                    return Ok(());
                }
                // Keep old keys for 2 s to accept in-flight server packets
                self.transition_recv_keys = self.session_keys.clone();
                self.transition_recv_deadline = Some(Instant::now() + Duration::from_secs(2));
                self.transition_recv_window = std::mem::take(&mut self.recv_window);
                self.session_keys = Some(new_keys);
                self.counter = 0;
                self.recv_window.reset();
                if let Some(upload_state) = &self.upload_state {
                    let mut state = upload_state.lock().unwrap_or_else(|e| e.into_inner());
                    state.keys = self.session_keys.clone().expect("keys set");
                    state.counter = 0;
                }
                info!("Inline PFS rekey complete — new session keys active");
            }
            ControlPayload::ServerHello {
                server_eph_pub,
                signature,
                network_config,
            } => {
                // Verify ed25519 signature over (server_eph_pub || client_eph_pub).
                // The server signs this tuple in session.rs create_session().
                if let Some(signing_key) = &self.config.server_signing_key {
                    use ed25519_dalek::{Signature, Verifier, VerifyingKey};
                    match VerifyingKey::from_bytes(signing_key) {
                        Ok(vk) => {
                            let mut msg = Vec::with_capacity(64);
                            msg.extend_from_slice(&server_eph_pub);
                            msg.extend_from_slice(&self.keypair.public_key_bytes());
                            let sig = Signature::from_bytes(&signature);
                            if vk.verify(&msg, &sig).is_err() {
                                error!(
                                    "ServerHello rejected: ed25519 signature verification failed \
                                     — possible MITM attack"
                                );
                                return Err(Error::Crypto("ServerHello signature invalid".into()));
                            }
                        }
                        Err(e) => {
                            error!("ServerHello: invalid signing key in config: {}", e);
                            return Err(Error::Crypto(format!(
                                "Invalid server signing key: {}",
                                e
                            )));
                        }
                    }
                }

                info!("ServerHello received — completing PFS ratchet");

                if let Some(network_config) = network_config {
                    if let Some(ka) = network_config.keepalive_secs.filter(|&s| s > 0) {
                        self.keepalive_interval = Duration::from_secs(ka as u64);
                    }
                    self.apply_server_network_override(network_config)?;
                }

                // Compute DH2 = client_eph * server_eph for PFS (CRIT-3)
                let dh2 = self.keypair.compute_shared(&server_eph_pub)?;

                // Derive ratcheted keys using current session_key as PSK
                let current_key = self
                    .session_keys
                    .as_ref()
                    .ok_or(Error::Session("No session keys for ratchet".into()))?
                    .session_key;
                let ratcheted = crypto::derive_session_keys(
                    &dh2,
                    Some(&current_key),
                    &self.keypair.public_key_bytes(),
                );

                // Keep accepting old inbound keys until the server proves it has
                // switched too. Outbound traffic moves to ratcheted keys now.
                self.transition_recv_keys = self.session_keys.clone();
                self.transition_recv_deadline = Some(Instant::now() + Duration::from_secs(2));
                self.transition_recv_window = std::mem::take(&mut self.recv_window);

                // Switch to ratcheted keys — outbound uses the new keys immediately.
                self.session_keys = Some(ratcheted);
                self.counter = 0;
                self.recv_window.reset();
                if let Some(upload_state) = &self.upload_state {
                    let mut state = upload_state.lock().unwrap_or_else(|e| e.into_inner());
                    state.keys = self.session_keys.clone().expect("session keys set");
                    state.counter = 0;
                    info!("Outbound ratchet activated — upload switched to new keys");
                }
                info!("PFS ratchet complete — forward secrecy established");

                // Send mTLS ClientCert now that the PFS ratchet is complete.
                // Sending it here ensures the cert is protected by the ratcheted
                // session keys, not the initial zero-RTT keys.
                if let Some(cert) = self.config.mtls_cert.clone() {
                    if let Err(e) = self
                        .send_control(&ControlPayload::ClientCert {
                            cert_bytes: cert.clone(),
                        })
                        .await
                    {
                        warn!("mTLS: failed to queue ClientCert after ratchet: {}", e);
                    } else {
                        debug!(
                            "mTLS: ClientCert queued after PFS ratchet ({} bytes)",
                            cert.len()
                        );
                    }
                }

                let _ = self
                    .send_control(&ControlPayload::RecordingStatusRequest)
                    .await;

                // Warmup: 4 keepalives (100 ms apart) to force CGNAT to refresh
                // its inbound port mapping after reconnect.  Fallback for carriers
                // that delay updating the entry even after local-port reuse.
                for _ in 0..4u8 {
                    tokio::time::sleep(Duration::from_millis(100)).await;
                    let _ = self
                        .send_control(&ControlPayload::Keepalive { send_ts: 0 })
                        .await;
                }

                // Device enrollment: prove static key ownership to server.
                // Sent after ratchet so it is protected by PFS session keys.
                if let Some(ref skp) = self.static_keypair {
                    match skp.compute_shared(&self.config.server_public_key) {
                        Ok(dh_proof) => {
                            let enrollment = ControlPayload::DeviceEnrollment {
                                static_pub: skp.public_key_bytes(),
                                dh_proof,
                            };
                            if let Err(e) = self.send_control(&enrollment).await {
                                warn!("DeviceEnrollment send failed: {}", e);
                            }
                        }
                        Err(e) => warn!("DeviceEnrollment DH failed: {}", e),
                    }
                }
            }
            ControlPayload::Keepalive { .. } => {
                debug!("Keepalive from server");
            }
            ControlPayload::TimeSync { server_ts_ms } => {
                debug!("Time sync: server_ts={}", server_ts_ms);
            }
            ControlPayload::Shutdown { reason } => {
                info!("Server requested shutdown (reason: {})", reason);
                self.disconnect().await;
                return Err(Error::Session(format!("server shutdown: {}", reason)));
            }
            ControlPayload::RecordingAck { session_id, status } => {
                if status == "started" {
                    self.active_recording_session = Some(session_id);
                } else if status == "analyzing" {
                    self.active_recording_session = None;
                }
                crate::record_cmd::handle_recording_ack(&session_id, &status);
            }
            ControlPayload::RecordingComplete {
                service,
                mask_id,
                confidence,
            } => {
                self.active_recording_session = None;
                crate::record_cmd::handle_recording_complete(&service, &mask_id, confidence);
            }
            ControlPayload::RecordingFailed { reason } => {
                self.active_recording_session = None;
                crate::record_cmd::handle_recording_failed(&reason);
            }
            ControlPayload::RecordingStatus {
                can_record,
                active_service,
            } => {
                crate::record_cmd::handle_recording_status(can_record, active_service.as_deref());
            }
            ControlPayload::CertRejected {} => {
                warn!("mTLS: server rejected the certificate — re-provision your mTLS cert");
            }
            ControlPayload::KeepaliveAck { echo_ts } => {
                // Use echoed client timestamp for RTT when available (server ≥ 0.9.0),
                // fall back to the stored send-time for older servers.
                let sent_ms = if echo_ts > 0 {
                    echo_ts
                } else {
                    self.keepalive_sent_ms.load(Ordering::Relaxed)
                };
                if sent_ms > 0 {
                    let now_ms = std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .unwrap_or_default()
                        .as_millis() as u64;
                    let rtt_us = now_ms.saturating_sub(sent_ms).saturating_mul(1000);
                    self.quality_tracker.record_rtt(rtt_us);
                    let score = self.quality_tracker.score();
                    Self::write_quality_file(
                        score,
                        self.quality_tracker.rtt_ms(),
                        self.quality_tracker.jitter_ms(),
                        self.adaptive_level as u8,
                    );
                    let new_level = AdaptiveLevel::suggest(score);
                    if new_level != self.adaptive_level {
                        self.adaptive_level = new_level;
                        self.keepalive_interval = Duration::from_secs(new_level.keepalive_secs());
                        info!(
                            "Adaptive level → {:?} (score={}), keepalive={}s",
                            new_level,
                            score,
                            new_level.keepalive_secs()
                        );
                    }
                    let _ = self
                        .send_control(&ControlPayload::QualityReport {
                            quality: score,
                            rtt_ms: self.quality_tracker.rtt_ms(),
                            loss_ppm: self.quality_tracker.loss_ppm(),
                            jitter_ms: self.quality_tracker.jitter_ms(),
                        })
                        .await;
                }
            }
            ControlPayload::AdaptiveHint { level } => {
                let new_level = AdaptiveLevel::from_u8(level);
                if new_level != self.adaptive_level {
                    self.adaptive_level = new_level;
                    self.keepalive_interval = Duration::from_secs(new_level.keepalive_secs());
                    info!("Server adaptive hint → {:?}", new_level);
                }
            }
            _ => {}
        }
        Ok(())
    }

    /// Send initial handshake packet with eph_pub to establish server-side session
    async fn send_init(&mut self) -> Result<()> {
        let keys = self
            .session_keys
            .as_ref()
            .ok_or(Error::Session("No session keys".into()))?;

        let mimicry = self
            .mimicry_engine
            .as_mut()
            .ok_or(Error::Session("No mimicry engine".into()))?;

        // Build keepalive control as init payload
        let keepalive = ControlPayload::Keepalive { send_ts: 0 };
        let encoded = keepalive.encode()?;
        let seq_num = self.send_seq as u16;
        self.send_seq = self.send_seq.wrapping_add(1);
        let inner_payload = build_inner_packet(InnerType::Control, seq_num, &encoded);

        // Include eph_pub (obfuscated) in the init packet
        let obf = obfuscate_client_eph_pub(&self.keypair, &self.config.server_public_key);
        debug!("Client obfuscated eph_pub: {}", hex::encode(&obf));
        debug!(
            "Client original eph_pub: {}",
            hex::encode(self.keypair.public_key_bytes())
        );

        let aivpn_packet =
            mimicry.build_packet(&inner_payload, keys, &mut self.counter, Some(&obf))?;

        let socket = self.udp_socket.as_ref().ok_or(Error::Session(
            "UDP socket not initialized before send_init".into(),
        ))?;
        socket.send(&aivpn_packet).await?;

        info!("Sent init handshake ({} bytes)", aivpn_packet.len());
        Ok(())
    }

    async fn send_control(&mut self, payload: &ControlPayload) -> Result<()> {
        if let Some(tx) = &self.control_tx {
            tx.send(payload.clone())
                .await
                .map_err(|e| Error::Channel(e.to_string()))?;
        } else {
            warn!("control_tx not initialized, dropping control message");
        }
        Ok(())
    }

    /// Update mask profile
    pub fn update_mask(&mut self, new_mask: MaskProfile) {
        let new_mdh_len = packet_mdh_len_for_mask(&new_mask);
        if new_mdh_len != self.recv_mdh_len {
            self.prev_recv_mdh_len = Some(self.recv_mdh_len);
        }
        self.recv_mdh_len = new_mdh_len;
        info!(
            "Updating mask to {} (mdh_len: {})",
            new_mask.mask_id, new_mdh_len
        );
        if let Some(ref mut engine) = self.mimicry_engine {
            engine.update_mask(new_mask.clone());
        }
        let mut pending = self.pending_mask.lock().unwrap();
        *pending = Some(new_mask);
    }

    /// Get current state
    pub fn state(&self) -> ClientState {
        self.state.clone()
    }

    /// Check if connected
    pub fn is_connected(&self) -> bool {
        self.state == ClientState::Connected
    }

    /// Get traffic statistics
    pub fn bytes_sent(&self) -> u64 {
        self.bytes_sent.load(Ordering::Relaxed)
    }

    pub fn bytes_received(&self) -> u64 {
        self.bytes_received.load(Ordering::Relaxed)
    }

    fn write_quality_file(score: u8, rtt_ms: u16, jitter_ms: u16, adaptive_level: u8) {
        #[cfg(windows)]
        let path = std::env::temp_dir().join("aivpn-quality.json");
        #[cfg(not(windows))]
        let path = std::path::PathBuf::from("/var/run/aivpn/quality.json");
        #[cfg(not(windows))]
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        if let Err(e) = std::fs::write(
            &path,
            format!(
                r#"{{"quality":{},"rtt_ms":{},"jitter_ms":{},"adaptive":{}}}"#,
                score, rtt_ms, jitter_ms, adaptive_level
            ),
        ) {
            debug!("quality file write failed: {e}");
        }
    }
}

impl Drop for AivpnClient {
    fn drop(&mut self) {
        // Zeroize sensitive data
        self.session_keys = None;
    }
}

/// Load static X25519 keypair from `~/.config/aivpn/device.key` or generate and save a new one.
/// Returns None when HOME is unset or on unrecoverable I/O errors — device binding is optional.
fn load_or_generate_static_keypair() -> Option<KeyPair> {
    use std::fs;
    #[cfg(unix)]
    use std::os::unix::fs::PermissionsExt;

    let home = dirs_home()?; // skip persistence when HOME is unset
    let dir = home.join(".config").join("aivpn");
    let path = dir.join("device.key");

    if path.exists() {
        match fs::read(&path) {
            Ok(bytes) if bytes.len() == 32 => {
                let mut arr = [0u8; 32];
                arr.copy_from_slice(&bytes);
                return Some(KeyPair::from_private_key(arr));
            }
            Ok(_) => {
                warn!("device.key has wrong size — regenerating");
            }
            Err(e) => {
                warn!("Cannot read device.key: {}", e);
                return None;
            }
        }
    }

    // Generate new keypair and persist atomically with correct permissions from the start.
    let kp = KeyPair::generate();
    let priv_bytes = kp.export_private_key();

    if let Err(e) = fs::create_dir_all(&dir) {
        warn!("Cannot create ~/.config/aivpn: {}", e);
        return Some(kp); // proceed without persistence
    }
    // Tighten directory to owner-only (700) so siblings are not enumerable.
    #[cfg(unix)]
    let _ = fs::set_permissions(&dir, fs::Permissions::from_mode(0o700));

    // Write to a temp sibling atomically, then rename.
    let tmp_path = path.with_extension("tmp");
    let write_result = (|| -> std::io::Result<()> {
        #[cfg(unix)]
        {
            use std::io::Write;
            use std::os::unix::fs::OpenOptionsExt;
            let mut f = std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode(0o600)
                .open(&tmp_path)?;
            f.write_all(&priv_bytes)?;
            f.sync_all()?;
        }
        #[cfg(not(unix))]
        fs::write(&tmp_path, &priv_bytes)?;
        fs::rename(&tmp_path, &path)
    })();

    match write_result {
        Ok(()) => info!("New device keypair generated and saved to {:?}", path),
        Err(e) => {
            warn!("Cannot write device.key: {}", e);
            let _ = fs::remove_file(&tmp_path);
        }
    }
    Some(kp)
}

fn dirs_home() -> Option<std::path::PathBuf> {
    #[cfg(windows)]
    return std::env::var_os("USERPROFILE").map(std::path::PathBuf::from);
    #[cfg(not(windows))]
    std::env::var_os("HOME").map(std::path::PathBuf::from)
}

/// Load or generate the static device keypair and return the base64-encoded public key.
pub fn device_public_key_b64() -> Option<String> {
    use base64::Engine;
    let kp = load_or_generate_static_keypair()?;
    Some(base64::engine::general_purpose::STANDARD.encode(kp.public_key_bytes()))
}
