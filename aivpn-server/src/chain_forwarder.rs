//! Multi-hop chain forwarder — entry node side.
//!
//! When `pool.exit_node` is set in `server.json`, the gateway wraps every
//! decrypted client IP payload in a `ControlPayload::ChainForward` packet and
//! sends it to the exit node via the shared pool session.  The exit node's
//! gateway receives the packet, decrypts it (same pool session keys), and
//! injects the IP payload into its TUN interface — routing it to the internet
//! on behalf of the origin client.
//!
//! From the client's perspective nothing changes: it only ever speaks to the
//! entry node.  The exit IP address the internet sees belongs to the exit node.
//!
//! ## server.json (entry node only)
//! ```json
//! {
//!   "pool": {
//!     "sync_key": "<base64 32-byte key>",
//!     "exit_node": "exit.example.com:443"
//!   }
//! }
//! ```
//!
//! The exit node needs no additional config — its gateway already handles
//! `ChainForward` in the `handle_control_message()` match.

use std::net::SocketAddr;
use std::sync::{
    atomic::{AtomicU64, Ordering},
    Arc,
};

use rand::{rngs::OsRng, Rng, RngCore};
use tokio::net::UdpSocket;
use tracing::debug;

use aivpn_common::crypto::{
    self, encrypt_payload, SessionKeys, DEFAULT_WINDOW_MS, NONCE_SIZE, TAG_SIZE,
};
use aivpn_common::error::Result;
use aivpn_common::protocol::{ControlPayload, InnerHeader, InnerType};

/// Forwards IP payloads to the designated exit node as `ChainForward` control packets.
pub struct ChainForwarder {
    exit_addr: SocketAddr,
    session_keys: SessionKeys,
    mdh: Vec<u8>,
    send_counter: AtomicU64,
    socket: UdpSocket,
}

impl ChainForwarder {
    /// Create a forwarder.  Returns `None` if `sync_key` is all-zero or the
    /// bind fails.
    pub async fn new(exit_node: &str, sync_key: [u8; 32], mdh: Vec<u8>) -> Option<Arc<Self>> {
        if sync_key == [0u8; 32] {
            return None;
        }
        let exit_addr: SocketAddr = exit_node.parse().ok()?;
        let socket = UdpSocket::bind("0.0.0.0:0").await.ok()?;
        let session_keys = SessionKeys {
            session_key: blake3::derive_key("aivpn-pool-enc-v1", &sync_key),
            tag_secret: blake3::derive_key("aivpn-pool-tag-v1", &sync_key),
            prng_seed: blake3::derive_key("aivpn-pool-prng-v1", &sync_key),
        };
        // Counter is used only for resonance tag generation, not as AEAD nonce.
        let send_counter = AtomicU64::new(0);
        Some(Arc::new(Self {
            exit_addr,
            session_keys,
            mdh,
            send_counter,
            socket,
        }))
    }

    /// Wrap `ip_payload` in a `ChainForward` control packet and transmit to the exit node.
    pub async fn forward(&self, ip_payload: Vec<u8>) {
        match self.build_packet(ip_payload) {
            Ok(pkt) => {
                if let Err(e) = self.socket.send_to(&pkt, self.exit_addr).await {
                    debug!("chain_forward: send to {} failed: {}", self.exit_addr, e);
                }
            }
            Err(e) => debug!("chain_forward: build packet failed: {}", e),
        }
    }

    fn build_packet(&self, ip_payload: Vec<u8>) -> Result<Vec<u8>> {
        let encoded = ControlPayload::ChainForward { payload: ip_payload }.encode()?;
        let mut inner = InnerHeader {
            inner_type: InnerType::Control,
            seq_num: 0,
        }
        .encode()
        .to_vec();
        inner.extend_from_slice(&encoded);

        let counter = self.send_counter.fetch_add(1, Ordering::Relaxed);
        // Use a fresh random nonce for each packet — safe for ChaCha20-Poly1305
        // at any realistic packet rate (2^-32 collision probability per 2^32 pkts).
        // The counter is kept only for resonance tag generation below.
        let mut nonce = [0u8; NONCE_SIZE];
        OsRng.fill_bytes(&mut nonce);

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
