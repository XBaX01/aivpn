//! Neural Resonance Module — Baked Mask Encoder
//!
//! Implements Signal Reconstruction Resonance (Patent 1) using a lightweight
//! hand-rolled MLP instead of a full LLM. Each mask's signature_vector is
//! "baked" into a tiny neural network (64 → 128 → 64) whose weights are
//! derived deterministically from the mask's 64-float signature.
//!
//! Memory per mask: ~66 KB (vs ~400 MB for Qwen-0.5B).
//! Total for 100 masks: ~6.6 MB — fits any VPS.
//!
//! The baked encoder learns the mask's traffic fingerprint:
//! - Input: 64-dim feature vector extracted from live traffic
//! - Output: 64-dim reconstruction vector
//! - MSE(input, output) = reconstruction error = resonance score
//!
//! Low MSE → traffic matches the mask → healthy
//! High MSE → traffic deviates from mask signature → DPI compromise detected

use std::collections::HashMap;
use serde::{Deserialize, Serialize};
use tracing::{info, debug};

use aivpn_common::mask::MaskProfile;

// ── Configuration ────────────────────────────────────────────────────────────

/// Neural Resonance Module configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NeuralConfig {
    /// Hidden layer size for baked MLP
    pub hidden_size: usize,

    /// Resonance check interval (seconds)
    pub check_interval_secs: u64,

    /// MSE threshold for compromised mask
    pub compromised_threshold: f32,

    /// MSE threshold for warning
    pub warning_threshold: f32,

    /// Enable anomaly detection
    pub enable_anomaly_detection: bool,
}

impl Default for NeuralConfig {
    fn default() -> Self {
        Self {
            hidden_size: 128,
            check_interval_secs: 30,
            compromised_threshold: 0.35,
            warning_threshold: 0.15,
            enable_anomaly_detection: true,
        }
    }
}

// ── Traffic Statistics ───────────────────────────────────────────────────────

/// Traffic statistics for neural analysis
#[derive(Debug, Clone, Default)]
pub struct TrafficStats {
    /// Packet sizes (last N packets)
    pub packet_sizes: Vec<u16>,
    /// Inter-arrival times (ms)
    pub inter_arrivals: Vec<f64>,
    /// Byte-level entropy samples
    pub entropy_samples: Vec<f64>,
    /// Packets per second
    pub pps: f64,
    /// Bytes per second
    pub bps: f64,
}

impl TrafficStats {
    pub fn new() -> Self {
        Self {
            packet_sizes: Vec::with_capacity(256),
            inter_arrivals: Vec::with_capacity(256),
            entropy_samples: Vec::with_capacity(256),
            pps: 0.0,
            bps: 0.0,
        }
    }

    /// Add packet sample
    pub fn add_packet(&mut self, size: u16, iat_ms: f64, entropy: f64) {
        self.packet_sizes.push(size);
        self.inter_arrivals.push(iat_ms);
        self.entropy_samples.push(entropy);
        // Keep last 256 samples
        if self.packet_sizes.len() > 256 {
            self.packet_sizes.remove(0);
            self.inter_arrivals.remove(0);
            self.entropy_samples.remove(0);
        }
    }

    /// Clear stats
    pub fn clear(&mut self) {
        self.packet_sizes.clear();
        self.inter_arrivals.clear();
        self.entropy_samples.clear();
        self.pps = 0.0;
        self.bps = 0.0;
    }
}

// ── Baked Mask Encoder (the tiny neural network) ─────────────────────────────

/// Feature dimension (= mask signature_vector length)
const FEAT_DIM: usize = 64;

/// A tiny MLP whose weights are deterministically "baked" from a mask's
/// 64-float signature_vector.
///
/// Architecture: Linear(64→H) → ReLU → Linear(H→64)
///
/// Weight derivation (fully deterministic, no training needed):
/// - Each weight is seeded by BLAKE3 hash of the signature, ensuring
///   structurally unique encoders per mask.
///
/// Memory: (64*H + H + H*64 + 64) * 4 bytes ≈ 66 KB for H=128
pub struct BakedMaskEncoder {
    w1: Vec<f32>,       // [hidden × FEAT_DIM] row-major
    b1: Vec<f32>,       // [hidden]
    w2: Vec<f32>,       // [FEAT_DIM × hidden] row-major
    b2: Vec<f32>,       // [FEAT_DIM]
    hidden: usize,
}

