//! Mimicry Engine
//!
//! Shapes traffic to match Mask profile characteristics.
//! Also provides [`MimicryEncryptor`], a [`crate::upload_pipeline::PacketEncryptor`]
//! implementation for mobile cores (iOS, Android) that wraps MimicryEngine with
//! session-key storage and an atomic pending-mask slot.

use rand::rngs::StdRng;
use rand::{RngCore, SeedableRng};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tracing::debug;

use crate::client_wire::build_inner_packet;
use crate::crypto::{self, encrypt_payload, SessionKeys, NONCE_SIZE, POLY1305_TAG_SIZE, TAG_SIZE};
use crate::error::Result;
use crate::fec::FecEncoder;
use crate::mask::{IATDistribution, MaskProfile, PaddingStrategy, SizeDistribution, SpoofProtocol};
use crate::protocol::{ControlPayload, InnerType, MAX_PACKET_SIZE};
use crate::upload_pipeline::PacketEncryptor;

const SAFE_OUTER_PACKET_BUDGET: usize = 1380;

// ──────────── MimicryState ────────────

pub struct MimicryState {
    pub current_state: u16,
    pub packets_in_state: u32,
    pub state_start: Instant,
    pub size_override: Option<SizeDistribution>,
    pub iat_override: Option<IATDistribution>,
    pub padding_override: Option<PaddingStrategy>,
}

// ──────────── MimicryEngine ────────────

pub struct MimicryEngine {
    mask: MaskProfile,
    state: MimicryState,
    rng: StdRng,
}

impl MimicryEngine {
    pub fn new(mask: MaskProfile) -> Self {
        let initial_state = mask.initial_state();
        Self {
            mask,
            state: MimicryState {
                current_state: initial_state,
                packets_in_state: 0,
                state_start: Instant::now(),
                size_override: None,
                iat_override: None,
                padding_override: None,
            },
            rng: StdRng::from_entropy(),
        }
    }

    pub fn mask(&self) -> &MaskProfile {
        &self.mask
    }

    pub fn update_mask(&mut self, new_mask: MaskProfile) {
        debug!("Updating mask to {}", new_mask.mask_id);
        self.mask = new_mask;
        self.state = MimicryState {
            current_state: self.mask.initial_state(),
            packets_in_state: 0,
            state_start: Instant::now(),
            size_override: None,
            iat_override: None,
            padding_override: None,
        };
    }

    pub fn sample_packet_size(&mut self) -> u16 {
        if let Some(ref d) = self.state.size_override {
            d.sample(&mut self.rng)
        } else {
            self.mask.size_distribution.sample(&mut self.rng)
        }
    }

    pub fn sample_iat(&mut self) -> f64 {
        if let Some(ref d) = self.state.iat_override {
            d.sample(&mut self.rng)
        } else {
            self.mask.iat_distribution.sample(&mut self.rng)
        }
    }

    pub fn calc_padding(&mut self, payload_size: usize, target_size: u16) -> u16 {
        let strategy = self
            .state
            .padding_override
            .as_ref()
            .unwrap_or(&self.mask.padding_strategy);
        strategy.calc_padding(payload_size, target_size, &mut self.rng)
    }

    pub async fn apply_timing(&mut self) {
        let iat_ms = self.sample_iat();
        if iat_ms > 0.0 {
            tokio::time::sleep(Duration::from_secs_f64(iat_ms / 1000.0)).await;
        }
    }

    pub fn update_fsm(&mut self) {
        let duration_ms = self.state.state_start.elapsed().as_millis() as u64;
        let (new_state, size_override, iat_override, padding_override) =
            self.mask.process_transition(
                self.state.current_state,
                self.state.packets_in_state,
                duration_ms,
            );
        if new_state != self.state.current_state {
            debug!(
                "FSM transition: {} -> {}",
                self.state.current_state, new_state
            );
            self.state.current_state = new_state;
            self.state.packets_in_state = 0;
            self.state.state_start = Instant::now();
            self.state.size_override = size_override;
            self.state.iat_override = iat_override;
            self.state.padding_override = padding_override;
        }
        self.state.packets_in_state += 1;
    }

