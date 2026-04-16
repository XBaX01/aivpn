//! Mask Generator — Analysis, Profile Building, and Self-Testing
//!
//! Analyzes recorded traffic metadata to generate MaskProfile:
//! 1. Statistical analysis (size distribution, IAT patterns, FSM states)
//! 2. Build MaskProfile from analysis results
//! 3. Self-test via Kolmogorov-Smirnov test
//! 4. Store and broadcast

use std::sync::Arc;

use tracing::{info, error};

use aivpn_common::mask::*;
use aivpn_common::recording::{PacketMetadata, Direction};
use aivpn_common::error::{Error, Result};

use crate::mask_store::{MaskStore, MaskEntry, MaskStats};

// ─── Analysis Result ─────────────────────────────────────────────────────────

/// Result of traffic analysis
#[allow(dead_code)]
struct AnalysisResult {
    uplink_size_modes: Vec<Mode>,
    uplink_size_mean: f32,
    uplink_size_std: f32,
    downlink_size_modes: Vec<Mode>,
    uplink_iat_mean_ms: f32,
    uplink_iat_std_ms: f32,
    uplink_periods: Vec<Period>,
    downlink_iat_mean_ms: f32,
    downlink_iat_std_ms: f32,
    downlink_periods: Vec<Period>,
    header_template: Vec<u8>,
    header_spec: Option<HeaderSpec>,
    fsm_states: Vec<FSMState>,
    fsm_initial_state: u16,
    mean_entropy: f32,
    total_packets: u64,
    duration_secs: u64,
    confidence: f32,
}

/// Statistical mode (peak in distribution)
struct Mode {
    center: f32,
    std_dev: f32,
    weight: f32,
}

/// Periodic IAT pattern
#[allow(dead_code)]
struct Period {
    period_ms: f32,
    jitter_ms: f32,
    weight: f32,
}

/// Self-test result
struct SelfTestResult {
    ks_size: f32,
    ks_iat: f32,
    entropy_match: f32,
    passed: bool,
    confidence: f32,
}

// ─── Main Pipeline ───────────────────────────────────────────────────────────

/// Generate mask from recorded traffic and store it
pub async fn generate_and_store_mask(
    service: &str,
    packets: &[PacketMetadata],
    store: &Arc<MaskStore>,
) -> Result<String> {
    // 1. Analyze traffic
    let analysis = analyze_traffic(service, packets)?;
    info!(
        "Analysis complete for '{}': {} packets, {} modes, confidence={:.2}",
        service, analysis.total_packets, analysis.uplink_size_modes.len(), analysis.confidence
    );

    // 2. Build MaskProfile
    let profile = build_mask_profile(service, &analysis)?;

    // 3. Self-test
    let test = self_test(&profile, packets)?;
    if !test.passed {
        return Err(Error::Mask(format!(
            "Self-test failed: KS_size={:.3}, KS_iat={:.3}, entropy={:.3}",
            test.ks_size, test.ks_iat, test.entropy_match
        )));
    }
    info!(
        "Self-test passed for '{}': KS_size={:.3}, KS_iat={:.3}, confidence={:.2}",
        service, test.ks_size, test.ks_iat, test.confidence
    );

    // 4. Store
    let mask_id = profile.mask_id.clone();
    store.add_mask(MaskEntry {
        profile,
        stats: MaskStats {
            mask_id: mask_id.clone(),
            times_used: 0,
            times_failed: 0,
            success_rate: 1.0,
            confidence: test.confidence,
            is_active: true,
            created_by: "auto".into(),
            created_at: current_unix_secs(),
            last_used: None,
        },
    })?;

    // 5. Register in catalog
    store.register_in_catalog(&mask_id)?;

    // 6. Broadcast to clients
    if let Err(e) = store.broadcast_mask_update(&mask_id).await {
        error!("Failed to broadcast mask '{}': {}", mask_id, e);
    }

    Ok(mask_id)
}

// ─── Traffic Analysis ────────────────────────────────────────────────────────

