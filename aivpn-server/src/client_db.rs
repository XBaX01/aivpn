//! Client Database
//!
//! Manages registered VPN clients with pre-shared keys, static IPs,
//! and per-client statistics. Persisted to JSON file.

use std::net::Ipv4Addr;
use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use parking_lot::RwLock;
use rand::RngCore;
use serde::{Deserialize, Serialize};
use tracing::{info, warn};

use aivpn_common::error::{Error, Result};
use aivpn_common::network_config::VpnNetworkConfig;

/// Client configuration and credentials
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClientConfig {
    /// Unique client ID (UUID-like hex string)
    pub id: String,
    /// Human-readable name
    pub name: String,
    /// Pre-shared key (32 bytes, base64-encoded in JSON)
    #[serde(with = "base64_bytes")]
    pub psk: [u8; 32],
    /// Assigned static VPN IP
    pub vpn_ip: Ipv4Addr,
    /// Whether client is enabled
    pub enabled: bool,
    /// Creation timestamp
    pub created_at: DateTime<Utc>,
    /// Traffic and connection statistics
    pub stats: ClientStats,
}

/// Per-client traffic statistics
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ClientStats {
    pub bytes_in: u64,
    pub bytes_out: u64,
    pub last_connected: Option<DateTime<Utc>>,
    pub total_connections: u64,
    pub last_handshake: Option<DateTime<Utc>>,
}

/// Persistent client database
#[derive(Debug, Clone, Serialize, Deserialize)]
struct ClientDbFile {
    clients: Vec<ClientConfig>,
    /// Next host offset within the configured VPN subnet to assign.
    #[serde(default = "default_next_host_offset", alias = "next_octet")]
    next_host_offset: u32,
}

fn default_next_host_offset() -> u32 {
    2
}

impl Default for ClientDbFile {
    fn default() -> Self {
        Self {
            clients: Vec::new(),
            next_host_offset: default_next_host_offset(),
        }
    }
}

/// Thread-safe client database with file persistence
pub struct ClientDatabase {
    data: RwLock<ClientDbFile>,
    file_path: PathBuf,
    network_config: VpnNetworkConfig,
}

impl ClientDatabase {
    /// Load or create client database from file
    pub fn load(file_path: &Path, network_config: VpnNetworkConfig) -> Result<Self> {
        network_config.validate()?;
        let data = if file_path.exists() {
            let content = std::fs::read_to_string(file_path)
                .map_err(|e| Error::Session(format!("Failed to read client DB: {}", e)))?;
            serde_json::from_str(&content)
                .map_err(|e| Error::Session(format!("Failed to parse client DB: {}", e)))?
        } else {
            ClientDbFile::default()
        };

        Ok(Self {
            data: RwLock::new(data),
            file_path: file_path.to_path_buf(),
            network_config,
        })
    }

    /// Save database to file
    pub fn save(&self) -> Result<()> {
        let data = self.data.read();
        let content = serde_json::to_string_pretty(&*data)
            .map_err(|e| Error::Session(format!("Failed to serialize client DB: {}", e)))?;

        // Write atomically via temp file
        let tmp_path = self.file_path.with_extension("tmp");
        std::fs::write(&tmp_path, &content)
            .map_err(|e| Error::Session(format!("Failed to write client DB: {}", e)))?;
        std::fs::rename(&tmp_path, &self.file_path)
            .map_err(|e| Error::Session(format!("Failed to rename client DB: {}", e)))?;

        Ok(())
    }

    /// Add a new client, returns the generated config
    pub fn add_client(&self, name: &str) -> Result<ClientConfig> {
        let mut data = self.data.write();

        // Check name uniqueness
        if data.clients.iter().any(|c| c.name == name) {
            return Err(Error::Session(format!("Client '{}' already exists", name)));
        }

        // Allocate VPN IP
        let vpn_ip = self.allocate_vpn_ip(&mut data)?;

        // Generate random ID and PSK
        let mut id_bytes = [0u8; 8];
        let mut psk = [0u8; 32];
        chacha20poly1305::aead::OsRng.fill_bytes(&mut id_bytes);
        chacha20poly1305::aead::OsRng.fill_bytes(&mut psk);

        let id = id_bytes.iter().map(|b| format!("{:02x}", b)).collect::<String>();

        let client = ClientConfig {
            id,
            name: name.to_string(),
            psk,
            vpn_ip,
            enabled: true,
            created_at: Utc::now(),
            stats: ClientStats::default(),
        };

        data.clients.push(client.clone());
        drop(data);

        self.save()?;
        Ok(client)
    }

    pub fn network_config(&self) -> VpnNetworkConfig {
        self.network_config
    }

    /// Remove a client by ID
    pub fn remove_client(&self, client_id: &str) -> Result<()> {
        let mut data = self.data.write();
        let before = data.clients.len();
        data.clients.retain(|c| c.id != client_id);
        if data.clients.len() == before {
            return Err(Error::Session(format!("Client '{}' not found", client_id)));
        }
        drop(data);
        self.save()?;
        Ok(())
    }

    /// Get all clients
    pub fn list_clients(&self) -> Vec<ClientConfig> {
        self.data.read().clients.clone()
    }

    /// Find client by PSK (used during handshake to identify the connecting client)
    pub fn find_by_psk(&self, psk: &[u8; 32]) -> Option<ClientConfig> {
        let data = self.data.read();
        data.clients
            .iter()
            .find(|c| c.enabled && subtle::ConstantTimeEq::ct_eq(&c.psk[..], &psk[..]).into())
            .cloned()
    }

