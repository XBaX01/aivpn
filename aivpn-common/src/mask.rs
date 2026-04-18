//! Mask System (Traffic Mimicry Profiles)
//! 
//! Implements Mask profiles that define traffic shaping behavior

use serde::{Deserialize, Serialize};
use rand::{Rng, distributions::Distribution};
use rand::distributions::weighted::WeightedIndex;

use crate::error::{Error, Result};

/// Rotating bootstrap descriptor.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BootstrapDescriptor {
    pub descriptor_id: String,
    pub version: u16,
    pub created_at: u64,
    pub expires_at: u64,
    pub base_mask_ids: Vec<String>,
    #[serde(default)]
    pub embedded_masks: Vec<MaskProfile>,
    pub candidate_count: u8,
    #[serde(with = "serde_bytes")]
    pub kdf_salt: [u8; 32],
    #[serde(with = "serde_bytes")]
    #[serde(default = "default_signature")]
    pub signature: [u8; 64],
}

impl BootstrapDescriptor {
    pub fn is_valid_at(&self, unix_secs: u64) -> bool {
        unix_secs >= self.created_at && unix_secs <= self.expires_at
    }

    pub fn signing_bytes(&self) -> Vec<u8> {
        let mut unsigned = self.clone();
        unsigned.signature = [0u8; 64];
        rmp_serde::to_vec(&unsigned).expect("bootstrap descriptor serializable")
    }
}

/// Multi-channel bootstrap descriptor distribution.
///
/// Each channel represents a different method to fetch bootstrap descriptors.
/// Using multiple channels makes it harder for censors to block all distribution points.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum BootstrapChannel {
    /// CDN-based distribution (e.g., Cloudflare, AWS CloudFront)
    CDN {
        /// CDN URL endpoint
        url: String,
        /// CDN provider name for logging/analytics
        provider: String,
    },
    /// Telegram bot-based distribution
    Telegram {
        /// Telegram bot username
        bot_username: String,
        /// Bot token (optional, can use public bot)
        token: Option<String>,
    },
    /// GitHub releases/assets distribution
    GitHub {
        /// Repository in format "owner/repo"
        repo: String,
        /// Asset name pattern (e.g., "bootstrap-descriptors-v*.json")
        asset_name: String,
    },
    /// IPFS-based distribution (content-addressed)
    IPFS {
        /// IPFS content hash (CID)
        hash: String,
        /// Gateway URL (optional, uses default gateway if None)
        gateway: Option<String>,
    },
    /// Email-based distribution (for enterprise deployments)
    Email {
        /// Email address to request descriptors from
        address: String,
        /// Subject line pattern for automated requests
        subject_pattern: String,
    },
}

impl BootstrapChannel {
    /// Get a human-readable name for this channel
    pub fn name(&self) -> &str {
        match self {
            BootstrapChannel::CDN { provider, .. } => provider.as_str(),
            BootstrapChannel::Telegram { bot_username, .. } => bot_username.as_str(),
            BootstrapChannel::GitHub { repo, .. } => repo.as_str(),
            BootstrapChannel::IPFS { hash, .. } => hash.as_str(),
            BootstrapChannel::Email { address, .. } => address.as_str(),
        }
    }
    
    /// Get the channel type name
    pub fn channel_type(&self) -> &str {
        match self {
            BootstrapChannel::CDN { .. } => "CDN",
            BootstrapChannel::Telegram { .. } => "Telegram",
            BootstrapChannel::GitHub { .. } => "GitHub",
            BootstrapChannel::IPFS { .. } => "IPFS",
            BootstrapChannel::Email { .. } => "Email",
        }
    }
}

/// Configuration for multi-channel bootstrap descriptor distribution.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BootstrapConfig {
    /// List of channels to try (in order of preference)
    pub channels: Vec<BootstrapChannel>,
    /// Maximum age of descriptors to accept (seconds)
    pub max_descriptor_age: u64,
    /// Minimum number of channels that must succeed
    pub min_success_channels: usize,
    /// Background refresh interval (seconds)
    pub refresh_interval: u64,
    /// Whether to use random delay before first refresh
    pub randomize_first_refresh: bool,
}

impl Default for BootstrapConfig {
    fn default() -> Self {
        Self {
            channels: Vec::new(),
            max_descriptor_age: 86400, // 24 hours
            min_success_channels: 1,
            refresh_interval: 3600, // 1 hour
            randomize_first_refresh: true,
        }
    }
}

impl BootstrapConfig {
    /// Create a new bootstrap config with the given channels
    pub fn new(channels: Vec<BootstrapChannel>) -> Self {
        Self {
            channels,
            ..Default::default()
        }
    }
    
