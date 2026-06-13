//! Site-to-site VPN — connects two or more AIVPN servers so end-hosts on each
//! site can reach each other without any VPN client software.
//!
//! ## How it works
//! Each peer uses the same crypto infrastructure as pool sync (same
//! `sync_key`-derived `SessionKeys`, `AtomicU64` nonce management,
//! tag-window tolerance).  Peers exchange subnet advertisements via
//! `ControlPayload::RouteSync` (0x13) and the gateway installs kernel routes
//! when it receives them.
//!
//! ## server.json
//! ```json
//! {
//!   "site_to_site": {
//!     "local_subnets": ["192.168.1.0/24"],
//!     "peers": [
//!       {
//!         "name": "office-b",
//!         "endpoint": "office-b.example.com:443",
//!         "sync_key": "<base64 32-byte key>",
//!         "remote_subnets": ["192.168.2.0/24"]
//!       }
//!     ]
//!   }
//! }
//! ```

use std::net::{IpAddr, SocketAddr};
use std::str::FromStr;
use std::sync::{
    atomic::{AtomicU64, Ordering},
    Arc, OnceLock,
};
use std::time::Duration;

use base64::Engine as _;
use rand::Rng;
use serde::{Deserialize, Serialize};
use tokio::net::UdpSocket;
use tracing::{info, warn};

/// Caps to bound deserialization and subprocess spawning.
const MAX_SUBNETS_JSON_BYTES: usize = 4096;
const MAX_SUBNETS_PER_MSG: usize = 64;

/// Stored once at startup so `handle_route_sync` can authenticate senders.
static SITE_CONFIG: OnceLock<SiteToSiteConfig> = OnceLock::new();

use aivpn_common::crypto::{
    self, encrypt_payload, SessionKeys, DEFAULT_WINDOW_MS, NONCE_SIZE, TAG_SIZE,
};
use aivpn_common::error::{Error, Result};
use aivpn_common::protocol::{ControlPayload, InnerHeader, InnerType};

const ROUTE_SYNC_INTERVAL: Duration = Duration::from_secs(30);

/// Configuration for one site-to-site peer.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SitePeerConfig {
    pub name: String,
    /// VPN endpoint `host:port` — must be the same port the VPN listens on.
    pub endpoint: String,
    /// 32-byte BLAKE3 key (base64).  Must match the peer's config exactly.
    pub sync_key: String,
    /// Subnets the remote site will advertise; pre-installed at startup.
    #[serde(default)]
    pub remote_subnets: Vec<String>,
}

/// Top-level site-to-site block (`"site_to_site"` in `server.json`).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SiteToSiteConfig {
    /// Local subnets advertised to all peers.
    #[serde(default)]
    pub local_subnets: Vec<String>,
    #[serde(default)]
    pub peers: Vec<SitePeerConfig>,
}

struct SitePeer {
    cfg: SitePeerConfig,
    local_subnets: Vec<String>,
    session_keys: SessionKeys,
    mdh: Vec<u8>,
    send_counter: AtomicU64,
}

impl SitePeer {
    fn new(cfg: SitePeerConfig, local_subnets: Vec<String>, mdh: Vec<u8>) -> Option<Arc<Self>> {
        let raw: [u8; 32] = base64::engine::general_purpose::STANDARD
            .decode(&cfg.sync_key)
            .ok()
            .and_then(|b| b.try_into().ok())?;

        if raw == [0u8; 32] {
            warn!("site_sync: peer {} has zero sync_key — skipped", cfg.name);
            return None;
        }

        let session_keys = SessionKeys {
            session_key: blake3::derive_key("aivpn-pool-enc-v1", &raw),
            tag_secret: blake3::derive_key("aivpn-pool-tag-v1", &raw),
            prng_seed: blake3::derive_key("aivpn-pool-prng-v1", &raw),
        };
        let send_counter = AtomicU64::new(crypto::current_timestamp_ms() / 5_000);

        Some(Arc::new(Self {
            cfg,
            local_subnets,
            session_keys,
            mdh,
            send_counter,
        }))
    }