    /// Find client by VPN IP
    pub fn find_by_vpn_ip(&self, ip: &Ipv4Addr) -> Option<ClientConfig> {
        let data = self.data.read();
        data.clients.iter().find(|c| c.vpn_ip == *ip).cloned()
    }

    /// Find client by ID
    pub fn find_by_id(&self, id: &str) -> Option<ClientConfig> {
        let data = self.data.read();
        data.clients.iter().find(|c| c.id == id).cloned()
    }

    /// Update client stats (called from gateway on traffic)
    pub fn record_handshake(&self, client_id: &str) {
        let mut data = self.data.write();
        if let Some(client) = data.clients.iter_mut().find(|c| c.id == client_id) {
            client.stats.total_connections += 1;
            client.stats.last_handshake = Some(Utc::now());
            client.stats.last_connected = Some(Utc::now());
        }
    }

    /// Update traffic counters
    pub fn record_traffic(&self, client_id: &str, bytes_in: u64, bytes_out: u64) {
        let mut data = self.data.write();
        if let Some(client) = data.clients.iter_mut().find(|c| c.id == client_id) {
            client.stats.bytes_in += bytes_in;
            client.stats.bytes_out += bytes_out;
            client.stats.last_connected = Some(Utc::now());
        }
    }

    /// Persist stats periodically (called from a background task)
    pub fn flush_stats(&self) {
        if let Err(e) = self.save() {
            warn!("Failed to flush client stats: {}", e);
        }
    }

    /// Reload client database from disk if the file has changed.
    /// Preserves in-memory traffic stats for existing clients.
    /// Returns true if a reload was performed.
    pub fn reload_if_changed(&self) -> bool {
        let metadata = match std::fs::metadata(&self.file_path) {
            Ok(m) => m,
            Err(_) => return false,
        };

        let _ = metadata;

        match self.reload_from_disk() {
            Ok(true) => {
                info!("Client database reloaded from disk ({} clients)", self.list_clients().len());
                true
            }
            Ok(false) => false,
            Err(e) => {
                warn!("Failed to reload client DB: {}", e);
                false
            }
        }
    }

    /// Internal: reload from disk, merging with in-memory stats.
    /// Returns Ok(true) if data changed, Ok(false) if unchanged.
    fn reload_from_disk(&self) -> Result<bool> {
        let content = std::fs::read_to_string(&self.file_path)
            .map_err(|e| Error::Session(format!("Failed to read client DB for reload: {}", e)))?;
        let new_data: ClientDbFile = serde_json::from_str(&content)
            .map_err(|e| Error::Session(format!("Failed to parse client DB for reload: {}", e)))?;

        let mut data = self.data.write();

        // Check if anything actually changed (compare client count and IDs)
        let old_ids: std::collections::HashSet<String> = data.clients.iter().map(|c| c.id.clone()).collect();
        let new_ids: std::collections::HashSet<String> = new_data.clients.iter().map(|c| c.id.clone()).collect();
        let changed = old_ids != new_ids;

        if !changed {
            return Ok(false);
        }

        // Build a map of existing stats by client ID
        let mut stats_map: std::collections::HashMap<String, ClientStats> = std::collections::HashMap::new();
        for client in &data.clients {
            stats_map.insert(client.id.clone(), client.stats.clone());
        }

        // Replace clients list, preserving stats for existing clients
        data.clients = new_data.clients.into_iter().map(|mut c| {
            if let Some(saved_stats) = stats_map.get(&c.id) {
                c.stats = saved_stats.clone();
            }
            c
        }).collect();
        data.next_host_offset = new_data.next_host_offset;

        Ok(true)
    }

    fn allocate_vpn_ip(&self, data: &mut ClientDbFile) -> Result<Ipv4Addr> {
        let max_host_offset = self.network_config.max_host_offset();
        if max_host_offset < 1 {
            return Err(Error::Session("Configured VPN subnet has no usable host addresses".into()));
        }

        let mut candidate_offset = if data.next_host_offset == 0 {
            default_next_host_offset()
        } else {
            data.next_host_offset
        };

        for _ in 0..max_host_offset {
            if let Some(candidate_ip) = self.network_config.ip_for_host_offset(candidate_offset) {
                let already_used = data.clients.iter().any(|client| client.vpn_ip == candidate_ip);
                if candidate_ip != self.network_config.server_vpn_ip && !already_used {
                    data.next_host_offset = if candidate_offset >= max_host_offset {
                        1
                    } else {
                        candidate_offset + 1
                    };
                    return Ok(candidate_ip);
                }
            }

            candidate_offset = if candidate_offset >= max_host_offset {
                1
            } else {
                candidate_offset + 1
            };
        }

        Err(Error::Session("No more VPN IPs available in configured subnet".into()))
    }
}

/// Custom serde module for [u8; 32] as base64
mod base64_bytes {
    use serde::{Deserialize, Deserializer, Serialize, Serializer};

    pub fn serialize<S: Serializer>(bytes: &[u8; 32], serializer: S) -> std::result::Result<S::Ok, S::Error> {
        use base64::Engine;
        let b64 = base64::engine::general_purpose::STANDARD.encode(bytes);
        b64.serialize(serializer)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> std::result::Result<[u8; 32], D::Error> {
        use base64::Engine;
        let s = String::deserialize(deserializer)?;
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(&s)
            .map_err(serde::de::Error::custom)?;
        if bytes.len() != 32 {
            return Err(serde::de::Error::custom(format!(
                "PSK must be 32 bytes, got {}",
                bytes.len()
            )));
        }
        let mut arr = [0u8; 32];
        arr.copy_from_slice(&bytes);
        Ok(arr)
    }
}