    /// Add a CDN channel
    pub fn with_cdn(mut self, url: impl Into<String>, provider: impl Into<String>) -> Self {
        self.channels.push(BootstrapChannel::CDN {
            url: url.into(),
            provider: provider.into(),
        });
        self
    }
    
    /// Add a Telegram channel
    pub fn with_telegram(mut self, bot_username: impl Into<String>) -> Self {
        self.channels.push(BootstrapChannel::Telegram {
            bot_username: bot_username.into(),
            token: None,
        });
        self
    }
    
    /// Add a GitHub channel
    pub fn with_github(mut self, repo: impl Into<String>, asset_name: impl Into<String>) -> Self {
        self.channels.push(BootstrapChannel::GitHub {
            repo: repo.into(),
            asset_name: asset_name.into(),
        });
        self
    }
    
    /// Add an IPFS channel
    pub fn with_ipfs(mut self, hash: impl Into<String>) -> Self {
        self.channels.push(BootstrapChannel::IPFS {
            hash: hash.into(),
            gateway: None,
        });
        self
    }
}

pub fn current_unix_secs() -> u64 {
    crate::crypto::current_timestamp_ms() / 1000
}

fn derive_bootstrap_seed(
    descriptor: &BootstrapDescriptor,
    preshared_key: Option<&[u8; 32]>,
    slot: u8,
) -> [u8; 32] {
    let mut hasher = blake3::Hasher::new();
    hasher.update(&descriptor.kdf_salt);
    hasher.update(descriptor.descriptor_id.as_bytes());
    hasher.update(&[slot]);
    match preshared_key {
        Some(psk) => {
            hasher.update(psk);
        }
        None => {
            hasher.update(&[0u8; 32]);
        }
    };
    let hash = hasher.finalize();
    let mut seed = [0u8; 32];
    seed.copy_from_slice(&hash.as_bytes()[..32]);
    seed
}

pub fn derive_bootstrap_candidate(
    descriptor: &BootstrapDescriptor,
    preshared_key: Option<&[u8; 32]>,
    slot: u8,
) -> Option<MaskProfile> {
    let embedded_masks = &descriptor.embedded_masks;
    let base_ids = if descriptor.base_mask_ids.is_empty() && embedded_masks.is_empty() {
        preset_masks::all()
            .into_iter()
            .map(|mask| mask.mask_id)
            .collect::<Vec<_>>()
    } else {
        descriptor.base_mask_ids.clone()
    };
    if base_ids.is_empty() && embedded_masks.is_empty() {
        return None;
    }

    let seed = derive_bootstrap_seed(descriptor, preshared_key, slot);
    let selector_len = if !embedded_masks.is_empty() { embedded_masks.len() } else { base_ids.len() };
    let base_index = (seed[0] as usize) % selector_len;
    let mut mask = if !embedded_masks.is_empty() {
        embedded_masks[base_index].clone()
    } else {
        preset_masks::by_id(&base_ids[base_index])?
    };
    let extra_gap_len = (seed[1] % 9) as usize;

    if extra_gap_len > 0 {
        let mut fields = mask.header_spec
            .as_ref()
            .map(HeaderSpec::fields)
            .unwrap_or_else(|| vec![HeaderField::Fixed { bytes: mask.header_template.clone() }]);
        fields.push(HeaderField::Random { len: extra_gap_len });
        mask.header_spec = Some(HeaderSpec::Structured { fields });
        mask.eph_pub_offset = mask.eph_pub_offset.saturating_add(extra_gap_len as u16);
    }

    mask.mask_id = format!(
        "bootstrap:{}:{}:{}:{:02x}{:02x}",
        descriptor.descriptor_id,
        if !embedded_masks.is_empty() { &embedded_masks[base_index].mask_id } else { &base_ids[base_index] },
        slot,
        seed[0],
        seed[1]
    );
    Some(mask)
}

pub fn derive_bootstrap_candidates(
    descriptor: &BootstrapDescriptor,
    preshared_key: Option<&[u8; 32]>,
) -> Vec<MaskProfile> {
    (0..descriptor.candidate_count)
        .filter_map(|slot| derive_bootstrap_candidate(descriptor, preshared_key, slot))
        .collect()
}

