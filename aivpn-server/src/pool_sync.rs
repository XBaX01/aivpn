//! Server pool synchronisation — keeps clients.json in sync across pool nodes.
//!
//! ## Design
//! Pool nodes share a secret `sync_key` (32-byte BLAKE3 key, base64 in server.json).
//! From this key every node derives identical `SessionKeys` and registers a
//! *synthetic cluster session* in the `SessionManager`.  Outbound sync packets are
//! standard VPN UDP datagrams (8-byte resonance tag + MDH + ChaCha20-Poly1305
//! ciphertext) sent to the peer's VPN port — indistinguishable from client traffic.
//!
//! Incoming PoolSync packets are processed by the gateway's normal receive loop
//! and dispatched to `handle_control_message` as `ControlPayload::PoolSync`.
//!
//! ## Replay protection
//! The resonance counter is set to `unix_ms / 5_000` (5-second buckets).
//! Both sender and receiver independently compute the same counter for the same
//! wall-clock window.  The 511-counter tag window covers ±42 minutes of clock
//! drift, making time-based replay attacks impractical.

use std::net::SocketAddr;
use std::sync::{
    atomic::{AtomicU64, Ordering},
    Arc,
};
use std::time::Duration;

use rand::Rng;
use tracing::{info, warn};

use tokio::net::UdpSocket;

use aivpn_common::crypto::{
    self, encrypt_payload, SessionKeys, DEFAULT_WINDOW_MS, NONCE_SIZE, TAG_SIZE,
};
use aivpn_common::error::{Error, Result};
use aivpn_common::event_log::{AivpnEvent, EventBus, PeerSyncAction};
use aivpn_common::protocol::{ControlPayload, InnerHeader, InnerType};
use base64::Engine as _;

use crate::client_db::ClientDatabase;
use crate::session::SessionManager;

const SYNC_INTERVAL: Duration = Duration::from_secs(5);
const TAG_REFRESH_INTERVAL: Duration = Duration::from_secs(60);

/// Pool configuration stored in server.json under `"pool"`.
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct PoolSyncConfig {
    /// Peer server addresses in `host:vpn_port` format (same port clients use).
    #[serde(default)]
    pub peers: Vec<String>,
    /// Deprecated — pool sync now uses the VPN port directly, not a separate port.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sync_port: Option<u16>,
    /// 32-byte BLAKE3 key (base64).  All pool nodes must share this exact value.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sync_key: Option<String>,
    /// Optional exit node for multi-hop routing (`host:port`).  When set,
    /// the entry node wraps client IP payloads in `ChainForward` and relays
    /// them to this address; the exit node routes to the internet.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exit_node: Option<String>,
    /// Set to `true` on the node that acts as the exit point for multi-hop
    /// traffic.  When `false` (default), incoming `ChainForward` messages are
    /// rejected, preventing this node from being used as an open relay.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exit_node_enabled: Option<bool>,
}

/// Manages outbound peer synchronisation using the main VPN protocol.
pub struct PeerSyncer {
    db: Arc<ClientDatabase>,
    peers: Vec<String>,
    sync_key: [u8; 32],
    session_keys: SessionKeys,
    mdh: Vec<u8>,
    events: EventBus,
    /// Strictly-monotonic per-node send counter.  Initialised to the current
    /// 5-second time bucket so the value starts inside the receiver's tag window,
    /// then incremented atomically for every outbound packet.  This guarantees a
    /// unique (key, nonce) pair per message and prevents nonce reuse under
    /// ChaCha20-Poly1305.
    send_counter: AtomicU64,
}

impl PeerSyncer {
    /// Returns `None` if `sync_key` is absent or zero (sync disabled for safety).
    pub fn new(
        db: Arc<ClientDatabase>,
        config: &PoolSyncConfig,
        mdh: Vec<u8>,
        events: EventBus,
    ) -> Option<Arc<Self>> {
        let sync_key: [u8; 32] = config
            .sync_key
            .as_deref()
            .and_then(|k| base64::engine::general_purpose::STANDARD.decode(k).ok())
            .and_then(|b| b.try_into().ok())
            .unwrap_or([0u8; 32]);

        if sync_key == [0u8; 32] {
            warn!("pool_sync: sync_key not configured — pool sync disabled for security");
            return None;
        }

        let session_keys = SessionKeys {
            session_key: blake3::derive_key("aivpn-pool-enc-v1", &sync_key),
            tag_secret: blake3::derive_key("aivpn-pool-tag-v1", &sync_key),
            prng_seed: blake3::derive_key("aivpn-pool-prng-v1", &sync_key),
        };

        // Seed the counter at the current time bucket so it starts inside the
        // receiver's expected-tag window; each send increments it atomically.
        let send_counter = AtomicU64::new(crypto::current_timestamp_ms() / 5_000);

        Some(Arc::new(Self {
            db,
            peers: config.peers.clone(),
            sync_key,
            session_keys,
            mdh,
            events,
            send_counter,
        }))
    }

