//! Recording Manager — Active Recording Session Management
//!
//! Manages the lifecycle of traffic recording sessions:
//! - Start/stop recording for a VPN session
//! - Record packet metadata into the active session
//! - Trigger async mask generation on stop
//! - O(1) is_recording check for hot-path performance

use std::sync::Arc;

use dashmap::DashMap;
use tracing::{info, warn, error};

use aivpn_common::recording::{PacketMetadata, RecordingSession};

use crate::mask_gen::generate_and_store_mask;
use crate::mask_store::MaskStore;

/// Recording Manager — manages active recording sessions
pub struct RecordingManager {
    /// Active recordings: session_id → RecordingSession
    active: DashMap<[u8; 16], RecordingSession>,
    /// Mask store reference for saving generated masks
    store: Arc<MaskStore>,
}

impl RecordingManager {
    /// Create a new RecordingManager
    pub fn new(store: Arc<MaskStore>) -> Self {
        Self {
            active: DashMap::new(),
            store,
        }
    }

    /// Start recording for a session
    pub fn start(&self, session_id: [u8; 16], service: String, admin_key_id: String) {
        let session = RecordingSession::new(session_id, service.clone(), admin_key_id);
        self.active.insert(session_id, session);
        info!(
            "Recording started for service '{}' (session {:02x}{:02x}...)",
            service, session_id[0], session_id[1]
        );
    }

    /// Record a packet's metadata into the active session
    pub fn record_packet(&self, session_id: [u8; 16], meta: PacketMetadata) {
        if let Some(mut session) = self.active.get_mut(&session_id) {
            session.record(meta);
        }
    }

    /// Stop recording and trigger async mask generation
    ///
    /// Returns the service name if recording was active and had enough data,
    /// or None if recording was too short / had too few packets.
    pub fn stop(&self, session_id: [u8; 16]) -> Option<String> {
        let session = self.active.remove(&session_id)?.1;

        let service = session.service.clone();
        let total = session.total_packets;
        let duration = session.duration_secs();

        if !session.has_enough_data() {
            warn!(
                "Recording for '{}' stopped with insufficient data: {} packets, {}s (need {} packets, {}s)",
                service,
                total,
                duration,
                aivpn_common::recording::MIN_RECORDING_PACKETS,
                aivpn_common::recording::MIN_RECORDING_DURATION_SECS,
            );
            return None;
        }

        info!(
            "Recording stopped for '{}': {} packets, {}s — generating mask...",
            service, total, duration,
        );

        let store = self.store.clone();
        let packets = session.packets;

        // Spawn async mask generation task
        tokio::spawn(async move {
            match generate_and_store_mask(&service, &packets, &store).await {
                Ok(mask_id) => {
                    info!("✅ Mask generated: '{}' for service '{}'", mask_id, service);
                }
                Err(e) => {
                    error!("❌ Mask generation failed for '{}': {}", service, e);
                }
            }
        });

        Some(session.service)
    }

    /// Check if a session is currently being recorded (O(1))
    pub fn is_recording(&self, session_id: &[u8; 16]) -> bool {
        self.active.contains_key(session_id)
    }

    /// Get status of a recording session
    pub fn status(&self, session_id: &[u8; 16]) -> Option<RecordingStatus> {
        self.active.get(session_id).map(|session| RecordingStatus {
            service: session.service.clone(),
            total_packets: session.total_packets,
            duration_secs: session.duration_secs(),
            uplink_count: session.running_stats.uplink_count,
            downlink_count: session.running_stats.downlink_count,
            mean_entropy: session.running_stats.mean_entropy(),
        })
    }

    /// Get all active recording session IDs
    pub fn active_sessions(&self) -> Vec<[u8; 16]> {
        self.active.iter().map(|e| *e.key()).collect()
    }
}

/// Status information for a recording session
#[derive(Debug, Clone)]
pub struct RecordingStatus {
    pub service: String,
    pub total_packets: u64,
    pub duration_secs: u64,
    pub uplink_count: u64,
    pub downlink_count: u64,
    pub mean_entropy: f64,
}
