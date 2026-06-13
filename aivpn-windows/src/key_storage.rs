//! Connection key storage
//!
//! Stores keys encrypted at rest via Windows DPAPI (CryptProtectData /
//! CryptUnprotectData).  The encrypted blob is base64-encoded before being
//! written to %LOCALAPPDATA%/AIVPN/keys.json so the file remains valid JSON.
//! Encryption is tied to the current Windows user account — other users
//! (and offline attackers) cannot decrypt the stored connection keys.

use base64::Engine;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

// ── DPAPI helpers (Windows-only) ─────────────────────────────────────────────

#[cfg(windows)]
mod dpapi {
    use std::ptr;
    use winapi::um::dpapi::{CryptProtectData, CryptUnprotectData};
    use winapi::um::winbase::LocalFree;
    use winapi::um::wincrypt::DATA_BLOB;

    pub fn encrypt(plaintext: &[u8]) -> Option<Vec<u8>> {
        let mut input = DATA_BLOB {
            cbData: plaintext.len() as u32,
            pbData: plaintext.as_ptr() as *mut u8,
        };
        let mut output = DATA_BLOB {
            cbData: 0,
            pbData: ptr::null_mut(),
        };
        let ok = unsafe {
            CryptProtectData(
                &mut input,
                ptr::null(),
                ptr::null_mut(),
                ptr::null_mut(),
                ptr::null_mut(),
                0,
                &mut output,
            )
        };
        if ok == 0 || output.pbData.is_null() {
            return None;
        }
        let bytes =
            unsafe { std::slice::from_raw_parts(output.pbData, output.cbData as usize).to_vec() };
        unsafe { LocalFree(output.pbData as _) };
        Some(bytes)
    }

    pub fn decrypt(ciphertext: &[u8]) -> Option<Vec<u8>> {
        let mut input = DATA_BLOB {
            cbData: ciphertext.len() as u32,
            pbData: ciphertext.as_ptr() as *mut u8,
        };
        let mut output = DATA_BLOB {
            cbData: 0,
            pbData: ptr::null_mut(),
        };
        let ok = unsafe {
            CryptUnprotectData(
                &mut input,
                ptr::null_mut(),
                ptr::null_mut(),
                ptr::null_mut(),
                ptr::null_mut(),
                0,
                &mut output,
            )
        };
        if ok == 0 || output.pbData.is_null() {
            return None;
        }
        let bytes =
            unsafe { std::slice::from_raw_parts(output.pbData, output.cbData as usize).to_vec() };
        unsafe { LocalFree(output.pbData as _) };
        Some(bytes)
    }
}

/// Encrypt a connection-key string for storage.
/// On Windows uses DPAPI; on other platforms returns the plaintext unchanged
/// (compilation for non-Windows targets is only for `cargo check` purposes).
fn protect_key(key: &str) -> String {
    #[cfg(windows)]
    {
        if let Some(encrypted) = dpapi::encrypt(key.as_bytes()) {
            return base64::engine::general_purpose::STANDARD.encode(&encrypted);
        }
    }
    key.to_string()
}

/// Decrypt a stored connection-key string.
/// Handles both encrypted (base64 DPAPI blob) and legacy plaintext values so
/// existing key files are migrated transparently on first load.
fn unprotect_key(stored: &str) -> String {
    #[cfg(windows)]
    {
        if let Ok(blob) = base64::engine::general_purpose::STANDARD.decode(stored) {
            if let Some(plaintext) = dpapi::decrypt(&blob) {
                if let Ok(s) = String::from_utf8(plaintext) {
                    return s;
                }
            }
        }
    }
    // Not an encrypted blob (legacy plaintext) — return as-is.
    stored.to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConnectionKey {
    pub name: String,
    pub key: String,
    #[serde(default)]
    pub server_addr: String,
    #[serde(default)]
    pub vpn_ip: String,
    #[serde(default)]
    pub full_tunnel: bool,
    #[serde(default)]
    pub proxy_listen: Option<String>,
    /// Path to client mTLS cert file. Stored plaintext — only `key` is DPAPI-encrypted.
    #[serde(default)]
    pub mtls_cert_path: Option<String>,
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

        Some(Self {
            name: name.to_string(),
            key: key.trim().to_string(),
            server_addr,
            vpn_ip,
            full_tunnel: false,
            proxy_listen: None,
            mtls_cert_path: None,
        })
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct KeyStorage {
    pub keys: Vec<ConnectionKey>,
    #[serde(default)]
    pub selected: Option<usize>,
}

impl KeyStorage {
    pub fn load() -> Self {
        let path = Self::storage_path();
        if path.exists() {
            if let Ok(data) = std::fs::read_to_string(&path) {
                if let Ok(mut storage) = serde_json::from_str::<KeyStorage>(&data) {
                    // Decrypt keys from disk (transparent migration of legacy plaintext)
                    for k in &mut storage.keys {
                        k.key = unprotect_key(&k.key);
                    }
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
        }
    }

    pub fn save(&self) {
        let path = Self::storage_path();
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        // Encrypt keys before writing to disk
        let on_disk = KeyStorage {
            keys: self
                .keys
                .iter()
                .map(|k| {
                    let mut k2 = k.clone();
                    k2.key = protect_key(&k.key);
                    k2
                })
                .collect(),
            selected: self.selected,
        };
        if let Ok(json) = serde_json::to_string_pretty(&on_disk) {
            let _ = std::fs::write(&path, json);
        }
    }

    pub fn add_key(
        &mut self,
        name: &str,
        key: &str,
        full_tunnel: bool,
        proxy_listen: Option<String>,
        mtls_cert_path: Option<String>,
    ) -> Result<(), String> {
        // Validate key format
        let mut ck = ConnectionKey::from_key_string(name, key)
            .ok_or_else(|| "Invalid connection key format".to_string())?;
        ck.full_tunnel = full_tunnel;
        ck.proxy_listen = proxy_listen;
        ck.mtls_cert_path = mtls_cert_path;

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

    pub fn update_key(
        &mut self,
        idx: usize,
        name: &str,
        key: &str,
        full_tunnel: bool,
        proxy_listen: Option<String>,
        mtls_cert_path: Option<String>,
    ) -> Result<(), String> {
        if idx >= self.keys.len() {
            return Err("Invalid key index".to_string());
        }
        let mut ck = ConnectionKey::from_key_string(name, key)
            .ok_or_else(|| "Invalid connection key format".to_string())?;
        ck.full_tunnel = full_tunnel;
        ck.proxy_listen = proxy_listen;
        ck.mtls_cert_path = mtls_cert_path;
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