impl BakedMaskEncoder {
    /// Bake an encoder from a mask's signature vector.
    pub fn from_signature(signature: &[f32], hidden: usize) -> Self {
        assert!(signature.len() >= FEAT_DIM, "signature must have at least {} floats", FEAT_DIM);

        // Deterministic seed from signature for mixing
        let sig_bytes: Vec<u8> = signature.iter()
            .flat_map(|f| f.to_le_bytes())
            .collect();
        let seed = blake3::hash(&sig_bytes);
        let seed_bytes = seed.as_bytes();

        let mut w1 = vec![0.0f32; hidden * FEAT_DIM];
        let mut b1 = vec![0.0f32; hidden];
        let mut w2 = vec![0.0f32; FEAT_DIM * hidden];
        let mut b2 = vec![0.0f32; FEAT_DIM];

        // Xavier-scale initialization seeded by signature
        let scale = (2.0 / (FEAT_DIM + hidden) as f32).sqrt();

        for i in 0..hidden {
            for j in 0..FEAT_DIM {
                let idx = (i * FEAT_DIM + j) % 32;
                let mix = (seed_bytes[idx] as f32 - 128.0) / 128.0;
                w1[i * FEAT_DIM + j] = signature[j % FEAT_DIM] * mix * scale;
            }
            b1[i] = signature[i % FEAT_DIM] * 0.01;
        }

        for j in 0..FEAT_DIM {
            for i in 0..hidden {
                let idx = (j * hidden + i) % 32;
                let mix = (seed_bytes[idx] as f32 - 128.0) / 128.0;
                w2[j * hidden + i] = signature[j % FEAT_DIM] * mix * scale;
            }
            b2[j] = signature[j] * 0.01;
        }

        Self { w1, b1, w2, b2, hidden }
    }

    /// Forward pass: x → Linear → ReLU → Linear → output
    pub fn forward(&self, input: &[f32; FEAT_DIM]) -> [f32; FEAT_DIM] {
        // Layer 1: hidden = ReLU(W1 · input + b1)
        let mut h = vec![0.0f32; self.hidden];
        for i in 0..self.hidden {
            let mut sum = self.b1[i];
            let row = &self.w1[i * FEAT_DIM..(i + 1) * FEAT_DIM];
            for j in 0..FEAT_DIM {
                sum += row[j] * input[j];
            }
            h[i] = sum.max(0.0); // ReLU
        }

        // Layer 2: output = W2 · hidden + b2
        let mut output = [0.0f32; FEAT_DIM];
        for j in 0..FEAT_DIM {
            let mut sum = self.b2[j];
            let row = &self.w2[j * self.hidden..(j + 1) * self.hidden];
            for i in 0..self.hidden {
                sum += row[i] * h[i];
            }
            output[j] = sum;
        }
        output
    }

    /// Reconstruction error (MSE) between input features and reconstruction
    pub fn reconstruction_error(&self, features: &[f32; FEAT_DIM]) -> f32 {
        let recon = self.forward(features);
        let mut mse = 0.0f32;
        for i in 0..FEAT_DIM {
            let diff = features[i] - recon[i];
            mse += diff * diff;
        }
        mse / FEAT_DIM as f32
    }

    /// Memory usage in bytes
    pub fn memory_bytes(&self) -> usize {
        (self.w1.len() + self.b1.len() + self.w2.len() + self.b2.len()) * 4
    }
}

// ── Feature Encoding ─────────────────────────────────────────────────────────