/// Mask profile for traffic mimicry
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MaskProfile {
    /// Unique identifier
    pub mask_id: String,
    /// Profile version
    pub version: u16,
    /// Creation timestamp
    pub created_at: u64,
    /// Expiration timestamp
    pub expires_at: u64,

    /// Protocol to spoof
    pub spoof_protocol: SpoofProtocol,
    /// Header template bytes (static, for legacy compatibility)
    pub header_template: Vec<u8>,
    /// Offset for ephemeral public key in header
    pub eph_pub_offset: u16,
    /// Length of ephemeral public key (always 32)
    pub eph_pub_length: u16,

    /// Packet size distribution
    pub size_distribution: SizeDistribution,
    /// Inter-arrival time distribution
    pub iat_distribution: IATDistribution,
    /// Padding strategy
    pub padding_strategy: PaddingStrategy,

    /// FSM states for behavioral mimicry
    pub fsm_states: Vec<FSMState>,
    /// Initial FSM state
    pub fsm_initial_state: u16,

    /// Neural resonance signature (64 floats)
    pub signature_vector: Vec<f32>,

    /// Reverse profile for server->client traffic
    pub reverse_profile: Option<Box<MaskProfile>>,

    /// Ed25519 signature (64 bytes)
    #[serde(with = "serde_bytes")]
    #[serde(default = "default_signature")]
    pub signature: [u8; 64],

    /// Dynamic header specification (Issue #30 fix)
    /// If present, clients should use this for per-packet header generation
    /// instead of the static header_template.
    /// Added in version 2, legacy clients ignore this field.
    #[serde(default)]
    pub header_spec: Option<HeaderSpec>,
}

fn default_signature() -> [u8; 64] {
    [0u8; 64]
}

/// Protocol spoofing types
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[allow(non_camel_case_types)]
pub enum SpoofProtocol {
    None,
    QUIC,
    WebRTC_STUN,
    HTTPS_H2,
    DNS_over_UDP,
}

