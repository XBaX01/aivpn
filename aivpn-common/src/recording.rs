//! Auto Mask Recording — Shared Data Models
//!
//! Packet metadata, recording sessions, and incremental statistics
//! for automatic mask generation from live traffic analysis.

use serde::{Deserialize, Serialize};
use std::time::Instant;

/// Direction of packet flow
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Direction {
    Uplink,
    Downlink,
}

/// Metadata extracted from a single packet (no payload — privacy-safe)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PacketMetadata {
    /// Packet direction (uplink or downlink)
    pub direction: Direction,
    /// Total packet size in bytes
    pub size: u16,
    /// Inter-arrival time from previous packet in milliseconds
    pub iat_ms: f64,
    /// Shannon entropy of packet payload (0.0–8.0)
    pub entropy: f32,
    /// First 16 bytes of the mask-dependent header
    pub header_prefix: Vec<u8>,
    /// Monotonic timestamp in nanoseconds
    pub timestamp_ns: u64,
}

/// Running statistics computed incrementally (O(1) per update)
#[derive(Debug, Clone, Default)]
pub struct RunningStats {
    pub uplink_count: u64,
    pub uplink_sum: u64,
    pub uplink_sum_sq: u64,
    pub downlink_count: u64,
    pub downlink_sum: u64,
    pub downlink_sum_sq: u64,
    pub uplink_iat_sum: f64,
    pub uplink_iat_sum_sq: f64,
    pub downlink_iat_sum: f64,
    pub downlink_iat_sum_sq: f64,
    pub entropy_sum: f64,
    pub entropy_count: u64,
}

impl RunningStats {
    /// Update statistics with a new packet metadata entry
    pub fn update(&mut self, meta: &PacketMetadata) {
        match meta.direction {
            Direction::Uplink => {
                self.uplink_count += 1;
                self.uplink_sum += meta.size as u64;
                self.uplink_sum_sq += (meta.size as u64).pow(2);
                self.uplink_iat_sum += meta.iat_ms;
                self.uplink_iat_sum_sq += meta.iat_ms.powi(2);
            }
            Direction::Downlink => {
                self.downlink_count += 1;
                self.downlink_sum += meta.size as u64;
                self.downlink_sum_sq += (meta.size as u64).pow(2);
                self.downlink_iat_sum += meta.iat_ms;
                self.downlink_iat_sum_sq += meta.iat_ms.powi(2);
            }
        }
        self.entropy_count += 1;
        self.entropy_sum += meta.entropy as f64;
    }

    /// Get mean entropy across all recorded packets
    pub fn mean_entropy(&self) -> f64 {
        if self.entropy_count == 0 {
            return 0.0;
        }
        self.entropy_sum / self.entropy_count as f64
    }
}

/// Active recording session on the server
pub struct RecordingSession {
    /// Unique session ID (from VPN session)
    pub session_id: [u8; 16],
    /// Service name being recorded (e.g. "yandex_telemost")
    pub service: String,
    /// Admin key ID that authorized this recording
    pub admin_key_id: String,
    /// When recording started
    pub started_at: Instant,
    /// Collected packet metadata (capped at MAX_RECORDING_PACKETS)
    pub packets: Vec<PacketMetadata>,
    /// Total packets observed (may exceed stored packets)
    pub total_packets: u64,
    /// Incremental statistics (always up to date)
    pub running_stats: RunningStats,
}

/// Maximum packets stored in a single recording session
pub const MAX_RECORDING_PACKETS: usize = 100_000;

/// Minimum packets required for mask generation
pub const MIN_RECORDING_PACKETS: u64 = 500;

/// Minimum recording duration in seconds
pub const MIN_RECORDING_DURATION_SECS: u64 = 60;

impl RecordingSession {
    /// Create a new recording session
    pub fn new(session_id: [u8; 16], service: String, admin_key_id: String) -> Self {
        Self {
            session_id,
            service,
            admin_key_id,
            started_at: Instant::now(),
            packets: Vec::with_capacity(50_000),
            total_packets: 0,
            running_stats: RunningStats::default(),
        }
    }

    /// Record a packet's metadata
    pub fn record(&mut self, meta: PacketMetadata) {
        if self.packets.len() < MAX_RECORDING_PACKETS {
            self.packets.push(meta.clone());
        }
        self.total_packets += 1;
        self.running_stats.update(&meta);
    }

    /// Check if we have enough data for mask generation
    pub fn has_enough_data(&self) -> bool {
        self.total_packets >= MIN_RECORDING_PACKETS
            && self.started_at.elapsed().as_secs() >= MIN_RECORDING_DURATION_SECS
    }

    /// Get recording duration in seconds
    pub fn duration_secs(&self) -> u64 {
        self.started_at.elapsed().as_secs()
    }
}