fn analyze_traffic(_service: &str, packets: &[PacketMetadata]) -> Result<AnalysisResult> {
    let uplink: Vec<&PacketMetadata> = packets
        .iter()
        .filter(|p| p.direction == Direction::Uplink)
        .collect();
    let downlink: Vec<&PacketMetadata> = packets
        .iter()
        .filter(|p| p.direction == Direction::Downlink)
        .collect();

    if uplink.len() < 100 {
        return Err(Error::Mask("Too few uplink packets (need >= 100)".into()));
    }

    // Size distributions
    let uplink_sizes: Vec<u16> = uplink.iter().map(|p| p.size).collect();
    let uplink_modes = find_modes_histogram(&uplink_sizes, 32);
    let uplink_size_mean = mean_u16(&uplink_sizes);
    let uplink_size_std = std_dev_u16(&uplink_sizes);

    let downlink_sizes: Vec<u16> = downlink.iter().map(|p| p.size).collect();
    let downlink_modes = find_modes_histogram(&downlink_sizes, 32);

    // IAT distributions
    let uplink_iats: Vec<f64> = uplink.iter().map(|p| p.iat_ms).collect();
    let uplink_iat_mean = mean_f64(&uplink_iats);
    let uplink_iat_std = std_dev_f64(&uplink_iats);
    let uplink_periods = find_periods(&uplink_iats);

    let downlink_iats: Vec<f64> = downlink.iter().map(|p| p.iat_ms).collect();
    let downlink_iat_mean = mean_f64(&downlink_iats);
    let downlink_iat_std = std_dev_f64(&downlink_iats);
    let downlink_periods = find_periods(&downlink_iats);

    // Header consensus
    let headers: Vec<Vec<u8>> = packets.iter().map(|p| p.header_prefix.clone()).collect();
    let header_template = header_consensus(&headers);
    
    // Infer HeaderSpec from traffic patterns (Issue #30 fix)
    let header_spec = infer_header_spec(&headers);

    // FSM from size change-point detection
    let (fsm_states, fsm_initial) = build_fsm_from_sizes(&uplink_sizes);

    // Entropy
    let entropies: Vec<f32> = packets.iter().map(|p| p.entropy).collect();
    let mean_entropy = mean_f32(&entropies);

    // Confidence score
    let confidence = compute_confidence(
        packets.len(),
        uplink_modes.len(),
        uplink_iat_std as f32,
        mean_entropy,
    );

    // Duration
    let duration_secs = if packets.len() >= 2 {
        let first_ts = packets.first().map(|p| p.timestamp_ns).unwrap_or(0);
        let last_ts = packets.last().map(|p| p.timestamp_ns).unwrap_or(0);
        (last_ts.saturating_sub(first_ts)) / 1_000_000_000
    } else {
        0
    };

    Ok(AnalysisResult {
        uplink_size_modes: uplink_modes,
        uplink_size_mean,
        uplink_size_std,
        downlink_size_modes: downlink_modes,
        uplink_iat_mean_ms: uplink_iat_mean as f32,
        uplink_iat_std_ms: uplink_iat_std as f32,
        uplink_periods,
        downlink_iat_mean_ms: downlink_iat_mean as f32,
        downlink_iat_std_ms: downlink_iat_std as f32,
        downlink_periods,
        header_template,
        header_spec,
        fsm_states,
        fsm_initial_state: fsm_initial,
        mean_entropy,
        total_packets: packets.len() as u64,
        duration_secs,
        confidence,
    })
}

// ─── Mode Detection (Histogram) ─────────────────────────────────────────────