    /// Register the synthetic cluster session and spawn background tasks.
    pub fn start(self: Arc<Self>, session_manager: Arc<SessionManager>) {
        // Sentinel addr — the cluster session has no single client addr; incoming
        // packets arrive from various peer IPs and are matched by resonance tag.
        let sentinel: SocketAddr = "0.0.0.0:0".parse().unwrap();
        let session_id = session_manager.create_pool_peer_session(&self.sync_key, sentinel);
        info!("pool_sync: active ({} peers)", self.peers.len());

        // Periodically refresh the time-bucket counter in the tag window.
        let sm = session_manager.clone();
        tokio::spawn(async move {
            let mut ticker = tokio::time::interval(TAG_REFRESH_INTERVAL);
            loop {
                ticker.tick().await;
                sm.refresh_pool_peer_tags(&session_id);
            }
        });

        // One outbound loop per configured peer.
        for peer in self.peers.clone() {
            let me = self.clone();
            tokio::spawn(async move {
                me.outbound_loop(peer).await;
            });
        }
    }

    async fn outbound_loop(self: Arc<Self>, peer: String) {
        let socket = match UdpSocket::bind("0.0.0.0:0").await {
            Ok(s) => s,
            Err(e) => {
                warn!("pool_sync: bind failed for peer {}: {}", peer, e);
                return;
            }
        };

        let mut ticker = tokio::time::interval(SYNC_INTERVAL);
        loop {
            ticker.tick().await;
            match self.push_to_peer(&socket, &peer).await {
                Ok(n) => {
                    self.events.emit(AivpnEvent::PeerSync {
                        peer: peer.clone(),
                        action: PeerSyncAction::FullSync,
                        clients_synced: n as u32,
                    });
                }
                Err(e) => warn!("pool_sync: send to {} failed: {}", peer, e),
            }
        }
    }

    async fn push_to_peer(&self, socket: &UdpSocket, peer: &str) -> Result<usize> {
        let peer_addr: SocketAddr = peer
            .parse()
            .map_err(|_| Error::Session(format!("pool_sync: invalid peer addr: {}", peer)))?;

        let clients = self.db.list_clients();
        let n = clients.len();
        let clients_json = serde_json::to_vec(&clients)
            .map_err(|e| Error::Session(format!("pool_sync: serialize: {}", e)))?;

        let payload = ControlPayload::PoolSync { clients_json };
        let packet = self.build_sync_packet(&payload)?;
        socket
            .send_to(&packet, peer_addr)
            .await
            .map_err(|e| Error::Session(format!("pool_sync: udp send: {}", e)))?;
        Ok(n)
    }

    /// Build a VPN UDP packet carrying a PoolSync control message.
    /// Wire format: [8-byte resonance tag][MDH][ChaCha20-Poly1305 ciphertext]
    fn build_sync_packet(&self, payload: &ControlPayload) -> Result<Vec<u8>> {
        let encoded = payload.encode()?;

        let inner_header = InnerHeader {
            inner_type: InnerType::Control,
            seq_num: 0,
        };
        let mut inner_payload = inner_header.encode().to_vec();
        inner_payload.extend_from_slice(&encoded);

        // Atomically increment the per-node send counter.  Initialised near
        // the current 5-second time bucket so the receiver's tag window covers
        // it; incrementing per-packet guarantees a unique (key, nonce) pair for
        // every message, preventing ChaCha20-Poly1305 nonce reuse.
        let counter = self.send_counter.fetch_add(1, Ordering::Relaxed);
        let mut nonce = [0u8; NONCE_SIZE];
        nonce[..8].copy_from_slice(&counter.to_le_bytes());

        let pad_len: u16 = 16;
        let mut padded = Vec::with_capacity(2 + inner_payload.len() + pad_len as usize);
        padded.extend_from_slice(&pad_len.to_le_bytes());
        padded.extend_from_slice(&inner_payload);
        let mut rng = rand::thread_rng();
        for _ in 0..pad_len {
            padded.push(rng.gen::<u8>());
        }

        let ciphertext = encrypt_payload(&self.session_keys.session_key, &nonce, &padded)?;

        let time_window =
            crypto::compute_time_window(crypto::current_timestamp_ms(), DEFAULT_WINDOW_MS);
        let tag =
            crypto::generate_resonance_tag(&self.session_keys.tag_secret, counter, time_window);

        let mut packet = Vec::with_capacity(TAG_SIZE + self.mdh.len() + ciphertext.len());
        packet.extend_from_slice(&tag);
        packet.extend_from_slice(&self.mdh);
        packet.extend_from_slice(&ciphertext);
        Ok(packet)
    }
}
