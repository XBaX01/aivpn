//! Shared upload pipeline for AIVPN clients.
//!
//! Both the CLI client and Android core use this module to avoid duplicating
//! the biased-select + burst-drain + keepalive upload loop.

use std::sync::Arc;
use std::time::Duration;

use crate::client_wire::{build_inner_packet, build_random_mdh_packet, DEFAULT_MDH_LEN};
use crate::fec::FecEncoder;
use crate::crypto::SessionKeys;
use crate::error::{Error, Result};
use crate::protocol::{ControlPayload, InnerType};
use tokio::net::UdpSocket;
use tokio::sync::mpsc;
use tokio::time;

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
    /// Encrypt an arbitrary control message into a ready-to-send UDP datagram.
    fn encrypt_control(&mut self, payload: &ControlPayload) -> Result<Vec<u8>>;
    /// Encrypt a keepalive control message into a ready-to-send UDP datagram.
    fn encrypt_keepalive(&mut self) -> Result<Vec<u8>>;
    /// Called after a data datagram has been successfully sent.
    /// Use this for stats tracking, FSM transitions, etc.
    fn on_data_sent(&mut self, payload_len: usize);
    /// Return a pre-encrypted FEC repair datagram if one is ready, else None.
    /// Called after every data send; default is a no-op.
    fn take_fec_repair(&mut self) -> Option<Vec<u8>> {
        None
    }
    /// Encrypt a keepalive stamped with `send_ts` (milliseconds since UNIX epoch).
    /// The server echoes `send_ts` in `KeepaliveAck` so the client can measure RTT.
    /// Default delegates to `encrypt_keepalive()` with `send_ts = 0` (no RTT tracking).
    fn encrypt_keepalive_ts(&mut self, _send_ts: u64) -> Result<Vec<u8>> {
        self.encrypt_keepalive()
    }
}

// ──────────── Ready-made encryptor: zero MDH ────────────

/// Encryptor using random MDH — suitable for Android and any
/// client that does not require Mimicry traffic shaping.
/// Each packet gets fresh random MDH bytes (Issue #30 fix).
pub struct ZeroMdhEncryptor {
    keys: SessionKeys,
    counter: u64,
    seq: u16,
    mdh_len: usize,
    fec_encoder: Option<FecEncoder>,
    pending_fec: Option<Vec<u8>>,
}

impl ZeroMdhEncryptor {
    pub fn new(keys: SessionKeys, counter: u64, seq: u16) -> Self {
        Self {
            keys,
            counter,
            seq,
            mdh_len: DEFAULT_MDH_LEN,
            fec_encoder: None,
            pending_fec: None,
        }
    }

    pub fn with_mdh_len(keys: SessionKeys, counter: u64, seq: u16, mdh_len: usize) -> Self {
        Self {
            keys,
            counter,
            seq,
            mdh_len,
            fec_encoder: None,
            pending_fec: None,
        }
    }

    /// Enable XOR FEC with the given group size. group_size=0 disables FEC.
    pub fn set_fec_group(&mut self, group_size: u8) {
        self.fec_encoder = if group_size > 0 {
            Some(FecEncoder::new(group_size, 1500))
        } else {
            None
        };
    }
}

impl PacketEncryptor for ZeroMdhEncryptor {
    fn encrypt_data(&mut self, payload: &[u8]) -> Result<Vec<u8>> {
        let inner = build_inner_packet(InnerType::Data, self.seq, payload);
        self.seq = self.seq.wrapping_add(1);
        let pkt = build_random_mdh_packet(&self.keys, &mut self.counter, &inner, None, self.mdh_len)?;
        if let Some(fec) = self.fec_encoder.as_mut() {
            if let Some(repair) = fec.feed(payload) {
                let repair_inner =
                    build_inner_packet(InnerType::FecRepair, self.seq, &repair.encode());
                self.seq = self.seq.wrapping_add(1);
                if let Ok(enc) = build_random_mdh_packet(
                    &self.keys, &mut self.counter, &repair_inner, None, self.mdh_len,
                ) {
                    self.pending_fec = Some(enc);
                }
            }
        }
        Ok(pkt)
    }

    fn encrypt_control(&mut self, payload: &ControlPayload) -> Result<Vec<u8>> {
        let bytes = payload.encode()?;
        let inner = build_inner_packet(InnerType::Control, self.seq, &bytes);
        self.seq = self.seq.wrapping_add(1);
        build_random_mdh_packet(&self.keys, &mut self.counter, &inner, None, self.mdh_len)
    }