    pub fn build_mdh(&mut self, eph_pub: Option<&[u8; 32]>) -> Vec<u8> {
        let mut mdh = if let Some(ref spec) = self.mask.header_spec {
            spec.generate(&mut self.rng)
        } else {
            self.mask.header_template.clone()
        };
        if let Some(eph) = eph_pub {
            let offset = self.mask.eph_pub_offset as usize;
            let len = self.mask.eph_pub_length as usize;
            let required = offset + len;
            if mdh.len() < required {
                mdh.resize(required, 0);
            }
            mdh[offset..offset + len].copy_from_slice(eph);
        }
        mdh
    }

    pub fn build_packet(
        &mut self,
        plaintext: &[u8],
        keys: &SessionKeys,
        counter: &mut u64,
        eph_pub: Option<&[u8; 32]>,
    ) -> Result<Vec<u8>> {
        let mdh = self.build_mdh(eph_pub);
        let target_size = self.sample_packet_size();
        let base_overhead = TAG_SIZE + mdh.len() + 2 + plaintext.len() + POLY1305_TAG_SIZE;
        let requested_pad_len = self.calc_padding(base_overhead, target_size);
        let packet_budget = SAFE_OUTER_PACKET_BUDGET.min(MAX_PACKET_SIZE);
        let max_pad_len = packet_budget.saturating_sub(base_overhead) as u16;
        let pad_len = requested_pad_len.min(max_pad_len);

        let mut padded = Vec::with_capacity(2 + plaintext.len() + pad_len as usize);
        padded.extend_from_slice(&pad_len.to_le_bytes());
        padded.extend_from_slice(plaintext);
        padded.resize(2 + plaintext.len() + pad_len as usize, 0);
        self.rng.fill_bytes(&mut padded[2 + plaintext.len()..]);

        let packet_counter = *counter;
        let nonce = self.generate_nonce(packet_counter);
        let ciphertext = encrypt_payload(&keys.session_key, &nonce, &padded)?;

        let time_window = crypto::compute_time_window(
            crypto::current_timestamp_ms(),
            crate::crypto::DEFAULT_WINDOW_MS,
        );
        let tag = crypto::generate_resonance_tag(&keys.tag_secret, packet_counter, time_window);
        *counter += 1;

        let mut packet = Vec::with_capacity(TAG_SIZE + mdh.len() + ciphertext.len());
        packet.extend_from_slice(&tag);
        packet.extend_from_slice(&mdh);
        packet.extend_from_slice(&ciphertext);
        Ok(packet)
    }

    fn generate_nonce(&self, counter: u64) -> [u8; NONCE_SIZE] {
        let mut nonce = [0u8; NONCE_SIZE];
        nonce[0..8].copy_from_slice(&counter.to_le_bytes());
        nonce
    }

    pub fn spoof_protocol(&self) -> SpoofProtocol {
        self.mask.spoof_protocol
    }
}

// ──────────── MimicryEncryptor ────────────
//
// Self-contained PacketEncryptor that owns SessionKeys + counter/seq + MimicryEngine.
// Designed for mobile cores (iOS, Android) which cannot share UploadCryptoState via Arc<Mutex<>>.
// pending_mask receives server MaskUpdate payloads from the control-message handler and is
// applied before the next data packet is encrypted.

pub struct MimicryEncryptor {
    engine: MimicryEngine,
    keys: SessionKeys,
    counter: u64,
    seq: u16,
    /// Written by the control-message handler; consumed on next encrypt call.
    pub pending_mask: Arc<Mutex<Option<MaskProfile>>>,
    fec_encoder: Option<FecEncoder>,
    pending_fec: Option<Vec<u8>>,
}

impl MimicryEncryptor {
    pub fn new(
        keys: SessionKeys,
        counter: u64,
        seq: u16,
        mask: MaskProfile,
        pending_mask: Arc<Mutex<Option<MaskProfile>>>,
    ) -> Self {
        Self {
            engine: MimicryEngine::new(mask),
            keys,
            counter,
            seq,
            pending_mask,
            fec_encoder: None,
            pending_fec: None,
        }
    }

    pub fn set_fec_group(&mut self, group_size: u8) {
        self.fec_encoder = if group_size > 0 {
            Some(FecEncoder::new(group_size, 1500))
        } else {
            None
        };
    }

    fn check_mask(&mut self) {
        if let Some(mask) = self
            .pending_mask
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .take()
        {
            self.engine.update_mask(mask);
        }
    }
}