fn find_modes_histogram(sizes: &[u16], num_bins: usize) -> Vec<Mode> {
    if sizes.is_empty() {
        return vec![Mode { center: 64.0, std_dev: 32.0, weight: 1.0 }];
    }

    let min = *sizes.iter().min().unwrap_or(&0);
    let max = *sizes.iter().max().unwrap_or(&1500);
    let bin_width = ((max - min) as f32 / num_bins as f32).max(1.0);

    let mut bins = vec![0usize; num_bins];
    for &size in sizes {
        let bin = ((size as f32 - min as f32) / bin_width).min(num_bins as f32 - 1.0) as usize;
        bins[bin] += 1;
    }

    let total = sizes.len() as f32;
    let mut modes = Vec::new();

    for i in 1..bins.len().saturating_sub(1) {
        if bins[i] > bins[i - 1] && bins[i] > bins[i + 1] && bins[i] > total as usize / 20 {
            let center = min as f32 + (i as f32 + 0.5) * bin_width;
            let weight = bins[i] as f32 / total;

            // Compute local std dev around this mode
            let mut sum_sq = 0.0f32;
            let mut count = 0usize;
            for (j, &bin_count) in bins.iter().enumerate() {
                if bin_count > 0 {
                    let bc = min as f32 + (j as f32 + 0.5) * bin_width;
                    sum_sq += (bin_count as f32) * (bc - center).powi(2);
                    count += bin_count;
                }
            }
            let std_dev = if count > 0 { (sum_sq / count as f32).sqrt() } else { bin_width };
            modes.push(Mode { center, std_dev, weight });
        }
    }

    // Fallback: single mode from mean/std
    if modes.is_empty() {
        let mean = mean_u16(sizes);
        let std = std_dev_u16(sizes);
        modes.push(Mode { center: mean, std_dev: std, weight: 1.0 });
    }

    modes
}

// ─── Period Detection (IAT) ─────────────────────────────────────────────────

fn find_periods(iats: &[f64]) -> Vec<Period> {
    if iats.len() < 10 {
        return vec![];
    }

    let mean_iat = mean_f64(iats);
    let std_iat = std_dev_f64(iats);

    if mean_iat < 1e-9 {
        return vec![];
    }

    if std_iat / mean_iat < 0.3 {
        // Stable single period
        vec![Period {
            period_ms: mean_iat as f32,
            jitter_ms: std_iat as f32,
            weight: 1.0,
        }]
    } else {
        // Bimodal — split by median
        let mut sorted = iats.to_vec();
        sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        let median = sorted[sorted.len() / 2];

        let low: Vec<f64> = iats.iter().filter(|&&x| x <= median).copied().collect();
        let high: Vec<f64> = iats.iter().filter(|&&x| x > median).copied().collect();

        let mut periods = Vec::new();
        if !low.is_empty() {
            periods.push(Period {
                period_ms: mean_f64(&low) as f32,
                jitter_ms: std_dev_f64(&low) as f32,
                weight: low.len() as f32 / iats.len() as f32,
            });
        }
        if !high.is_empty() {
            periods.push(Period {
                period_ms: mean_f64(&high) as f32,
                jitter_ms: std_dev_f64(&high) as f32,
                weight: high.len() as f32 / iats.len() as f32,
            });
        }
        periods
    }
}

// ─── Change Point Detection → FSM ───────────────────────────────────────────

