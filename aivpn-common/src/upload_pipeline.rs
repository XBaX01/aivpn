//! Shared upload pipeline for AIVPN clients.
//!
//! Both the CLI client and Android core use this module to avoid duplicating
//! the biased-select + burst-drain + keepalive upload loop.

use std::sync::Arc;
use std::time::Duration;

use tokio::net::UdpSocket;
use tokio::sync::mpsc;
use tokio::time;

use crate::client_wire::{build_inner_packet, build_zero_mdh_packet};
use crate::crypto::SessionKeys;
use crate::error::{Error, Result};
use crate::protocol::{ControlPayload, InnerType};

// ──────────── Configuration ────────────

/// Tuneable knobs for the upload pipeline shared by all clients.
pub struct UploadConfig {
    /// Maximum additional packets to drain from the channel after the first
    /// recv without yielding back to the async executor.
    pub burst_size: usize,
    /// How often a keepalive is sent when there is no data traffic.
    pub keepalive_interval: Duration,
}

impl Default for UploadConfig {
    fn default() -> Self {
        Self {
            burst_size: 63,
            keepalive_interval: Duration::from_secs(25),
        }
    }
}

// ──────────── Trait: pluggable packet encryption ────────────

/// Platform-specific packet encryption and framing.
///
/// The CLI client implements this via its MimicryEngine (variable MDH,
/// traffic-shaped padding, FSM updates). Android implements it via
/// [`ZeroMdhEncryptor`] (fixed zero-length MDH, random padding).
pub trait PacketEncryptor: Send {
    /// Encrypt a TUN data payload into a ready-to-send UDP datagram.
    fn encrypt_data(&mut self, payload: &[u8]) -> Result<Vec<u8>>;
    /// Encrypt a keepalive control message into a ready-to-send UDP datagram.
    fn encrypt_keepalive(&mut self) -> Result<Vec<u8>>;
    /// Called after a data datagram has been successfully sent.
    /// Use this for stats tracking, FSM transitions, etc.
    fn on_data_sent(&mut self, payload_len: usize);
}

// ──────────── Ready-made encryptor: zero MDH ────────────

/// Encryptor using `build_zero_mdh_packet` — suitable for Android and any
/// client that does not require Mimicry traffic shaping.
pub struct ZeroMdhEncryptor {
    keys: SessionKeys,
    counter: u64,
    seq: u16,
}

impl ZeroMdhEncryptor {
    pub fn new(keys: SessionKeys, counter: u64, seq: u16) -> Self {
        Self { keys, counter, seq }
    }
}

impl PacketEncryptor for ZeroMdhEncryptor {
    fn encrypt_data(&mut self, payload: &[u8]) -> Result<Vec<u8>> {
        let inner = build_inner_packet(InnerType::Data, self.seq, payload);
        self.seq = self.seq.wrapping_add(1);
        build_zero_mdh_packet(&self.keys, &mut self.counter, &inner, None)
    }

    fn encrypt_keepalive(&mut self) -> Result<Vec<u8>> {
        let keepalive = ControlPayload::Keepalive.encode()?;
        let inner = build_inner_packet(InnerType::Control, self.seq, &keepalive);
        self.seq = self.seq.wrapping_add(1);
        build_zero_mdh_packet(&self.keys, &mut self.counter, &inner, None)
    }

    fn on_data_sent(&mut self, _payload_len: usize) {}
}

// ──────────── The upload loop ────────────

/// Run the upload loop: pull TUN packets from `rx`, encrypt via `enc`, send
/// over `udp`. Uses biased `select!` to prioritise data over keepalives and a
/// burst-drain after the first recv to amortise per-packet scheduler overhead.
///
/// Returns `Err` on fatal I/O or channel close. Never returns `Ok` — the
/// caller is expected to `.abort()` the task when the session ends.
pub async fn run_upload_loop(
    rx: &mut mpsc::Receiver<Vec<u8>>,
    udp: &Arc<UdpSocket>,
    enc: &mut impl PacketEncryptor,
    config: &UploadConfig,
) -> Result<()> {
    let mut ka_interval = time::interval(config.keepalive_interval);
    ka_interval.tick().await; // skip the immediate first tick

    loop {
        tokio::select! {
            biased;

            // ── Data path (highest priority) ──
            maybe_pkt = rx.recv() => {
                let pkt_data = match maybe_pkt {
                    Some(p) => p,
                    None => return Err(Error::Channel("TUN->UDP channel closed".into())),
                };

                let encrypted = enc.encrypt_data(&pkt_data)?;
                udp.send(&encrypted).await.map_err(Error::Io)?;
                enc.on_data_sent(pkt_data.len());

                // Burst drain: process up to burst_size without yielding
                for _ in 0..config.burst_size {
                    match rx.try_recv() {
                        Ok(pkt) => {
                            let encrypted = enc.encrypt_data(&pkt)?;
                            udp.send(&encrypted).await.map_err(Error::Io)?;
                            enc.on_data_sent(pkt.len());
                        }
                        Err(mpsc::error::TryRecvError::Empty) => break,
                        Err(mpsc::error::TryRecvError::Disconnected) => {
                            return Err(Error::Channel("TUN->UDP channel closed".into()));
                        }
                    }
                }
            }

            // ── Keepalive (fires only when data path is idle) ──
            _ = ka_interval.tick() => {
                let encrypted = enc.encrypt_keepalive()?;
                udp.send(&encrypted).await.map_err(Error::Io)?;
            }
        }
    }
}