impl PacketEncryptor for MimicryEncryptor {
    fn encrypt_data(&mut self, payload: &[u8]) -> Result<Vec<u8>> {
        self.check_mask();
        let inner = build_inner_packet(InnerType::Data, self.seq, payload);
        self.seq = self.seq.wrapping_add(1);
        let pkt = self
            .engine
            .build_packet(&inner, &self.keys, &mut self.counter, None)?;
        self.engine.update_fsm();
        if let Some(fec) = self.fec_encoder.as_mut() {
            if let Some(repair) = fec.feed(payload) {
                let repair_inner =
                    build_inner_packet(InnerType::FecRepair, self.seq, &repair.encode());
                self.seq = self.seq.wrapping_add(1);
                if let Ok(enc) =
                    self.engine
                        .build_packet(&repair_inner, &self.keys, &mut self.counter, None)
                {
                    self.pending_fec = Some(enc);
                }
            }
        }
        Ok(pkt)
    }

    fn encrypt_control(&mut self, payload: &ControlPayload) -> Result<Vec<u8>> {
        self.check_mask();
        let bytes = payload.encode()?;
        let inner = build_inner_packet(InnerType::Control, self.seq, &bytes);
        self.seq = self.seq.wrapping_add(1);
        self.engine
            .build_packet(&inner, &self.keys, &mut self.counter, None)
    }

    fn encrypt_keepalive(&mut self) -> Result<Vec<u8>> {
        self.encrypt_keepalive_ts(0)
    }

    fn encrypt_keepalive_ts(&mut self, send_ts: u64) -> Result<Vec<u8>> {
        let payload = ControlPayload::Keepalive { send_ts };
        let bytes = payload.encode()?;
        let inner = build_inner_packet(InnerType::Control, self.seq, &bytes);
        self.seq = self.seq.wrapping_add(1);
        self.engine
            .build_packet(&inner, &self.keys, &mut self.counter, None)
    }

    fn on_data_sent(&mut self, _payload_len: usize) {}

    fn take_fec_repair(&mut self) -> Option<Vec<u8>> {
        self.pending_fec.take()
    }
}

/// Decode a MaskProfile from the raw bytes of a `ControlPayload::MaskUpdate` payload.
/// Returns `None` if deserialization fails (malformed or unknown format).
pub fn decode_mask_update(mask_data: &[u8]) -> Option<MaskProfile> {
    rmp_serde::from_slice::<MaskProfile>(mask_data).ok()
}

/// Derive the bootstrap mask from PSK so iOS/Android start with mimicry active
/// before the server sends a MaskUpdate.
pub fn bootstrap_mask_for_psk(psk: Option<&[u8; 32]>) -> MaskProfile {
    let presets = crate::mask::preset_masks::all();
    debug_assert!(!presets.is_empty(), "preset_masks::all() must not be empty");
    if presets.is_empty() {
        return crate::mask::preset_masks::bootstrap_default();
    }
    if let Some(key) = psk {
        let hash = blake3::derive_key("aivpn-bootstrap-mask-v1", key);
        let idx = hash[0] as usize % presets.len();
        presets[idx].clone()
    } else {
        presets[0].clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mask::preset_masks::webrtc_zoom_v3;

    #[test]
    fn test_mimicry_engine_builds_packet() {
        let mask = webrtc_zoom_v3();
        let mut engine = MimicryEngine::new(mask);
        let keys = SessionKeys {
            session_key: [0u8; 32],
            tag_secret: [0u8; 32],
            prng_seed: [0u8; 32],
        };
        let mut counter = 0u64;
        let packet = engine.build_packet(b"Hello, World!", &keys, &mut counter, None);
        assert!(packet.is_ok());
        assert_eq!(counter, 1);
    }

    #[test]
    fn test_mimicry_encryptor_data() {
        let mask = webrtc_zoom_v3();
        let keys = SessionKeys {
            session_key: [0u8; 32],
            tag_secret: [0u8; 32],
            prng_seed: [0u8; 32],
        };
        let pending = Arc::new(Mutex::new(None));
        let mut enc = MimicryEncryptor::new(keys, 0, 0, mask, pending);
        assert!(enc.encrypt_data(b"test payload").is_ok());
    }
}