fn build_fsm_from_sizes(sizes: &[u16]) -> (Vec<FSMState>, u16) {
    if sizes.len() < 50 {
        return (vec![FSMState {
            state_id: 0,
            transitions: vec![],
        }], 0);
    }

    // 1. Detect change points (mean shift > 2σ in window of 20)
    let window = 20;
    let _global_mean = mean_u16(sizes);
    let global_std = std_dev_u16(sizes);
    let threshold = 2.0 * global_std;

    let mut change_points = Vec::new();
    for i in window..sizes.len().saturating_sub(window) {
        let before = sizes[i - window..i].iter().map(|&x| x as f32).sum::<f32>() / window as f32;
        let after = sizes[i..i + window].iter().map(|&x| x as f32).sum::<f32>() / window as f32;
        if (before - after).abs() > threshold {
            if change_points.is_empty() || i - *change_points.last().unwrap() > 10 {
                change_points.push(i);
            }
        }
    }

    // 2. Create segments
    let mut segments: Vec<&[u16]> = Vec::new();
    let mut start = 0;
    for &cp in &change_points {
        if cp > start {
            segments.push(&sizes[start..cp]);
        }
        start = cp;
    }
    if start < sizes.len() {
        segments.push(&sizes[start..]);
    }

    // Limit number of segments to avoid explosion
    if segments.is_empty() {
        segments.push(sizes);
    }

    // 3. Cluster segments by mean (threshold 100 bytes)
    let seg_means: Vec<f32> = segments
        .iter()
        .map(|s| s.iter().map(|&x| x as f32).sum::<f32>() / s.len().max(1) as f32)
        .collect();

    let mut clusters: Vec<Vec<usize>> = Vec::new();
    for (i, &seg_mean) in seg_means.iter().enumerate() {
        let mut assigned = false;
        for cluster in &mut clusters {
            let cm: f32 = cluster.iter().map(|&j| seg_means[j]).sum::<f32>() / cluster.len() as f32;
            if (seg_mean - cm).abs() < 100.0 {
                cluster.push(i);
                assigned = true;
                break;
            }
        }
        if !assigned {
            clusters.push(vec![i]);
        }
    }

    // Limit to max 8 FSM states
    clusters.truncate(8);

    // 4. Build FSM states with transitions
    let mut transitions: Vec<Vec<(u16, u32)>> = vec![vec![]; clusters.len()];
    for i in 0..segments.len().saturating_sub(1) {
        let from = clusters.iter().position(|c| c.contains(&i)).unwrap_or(0) as u16;
        let to = clusters.iter().position(|c| c.contains(&(i + 1))).unwrap_or(0) as u16;
        if (from as usize) < clusters.len() && (to as usize) < clusters.len() {
            if let Some(e) = transitions[from as usize].iter_mut().find(|(s, _)| *s == to) {
                e.1 += 1;
            } else {
                transitions[from as usize].push((to, 1));
            }
        }
    }

    // 5. Convert to FSMState
    let fsm_states: Vec<FSMState> = clusters
        .iter()
        .enumerate()
        .map(|(i, _cluster)| {
            let total: u32 = transitions[i].iter().map(|(_, c)| c).sum();
            let trans: Vec<FSMTransition> = transitions[i]
                .iter()
                .map(|(next, count)| FSMTransition {
                    condition: TransitionCondition::Random(*count as f32 / total.max(1) as f32),
                    next_state: *next,
                    size_override: None,
                    iat_override: None,
                    padding_override: None,
                })
                .collect();

            FSMState {
                state_id: i as u16,
                transitions: trans,
            }
        })
        .collect();

    (fsm_states, 0)
}

// ─── Header Consensus ────────────────────────────────────────────────────────

fn header_consensus(headers: &[Vec<u8>]) -> Vec<u8> {
    if headers.is_empty() {
        return vec![0u8; 8];
    }

    let len = headers.iter().map(|h| h.len()).min().unwrap_or(8).min(16);
    let mut result = Vec::with_capacity(len);

    for i in 0..len {
        let mut counts = [0u32; 256];
        for h in headers {
            if i < h.len() {
                counts[h[i] as usize] += 1;
            }
        }
        let max_count = counts.iter().max().copied().unwrap_or(0);
        let max_byte = counts.iter().position(|&c| c == max_count).unwrap_or(0) as u8;

        // >70% same → fixed byte, otherwise random
        if max_count as f32 / headers.len() as f32 > 0.7 {
            result.push(max_byte);
        } else {
            result.push(rand::random());
        }
    }
    result
}

// ─── HeaderSpec Inference (Issue #30 fix) ─────────────────────────────────────

