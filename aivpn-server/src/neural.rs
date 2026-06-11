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

use serde::{Deserialize, Serialize};
use std::collections::{HashMap, VecDeque};
use tracing::{debug, info};

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
    pub packet_sizes: VecDeque<u16>,
    /// Inter-arrival times (ms)
    pub inter_arrivals: VecDeque<f64>,
    /// Byte-level entropy samples
    pub entropy_samples: VecDeque<f64>,
    /// Packets per second
    pub pps: f64,
    /// Bytes per second
    pub bps: f64,
}

impl TrafficStats {
    pub fn new() -> Self {
        Self {
            packet_sizes: VecDeque::with_capacity(256),
            inter_arrivals: VecDeque::with_capacity(256),
            entropy_samples: VecDeque::with_capacity(256),
            pps: 0.0,
            bps: 0.0,
        }
    }

    /// Add packet sample
    pub fn add_packet(&mut self, size: u16, iat_ms: f64, entropy: f64) {
        self.packet_sizes.push_back(size);
        self.inter_arrivals.push_back(iat_ms);
        self.entropy_samples.push_back(entropy);
        // Keep last 256 samples
        if self.packet_sizes.len() > 256 {
            self.packet_sizes.pop_front();
            self.inter_arrivals.pop_front();
            self.entropy_samples.pop_front();
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
    w1: Vec<f32>, // [hidden × FEAT_DIM] row-major
    b1: Vec<f32>, // [hidden]
    w2: Vec<f32>, // [FEAT_DIM × hidden] row-major
    b2: Vec<f32>, // [FEAT_DIM]
    hidden: usize,
}

impl BakedMaskEncoder {
    /// Bake an encoder from a mask's signature vector.
    pub fn from_signature(signature: &[f32], hidden: usize) -> Self {
        assert!(
            signature.len() >= FEAT_DIM,
            "signature must have at least {} floats",
            FEAT_DIM
        );

        // Deterministic seed: BLAKE3 hash of the signature serialized as LE f32s.
        // The XOF is expanded to give every weight an independent pseudo-random
        // value — this produces full-rank W1/W2 (the previous scheme made every
        // row a scalar multiple of signature[], collapsing to a rank-1 matrix).
        let sig_bytes: Vec<u8> = signature.iter().flat_map(|f| f.to_le_bytes()).collect();

        let mut w1 = vec![0.0f32; hidden * FEAT_DIM];
        let mut b1 = vec![0.0f32; hidden];
        let mut w2 = vec![0.0f32; FEAT_DIM * hidden];
        let mut b2 = vec![0.0f32; FEAT_DIM];

        let scale = (2.0 / (FEAT_DIM + hidden) as f32).sqrt();
        let total = (hidden * FEAT_DIM + hidden + FEAT_DIM * hidden + FEAT_DIM) * 2;
        let mut xof_buf = vec![0u8; total];
        blake3::Hasher::new()
            .update(&sig_bytes)
            .finalize_xof()
            .fill(&mut xof_buf);
        let mut cur = 0usize;

        for i in 0..hidden {
            for j in 0..FEAT_DIM {
                let v = i16::from_le_bytes([xof_buf[cur], xof_buf[cur + 1]]) as f32;
                w1[i * FEAT_DIM + j] = v / i16::MAX as f32 * scale;
                cur += 2;
            }
            let v = i16::from_le_bytes([xof_buf[cur], xof_buf[cur + 1]]) as f32;
            b1[i] = v / i16::MAX as f32 * 0.01;
            cur += 2;
        }
        for j in 0..FEAT_DIM {
            for i in 0..hidden {
                let v = i16::from_le_bytes([xof_buf[cur], xof_buf[cur + 1]]) as f32;
                w2[j * hidden + i] = v / i16::MAX as f32 * scale;
                cur += 2;
            }
            let v = i16::from_le_bytes([xof_buf[cur], xof_buf[cur + 1]]) as f32;
            b2[j] = v / i16::MAX as f32 * 0.01;
            cur += 2;
        }

        Self {
            w1,
            b1,
            w2,
            b2,
            hidden,
        }
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
            let mut binned = false;
            for j in 0..15 {
                if (size as usize) >= bins[j] && (size as usize) < bins[j + 1] {
                    features[j] += 1.0;
                    binned = true;
                    break;
                }
            }
            if !binned {
                features[15] += 1.0; // sizes >= 1280
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
        let variance = stats
            .inter_arrivals
            .iter()
            .map(|&x| (x - mean).powi(2))
            .sum::<f64>()
            / n;
        let std_dev = variance.sqrt();
        let max_val = stats
            .inter_arrivals
            .iter()
            .cloned()
            .fold(f64::NEG_INFINITY, f64::max);
        let min_val = stats
            .inter_arrivals
            .iter()
            .cloned()
            .fold(f64::INFINITY, f64::min);

        features[16] = (mean / 100.0) as f32;
        features[17] = (std_dev / 100.0) as f32;
        features[18] = (max_val / 1000.0) as f32;
        features[19] = (min_val / 1000.0) as f32;
        // Percentiles
        let mut sorted: Vec<f64> = stats.inter_arrivals.iter().cloned().collect();
        sorted.sort_unstable_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        features[20] = (sorted[sorted.len() / 4] / 100.0) as f32;
        features[21] = (sorted[sorted.len() / 2] / 100.0) as f32;
        features[22] = (sorted[sorted.len() * 3 / 4] / 100.0) as f32;
        features[23] = if mean > 0.0 {
            (std_dev / mean) as f32
        } else {
            0.0
        };

        // Block 2b (24-31): skewness, kurtosis, lag-1 autocorrelation, burst/gap
        // fractions, mean-absolute jitter, p10, p90 — reuses sorted from above.
        if std_dev > 1e-9 {
            let skew = stats
                .inter_arrivals
                .iter()
                .map(|&x| ((x - mean) / std_dev).powi(3))
                .sum::<f64>()
                / n;
            features[24] = (skew / 10.0).clamp(-1.0, 1.0) as f32;
            let kurt = stats
                .inter_arrivals
                .iter()
                .map(|&x| ((x - mean) / std_dev).powi(4))
                .sum::<f64>()
                / n
                - 3.0;
            features[25] = (kurt / 10.0).clamp(-1.0, 1.0) as f32;
        }
        let ns_iat = stats.inter_arrivals.len();
        if ns_iat >= 2 {
            let ac: f64 = stats
                .inter_arrivals
                .iter()
                .zip(stats.inter_arrivals.iter().skip(1))
                .map(|(&a, &b)| (a - mean) * (b - mean))
                .sum::<f64>()
                / (ns_iat - 1) as f64;
            features[26] = (ac / (variance + 1e-9)).clamp(-1.0, 1.0) as f32;
            if mean > 1e-9 {
                let jitter = stats
                    .inter_arrivals
                    .iter()
                    .zip(stats.inter_arrivals.iter().skip(1))
                    .map(|(&a, &b)| (a - b).abs())
                    .sum::<f64>()
                    / (ns_iat - 1) as f64;
                features[29] = (jitter / mean).clamp(0.0, 10.0) as f32 / 10.0;
            }
        }
        features[27] = stats.inter_arrivals.iter().filter(|&&t| t < 2.0).count() as f32 / n as f32;
        features[28] =
            stats.inter_arrivals.iter().filter(|&&t| t > 500.0).count() as f32 / n as f32;
        let ns_s = sorted.len();
        features[30] = (sorted[(ns_s / 10).max(0)] / 100.0) as f32;
        features[31] = (sorted[(ns_s * 9 / 10).min(ns_s - 1)] / 100.0) as f32;
    }

    // Block 3 (32–47): Entropy features
    if !stats.entropy_samples.is_empty() {
        let n = stats.entropy_samples.len() as f64;
        let mean = stats.entropy_samples.iter().sum::<f64>() / n;
        let variance = stats
            .entropy_samples
            .iter()
            .map(|&x| (x - mean).powi(2))
            .sum::<f64>()
            / n;
        features[32] = (mean / 8.0) as f32;
        features[33] = (variance.sqrt() / 8.0) as f32;
        let max_val = stats
            .entropy_samples
            .iter()
            .cloned()
            .fold(f64::NEG_INFINITY, f64::max);
        let min_val = stats
            .entropy_samples
            .iter()
            .cloned()
            .fold(f64::INFINITY, f64::min);
        features[34] = (max_val / 8.0) as f32;
        features[35] = (min_val / 8.0) as f32;

        // Block 3b (36-47): entropy percentiles, high/low fractions, skewness,
        // lag-1 autocorrelation, trend, IAT-entropy Pearson correlation.
        let mut ent_sorted: Vec<f64> = stats.entropy_samples.iter().cloned().collect();
        ent_sorted.sort_unstable_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        let ns_e = ent_sorted.len();
        features[36] = (ent_sorted[ns_e / 4] / 8.0) as f32;
        features[37] = (ent_sorted[ns_e / 2] / 8.0) as f32;
        features[38] = (ent_sorted[ns_e * 3 / 4] / 8.0) as f32;
        features[39] = stats.entropy_samples.iter().filter(|&&e| e > 7.5).count() as f32 / n as f32;
        features[40] = stats.entropy_samples.iter().filter(|&&e| e < 4.0).count() as f32 / n as f32;
        if variance > 1e-18 {
            let std_e = variance.sqrt();
            let skew = stats
                .entropy_samples
                .iter()
                .map(|&x| ((x - mean) / std_e).powi(3))
                .sum::<f64>()
                / n;
            features[41] = (skew / 3.0).clamp(-1.0, 1.0) as f32;
            if ns_e >= 2 {
                let ac: f64 = stats
                    .entropy_samples
                    .iter()
                    .zip(stats.entropy_samples.iter().skip(1))
                    .map(|(&a, &b)| (a - mean) * (b - mean))
                    .sum::<f64>()
                    / (ns_e - 1) as f64;
                features[42] = (ac / (variance + 1e-9)).clamp(-1.0, 1.0) as f32;
            }
        }
        if ns_e >= 4 {
            let half = ns_e / 2;
            let first: f64 = stats.entropy_samples.iter().take(half).sum::<f64>() / half as f64;
            let second: f64 =
                stats.entropy_samples.iter().skip(ns_e - half).sum::<f64>() / half as f64;
            features[43] = ((second - first) / 8.0) as f32;
        }
        if !stats.inter_arrivals.is_empty() {
            let m = stats.inter_arrivals.len().min(ns_e);
            let im = stats.inter_arrivals.iter().take(m).sum::<f64>() / m as f64;
            let em = stats.entropy_samples.iter().take(m).sum::<f64>() / m as f64;
            let cov = stats
                .inter_arrivals
                .iter()
                .take(m)
                .zip(stats.entropy_samples.iter().take(m))
                .map(|(&a, &b)| (a - im) * (b - em))
                .sum::<f64>()
                / m as f64;
            let iv = stats
                .inter_arrivals
                .iter()
                .take(m)
                .map(|&a| (a - im).powi(2))
                .sum::<f64>()
                / m as f64;
            let ev = stats
                .entropy_samples
                .iter()
                .take(m)
                .map(|&b| (b - em).powi(2))
                .sum::<f64>()
                / m as f64;
            let denom = (iv * ev).sqrt();
            features[44] = if denom > 1e-9 {
                (cov / denom).clamp(-1.0, 1.0) as f32
            } else {
                0.0
            };
        }
        // [45-47] reserved
    }

    // Block 4 (48–63): Temporal features
    features[48] = stats.pps as f32 / 1000.0;
    features[49] = stats.bps as f32 / 1_000_000.0;
    if !stats.packet_sizes.is_empty() {
        let n = stats.packet_sizes.len() as f32;
        let mean_size: f32 = stats.packet_sizes.iter().map(|&s| s as f32).sum::<f32>() / n;
        features[50] = mean_size / 1500.0;
        let var: f32 = stats
            .packet_sizes
            .iter()
            .map(|&s| (s as f32 - mean_size).powi(2))
            .sum::<f32>()
            / n;
        features[51] = var.sqrt() / 1500.0;
    }

    // Block 4b (52-63): packet size fractions, adjacent-size variation,
    // size percentiles (p10/p25/p75/p90), timing regularity index,
    // interactive-range IAT fraction, throughput proxy.
    if !stats.packet_sizes.is_empty() {
        let n = stats.packet_sizes.len() as f32;
        features[52] = stats.packet_sizes.iter().filter(|&&s| s > 1000).count() as f32 / n;
        features[53] = stats.packet_sizes.iter().filter(|&&s| s < 100).count() as f32 / n;
        features[54] = (1.0 - features[52] - features[53]).max(0.0);
        let ns_ps = stats.packet_sizes.len();
        if ns_ps >= 2 {
            features[55] = stats
                .packet_sizes
                .iter()
                .zip(stats.packet_sizes.iter().skip(1))
                .map(|(&a, &b)| (a as f32 - b as f32).abs() / 1500.0)
                .sum::<f32>()
                / (ns_ps - 1) as f32;
        }
        let mut sz_sorted: Vec<f32> = stats.packet_sizes.iter().map(|&s| s as f32).collect();
        sz_sorted.sort_unstable_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        let ns_sz = sz_sorted.len();
        features[56] = sz_sorted[(ns_sz / 10).max(0)] / 1500.0;
        features[57] = sz_sorted[ns_sz / 4] / 1500.0;
        features[58] = sz_sorted[ns_sz * 3 / 4] / 1500.0;
        features[59] = sz_sorted[(ns_sz * 9 / 10).min(ns_sz - 1)] / 1500.0;
    }
    if !stats.inter_arrivals.is_empty() {
        let n = stats.inter_arrivals.len() as f64;
        let mean = stats.inter_arrivals.iter().sum::<f64>() / n;
        let variance = stats
            .inter_arrivals
            .iter()
            .map(|&x| (x - mean).powi(2))
            .sum::<f64>()
            / n;
        if mean > 1e-9 {
            features[60] = (1.0 - variance.sqrt() / mean).clamp(0.0, 1.0) as f32;
        }
        // Fraction of IATs in the 10–100 ms range (interactive traffic indicator)
        features[61] = stats
            .inter_arrivals
            .iter()
            .filter(|&&t| t >= 10.0 && t <= 100.0)
            .count() as f32
            / n as f32;
        if !stats.packet_sizes.is_empty() {
            let mean_sz = stats.packet_sizes.iter().map(|&s| s as f32).sum::<f32>()
                / stats.packet_sizes.len() as f32;
            features[62] = (stats.pps as f32 * mean_sz / 1_000_000.0).clamp(0.0, 1.0);
        }
        // [63] reserved
    }

    features
}

// ── Per-mask MSE Auto-calibration ────────────────────────────────────────────

/// Minimum samples before the adaptive threshold is used instead of the
/// configured default.  500 packets ≈ a few seconds of typical VPN traffic.
const MIN_CALIBRATION_SAMPLES: u64 = 500;

/// Running MSE statistics for one mask, using Welford's online algorithm for
/// numerically stable mean and variance without storing raw samples.
#[derive(Debug, Default)]
struct MaskCalibration {
    count: u64,
    mean: f64,
    m2: f64,
}

impl MaskCalibration {
    fn update(&mut self, mse: f32) {
        self.count += 1;
        let x = mse as f64;
        let delta = x - self.mean;
        self.mean += delta / self.count as f64;
        self.m2 += delta * (x - self.mean);
    }

    fn std_dev(&self) -> f64 {
        if self.count < 2 {
            return 0.0;
        }
        (self.m2 / (self.count - 1) as f64).sqrt()
    }

    /// Returns `(compromised, warning)` adaptive thresholds once enough
    /// samples are collected:
    ///   compromised = mean + 3σ
    ///   warning     = mean + 1.5σ
    fn adaptive_thresholds(&self) -> Option<(f32, f32)> {
        if self.count < MIN_CALIBRATION_SAMPLES {
            return None;
        }
        let s = self.std_dev();
        Some(((self.mean + 3.0 * s) as f32, (self.mean + 1.5 * s) as f32))
    }
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
        let losses = self
            .mask_packet_loss
            .entry(mask_id.to_string())
            .or_default();
        losses.push(packet_loss);
        if losses.len() > 100 {
            losses.remove(0);
        }

        let rtts = self.mask_rtt.entry(mask_id.to_string()).or_default();
        rtts.push(rtt_ms);
        if rtts.len() > 100 {
            rtts.remove(0);
        }
    }

    pub fn is_anomalous(&self, mask_id: &str) -> bool {
        if let Some(losses) = self.mask_packet_loss.get(mask_id) {
            if losses.len() >= 10 {
                let avg = losses.iter().sum::<f64>() / losses.len() as f64;
                if avg > self.baseline_loss * 5.0 {
                    return true;
                }
            }
        }
        if let Some(rtts) = self.mask_rtt.get(mask_id) {
            if rtts.len() >= 10 {
                let avg = rtts.iter().sum::<f64>() / rtts.len() as f64;
                if avg > self.baseline_rtt * 3.0 {
                    return true;
                }
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

    /// Per-mask MSE calibration (auto-updates adaptive threshold)
    mask_calibration: dashmap::DashMap<String, MaskCalibration>,

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
            mask_calibration: dashmap::DashMap::new(),
            loaded: false,
        })
    }

    /// Load model (marks as ready — no external model files needed)
    pub fn load_model(&mut self) -> Result<(), String> {
        self.loaded = true;
        info!(
            "Baked Mask Encoder ready (hidden={}, ~{}KB per mask)",
            self.config.hidden_size,
            (FEAT_DIM * self.config.hidden_size * 2 + self.config.hidden_size + FEAT_DIM) * 4
                / 1024
        );
        Ok(())
    }

    /// Register mask — bakes its signature into a dedicated MLP encoder
    pub fn register_mask(&mut self, mask: &MaskProfile) -> Result<(), String> {
        if mask.signature_vector.len() < FEAT_DIM {
            return Err(format!(
                "Mask '{}' signature_vector too short: {} < {}",
                mask.mask_id,
                mask.signature_vector.len(),
                FEAT_DIM
            ));
        }
        let encoder =
            BakedMaskEncoder::from_signature(&mask.signature_vector, self.config.hidden_size);
        debug!(
            "Baked encoder for mask '{}' ({} bytes)",
            mask.mask_id,
            encoder.memory_bytes()
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

        // Update per-mask calibration and log when it first completes.
        {
            let mut cal = self
                .mask_calibration
                .entry(mask_id.to_string())
                .or_default();
            cal.update(mse);
            if cal.count == MIN_CALIBRATION_SAMPLES {
                if let Some((comp, warn)) = cal.adaptive_thresholds() {
                    info!(
                        "neural: mask '{}' calibrated — compromised={:.4} warning={:.4} (n={})",
                        mask_id, comp, warn, cal.count
                    );
                }
            }
        }

        // Use adaptive thresholds once calibrated; fall back to config values.
        let (comp_thresh, warn_thresh) = self
            .mask_calibration
            .get(mask_id)
            .and_then(|c| c.adaptive_thresholds())
            .unwrap_or((
                self.config.compromised_threshold,
                self.config.warning_threshold,
            ));

        let status = if mse > comp_thresh {
            ResonanceStatus::Compromised
        } else if mse > warn_thresh {
            ResonanceStatus::Warning
        } else {
            ResonanceStatus::Healthy
        };

        Ok(ResonanceResult {
            mse,
            status,
            message: None,
        })
    }

    /// Record telemetry for anomaly detection
    pub fn record_telemetry(&mut self, mask_id: &str, packet_loss: f64, rtt_ms: f64) {
        if self.config.enable_anomaly_detection {
            self.anomaly_detector
                .record_metrics(mask_id, packet_loss, rtt_ms);
        }
    }

    /// Check if mask is anomalous (possible DPI blocking)
    pub fn is_mask_anomalous(&self, mask_id: &str) -> bool {
        self.anomaly_detector.is_anomalous(mask_id)
    }

    /// Get or create session stats
    pub fn get_or_create_stats(&self, session_id: [u8; 16]) -> TrafficStats {
        self.session_stats
            .entry(session_id)
            .or_insert_with(TrafficStats::new)
            .clone()
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

    /// Calibration status for a mask: `(samples_seen, mean_mse, adaptive_threshold)`.
    /// Returns `None` if no data yet.  `adaptive_threshold` is 0.0 until
    /// MIN_CALIBRATION_SAMPLES are collected (config default is used until then).
    pub fn calibration_status(&self, mask_id: &str) -> Option<(u64, f32, f32)> {
        self.mask_calibration.get(mask_id).map(|c| {
            let thresh = c.adaptive_thresholds().map(|(t, _)| t).unwrap_or(0.0);
            (c.count, c.mean as f32, thresh)
        })
    }
}