    fn encrypt_keepalive(&mut self) -> Result<Vec<u8>> {
        self.encrypt_keepalive_ts(0)
    }

    fn encrypt_keepalive_ts(&mut self, send_ts: u64) -> Result<Vec<u8>> {
        let keepalive = ControlPayload::Keepalive { send_ts }.encode()?;
        let inner = build_inner_packet(InnerType::Control, self.seq, &keepalive);
        self.seq = self.seq.wrapping_add(1);
        build_random_mdh_packet(&self.keys, &mut self.counter, &inner, None, self.mdh_len)
    }

    fn on_data_sent(&mut self, _payload_len: usize) {}

    fn take_fec_repair(&mut self) -> Option<Vec<u8>> {
        self.pending_fec.take()
    }
}

// ──────────── The upload loop ────────────

/// Returns true for transient OS-level errors where retrying immediately
/// or just dropping the packet is safer than triggering a full reconnect.
fn is_transient_send_error(e: &std::io::Error) -> bool {
    use std::io::ErrorKind::*;
    matches!(
        e.kind(),
        NetworkUnreachable | HostUnreachable | NetworkDown | AddrNotAvailable | Interrupted
    )
}

/// Send helper that tolerates transient network errors (e.g. mid-switch on mobile).
/// Returns Ok(()) on success or transient error (logged, packet dropped).
/// Returns Err only on fatal errors (e.g. EBADF = socket closed).
async fn send_tolerant(udp: &UdpSocket, data: &[u8]) -> Result<()> {
    match udp.send(data).await {
        Ok(_) => Ok(()),
        Err(e) if is_transient_send_error(&e) => {
            tracing::debug!("upload: transient send error (dropped packet): {e}");
            Ok(())
        }
        Err(e) => Err(Error::Io(e)),
    }
}

/// Run the upload loop: pull TUN packets from `rx`, encrypt via `enc`, send
/// over `udp`. Uses biased `select!` to prioritise data over keepalives and a
/// burst-drain after the first recv to amortise per-packet scheduler overhead.
///
/// Returns `Err` on fatal I/O or channel close. Never returns `Ok` — the
/// caller is expected to `.abort()` the task when the session ends.
pub async fn run_upload_loop(
    rx: &mut mpsc::Receiver<Vec<u8>>,
    mut control_rx: Option<&mut mpsc::Receiver<ControlPayload>>,
    udp: &Arc<UdpSocket>,
    enc: &mut impl PacketEncryptor,
    config: &UploadConfig,
) -> Result<()> {
    let mut ka_interval = time::interval_at(tokio::time::Instant::now(), config.keepalive_interval);
    let mut data_packet_count: u64 = 0;

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
                send_tolerant(udp, &encrypted).await?;
                if let Some(repair) = enc.take_fec_repair() {
                    send_tolerant(udp, &repair).await?;
                }
                data_packet_count = data_packet_count.wrapping_add(1);
                enc.on_data_sent(pkt_data.len());

                // Burst drain: process up to burst_size without yielding
                for _ in 0..config.burst_size {
                    match rx.try_recv() {
                        Ok(pkt) => {
                            let encrypted = enc.encrypt_data(&pkt)?;
                            send_tolerant(udp, &encrypted).await?;
                            if let Some(repair) = enc.take_fec_repair() {
                                send_tolerant(udp, &repair).await?;
                            }
                            data_packet_count = data_packet_count.wrapping_add(1);
                            enc.on_data_sent(pkt.len());
                        }
                        Err(mpsc::error::TryRecvError::Empty) => break,
                        Err(mpsc::error::TryRecvError::Disconnected) => {
                            return Err(Error::Channel("TUN->UDP channel closed".into()));
                        }
                    }
                }
                // Suppress the next keepalive tick: a keepalive immediately after
                // real data wastes bandwidth and the server's ACK resets the peer's
                // rx-silence timer anyway.
                ka_interval.reset();
            }

            // ── Keepalive (fires only when data path is idle) ──
            _ = ka_interval.tick() => {
                let encrypted = enc.encrypt_keepalive()?;
                send_tolerant(udp, &encrypted).await?;
            }

            // ── Control payloads ──
            maybe_ctrl = async {
                if let Some(crx) = control_rx.as_mut() {
                    crx.recv().await
                } else {
                    std::future::pending().await
                }
            } => {
                if let Some(payload) = maybe_ctrl {
                    let encrypted = enc.encrypt_control(&payload)?;
                    send_tolerant(udp, &encrypted).await?;
                }
            }
        }
    }
}