    fn start(self: Arc<Self>) {
        // Pre-install statically-declared remote routes.
        for subnet in &self.cfg.remote_subnets {
            install_route(subnet, &self.cfg.endpoint);
        }
        tokio::spawn(async move { self.outbound_loop().await });
    }

    async fn outbound_loop(self: Arc<Self>) {
        let peer_addr: SocketAddr = match self.cfg.endpoint.parse() {
            Ok(a) => a,
            Err(_) => {
                warn!("site_sync: invalid endpoint: {}", self.cfg.endpoint);
                return;
            }
        };

        let socket = match UdpSocket::bind("0.0.0.0:0").await {
            Ok(s) => s,
            Err(e) => {
                warn!("site_sync: bind failed for {}: {}", self.cfg.name, e);
                return;
            }
        };

        info!(
            "site_sync: peer {} → {} ({} local subnets)",
            self.cfg.name,
            self.cfg.endpoint,
            self.local_subnets.len()
        );

        let mut ticker = tokio::time::interval(ROUTE_SYNC_INTERVAL);
        loop {
            ticker.tick().await;
            if let Err(e) = self.send_advert(&socket, peer_addr).await {
                warn!("site_sync: send to {} failed: {}", self.cfg.name, e);
            }
        }
    }

    async fn send_advert(&self, socket: &UdpSocket, peer: SocketAddr) -> Result<()> {
        let subnets_json = serde_json::to_vec(&self.local_subnets)
            .map_err(|e| Error::Session(format!("site_sync serialize: {}", e)))?;
        let pkt = self.build_packet(&ControlPayload::RouteSync { subnets_json })?;
        socket
            .send_to(&pkt, peer)
            .await
            .map_err(|e| Error::Session(format!("site_sync udp: {}", e)))?;
        Ok(())
    }

    fn build_packet(&self, payload: &ControlPayload) -> Result<Vec<u8>> {
        let encoded = payload.encode()?;
        let mut inner = InnerHeader {
            inner_type: InnerType::Control,
            seq_num: 0,
        }
        .encode()
        .to_vec();
        inner.extend_from_slice(&encoded);

        let counter = self.send_counter.fetch_add(1, Ordering::Relaxed);
        let mut nonce = [0u8; NONCE_SIZE];
        nonce[..8].copy_from_slice(&counter.to_le_bytes());

        let pad_len: u16 = 16;
        let mut padded = Vec::with_capacity(2 + inner.len() + pad_len as usize);
        padded.extend_from_slice(&pad_len.to_le_bytes());
        padded.extend_from_slice(&inner);
        let mut rng = rand::thread_rng();
        for _ in 0..pad_len {
            padded.push(rng.gen::<u8>());
        }

        let ciphertext = encrypt_payload(&self.session_keys.session_key, &nonce, &padded)?;
        let tw = crypto::compute_time_window(crypto::current_timestamp_ms(), DEFAULT_WINDOW_MS);
        let tag = crypto::generate_resonance_tag(&self.session_keys.tag_secret, counter, tw);

        let mut pkt = Vec::with_capacity(TAG_SIZE + self.mdh.len() + ciphertext.len());
        pkt.extend_from_slice(&tag);
        pkt.extend_from_slice(&self.mdh);
        pkt.extend_from_slice(&ciphertext);
        Ok(pkt)
    }
}

/// Start outbound route advertisement to all configured peers.
pub fn start(config: &SiteToSiteConfig, mdh: Vec<u8>) {
    // Store config so handle_route_sync can authenticate inbound messages.
    SITE_CONFIG.get_or_init(|| config.clone());

    for peer_cfg in &config.peers {
        if let Some(peer) = SitePeer::new(peer_cfg.clone(), config.local_subnets.clone(), mdh.clone()) {
            peer.start();
        }
    }
    info!(
        "site_sync: active ({} peers, {} local subnets)",
        config.peers.len(),
        config.local_subnets.len()
    );
}

