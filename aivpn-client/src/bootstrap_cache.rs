use std::fs;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use aivpn_common::error::{Error, Result};
use aivpn_common::mask::{
    current_unix_secs, derive_bootstrap_candidates, BootstrapDescriptor, MaskProfile,
};

const CACHE_FILE_NAME: &str = "bootstrap_descriptors.json";
const MAX_CACHED_DESCRIPTORS: usize = 8;

#[derive(Debug, Default, Serialize, Deserialize)]
struct BootstrapCacheFile {
    descriptors: Vec<BootstrapDescriptor>,
}

fn cache_dir() -> PathBuf {
    if let Ok(home) = std::env::var("HOME") {
        return PathBuf::from(home).join(".aivpn");
    }
    std::env::temp_dir().join("aivpn")
}

fn cache_path() -> PathBuf {
    cache_dir().join(CACHE_FILE_NAME)
}

fn load_cache_file() -> BootstrapCacheFile {
    let path = cache_path();
    fs::read_to_string(path)
        .ok()
        .and_then(|json| serde_json::from_str(&json).ok())
        .unwrap_or_default()
}

pub fn load_descriptors() -> Vec<BootstrapDescriptor> {
    let now = current_unix_secs();
    let mut descriptors = load_cache_file().descriptors;
    descriptors.retain(|descriptor| descriptor.expires_at.saturating_add(24 * 3600) >= now);
    descriptors.sort_by(|left, right| right.created_at.cmp(&left.created_at));
    descriptors
}

pub fn select_initial_mask(preshared_key: Option<&[u8; 32]>) -> Option<MaskProfile> {
    let now = current_unix_secs();
    for descriptor in load_descriptors() {
        if !descriptor.is_valid_at(now) {
            continue;
        }
        if let Some(mask) = derive_bootstrap_candidates(&descriptor, preshared_key).into_iter().next() {
            return Some(mask);
        }
    }
    None
}

pub fn store_descriptor(descriptor: BootstrapDescriptor) -> Result<()> {
    let mut cache = load_cache_file();
    cache.descriptors.retain(|existing| existing.descriptor_id != descriptor.descriptor_id);
    cache.descriptors.push(descriptor);
    cache.descriptors.sort_by(|left, right| right.created_at.cmp(&left.created_at));
    cache.descriptors.truncate(MAX_CACHED_DESCRIPTORS);

    let dir = cache_dir();
    fs::create_dir_all(&dir).map_err(Error::Io)?;
    let json = serde_json::to_string_pretty(&cache)
        .map_err(|e| Error::Session(format!("Failed to serialize bootstrap cache: {}", e)))?;
    fs::write(cache_path(), json).map_err(Error::Io)
}

/// Store a bootstrap descriptor after verifying its ed25519 signature.
///
/// `trusted_key` should be the operator's ed25519 signing public key. When `Some`, the
/// signature is verified and unsigned/invalid descriptors are rejected. When `None` the
/// descriptor is stored without signature verification — callers must only pass `None` in
/// development/test contexts where a signing key is not yet available.
///
/// TODO(production-secure): all call sites should supply the operator signing key once
/// a dedicated ed25519 signing keypair is added to the connection-key format.
pub fn store_verified_descriptor(
    descriptor: BootstrapDescriptor,
    trusted_key: Option<&[u8; 32]>,
) -> Result<()> {
    let sig_is_zero = descriptor.signature == [0u8; 64];

    match trusted_key {
        Some(key) => {
            if sig_is_zero {
                return Err(aivpn_common::error::Error::Session(format!(
                    "Bootstrap descriptor {} has no signature (all-zero) — rejecting under trusted key configuration",
                    descriptor.descriptor_id
                )));
            }
            match descriptor.verify_signature(key)? {
                true => {}
                false => return Err(aivpn_common::error::Error::Session(format!(
                    "Bootstrap descriptor {} has invalid ed25519 signature — rejecting",
                    descriptor.descriptor_id
                ))),
            }
        }
        None => {
            if !sig_is_zero {
                tracing::debug!(
                    descriptor_id = %descriptor.descriptor_id,
                    "Bootstrap descriptor has signature but no trusted key provided — storing without verification"
                );
            }
        }
    }

    store_descriptor(descriptor)
}

pub async fn refresh_from_urls(urls: &[String]) -> usize {
    let mut stored = 0usize;
    for url in urls {
        let Ok(response) = reqwest::get(url).await else {
            continue;
        };
        let Ok(body) = response.text().await else {
            continue;
        };

        let descriptors = serde_json::from_str::<Vec<BootstrapDescriptor>>(&body)
            .ok()
            .or_else(|| serde_json::from_str::<BootstrapDescriptor>(&body).ok().map(|descriptor| vec![descriptor]));

        let Some(descriptors) = descriptors else {
            continue;
        };

        for descriptor in descriptors {
            if store_verified_descriptor(descriptor, None).is_ok() {
                stored += 1;
            }
        }
    }
    stored
}

#[cfg(test)]
mod tests {
    use super::*;
    use aivpn_common::mask::BootstrapDescriptor;

    #[test]
    fn test_store_verified_descriptor_validation() {
        // Temporarily override HOME env var to prevent overwriting active cache
        let old_home = std::env::var("HOME").ok();
        let temp_dir = std::env::temp_dir().join("aivpn_test_cache");
        let _ = std::fs::create_dir_all(&temp_dir);
        std::env::set_var("HOME", &temp_dir);

        let mut desc = BootstrapDescriptor {
            descriptor_id: "test_desc".to_string(),
            version: 1,
            created_at: 0,
            expires_at: 9999999999,
            base_mask_ids: vec![],
            embedded_masks: vec![],
            candidate_count: 1,
            kdf_salt: [0u8; 32],
            signature: [0u8; 64],
        };

        // 1. None trusted key -> should succeed even with zero signature
        let res = store_verified_descriptor(desc.clone(), None);
        assert!(res.is_ok());

        // 2. Some trusted key, zero signature -> should fail under our fix!
        let dummy_key = [0u8; 32];
        let res = store_verified_descriptor(desc.clone(), Some(&dummy_key));
        assert!(res.is_err());
        if let Err(e) = res {
            assert!(e.to_string().contains("no signature") || e.to_string().contains("all-zero"));
        }

        // 3. Some trusted key, non-zero invalid signature -> should fail
        desc.signature = [1u8; 64];
        let res = store_verified_descriptor(desc.clone(), Some(&dummy_key));
        assert!(res.is_err());

        // Clean up environment and temp files
        if let Some(h) = old_home {
            std::env::set_var("HOME", h);
        } else {
            std::env::remove_var("HOME");
        }
        let _ = std::fs::remove_dir_all(temp_dir);
    }
}