/// Infer HeaderSpec from observed traffic patterns
///
/// Analyzes the consistency of header bytes across packets to determine
/// if the traffic matches known protocol patterns (STUN, QUIC, DNS, TLS)
/// and generates an appropriate HeaderSpec for dynamic per-packet generation.
fn infer_header_spec(headers: &[Vec<u8>]) -> Option<HeaderSpec> {
    if headers.is_empty() {
        return None;
    }
    
    // Analyze byte consistency at each position
    let max_len = headers.iter().map(|h| h.len()).max().unwrap_or(0).min(20);
    if max_len < 4 {
        return None;  // Not enough data
    }
    
    // Calculate consistency ratio for each byte position
    let mut consistency: Vec<f32> = Vec::with_capacity(max_len);
    for i in 0..max_len {
        let mut counts = [0u32; 256];
        for h in headers {
            if i < h.len() {
                counts[h[i] as usize] += 1;
            }
        }
        let max_count = counts.iter().max().copied().unwrap_or(0);
        let ratio = max_count as f32 / headers.len() as f32;
        consistency.push(ratio);
    }
    
    // Check for STUN pattern: first 2 bytes should be consistent (type),
    // bytes 4-7 should be magic cookie (0x21, 0x12, 0xA4, 0x42) or consistent
    if max_len >= 8 {
        let byte4_consistent = *consistency.get(4).unwrap_or(&0.0) > 0.5;
        let byte5_consistent = *consistency.get(5).unwrap_or(&0.0) > 0.5;
        let byte6_consistent = *consistency.get(6).unwrap_or(&0.0) > 0.5;
        let byte7_consistent = *consistency.get(7).unwrap_or(&0.0) > 0.5;
        
        // Check if bytes 4-7 look like STUN magic cookie
        let mut magic_counts = [0u32; 256];
        for h in headers {
            if h.len() >= 8 {
                magic_counts[h[4] as usize] += 1;
            }
        }
        let magic_0x21_ratio = magic_counts[0x21] as f32 / headers.len() as f32;
        
        if byte4_consistent && byte5_consistent && byte6_consistent && byte7_consistent
            && magic_0x21_ratio > 0.3 {
            return Some(HeaderSpec::StunBinding {
                magic_cookie: magic_0x21_ratio > 0.5,
                transaction_id: TransactionIdMode::Random,
            });
        }
    }
    
    // Check for QUIC pattern: first byte should be 0xC0-0xCF (long packet)
    let mut first_byte_counts = [0u32; 256];
    for h in headers {
        if !h.is_empty() {
            first_byte_counts[h[0] as usize] += 1;
        }
    }
    let quic_header_ratio = (0xC0..=0xCF).map(|b| first_byte_counts[b as usize]).sum::<u32>() as f32 / headers.len() as f32;
    
    if quic_header_ratio > 0.5 {
        // Check for consistent version bytes
        let mut version_counts: [u32; 256] = [0; 256];
        for h in headers {
            if h.len() >= 2 {
                version_counts[h[1] as usize] += 1;
            }
        }
        let max_ver = version_counts.iter().max().copied().unwrap_or(0);
        let version_consistent = max_ver as f32 / headers.len() as f32 > 0.5;
        
        if version_consistent {
            return Some(HeaderSpec::QuicInitial {
                version: 0x00000001,  // QUIC v1
                dcid_len: 8,
            });
        }
    }
    
    // Check for DNS pattern: bytes 2-3 should be consistent (flags)
    if max_len >= 4 {
        let flags_consistent = *consistency.get(2).unwrap_or(&0.0) > 0.5
            && *consistency.get(3).unwrap_or(&0.0) > 0.5;
        
        if flags_consistent {
            // Check for standard DNS query flags (0x0100)
            let mut flag_hi_counts = [0u32; 256];
            for h in headers {
                if h.len() >= 3 {
                    flag_hi_counts[h[2] as usize] += 1;
                }
            }
            let dns_flag_ratio = flag_hi_counts[0x01] as f32 / headers.len() as f32;
            
            if dns_flag_ratio > 0.3 {
                return Some(HeaderSpec::DnsQuery {
                    flags: 0x0100,
                });
            }
        }
    }
    
    // Check for TLS pattern: first byte should be consistent (content type)
    if !headers.is_empty() {
        let tls_content_types = [0x14, 0x16, 0x17, 0x15];  // Handshake, Alert, Application Data, ChangeCipherSpec
        let tls_ratio: u32 = tls_content_types.iter()
            .map(|&t| first_byte_counts[t as usize])
            .sum();
        let tls_ratio = tls_ratio as f32 / headers.len() as f32;
        
        if tls_ratio > 0.5 && *consistency.get(0).unwrap_or(&0.0) > 0.5 {
            // Determine most likely content type
            let dominant_type = first_byte_counts[0x17] > first_byte_counts[0x16];
            return Some(HeaderSpec::TlsRecord {
                content_type: if dominant_type { 0x17 } else { 0x16 },
                version: 0x0303,  // TLS 1.2
            });
        }
    }
    
    // Fallback: use RawPrefix with randomization for variable bytes
    if headers.len() >= 4 {
        let template = headers[0].clone();
        let randomize_indices: Vec<usize> = consistency.iter()
            .enumerate()
            .filter(|(_, &c)| c < 0.7)  // Randomize positions with <70% consistency
            .map(|(i, _)| i)
            .collect();
        
        if !randomize_indices.is_empty() && randomize_indices.len() < template.len() {
            return Some(HeaderSpec::RawPrefix {
                prefix_hex: hex::encode(&template),
                randomize_indices,
            });
        }
    }
    
    // No pattern detected
    None
}