/// Handle an incoming `RouteSync` control message.
///
/// `from_addr` must be the full `IP:port` string of the sending socket so we
/// can match it against configured peer endpoints.  Any of these conditions
/// causes a silent drop with a warning:
/// - No site-to-site config loaded
/// - Sender not in the configured peers list
/// - Payload exceeds 4 KiB
/// - More than 64 subnets in one message
/// - A subnet that is not in the peer's declared `remote_subnets` allowlist
/// - A subnet that is a default route, loopback, or link-local prefix
pub fn handle_route_sync(subnets_json: &[u8], from_addr: &str) {
    // 1. Authenticate: sender must be a configured peer endpoint.
    let config = match SITE_CONFIG.get() {
        Some(c) => c,
        None => {
            warn!("site_sync: RouteSync received but site-to-site not configured — dropping");
            return;
        }
    };
    let peer_cfg = match config.peers.iter().find(|p| p.endpoint == from_addr) {
        Some(p) => p,
        None => {
            warn!("site_sync: RouteSync from unconfigured peer {} — dropping", from_addr);
            return;
        }
    };

    // 2. Size guard before deserialization.
    if subnets_json.len() > MAX_SUBNETS_JSON_BYTES {
        warn!(
            "site_sync: RouteSync payload too large ({} bytes) from {} — dropping",
            subnets_json.len(),
            peer_cfg.name
        );
        return;
    }

    let subnets: Vec<String> = match serde_json::from_slice(subnets_json) {
        Ok(v) => v,
        Err(e) => {
            warn!("site_sync: invalid RouteSync payload from {}: {}", peer_cfg.name, e);
            return;
        }
    };

    if subnets.len() > MAX_SUBNETS_PER_MSG {
        warn!(
            "site_sync: RouteSync has {} subnets (max {}) from {} — dropping",
            subnets.len(),
            MAX_SUBNETS_PER_MSG,
            peer_cfg.name
        );
        return;
    }

    for subnet_str in &subnets {
        // 3. Allowlist: only install routes the peer is declared to advertise.
        if !peer_cfg.remote_subnets.iter().any(|a| a == subnet_str) {
            warn!(
                "site_sync: subnet {} not in allowlist for peer {} — skipped",
                subnet_str, peer_cfg.name
            );
            continue;
        }
        // 4. Safety: reject dangerous prefixes.
        if !is_safe_subnet(subnet_str) {
            warn!(
                "site_sync: unsafe subnet {} from peer {} — skipped",
                subnet_str, peer_cfg.name
            );
            continue;
        }
        info!("site_sync: installing route {} (peer: {})", subnet_str, peer_cfg.name);
        install_route(subnet_str, from_addr);
    }
}

/// Reject default routes, loopback, and link-local prefixes.
fn is_safe_subnet(s: &str) -> bool {
    let (addr_str, prefix_len) = match s.split_once('/') {
        Some((a, p)) => (a, p.parse::<u8>().unwrap_or(0)),
        None => return false,
    };
    if prefix_len == 0 {
        return false; // 0.0.0.0/0 or ::/0 — default route
    }
    let addr = match IpAddr::from_str(addr_str) {
        Ok(a) => a,
        Err(_) => return false,
    };
    if addr.is_loopback() {
        return false;
    }
    match addr {
        IpAddr::V4(v4) => !v4.is_link_local(),
        IpAddr::V6(v6) => (v6.segments()[0] & 0xffc0) != 0xfe80,
    }
}

/// Install a kernel route via `ip route add`.
fn install_route(subnet: &str, via: &str) {
    let gateway = via.split(':').next().unwrap_or(via);
    match std::process::Command::new("ip")
        .args(["route", "add", subnet, "via", gateway])
        .status()
    {
        Ok(s) if s.success() => info!("site_sync: route {} via {} OK", subnet, gateway),
        Ok(s) => warn!("site_sync: ip route add exit {}", s),
        Err(e) => warn!("site_sync: ip route add: {}", e),
    }
}
