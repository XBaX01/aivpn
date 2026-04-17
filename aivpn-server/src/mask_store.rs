//! Mask Store — Storage and Rating System for Auto-Generated Masks
//!
//! Stores MaskProfile + MaskStats pairs with automatic deactivation
//! when success rate drops below threshold. Persists to disk.

use std::path::PathBuf;
use std::sync::Arc;

use dashmap::DashMap;
use serde::{Deserialize, Serialize};
use tracing::{info, warn, error};

use aivpn_common::mask::{MaskProfile, preset_masks};
use aivpn_common::error::Result;

use crate::gateway::MaskCatalog;

/// Success rate threshold — masks below this are deactivated
const DEACTIVATION_THRESHOLD: f32 = 0.80;

/// Minimum usages before deactivation can trigger
const MIN_USAGES_FOR_DEACTIVATION: u64 = 100;

/// Mask statistics for rating system
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MaskStats {
    pub mask_id: String,
    pub times_used: u64,
    pub times_failed: u64,
    pub success_rate: f32,
    pub confidence: f32,
    pub is_active: bool,
    pub created_by: String,
    pub created_at: u64,
    pub last_used: Option<u64>,
}

/// Combined mask profile + statistics
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MaskEntry {
    pub profile: MaskProfile,
    pub stats: MaskStats,
}

/// Mask store with rating system and disk persistence
pub struct MaskStore {
    /// All masks (mask_id → MaskEntry)
    masks: DashMap<String, MaskEntry>,
    /// Reference to the gateway's mask catalog for registration
    catalog: Arc<MaskCatalog>,
    /// Storage directory for mask files
    storage_dir: PathBuf,
}

impl MaskStore {
    /// Create a new mask store
    pub fn new(catalog: Arc<MaskCatalog>, storage_dir: PathBuf) -> Self {
        let store = Self {
            masks: DashMap::new(),
            catalog,
            storage_dir,
        };
        store.bootstrap_builtin_masks();
        // Load existing masks from disk
        store.load_from_disk();
        store
    }

    /// Add a new mask entry
    pub fn add_mask(&self, entry: MaskEntry) -> Result<()> {
        let mask_id = entry.stats.mask_id.clone();
        info!("Storing mask '{}' (confidence: {:.2})", mask_id, entry.stats.confidence);

        // Save to disk
        self.save_to_disk(&mask_id, &entry);

        // Register in catalog for neural resonance
        self.catalog.register_mask(entry.profile.clone());

        // Insert into in-memory store
        self.masks.insert(mask_id, entry);
        Ok(())
    }

    /// Register mask in the gateway catalog
    pub fn register_in_catalog(&self, mask_id: &str) -> Result<()> {
        if let Some(entry) = self.masks.get(mask_id) {
            self.catalog.register_mask(entry.value().profile.clone());
        }
        Ok(())
    }

    /// Record successful usage of a mask
    pub fn record_usage(&self, mask_id: &str) {
        if let Some(mut entry) = self.masks.get_mut(mask_id) {
            entry.stats.times_used += 1;
            entry.stats.success_rate = if entry.stats.times_used > 0 {
                1.0 - entry.stats.times_failed as f32 / entry.stats.times_used as f32
            } else {
                1.0
            };
            entry.stats.last_used = Some(current_unix_secs());
            self.save_stats_to_disk(mask_id, &entry.stats);
        }
    }

    /// Record a failure (DPI block detected)
    pub fn record_failure(&self, mask_id: &str) {
        if let Some(mut entry) = self.masks.get_mut(mask_id) {
            entry.stats.times_used += 1;
            entry.stats.times_failed += 1;
            entry.stats.success_rate = if entry.stats.times_used > 0 {
                1.0 - entry.stats.times_failed as f32 / entry.stats.times_used as f32
            } else {
                1.0
            };

            // Auto-deactivation check
            if entry.stats.success_rate < DEACTIVATION_THRESHOLD
                && entry.stats.times_used > MIN_USAGES_FOR_DEACTIVATION
            {
                entry.stats.is_active = false;
                self.catalog.remove_mask(mask_id);
                warn!(
                    "Mask '{}' deactivated: success={:.1}% ({}/{} failures)",
                    mask_id,
                    entry.stats.success_rate * 100.0,
                    entry.stats.times_failed,
                    entry.stats.times_used
                );
            }
            self.save_stats_to_disk(mask_id, &entry.stats);
        }
    }

    /// List all masks with their stats
    pub fn list_masks(&self) -> Vec<MaskEntry> {
        self.masks.iter().map(|e| e.value().clone()).collect()
    }

    /// Get a specific mask entry
    pub fn get_mask(&self, mask_id: &str) -> Option<MaskEntry> {
        self.masks.get(mask_id).map(|e| e.value().clone())
    }