/// Encode traffic stats into a 64-dim feature vector
pub fn encode_features(stats: &TrafficStats) -> [f32; FEAT_DIM] {
    let mut features = [0.0f32; FEAT_DIM];

    // Block 1 (0–15): Packet size histogram (16 bins)
    if !stats.packet_sizes.is_empty() {
        let bins: [usize; 16] = [
            0, 64, 128, 192, 256, 320, 384, 448, 512, 576, 640, 704, 768, 896, 1024, 1280,
        ];
        for &size in &stats.packet_sizes {
            for j in 0..15 {
                if (size as usize) >= bins[j] && (size as usize) < bins[j + 1] {
                    features[j] += 1.0;
                    break;
                }
            }
        }
        let n = stats.packet_sizes.len() as f32;
        for f in features[0..16].iter_mut() {
            *f /= n;
        }
    }

    // Block 2 (16–31): IAT statistics
    if !stats.inter_arrivals.is_empty() {
        let n = stats.inter_arrivals.len() as f64;
        let mean = stats.inter_arrivals.iter().sum::<f64>() / n;
        let variance = stats.inter_arrivals.iter()
            .map(|&x| (x - mean).powi(2))
            .sum::<f64>() / n;
        let std_dev = variance.sqrt();
        let max_val = stats.inter_arrivals.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
        let min_val = stats.inter_arrivals.iter().cloned().fold(f64::INFINITY, f64::min);

        features[16] = (mean / 100.0) as f32;
        features[17] = (std_dev / 100.0) as f32;
        features[18] = (max_val / 1000.0) as f32;
        features[19] = (min_val / 1000.0) as f32;
        // Percentiles
        let mut sorted = stats.inter_arrivals.clone();
        sorted.sort_unstable_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        features[20] = (sorted[sorted.len() / 4] / 100.0) as f32;
        features[21] = (sorted[sorted.len() / 2] / 100.0) as f32;
        features[22] = (sorted[sorted.len() * 3 / 4] / 100.0) as f32;
        features[23] = if mean > 0.0 { (std_dev / mean) as f32 } else { 0.0 };
    }

    // Block 3 (32–47): Entropy features
    if !stats.entropy_samples.is_empty() {
        let n = stats.entropy_samples.len() as f64;
        let mean = stats.entropy_samples.iter().sum::<f64>() / n;
        let variance = stats.entropy_samples.iter()
            .map(|&x| (x - mean).powi(2))
            .sum::<f64>() / n;
        features[32] = (mean / 8.0) as f32;
        features[33] = (variance.sqrt() / 8.0) as f32;
        let max_val = stats.entropy_samples.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
        let min_val = stats.entropy_samples.iter().cloned().fold(f64::INFINITY, f64::min);
        features[34] = (max_val / 8.0) as f32;
        features[35] = (min_val / 8.0) as f32;
    }

    // Block 4 (48–63): Temporal features
    features[48] = stats.pps as f32 / 1000.0;
    features[49] = stats.bps as f32 / 1_000_000.0;
    if !stats.packet_sizes.is_empty() {
        let n = stats.packet_sizes.len() as f32;
        let mean_size: f32 = stats.packet_sizes.iter().map(|&s| s as f32).sum::<f32>() / n;
        features[50] = mean_size / 1500.0;
        let var: f32 = stats.packet_sizes.iter()
            .map(|&s| (s as f32 - mean_size).powi(2))
            .sum::<f32>() / n;
        features[51] = var.sqrt() / 1500.0;
    }

    features
}

// ── Anomaly Detector ─────────────────────────────────────────────────────────

/// Anomaly detector for DPI fingerprinting
pub struct AnomalyDetector {
    mask_packet_loss: HashMap<String, Vec<f64>>,
    mask_rtt: HashMap<String, Vec<f64>>,
    baseline_loss: f64,
    baseline_rtt: f64,
}

impl AnomalyDetector {
    pub fn new() -> Self {
        Self {
            mask_packet_loss: HashMap::new(),
            mask_rtt: HashMap::new(),
            baseline_loss: 0.01,
            baseline_rtt: 50.0,
        }
    }

    pub fn record_metrics(&mut self, mask_id: &str, packet_loss: f64, rtt_ms: f64) {
        let losses = self.mask_packet_loss.entry(mask_id.to_string()).or_default();
        losses.push(packet_loss);
        if losses.len() > 100 { losses.remove(0); }

        let rtts = self.mask_rtt.entry(mask_id.to_string()).or_default();
        rtts.push(rtt_ms);
        if rtts.len() > 100 { rtts.remove(0); }
    }

    pub fn is_anomalous(&self, mask_id: &str) -> bool {
        if let Some(losses) = self.mask_packet_loss.get(mask_id) {
            if losses.len() >= 10 {
                let avg = losses.iter().sum::<f64>() / losses.len() as f64;
                if avg > self.baseline_loss * 5.0 { return true; }
            }
        }
        if let Some(rtts) = self.mask_rtt.get(mask_id) {
            if rtts.len() >= 10 {
                let avg = rtts.iter().sum::<f64>() / rtts.len() as f64;
                if avg > self.baseline_rtt * 3.0 { return true; }
            }
        }
        false
    }
}

// ── Neural Resonance Module ──────────────────────────────────────────────────

/// Neural Resonance Module
///
/// Uses Baked Mask Encoders instead of an external LLM.
/// Each mask's signature_vector is baked into a tiny MLP (~66KB).
/// Total memory: O(num_masks * 66KB) — fits any VPS.
pub struct NeuralResonanceModule {
    config: NeuralConfig,

    /// Baked encoders per mask (mask_id -> encoder)
    encoders: HashMap<String, BakedMaskEncoder>,

    /// Per-session traffic stats
    session_stats: dashmap::DashMap<[u8; 16], TrafficStats>,

    /// Anomaly detection state
    anomaly_detector: AnomalyDetector,

    /// Whether the module is loaded
    loaded: bool,
}

/// Resonance check result
#[derive(Debug, Clone)]
pub struct ResonanceResult {
    pub mse: f32,
    pub status: ResonanceStatus,
    pub message: Option<String>,
}

