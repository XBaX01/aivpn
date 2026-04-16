//! Mask System (Traffic Mimicry Profiles)
//! 
//! Implements Mask profiles that define traffic shaping behavior

use serde::{Deserialize, Serialize};
use rand::{Rng, distributions::Distribution};
use rand::distributions::weighted::WeightedIndex;

use crate::error::{Error, Result};

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
    pub signature: [u8; 64],

    /// Dynamic header specification (Issue #30 fix)
    /// If present, clients should use this for per-packet header generation
    /// instead of the static header_template.
    /// Added in version 2, legacy clients ignore this field.
    #[serde(default)]
    pub header_spec: Option<HeaderSpec>,
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
    /// STUN Binding Request header
    /// Generates: type(2) + length(2) + magic_cookie(4) + transaction_id(12) = 20 bytes
    StunBinding {
        /// Include magic cookie 0x2112A442
        #[serde(default = "default_true")]
        magic_cookie: bool,
        /// Transaction ID generation mode
        #[serde(default)]
        transaction_id: TransactionIdMode,
    },
    /// QUIC Initial Packet header
    /// Generates: header_form(1) + version(4) + dcid_len(1) + dcid(8..20) = 14..26 bytes
    QuicInitial {
        /// QUIC version (default: 0x00000001 for QUIC v1)
        #[serde(default = "default_quic_version")]
        version: u32,
        /// Destination Connection ID length (8-20)
        #[serde(default = "default_dcid_len")]
        dcid_len: u8,
    },
    /// DNS Query header
    /// Generates: txid(2) + flags(2) + counts(8) = 12 bytes
    DnsQuery {
        /// DNS flags (default: 0x0100 for standard query)
        #[serde(default = "default_dns_flags")]
        flags: u16,
    },
    /// TLS Record Layer prefix
    /// Generates: type(1) + version(2) + length(2) = 5 bytes
    TlsRecord {
        /// Content type (default: 0x17 for application data)
        #[serde(default = "default_tls_content_type")]
        content_type: u8,
        /// TLS version (default: 0x0303 for TLS 1.2)
        #[serde(default = "default_tls_version")]
        version: u16,
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

/// Transaction ID generation mode for STUN
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum TransactionIdMode {
    /// Fully random 96-bit transaction ID
    Random,
    /// Use incremental counter (for correlation analysis)
    Counter {
        /// Starting value
        #[serde(default)]
        start: u32,
    },
}

impl Default for TransactionIdMode {
    fn default() -> Self {
        Self::Random
    }
}

fn default_true() -> bool { true }
fn default_quic_version() -> u32 { 0x00000001 }
fn default_dcid_len() -> u8 { 8 }
fn default_dns_flags() -> u16 { 0x0100 }
fn default_tls_content_type() -> u8 { 0x17 }
fn default_tls_version() -> u16 { 0x0303 }

impl HeaderSpec {
    /// Generate a header from this specification
    /// Returns different bytes on each call for randomizable fields
    pub fn generate<R: Rng>(&self, rng: &mut R) -> Vec<u8> {
        match self {
            Self::StunBinding { magic_cookie, transaction_id } => {
                let mut header = Vec::with_capacity(20);
                
                // STUN message type: Binding Request = 0x0001
                header.extend_from_slice(&[0x00, 0x01]);
                
                // Message length (will be filled by caller)
                header.extend_from_slice(&[0x00, 0x00]);
                
                // Magic cookie
                if *magic_cookie {
                    header.extend_from_slice(&0x2112A442u32.to_be_bytes());
                } else {
                    header.extend_from_slice(&[0x00, 0x00, 0x00, 0x00]);
                }
                
                // Transaction ID (12 bytes / 96 bits)
                match transaction_id {
                    TransactionIdMode::Random => {
                        let mut txid = [0u8; 12];
                        rng.fill_bytes(&mut txid);
                        header.extend_from_slice(&txid);
                    }
                    TransactionIdMode::Counter { start } => {
                        // Use counter in first 4 bytes, rest random
                        header.extend_from_slice(&start.to_be_bytes());
                        let mut rest = [0u8; 8];
                        rng.fill_bytes(&mut rest);
                        header.extend_from_slice(&rest);
                    }
                }
                
                header
            }
            Self::QuicInitial { version, dcid_len } => {
                let dcid_len = (*dcid_len).clamp(8, 20);
                let mut header = Vec::with_capacity(14 + dcid_len as usize);
                
                // Header form byte: long packet (1) + fixed bit (0) + spin bit (0) + reserved (00) + key phase (0) + packet number length (00)
                header.push(0xC0);
                
                // Version
                header.extend_from_slice(&version.to_be_bytes());
                
                // DCID length
                header.push(dcid_len);
                
                // DCID (random bytes)
                let mut dcid = vec![0u8; dcid_len as usize];
                rng.fill_bytes(&mut dcid);
                header.extend_from_slice(&dcid);
                
                header
            }
            Self::DnsQuery { flags } => {
                let mut header = Vec::with_capacity(12);
                
                // Transaction ID (random)
                header.extend_from_slice(&rng.gen::<u16>().to_be_bytes());
                
                // Flags
                header.extend_from_slice(&flags.to_be_bytes());
                
                // Counts: QDCOUNT=1, ANCOUNT=0, NSCOUNT=0, ARCOUNT=0
                header.extend_from_slice(&[0x00, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00]);
                
                header
            }
            Self::TlsRecord { content_type, version } => {
                let mut header = Vec::with_capacity(5);
                
                // Content type
                header.push(*content_type);
                
                // Version
                header.extend_from_slice(&version.to_be_bytes());
                
                // Length (will be filled by caller)
                header.extend_from_slice(&[0x00, 0x00]);
                
                header
            }
            Self::RawPrefix { prefix_hex, randomize_indices } => {
                let mut bytes = hex::decode(prefix_hex)
                    .unwrap_or_else(|_| vec![0x00, 0x01, 0x02, 0x03]);
                
                // Randomize specified indices
                for &idx in randomize_indices {
                    if idx < bytes.len() {
                        bytes[idx] = rng.gen();
                    }
                }
                
                bytes
            }
        }
    }
    
    /// Get the minimum header length for this spec
    pub fn min_length(&self) -> usize {
        match self {
            Self::StunBinding { magic_cookie, .. } => {
                if *magic_cookie { 20 } else { 16 }
            }
            Self::QuicInitial { dcid_len, .. } => {
                // header_form(1) + version(4) + dcid_len(1) + dcid(dcid_len)
                6 + (*dcid_len).clamp(8, 20) as usize
            }
            Self::DnsQuery { .. } => 12,
            Self::TlsRecord { .. } => 5,
            Self::RawPrefix { prefix_hex, .. } => {
                hex::decode(prefix_hex).map(|b| b.len()).unwrap_or(4)
            }
        }
    }
    
    /// Generate a static header template for legacy compatibility
    /// Uses a seeded RNG for deterministic output
    pub fn generate_static(&self) -> Vec<u8> {
        use rand::SeedableRng;
        let mut rng = rand::rngs::StdRng::seed_from_u64(0);
        self.generate(&mut rng)
    }
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

/// Pre-built mask catalog (MVP defaults)
pub mod preset_masks {
    use super::*;

    /// WebRTC Zoom-like profile (v3 - with HeaderSpec for Issue #30 fix)
    pub fn webrtc_zoom_v3() -> MaskProfile {
        // Generate static header template for legacy compatibility
        let header_spec = HeaderSpec::StunBinding {
            magic_cookie: true,
            transaction_id: TransactionIdMode::Random,
        };
        let header_template = header_spec.generate_static();
        
        MaskProfile {
            mask_id: "webrtc_zoom_v3".to_string(),
            version: 2,  // Version 2 for HeaderSpec support
            created_at: 0,
            expires_at: u64::MAX,
            spoof_protocol: SpoofProtocol::WebRTC_STUN,
            header_template,
            eph_pub_offset: 20,  // After STUN header (20 bytes)
            eph_pub_length: 32,
            size_distribution: SizeDistribution {
                dist_type: SizeDistType::Parametric,
                bins: vec![],
                parametric_type: Some(ParametricType::Bimodal),
                parametric_params: Some(vec![5.0, 0.5]), // Opus-like
            },
            iat_distribution: IATDistribution {
                dist_type: IATDistType::LogNormal,
                params: vec![2.5, 0.3], // ~12ms average
                jitter_range_ms: (5.0, 20.0),
            },
            padding_strategy: PaddingStrategy::RandomUniform { min: 0, max: 64 },
            fsm_states: vec![
                FSMState {
                    state_id: 0,
                    transitions: vec![
                        FSMTransition {
                            condition: TransitionCondition::AfterDuration(5000),
                            next_state: 1,
                            size_override: None,
                            iat_override: None,
                            padding_override: None,
                        }
                    ],
                },
                FSMState {
                    state_id: 1,
                    transitions: vec![],
                },
            ],
            fsm_initial_state: 0,
            signature_vector: vec![0.0; 64],
            reverse_profile: None,
            signature: [0u8; 64],
            header_spec: Some(header_spec),
        }
    }

    /// QUIC/HTTP3-like profile (v2 - with HeaderSpec for Issue #30 fix)
    pub fn quic_https_v2() -> MaskProfile {
        // Generate static header template for legacy compatibility
        let header_spec = HeaderSpec::QuicInitial {
            version: 0x00000001,  // QUIC v1
            dcid_len: 8,
        };
        let header_template = header_spec.generate_static();
        
        MaskProfile {
            mask_id: "quic_https_v2".to_string(),
            version: 2,  // Version 2 for HeaderSpec support
            created_at: 0,
            expires_at: u64::MAX,
            spoof_protocol: SpoofProtocol::QUIC,
            header_template,
            eph_pub_offset: 14,  // After QUIC short header (14 bytes with 8-byte DCID)
            eph_pub_length: 32,
            size_distribution: SizeDistribution {
                dist_type: SizeDistType::Histogram,
                bins: vec![
                    (64, 128, 0.3),
                    (256, 512, 0.4),
                    (768, 1200, 0.3),
                ],
                parametric_type: None,
                parametric_params: None,
            },
            iat_distribution: IATDistribution {
                dist_type: IATDistType::Exponential,
                params: vec![0.1], // Burst-idle pattern
                jitter_range_ms: (0.0, 10.0),
            },
            padding_strategy: PaddingStrategy::MatchDistribution,
            fsm_states: vec![
                FSMState {
                    state_id: 0,
                    transitions: vec![],
                },
            ],
            fsm_initial_state: 0,
            signature_vector: vec![0.0; 64],
            reverse_profile: None,
            signature: [0u8; 64],
            header_spec: Some(header_spec),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rand::rngs::StdRng;
    use rand::SeedableRng;

    #[test]
    fn test_stun_binding_generation() {
        let spec = HeaderSpec::StunBinding {
            magic_cookie: true,
            transaction_id: TransactionIdMode::Random,
        };
        
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
        let spec = HeaderSpec::QuicInitial {
            version: 0x00000001,
            dcid_len: 8,
        };
        
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
        let spec = HeaderSpec::DnsQuery {
            flags: 0x0100,
        };
        
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
        let spec = HeaderSpec::TlsRecord {
            content_type: 0x17,
            version: 0x0303,
        };
        
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
        let stun = HeaderSpec::StunBinding {
            magic_cookie: true,
            transaction_id: TransactionIdMode::Random,
        };
        assert_eq!(stun.min_length(), 20);
        
        let quic = HeaderSpec::QuicInitial {
            version: 0x00000001,
            dcid_len: 8,
        };
        // 1 (header_form) + 4 (version) + 1 (dcid_len) + 8 (dcid) = 14
        assert_eq!(quic.min_length(), 14);
        
        let dns = HeaderSpec::DnsQuery { flags: 0x0100 };
        assert_eq!(dns.min_length(), 12);
        
        let tls = HeaderSpec::TlsRecord {
            content_type: 0x17,
            version: 0x0303,
        };
        assert_eq!(tls.min_length(), 5);
    }

    #[test]
    fn test_static_generation_deterministic() {
        let spec = HeaderSpec::StunBinding {
            magic_cookie: true,
            transaction_id: TransactionIdMode::Random,
        };
        
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
}