// ─── Confidence Scoring ─────────────────────────────────────────────────────

fn compute_confidence(
    total_packets: usize,
    num_modes: usize,
    _iat_std: f32,
    mean_entropy: f32,
) -> f32 {
    let mut score = 0.0f32;

    if total_packets >= 10_000 { score += 0.3; }
    else if total_packets >= 5_000 { score += 0.25; }
    else if total_packets >= 1_000 { score += 0.2; }
    else if total_packets >= 500 { score += 0.15; }

    if num_modes >= 2 { score += 0.3; }
    else if num_modes == 1 { score += 0.2; }

    if mean_entropy > 7.0 { score += 0.2; }
    else if mean_entropy > 6.0 { score += 0.15; }

    score.min(1.0)
}

// ─── MaskProfile Builder ─────────────────────────────────────────────────────

fn build_mask_profile(service: &str, analysis: &AnalysisResult) -> Result<MaskProfile> {
    let mask_id = format!("auto_{}_v1", service.replace(' ', "_").to_lowercase());

    // Determine size distribution from detected modes
    let bins: Vec<(u16, u16, f32)> = analysis
        .uplink_size_modes
        .iter()
        .map(|mode| {
            let min = (mode.center - mode.std_dev).max(1.0) as u16;
            let max = (mode.center + mode.std_dev).max(min as f32 + 1.0) as u16;
            (min, max, mode.weight)
        })
        .collect();

    let size_distribution = if bins.is_empty() {
        SizeDistribution {
            dist_type: SizeDistType::Parametric,
            bins: vec![],
            parametric_type: Some(ParametricType::LogNormal),
            parametric_params: Some(vec![
                (analysis.uplink_size_mean.max(1.0)).ln() as f64,
                (analysis.uplink_size_std / analysis.uplink_size_mean.max(1.0)).max(0.1) as f64,
            ]),
        }
    } else {
        SizeDistribution {
            dist_type: SizeDistType::Histogram,
            bins,
            parametric_type: None,
            parametric_params: None,
        }
    };

    // IAT distribution from analysis
    let iat_distribution = if !analysis.uplink_periods.is_empty() {
        // Use Empirical type with actual period values for better statistical match
        if analysis.uplink_periods.len() >= 2 {
            // Bimodal: use Gamma distribution to capture wide variance
            let p1 = &analysis.uplink_periods[0];
            let p2 = &analysis.uplink_periods[1];
            let weighted_mean = p1.period_ms * p1.weight + p2.period_ms * p2.weight;
            let k = (weighted_mean / analysis.uplink_iat_std_ms.max(1.0)).powi(2).max(1.0);
            let theta = analysis.uplink_iat_std_ms.powi(2) as f64 / weighted_mean.max(1.0) as f64;
            IATDistribution {
                dist_type: IATDistType::Gamma,
                params: vec![k as f64, theta],
                jitter_range_ms: (0.0, analysis.uplink_iat_std_ms.max(1.0) as f64 * 0.1),
            }
        } else {
            let main_period = &analysis.uplink_periods[0];
            IATDistribution {
                dist_type: IATDistType::LogNormal,
                params: vec![
                    (main_period.period_ms.max(1.0)).ln() as f64,
                    (main_period.jitter_ms / main_period.period_ms.max(1.0)).max(0.1) as f64,
                ],
                jitter_range_ms: (
                    0.0,
                    analysis.uplink_iat_std_ms.max(1.0) as f64 * 0.1,
                ),
            }
        }
    } else {
        IATDistribution {
            dist_type: IATDistType::LogNormal,
            params: vec![
                (analysis.uplink_iat_mean_ms.max(1.0)).ln() as f64,
                (analysis.uplink_iat_std_ms / analysis.uplink_iat_mean_ms.max(1.0)).max(0.1) as f64,
            ],
            jitter_range_ms: (0.0, 10.0),
        }
    };

    // Determine eph_pub_offset based on header_spec or header_template
    let eph_pub_offset = if let Some(ref spec) = analysis.header_spec {
        spec.min_length() as u16
    } else {
        analysis.header_template.len().min(4) as u16
    };
    
    // Determine spoof_protocol based on header_spec
    let spoof_protocol = match &analysis.header_spec {
        Some(HeaderSpec::StunBinding { .. }) => SpoofProtocol::WebRTC_STUN,
        Some(HeaderSpec::QuicInitial { .. }) => SpoofProtocol::QUIC,
        Some(HeaderSpec::DnsQuery { .. }) => SpoofProtocol::DNS_over_UDP,
        Some(HeaderSpec::TlsRecord { .. }) => SpoofProtocol::HTTPS_H2,
        _ => SpoofProtocol::QUIC,
    };
    
    Ok(MaskProfile {
        mask_id,
        version: 2,  // Version 2 for HeaderSpec support
        created_at: current_unix_secs(),
        expires_at: current_unix_secs() + 365 * 24 * 3600, // 1 year
        spoof_protocol,
        header_template: analysis.header_template.clone(),
        eph_pub_offset,
        eph_pub_length: 32,
        size_distribution,
        iat_distribution,
        padding_strategy: PaddingStrategy::RandomUniform { min: 0, max: 64 },
        fsm_states: analysis.fsm_states.clone(),
        fsm_initial_state: analysis.fsm_initial_state,
        signature_vector: vec![0.0; 64], // TODO: generate from neural model
        reverse_profile: None,
        signature: [0u8; 64], // TODO: sign with Ed25519
        header_spec: analysis.header_spec.clone(),
    })
}

