//! Recording Manager — Active Recording Session Management
//!
//! Manages the lifecycle of traffic recording sessions:
//! - Start/stop recording for a VPN session
//! - Record packet metadata into the active session
//! - Trigger async mask generation on stop
//! - O(1) is_recording check for hot-path performance

use std::sync::Arc;

use dashmap::DashMap;
use tracing::{info, warn};

use aivpn_common::recording::{PacketMetadata, RecordingSession};

use crate::mask_store::MaskStore;

/// Recording Manager — manages active recording sessions
pub struct RecordingManager {
    /// Active recordings: session_id → RecordingSession
    active: DashMap<[u8; 16], RecordingSession>,
    /// Mask store reference for saving generated masks
    store: Arc<MaskStore>,
}

#[derive(Debug, Clone)]
pub struct CompletedRecording {
    pub session_id: [u8; 16],
    pub service: String,
    pub admin_key_id: String,
    pub packets: Vec<PacketMetadata>,
    pub total_packets: u64,
    pub duration_secs: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecordingStopReason {
    ManualStop,
    EnoughData,
    IdleTimeout,
    SessionEnded,
}

#[derive(Debug, Clone)]
pub struct IncompleteRecording {
    pub session_id: [u8; 16],
    pub service: String,
    pub admin_key_id: String,
    pub total_packets: u64,
    pub duration_secs: u64,
    pub reason: RecordingStopReason,
}

#[derive(Debug, Clone)]
pub enum RecordingStopOutcome {
    Completed(CompletedRecording),
    Incomplete(IncompleteRecording),
    NotFound,
}

impl RecordingManager {
    /// Create a new RecordingManager
    pub fn new(store: Arc<MaskStore>) -> Self {
        Self {
            active: DashMap::new(),
            store,
        }
    }

    pub fn store(&self) -> Arc<MaskStore> {
        self.store.clone()
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

    fn stop_inner(&self, session_id: [u8; 16], reason: RecordingStopReason) -> RecordingStopOutcome {
        let session = match self.active.remove(&session_id) {
            Some((_, session)) => session,
            None => return RecordingStopOutcome::NotFound,
        };

        let service = session.service.clone();
        let admin_key_id = session.admin_key_id.clone();
        let total = session.total_packets;
        let duration = session.duration_secs();

        if !session.has_enough_data() {
            warn!(
                "Recording for '{}' stopped with insufficient data: {} packets, {}s (reason: {:?}, need {} packets, {}s)",
                service,
                total,
                duration,
                reason,
                aivpn_common::recording::MIN_RECORDING_PACKETS,
                aivpn_common::recording::MIN_RECORDING_DURATION_SECS,
            );
            return RecordingStopOutcome::Incomplete(IncompleteRecording {
                session_id,
                service,
                admin_key_id,
                total_packets: total,
                duration_secs: duration,
                reason,
            });
        }

        info!(
            "Recording stopped for '{}': {} packets, {}s (reason: {:?}) — generating mask...",
            service, total, duration, reason,
        );

        RecordingStopOutcome::Completed(CompletedRecording {
            session_id,
            service,
            admin_key_id,
            packets: session.packets,
            total_packets: total,
            duration_secs: duration,
        })
    }

    /// Stop recording and return the captured session if it has enough data.
    pub fn stop(&self, session_id: [u8; 16]) -> RecordingStopOutcome {
        self.stop_inner(session_id, RecordingStopReason::ManualStop)
    }

    pub fn stop_for_session_end(&self, session_id: [u8; 16]) -> RecordingStopOutcome {
        self.stop_inner(session_id, RecordingStopReason::SessionEnded)
    }

    pub fn take_ready_or_stale(&self, idle_timeout_secs: u64) -> Vec<RecordingStopOutcome> {
        let mut to_finish = Vec::new();
        for entry in self.active.iter() {
            let session = entry.value();
            if session.has_enough_data() {
                to_finish.push((*entry.key(), RecordingStopReason::EnoughData));
            } else if session.is_idle_timed_out(idle_timeout_secs) {
                to_finish.push((*entry.key(), RecordingStopReason::IdleTimeout));
            }
        }

        to_finish
            .into_iter()
            .map(|(session_id, reason)| self.stop_inner(session_id, reason))
            .filter(|outcome| !matches!(outcome, RecordingStopOutcome::NotFound))
            .collect()
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
