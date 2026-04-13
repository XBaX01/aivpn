//! Connection key storage
//!
//! Stores keys in JSON file at %APPDATA%/AIVPN/keys.json

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

use aivpn_common::network_config::ClientNetworkConfig;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConnectionKey {
    pub name: String,
    pub key: String,
    #[serde(default)]
    pub server_addr: String,
    #[serde(default)]
    pub vpn_ip: String,
    #[serde(default)]
    pub network_config: Option<ClientNetworkConfig>,
}

impl ConnectionKey {
    pub fn from_key_string(name: &str, key: &str) -> Option<Self> {
        let payload = key.trim().strip_prefix("aivpn://").unwrap_or(key.trim());
        let json_bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .decode(payload)
            .ok()?;
        let json: serde_json::Value = serde_json::from_slice(&json_bytes).ok()?;
        let server_addr = json["s"].as_str().unwrap_or("").to_string();
        let vpn_ip = json["i"].as_str().unwrap_or("").to_string();
        let network_config = json
            .get("n")
            .cloned()
            .and_then(|value| serde_json::from_value::<ClientNetworkConfig>(value).ok());

        Some(Self {
            name: name.to_string(),
            key: key.trim().to_string(),
            server_addr,
            vpn_ip,
            network_config,
        })
    }
}

use base64::Engine;

#[derive(Debug, Serialize, Deserialize)]
pub struct KeyStorage {
    pub keys: Vec<ConnectionKey>,
    #[serde(default)]
    pub selected: Option<usize>,
    #[serde(default)]
    pub full_tunnel: bool,
}

impl KeyStorage {
    pub fn load() -> Self {
        let path = Self::storage_path();
        if path.exists() {
            if let Ok(data) = std::fs::read_to_string(&path) {
                if let Ok(mut storage) = serde_json::from_str::<KeyStorage>(&data) {
                    // Validate selected index
                    if let Some(idx) = storage.selected {
                        if idx >= storage.keys.len() {
                            storage.selected = None;
                        }
                    }
                    return storage;
                }
            }
        }
        Self {
            keys: Vec::new(),
            selected: None,
            full_tunnel: false,
        }
    }

    pub fn save(&self) {
        let path = Self::storage_path();
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        if let Ok(json) = serde_json::to_string_pretty(self) {
            let _ = std::fs::write(&path, json);
        }
    }

    pub fn add_key(&mut self, name: &str, key: &str) -> Result<(), String> {
        // Validate key format
        let ck = ConnectionKey::from_key_string(name, key)
            .ok_or_else(|| "Invalid connection key format".to_string())?;

        // Check for duplicates
        if self.keys.iter().any(|k| k.key == ck.key) {
            return Err("This key already exists".to_string());
        }

        self.keys.push(ck);
        if self.selected.is_none() {
            self.selected = Some(self.keys.len() - 1);
        }
        self.save();
        Ok(())
    }

    pub fn update_key(&mut self, idx: usize, name: &str, key: &str) -> Result<(), String> {
        if idx >= self.keys.len() {
            return Err("Invalid key index".to_string());
        }
        let ck = ConnectionKey::from_key_string(name, key)
            .ok_or_else(|| "Invalid connection key format".to_string())?;
        self.keys[idx] = ck;
        self.save();
        Ok(())
    }

    pub fn remove_key(&mut self, idx: usize) {
        if idx < self.keys.len() {
            self.keys.remove(idx);
            match self.selected {
                Some(sel) if sel == idx => {
                    self.selected = if self.keys.is_empty() {
                        None
                    } else {
                        Some(sel.min(self.keys.len() - 1))
                    };
                }
                Some(sel) if sel > idx => {
                    self.selected = Some(sel - 1);
                }
                _ => {}
            }
            self.save();
        }
    }

    pub fn selected_key(&self) -> Option<&ConnectionKey> {
        self.selected.and_then(|idx| self.keys.get(idx))
    }

    fn storage_path() -> PathBuf {
        dirs::data_local_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("AIVPN")
            .join("keys.json")
    }
}