// ─── Self-Test (Kolmogorov-Smirnov) ─────────────────────────────────────────

fn self_test(profile: &MaskProfile, packets: &[PacketMetadata]) -> Result<SelfTestResult> {
    // Generate synthetic samples from the mask profile
    let mut rng = rand::thread_rng();
    let sample_count = packets
        .iter()
        .filter(|p| p.direction == Direction::Uplink)
        .count()
        .min(5000);

    let synthetic_sizes: Vec<f64> = (0..sample_count)
        .map(|_| profile.size_distribution.sample(&mut rng) as f64)
        .collect();
    let synthetic_iats: Vec<f64> = (0..sample_count)
        .map(|_| profile.iat_distribution.sample(&mut rng))
        .collect();

    // Real data
    let real_sizes: Vec<f64> = packets
        .iter()
        .filter(|p| p.direction == Direction::Uplink)
        .take(5000)
        .map(|p| p.size as f64)
        .collect();
    let real_iats: Vec<f64> = packets
        .iter()
        .filter(|p| p.direction == Direction::Uplink)
        .take(5000)
        .map(|p| p.iat_ms)
        .collect();

    // KS tests
    let ks_size = ks_test(&synthetic_sizes, &real_sizes);
    let ks_iat = ks_test(&synthetic_iats, &real_iats);

    // Entropy match
    let real_entropy: f64 =
        packets.iter().map(|p| p.entropy as f64).sum::<f64>() / packets.len().max(1) as f64;
    let entropy_match = (real_entropy - 7.0).abs().min(1.0) as f32; // Expect high entropy

    // KS test acceptance thresholds:
    // DPI classifiers typically operate at KS > 0.5, so 0.4 gives sufficient margin.
    // Size distribution is harder to match exactly due to bimodality.
    let ks_threshold = 0.4;
    let passed = ks_size < ks_threshold && ks_iat < ks_threshold && entropy_match < 0.5;
    let confidence = if passed {
        // Penalize higher KS values proportionally
        let ks_avg = (ks_size + ks_iat) / 2.0;
        (1.0 - ks_avg / ks_threshold).max(0.1)
    } else {
        0.0
    };

    Ok(SelfTestResult {
        ks_size,
        ks_iat,
        entropy_match,
        passed,
        confidence,
    })
}