    /// Delete a mask
    pub fn delete_mask(&self, mask_id: &str) {
        self.masks.remove(mask_id);
        self.catalog.remove_mask(mask_id);
        // Remove disk files
        let json_path = self.storage_dir.join(format!("{}.json", mask_id));
        let stats_path = self.storage_dir.join(format!("{}.stats", mask_id));
        let _ = std::fs::remove_file(&json_path);
        let _ = std::fs::remove_file(&stats_path);
        info!("Deleted mask '{}'", mask_id);
    }

    /// Broadcast mask update to all connected clients (placeholder)
    pub async fn broadcast_mask_update(&self, mask_id: &str) -> Result<()> {
        if let Some(entry) = self.masks.get(mask_id) {
            // Serialize mask profile for distribution
            let _profile_data = rmp_serde::to_vec(&entry.value().profile)
                .map_err(|e| aivpn_common::error::Error::Serialization(e.to_string()))?;
            // TODO: broadcast to all active sessions via ControlPayload::MaskUpdate
            info!("Broadcast mask '{}' to all clients", mask_id);
        }
        Ok(())
    }

    fn save_stats_to_disk(&self, mask_id: &str, stats: &MaskStats) {
        let _ = std::fs::create_dir_all(&self.storage_dir);
        let stats_path = self.storage_dir.join(format!("{}.stats", mask_id));
        match serde_json::to_string_pretty(stats) {
            Ok(json) => {
                if let Err(e) = std::fs::write(&stats_path, json) {
                    error!("Failed to save mask stats {}: {}", mask_id, e);
                }
            }
            Err(e) => error!("Failed to serialize mask stats {}: {}", mask_id, e),
        }
    }

    fn bootstrap_builtin_masks(&self) {
        let _ = std::fs::create_dir_all(&self.storage_dir);
        for profile in preset_masks::all() {
            let mask_id = profile.mask_id.clone();
            let json_path = self.storage_dir.join(format!("{}.json", mask_id));
            let stats_path = self.storage_dir.join(format!("{}.stats", mask_id));
            if json_path.exists() && stats_path.exists() {
                continue;
            }
            let entry = MaskEntry {
                profile,
                stats: MaskStats {
                    mask_id: mask_id.clone(),
                    times_used: 0,
                    times_failed: 0,
                    success_rate: 1.0,
                    confidence: 1.0,
                    is_active: true,
                    created_by: "builtin".into(),
                    created_at: current_unix_secs(),
                    last_used: None,
                },
            };
            self.save_to_disk(&mask_id, &entry);
        }
    }

    /// Save mask entry to disk
    fn save_to_disk(&self, mask_id: &str, entry: &MaskEntry) {
        let _ = std::fs::create_dir_all(&self.storage_dir);

        let json_path = self.storage_dir.join(format!("{}.json", mask_id));
        match serde_json::to_string_pretty(&entry.profile) {
            Ok(json) => {
                if let Err(e) = std::fs::write(&json_path, json) {
                    error!("Failed to save mask profile {}: {}", mask_id, e);
                }
            }
            Err(e) => error!("Failed to serialize mask profile {}: {}", mask_id, e),
        }

        self.save_stats_to_disk(mask_id, &entry.stats);
    }

    /// Load masks from disk on startup
    fn load_from_disk(&self) {
        let dir = &self.storage_dir;
        if !dir.exists() {
            return;
        }

        let entries = match std::fs::read_dir(dir) {
            Ok(e) => e,
            Err(_) => return,
        };

        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) == Some("json") {
                let mask_id = path
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .unwrap_or("")
                    .to_string();

                if mask_id.is_empty() {
                    continue;
                }

                // Load profile
                let profile: MaskProfile = match std::fs::read_to_string(&path)
                    .ok()
                    .and_then(|json| serde_json::from_str(&json).ok())
                {
                    Some(p) => p,
                    None => continue,
                };

                // Load stats
                let stats_path = dir.join(format!("{}.stats", mask_id));
                let stats: MaskStats = std::fs::read_to_string(&stats_path)
                    .ok()
                    .and_then(|json| serde_json::from_str(&json).ok())
                    .unwrap_or(MaskStats {
                        mask_id: mask_id.clone(),
                        times_used: 0,
                        times_failed: 0,
                        success_rate: 1.0,
                        confidence: 0.0,
                        is_active: true,
                        created_by: "loaded".into(),
                        created_at: 0,
                        last_used: None,
                    });

                info!("Loaded mask '{}' from disk (success: {:.1}%)", mask_id, stats.success_rate * 100.0);

                // Register only active masks in the live catalog
                if stats.is_active {
                    self.catalog.register_mask(profile.clone());
                }

                self.masks.insert(mask_id, MaskEntry { profile, stats });
            }
        }
    }
}

/// Get current Unix timestamp in seconds
fn current_unix_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}