/// Packet size distribution
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SizeDistribution {
    pub dist_type: SizeDistType,
    pub bins: Vec<(u16, u16, f32)>, // (min, max, probability)
    pub parametric_type: Option<ParametricType>,
    pub parametric_params: Option<Vec<f64>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SizeDistType {
    Histogram,
    Parametric,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ParametricType {
    LogNormal,
    Gamma,
    Bimodal,
}

impl SizeDistribution {
    /// Sample a packet size from the distribution
    pub fn sample<R: Rng>(&self, rng: &mut R) -> u16 {
        match self.dist_type {
            SizeDistType::Histogram => {
                if self.bins.is_empty() {
                    return 64; // Default
                }
                
                // Weighted random selection of bin
                let weights: Vec<f32> = self.bins.iter().map(|(_, _, p)| *p).collect();
                if let Ok(dist) = WeightedIndex::new(&weights) {
                    let bin_idx = dist.sample(rng);
                    let (min, max, _) = self.bins[bin_idx];
                    rng.gen_range(min..=max)
                } else {
                    64
                }
            }
            SizeDistType::Parametric => {
                match self.parametric_type {
                    Some(ParametricType::LogNormal) => {
                        if let Some(params) = &self.parametric_params {
                            let mu: f64 = params[0];
                            let sigma: f64 = params[1];
                            // Box-Muller transform: generate standard normal from two uniform samples
                            let u1: f64 = rng.gen::<f64>().max(1e-10); // avoid ln(0)
                            let u2: f64 = rng.gen();
                            let z = (-2.0 * u1.ln()).sqrt() * (2.0 * std::f64::consts::PI * u2).cos();
                            // LogNormal: exp(mu + sigma * z)
                            let sample = (mu + sigma * z).exp();
                            (sample as u16).max(1)
                        } else {
                            rng.gen_range(64..512)
                        }
                    }
                    _ => rng.gen_range(64..512),
                }
            }
        }
    }
}

/// Inter-arrival time distribution
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IATDistribution {
    pub dist_type: IATDistType,
    pub params: Vec<f64>,
    pub jitter_range_ms: (f64, f64),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum IATDistType {
    Exponential,
    LogNormal,
    Gamma,
    Empirical,
}

impl IATDistribution {
    /// Sample an inter-arrival time in milliseconds
    pub fn sample<R: Rng>(&self, rng: &mut R) -> f64 {
        let base_iat = match self.dist_type {
            IATDistType::Exponential => {
                let lambda: f64 = self.params[0];
                let val: f64 = rng.gen::<f64>().max(1e-10);
                -(1.0 - val).ln() / lambda
            }
            IATDistType::LogNormal => {
                let mu: f64 = self.params[0];
                let sigma: f64 = self.params[1];
                // Box-Muller transform for proper normal distribution
                let u1: f64 = rng.gen::<f64>().max(1e-10);
                let u2: f64 = rng.gen();
                let z = (-2.0 * u1.ln()).sqrt() * (2.0 * std::f64::consts::PI * u2).cos();
                (mu + sigma * z).exp()
            }
            IATDistType::Gamma => {
                // Simplified gamma sampling (sum of k exponentials for integer k)
                let k: f64 = self.params[0];
                let theta: f64 = self.params[1];
                let sum: f64 = (0..k.max(1.0) as i32)
                    .map(|_| {
                        let val: f64 = rng.gen::<f64>().max(1e-10);
                        -(1.0 - val).ln()
                    })
                    .sum();
                sum * theta
            }
            IATDistType::Empirical => {
                let idx = rng.gen_range(0..self.params.len());
                self.params[idx]
            }
        };

        // Add jitter
        let jitter = rng.gen_range(self.jitter_range_ms.0..=self.jitter_range_ms.1);
        (base_iat + jitter).max(0.0)
    }
}

/// Padding strategy
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum PaddingStrategy {
    RandomUniform { min: u16, max: u16 },
    MatchDistribution,
    Fixed { size: u16 },
}

impl PaddingStrategy {
    /// Calculate padding length for a given payload
    pub fn calc_padding<R: Rng>(&self, payload_size: usize, target_size: u16, rng: &mut R) -> u16 {
        match self {
            Self::RandomUniform { min, max } => rng.gen_range(*min..=*max),
            Self::MatchDistribution => {
                if target_size as usize > payload_size {
                    (target_size as usize - payload_size) as u16
                } else {
                    0
                }
            }
            Self::Fixed { size } => *size,
        }
    }
}

/// Header Specification for dynamic per-packet header generation
///
/// Instead of storing fixed header bytes, HeaderSpec declares how to generate
/// headers dynamically. This solves Issue #30 (WireGuard detection) by ensuring
/// each packet has a unique but protocol-valid header.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum HeaderSpec {
    /// Structured header semantics expressed as typed fields.
    Structured {
        fields: Vec<HeaderField>,
    },
    /// Raw prefix with per-packet randomization
    /// Uses fixed bytes with optional random positions
    RawPrefix {
        /// Fixed prefix bytes (hex string)
        prefix_hex: String,
        /// Indices of bytes to randomize on each packet (0-indexed)
        #[serde(default)]
        randomize_indices: Vec<usize>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind")]
pub enum HeaderField {
    Fixed { bytes: Vec<u8> },
    Random { len: usize },
    Length { len: usize, endian: HeaderEndian },
    Id { len: usize, mode: IdFieldMode },
    CounterLike {
        len: usize,
        endian: HeaderEndian,
        #[serde(default)]
        start: u64,
        #[serde(default = "default_counter_step")]
        step: u64,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum HeaderEndian {
    Big,
    Little,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum IdFieldMode {
    Random,
    Zero,
}

impl Default for IdFieldMode {
    fn default() -> Self {
        Self::Random
    }
}

fn default_counter_step() -> u64 { 1 }

impl HeaderSpec {
    pub fn structured(fields: Vec<HeaderField>) -> Self {
        Self::Structured { fields }
    }

    pub fn stun_binding() -> Self {
        Self::stun_binding_with_cookie(true)
    }

    pub fn stun_binding_with_cookie(magic_cookie: bool) -> Self {
        Self::structured(vec![
            HeaderField::Fixed { bytes: vec![0x00, 0x01] },
            HeaderField::Length { len: 2, endian: HeaderEndian::Big },
            HeaderField::Fixed {
                bytes: if magic_cookie {
                    vec![0x21, 0x12, 0xA4, 0x42]
                } else {
                    vec![0x00, 0x00, 0x00, 0x00]
                },
            },
            HeaderField::Id { len: 12, mode: IdFieldMode::Random },
        ])
    }

    pub fn quic_initial(version: u32, dcid_len: u8) -> Self {
        let dcid_len = dcid_len.clamp(8, 20);
        Self::structured(vec![
            HeaderField::Fixed { bytes: vec![0xC0] },
            HeaderField::Fixed { bytes: version.to_be_bytes().to_vec() },
            HeaderField::Fixed { bytes: vec![dcid_len] },
            HeaderField::Id { len: dcid_len as usize, mode: IdFieldMode::Random },
        ])
    }

    pub fn dns_query(flags: u16) -> Self {
        Self::structured(vec![
            HeaderField::Id { len: 2, mode: IdFieldMode::Random },
            HeaderField::Fixed { bytes: flags.to_be_bytes().to_vec() },
            HeaderField::Fixed {
                bytes: vec![0x00, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00],
            },
        ])
    }

    pub fn tls_record(content_type: u8, version: u16) -> Self {
        Self::structured(vec![
            HeaderField::Fixed { bytes: vec![content_type] },
            HeaderField::Fixed { bytes: version.to_be_bytes().to_vec() },
            HeaderField::Length { len: 2, endian: HeaderEndian::Big },
        ])
    }

    pub fn fields(&self) -> Vec<HeaderField> {
        match self {
            Self::Structured { fields } => fields.clone(),
            Self::RawPrefix {
                prefix_hex,
                randomize_indices,
            } => {
                let bytes = hex::decode(prefix_hex).unwrap_or_else(|_| vec![0x00, 0x01, 0x02, 0x03]);
                if randomize_indices.is_empty() {
                    return vec![HeaderField::Fixed { bytes }];
                }
                let mut fields = Vec::new();
                let mut current_fixed = Vec::new();
                for (idx, byte) in bytes.iter().enumerate() {
                    if randomize_indices.contains(&idx) {
                        if !current_fixed.is_empty() {
                            fields.push(HeaderField::Fixed { bytes: std::mem::take(&mut current_fixed) });
                        }
                        fields.push(HeaderField::Random { len: 1 });
                    } else {
                        current_fixed.push(*byte);
                    }
                }
                if !current_fixed.is_empty() {
                    fields.push(HeaderField::Fixed { bytes: current_fixed });
                }
                fields
            }
        }
    }

    /// Generate a header from this specification
    /// Returns different bytes on each call for randomizable fields
    pub fn generate<R: Rng>(&self, rng: &mut R) -> Vec<u8> {
        let mut header = Vec::new();
        for field in self.fields() {
            match field {
                HeaderField::Fixed { bytes } => header.extend_from_slice(&bytes),
                HeaderField::Random { len } => {
                    let start = header.len();
                    header.resize(start + len, 0);
                    rng.fill_bytes(&mut header[start..start + len]);
                }
                HeaderField::Length { len, endian } => {
                    let bytes = encode_semantic_u64(0, len, endian);
                    header.extend_from_slice(&bytes);
                }
                HeaderField::Id { len, mode } => match mode {
                    IdFieldMode::Random => {
                        let start = header.len();
                        header.resize(start + len, 0);
                        rng.fill_bytes(&mut header[start..start + len]);
                    }
                    IdFieldMode::Zero => header.extend(std::iter::repeat(0u8).take(len)),
                },
                HeaderField::CounterLike { len, endian, start, step } => {
                    let raw = start.saturating_add(rng.gen_range(0..=step.max(1) * 1024));
                    let bytes = encode_semantic_u64(raw, len, endian);
                    header.extend_from_slice(&bytes);
                }
            }
        }
        header
    }
    
    /// Get the minimum header length for this spec
    pub fn min_length(&self) -> usize {
        self.fields()
            .into_iter()
            .map(|field| match field {
                HeaderField::Fixed { bytes } => bytes.len(),
                HeaderField::Random { len }
                | HeaderField::Length { len, .. }
                | HeaderField::Id { len, .. }
                | HeaderField::CounterLike { len, .. } => len,
            })
            .sum()
    }
    
    /// Generate a static header template for legacy compatibility
    /// Uses a seeded RNG for deterministic output
    pub fn generate_static(&self) -> Vec<u8> {
        use rand::SeedableRng;
        let mut rng = rand::rngs::StdRng::seed_from_u64(0);
        self.generate(&mut rng)
    }
}

fn encode_semantic_u64(value: u64, len: usize, endian: HeaderEndian) -> Vec<u8> {
    let mut bytes = match endian {
        HeaderEndian::Big => value.to_be_bytes().to_vec(),
        HeaderEndian::Little => value.to_le_bytes().to_vec(),
    };
    if len < bytes.len() {
        match endian {
            HeaderEndian::Big => bytes = bytes[bytes.len() - len..].to_vec(),
            HeaderEndian::Little => bytes.truncate(len),
        }
    } else if len > bytes.len() {
        let mut out = vec![0u8; len - bytes.len()];
        match endian {
            HeaderEndian::Big => {
                out.extend(bytes);
                bytes = out;
            }
            HeaderEndian::Little => {
                bytes.extend(out);
            }
        }
    }
    bytes
}

/// FSM state for behavioral mimicry
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FSMState {
    pub state_id: u16,
    pub transitions: Vec<FSMTransition>,
}

/// FSM transition
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FSMTransition {
    pub condition: TransitionCondition,
    pub next_state: u16,
    pub size_override: Option<SizeDistribution>,
    pub iat_override: Option<IATDistribution>,
    pub padding_override: Option<PaddingStrategy>,
}

/// Transition condition
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum TransitionCondition {
    AfterPackets(u32),
    AfterDuration(u64), // milliseconds
    OnPayloadType(u8),
    Random(f32), // probability per packet
}

impl MaskProfile {
    /// Verify Ed25519 signature over all profile fields except the signature itself
    pub fn verify_signature(&self, public_key: &[u8; 32]) -> Result<bool> {
        use ed25519_dalek::{Signature, VerifyingKey, Verifier};

        let vk = VerifyingKey::from_bytes(public_key)
            .map_err(|e| Error::Crypto(format!("Invalid Ed25519 public key: {}", e)))?;

        // Build canonical message: mask_id || version || header_template
        let mut message = Vec::new();
        message.extend_from_slice(self.mask_id.as_bytes());
        message.extend_from_slice(&self.version.to_le_bytes());
        message.extend_from_slice(&self.header_template);

        let sig = Signature::from_bytes(&self.signature);
        match vk.verify(&message, &sig) {
            Ok(()) => Ok(true),
            Err(_) => Ok(false),
        }
    }

    /// Get initial FSM state
    pub fn initial_state(&self) -> u16 {
        self.fsm_initial_state
    }

    /// Process FSM transition
    pub fn process_transition(
        &self,
        current_state: u16,
        packets_in_state: u32,
        duration_in_state_ms: u64,
    ) -> (u16, Option<SizeDistribution>, Option<IATDistribution>, Option<PaddingStrategy>) {
        let state = self.fsm_states.iter().find(|s| s.state_id == current_state);
        if let Some(state) = state {
            for transition in &state.transitions {
                let should_transition = match &transition.condition {
                    TransitionCondition::AfterPackets(n) => packets_in_state >= *n,
                    TransitionCondition::AfterDuration(ms) => duration_in_state_ms >= *ms,
                    TransitionCondition::Random(prob) => rand::thread_rng().gen_range(0.0..1.0) < *prob,
                    TransitionCondition::OnPayloadType(_) => false, // Handled separately
                };

                if should_transition {
                    return (
                        transition.next_state,
                        transition.size_override.clone(),
                        transition.iat_override.clone(),
                        transition.padding_override.clone(),
                    );
                }
            }
        }
        (current_state, None, None, None)
    }
}

#[cfg(test)]
mod distribution_tests {
    use super::{IATDistribution, IATDistType};
    use rand::{rngs::StdRng, SeedableRng};

    #[test]
    fn iat_sampling_uses_symmetric_jitter_range() {
        let dist = IATDistribution {
            dist_type: IATDistType::Empirical,
            params: vec![50.0],
            jitter_range_ms: (-10.0, 10.0),
        };
        let mut rng = StdRng::seed_from_u64(7);
        let samples: Vec<f64> = (0..256).map(|_| dist.sample(&mut rng)).collect();

        assert!(samples.iter().any(|&value| value < 50.0));
        assert!(samples.iter().any(|&value| value > 50.0));
    }
}

/// File-backed preset mask catalog
pub mod preset_masks {
    use super::*;
    use std::sync::OnceLock;

    static WEBRTC_ZOOM_V3: OnceLock<MaskProfile> = OnceLock::new();
    static QUIC_HTTPS_V2: OnceLock<MaskProfile> = OnceLock::new();
    static WEBRTC_YANDEX_TELEMOST_V1: OnceLock<MaskProfile> = OnceLock::new();
    static WEBRTC_VK_TEAMS_V1: OnceLock<MaskProfile> = OnceLock::new();
    static WEBRTC_SBERJAZZ_V1: OnceLock<MaskProfile> = OnceLock::new();

    fn parse_mask(json: &str) -> MaskProfile {
        serde_json::from_str(json).expect("valid preset mask asset")
    }

    fn load_webrtc_zoom_v3() -> MaskProfile {
        parse_mask(include_str!("../../mask-assets/webrtc_zoom_v3.json"))
    }

    fn load_quic_https_v2() -> MaskProfile {
        parse_mask(include_str!("../../mask-assets/quic_https_v2.json"))
    }

    fn load_webrtc_yandex_telemost_v1() -> MaskProfile {
        parse_mask(include_str!("../../mask-assets/webrtc_yandex_telemost_v1.json"))
    }

    fn load_webrtc_vk_teams_v1() -> MaskProfile {
        parse_mask(include_str!("../../mask-assets/webrtc_vk_teams_v1.json"))
    }

    fn load_webrtc_sberjazz_v1() -> MaskProfile {
        parse_mask(include_str!("../../mask-assets/webrtc_sberjazz_v1.json"))
    }

    pub fn webrtc_zoom_v3() -> MaskProfile {
        WEBRTC_ZOOM_V3.get_or_init(load_webrtc_zoom_v3).clone()
    }

    pub fn quic_https_v2() -> MaskProfile {
        QUIC_HTTPS_V2.get_or_init(load_quic_https_v2).clone()
    }

    pub fn webrtc_yandex_telemost_v1() -> MaskProfile {
        WEBRTC_YANDEX_TELEMOST_V1.get_or_init(load_webrtc_yandex_telemost_v1).clone()
    }

    pub fn webrtc_vk_teams_v1() -> MaskProfile {
        WEBRTC_VK_TEAMS_V1.get_or_init(load_webrtc_vk_teams_v1).clone()
    }

    pub fn webrtc_sberjazz_v1() -> MaskProfile {
        WEBRTC_SBERJAZZ_V1.get_or_init(load_webrtc_sberjazz_v1).clone()
    }

    pub fn all() -> Vec<MaskProfile> {
        vec![
            webrtc_zoom_v3(),
            quic_https_v2(),
            webrtc_yandex_telemost_v1(),
            webrtc_vk_teams_v1(),
            webrtc_sberjazz_v1(),
        ]
    }

    pub fn by_id(mask_id: &str) -> Option<MaskProfile> {
        match mask_id {
            "webrtc_zoom_v3" => Some(webrtc_zoom_v3()),
            "quic_https_v2" => Some(quic_https_v2()),
            "webrtc_yandex_telemost_v1" => Some(webrtc_yandex_telemost_v1()),
            "webrtc_vk_teams_v1" => Some(webrtc_vk_teams_v1()),
            "webrtc_sberjazz_v1" => Some(webrtc_sberjazz_v1()),
            _ => None,
        }
    }

    pub fn bootstrap_default() -> MaskProfile {
        webrtc_zoom_v3()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rand::rngs::StdRng;
    use rand::SeedableRng;

    #[test]
    fn test_stun_binding_generation() {
        let spec = HeaderSpec::stun_binding();
        
        // Generate two headers - they should differ in transaction_id
        let mut rng = StdRng::seed_from_u64(42);
        let header1 = spec.generate(&mut rng);
        let header2 = spec.generate(&mut rng);
        
        assert_eq!(header1.len(), 20);
        assert_eq!(header2.len(), 20);
        
        // First 8 bytes should be the same (type + length + magic cookie)
        assert_eq!(&header1[0..2], &[0x00, 0x01]); // Binding Request
        assert_eq!(&header1[4..8], &[0x21, 0x12, 0xA4, 0x42]); // Magic cookie
        
        // Transaction IDs should differ
        assert_ne!(&header1[8..], &header2[8..]);
    }

    #[test]
    fn test_quic_initial_generation() {
        let spec = HeaderSpec::quic_initial(0x00000001, 8);
        
        let mut rng = StdRng::seed_from_u64(42);
        let header1 = spec.generate(&mut rng);
        let header2 = spec.generate(&mut rng);
        
        assert_eq!(header1.len(), 14); // 1 + 4 + 1 + 8
        assert_eq!(header2.len(), 14);
        
        // First byte should be 0xC0 (long packet)
        assert_eq!(header1[0], 0xC0);
        
        // Version bytes
        assert_eq!(&header1[1..5], &0x00000001u32.to_be_bytes());
        
        // DCID length
        assert_eq!(header1[5], 8);
        
        // DCID should differ between generations
        assert_ne!(&header1[6..], &header2[6..]);
    }

    #[test]
    fn test_dns_query_generation() {
        let spec = HeaderSpec::dns_query(0x0100);
        
        let mut rng = StdRng::seed_from_u64(42);
        let header1 = spec.generate(&mut rng);
        let header2 = spec.generate(&mut rng);
        
        assert_eq!(header1.len(), 12);
        assert_eq!(header2.len(), 12);
        
        // Flags should be consistent
        assert_eq!(&header1[2..4], &[0x01, 0x00]);
        assert_eq!(&header2[2..4], &[0x01, 0x00]);
        
        // Transaction ID should differ
        assert_ne!(&header1[0..2], &header2[0..2]);
        
        // Counts should be standard DNS query
        assert_eq!(&header1[4..], &[0x00, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00]);
    }

    #[test]
    fn test_tls_record_generation() {
        let spec = HeaderSpec::tls_record(0x17, 0x0303);
        
        let mut rng = StdRng::seed_from_u64(42);
        let header = spec.generate(&mut rng);
        
        assert_eq!(header.len(), 5);
        assert_eq!(header[0], 0x17); // Application data
        assert_eq!(&header[1..3], &[0x03, 0x03]); // TLS 1.2
        assert_eq!(&header[3..5], &[0x00, 0x00]); // Length (to be filled)
    }

    #[test]
    fn test_raw_prefix_generation() {
        let spec = HeaderSpec::RawPrefix {
            prefix_hex: "010203040506".to_string(),
            randomize_indices: vec![2, 4],
        };
        
        let mut rng = StdRng::seed_from_u64(42);
        let header1 = spec.generate(&mut rng);
        let header2 = spec.generate(&mut rng);
        
        assert_eq!(header1.len(), 6);
        assert_eq!(header2.len(), 6);
        
        // Fixed bytes should be the same
        assert_eq!(header1[0], header2[0]); // 0x01
        assert_eq!(header1[1], header2[1]); // 0x02
        assert_eq!(header1[3], header2[3]); // 0x04
        assert_eq!(header1[5], header2[5]); // 0x06
        
        // Randomized bytes should differ
        assert_ne!(header1[2], header2[2]);
        assert_ne!(header1[4], header2[4]);
    }

    #[test]
    fn test_header_spec_min_length() {
        let stun = HeaderSpec::stun_binding();
        assert_eq!(stun.min_length(), 20);
        
        let quic = HeaderSpec::quic_initial(0x00000001, 8);
        // 1 (header_form) + 4 (version) + 1 (dcid_len) + 8 (dcid) = 14
        assert_eq!(quic.min_length(), 14);
        
        let dns = HeaderSpec::dns_query(0x0100);
        assert_eq!(dns.min_length(), 12);
        
        let tls = HeaderSpec::tls_record(0x17, 0x0303);
        assert_eq!(tls.min_length(), 5);
    }

    #[test]
    fn test_static_generation_deterministic() {
        let spec = HeaderSpec::stun_binding();
        
        let static1 = spec.generate_static();
        let static2 = spec.generate_static();
        
        // Static generation should be deterministic
        assert_eq!(static1, static2);
    }

    #[test]
    fn test_preset_masks_have_header_spec() {
        let mask = preset_masks::webrtc_zoom_v3();
        assert!(mask.header_spec.is_some());
        assert_eq!(mask.version, 2);
        
        let mask2 = preset_masks::quic_https_v2();
        assert!(mask2.header_spec.is_some());
        assert_eq!(mask2.version, 2);
    }

    #[test]
    fn bootstrap_derivation_is_deterministic() {
        let descriptor = BootstrapDescriptor {
            descriptor_id: "epoch-1".into(),
            version: 1,
            created_at: 0,
            expires_at: u64::MAX,
            base_mask_ids: vec!["webrtc_zoom_v3".into(), "quic_https_v2".into()],
            embedded_masks: Vec::new(),
            candidate_count: 4,
            kdf_salt: [7u8; 32],
            signature: [0u8; 64],
        };
        let psk = [3u8; 32];
        let left = derive_bootstrap_candidates(&descriptor, Some(&psk));
        let right = derive_bootstrap_candidates(&descriptor, Some(&psk));

        assert_eq!(left.len(), right.len());
        for (lhs, rhs) in left.iter().zip(right.iter()) {
            assert_eq!(lhs.mask_id, rhs.mask_id);
            assert_eq!(lhs.eph_pub_offset, rhs.eph_pub_offset);
            assert_eq!(lhs.header_spec.as_ref().map(|s| s.min_length()), rhs.header_spec.as_ref().map(|s| s.min_length()));
        }
    }

    #[test]
    fn bootstrap_derivation_varies_across_psks() {
        let descriptor = BootstrapDescriptor {
            descriptor_id: "epoch-2".into(),
            version: 1,
            created_at: 0,
            expires_at: u64::MAX,
            base_mask_ids: vec!["webrtc_zoom_v3".into(), "quic_https_v2".into()],
            embedded_masks: Vec::new(),
            candidate_count: 4,
            kdf_salt: [11u8; 32],
            signature: [0u8; 64],
        };

        let first = derive_bootstrap_candidates(&descriptor, Some(&[1u8; 32]));
        let second = derive_bootstrap_candidates(&descriptor, Some(&[2u8; 32]));

        assert_ne!(first[0].mask_id, second[0].mask_id);
    }

    #[test]
    fn bootstrap_derivation_supports_embedded_masks() {
        let descriptor = BootstrapDescriptor {
            descriptor_id: "epoch-3".into(),
            version: 1,
            created_at: 0,
            expires_at: u64::MAX,
            base_mask_ids: Vec::new(),
            embedded_masks: vec![preset_masks::webrtc_zoom_v3()],
            candidate_count: 1,
            kdf_salt: [13u8; 32],
            signature: [0u8; 64],
        };

        let masks = derive_bootstrap_candidates(&descriptor, Some(&[9u8; 32]));
        assert_eq!(masks.len(), 1);
        assert!(masks[0].mask_id.starts_with("bootstrap:epoch-3:webrtc_zoom_v3:"));
    }
}