/// Two-sample Kolmogorov-Smirnov test statistic
fn ks_test(sample1: &[f64], sample2: &[f64]) -> f32 {
    if sample1.is_empty() || sample2.is_empty() {
        return 1.0;
    }

    // Sort both samples
    let mut s1 = sample1.to_vec();
    let mut s2 = sample2.to_vec();
    s1.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    s2.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));

    let n1 = s1.len() as f64;
    let n2 = s2.len() as f64;
    let mut max_diff: f64 = 0.0;

    let mut i = 0usize;
    let mut j = 0usize;

    // Merge-walk both sorted samples for O(n log n) KS statistic
    while i < s1.len() && j < s2.len() {
        let cdf1 = (i + 1) as f64 / n1;
        let cdf2 = (j + 1) as f64 / n2;

        if s1[i] <= s2[j] {
            max_diff = max_diff.max((cdf1 - (j as f64 / n2)).abs());
            i += 1;
        } else {
            max_diff = max_diff.max(((i as f64 / n1) - cdf2).abs());
            j += 1;
        }
    }

    // Handle remaining elements
    while i < s1.len() {
        let cdf1 = (i + 1) as f64 / n1;
        max_diff = max_diff.max((cdf1 - 1.0).abs());
        i += 1;
    }
    while j < s2.len() {
        let cdf2 = (j + 1) as f64 / n2;
        max_diff = max_diff.max((1.0 - cdf2).abs());
        j += 1;
    }

    max_diff as f32
}

// ─── Statistical Helpers ─────────────────────────────────────────────────────

fn mean_u16(data: &[u16]) -> f32 {
    if data.is_empty() { return 0.0; }
    data.iter().map(|&x| x as f32).sum::<f32>() / data.len() as f32
}

fn std_dev_u16(data: &[u16]) -> f32 {
    if data.is_empty() { return 0.0; }
    let m = mean_u16(data);
    let variance = data.iter().map(|&x| (x as f32 - m).powi(2)).sum::<f32>() / data.len() as f32;
    variance.sqrt()
}

fn mean_f64(data: &[f64]) -> f64 {
    if data.is_empty() { return 0.0; }
    data.iter().sum::<f64>() / data.len() as f64
}

fn std_dev_f64(data: &[f64]) -> f64 {
    if data.is_empty() { return 0.0; }
    let m = mean_f64(data);
    let variance = data.iter().map(|x| (x - m).powi(2)).sum::<f64>() / data.len() as f64;
    variance.sqrt()
}

fn mean_f32(data: &[f32]) -> f32 {
    if data.is_empty() { return 0.0; }
    data.iter().sum::<f32>() / data.len() as f32
}

fn current_unix_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}