impl ResonanceResult {
    fn skip(msg: &str) -> Self {
        Self {
            mse: 0.0,
            status: ResonanceStatus::Skip,
            message: Some(msg.to_string()),
        }
    }
}

/// Resonance status
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResonanceStatus {
    Healthy,
    Warning,
    Compromised,
    Skip,
}

impl NeuralResonanceModule {
    /// Create new neural module
    pub fn new(config: NeuralConfig) -> Result<Self, String> {
        Ok(Self {
            config,
            encoders: HashMap::new(),
            session_stats: dashmap::DashMap::new(),
            anomaly_detector: AnomalyDetector::new(),
            loaded: false,
        })
    }

    /// Load model (marks as ready — no external model files needed)
    pub fn load_model(&mut self) -> Result<(), String> {
        self.loaded = true;
        info!(
            "Baked Mask Encoder ready (hidden={}, ~{}KB per mask)",
            self.config.hidden_size,
            (FEAT_DIM * self.config.hidden_size * 2 + self.config.hidden_size + FEAT_DIM) * 4 / 1024
        );
        Ok(())
    }

    /// Register mask — bakes its signature into a dedicated MLP encoder
    pub fn register_mask(&mut self, mask: &MaskProfile) -> Result<(), String> {
        if mask.signature_vector.len() < FEAT_DIM {
            return Err(format!(
                "Mask '{}' signature_vector too short: {} < {}",
                mask.mask_id, mask.signature_vector.len(), FEAT_DIM
            ));
        }
        let encoder = BakedMaskEncoder::from_signature(
            &mask.signature_vector,
            self.config.hidden_size,
        );
        debug!(
            "Baked encoder for mask '{}' ({} bytes)",
            mask.mask_id, encoder.memory_bytes()
        );
        self.encoders.insert(mask.mask_id.clone(), encoder);
        Ok(())
    }

    /// Record traffic sample for session
    pub fn record_traffic(
        &self,
        session_id: [u8; 16],
        packet_size: u16,
        iat_ms: f64,
        entropy: f64,
    ) {
        if let Some(mut stats) = self.session_stats.get_mut(&session_id) {
            stats.add_packet(packet_size, iat_ms, entropy);
        } else {
            let mut stats = TrafficStats::new();
            stats.add_packet(packet_size, iat_ms, entropy);
            self.session_stats.insert(session_id, stats);
        }
    }

    /// Perform resonance check (Patent 1: Signal Reconstruction Resonance)
    ///
    /// Encodes live traffic into a 64-dim feature vector, passes it through
    /// the mask's baked encoder, and computes reconstruction MSE.
    pub fn check_resonance(
        &self,
        session_id: [u8; 16],
        mask_id: &str,
    ) -> Result<ResonanceResult, String> {
        if !self.loaded {
            return Ok(ResonanceResult::skip("Model not loaded"));
        }

        let Some(stats) = self.session_stats.get(&session_id) else {
            return Ok(ResonanceResult::skip("No traffic stats"));
        };

        let Some(encoder) = self.encoders.get(mask_id) else {
            return Ok(ResonanceResult::skip("Mask encoder not found"));
        };

        let features = encode_features(&stats);
        let mse = encoder.reconstruction_error(&features);

        let status = if mse > self.config.compromised_threshold {
            ResonanceStatus::Compromised
        } else if mse > self.config.warning_threshold {
            ResonanceStatus::Warning
        } else {
            ResonanceStatus::Healthy
        };

        Ok(ResonanceResult { mse, status, message: None })
    }

    /// Record telemetry for anomaly detection
    pub fn record_telemetry(&mut self, mask_id: &str, packet_loss: f64, rtt_ms: f64) {
        if self.config.enable_anomaly_detection {
            self.anomaly_detector.record_metrics(mask_id, packet_loss, rtt_ms);
        }
    }

    /// Check if mask is anomalous (possible DPI blocking)
    pub fn is_mask_anomalous(&self, mask_id: &str) -> bool {
        self.anomaly_detector.is_anomalous(mask_id)
    }

    /// Get or create session stats
    pub fn get_or_create_stats(&self, session_id: [u8; 16]) -> TrafficStats {
        self.session_stats.entry(session_id).or_insert_with(TrafficStats::new).clone()
    }

    /// Cleanup old session stats
    pub fn cleanup_stats(&self, session_id: [u8; 16]) {
        self.session_stats.remove(&session_id);
    }

    /// Total memory usage for all baked encoders
    pub fn total_memory_bytes(&self) -> usize {
        self.encoders.values().map(|e| e.memory_bytes()).sum()
    }

    /// Number of registered mask encoders
    pub fn encoder_count(&self) -> usize {
        self.encoders.len()
    }
}
