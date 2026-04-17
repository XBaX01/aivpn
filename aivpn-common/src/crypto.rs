//! Cryptographic primitives for AIVPN
//! 
//! Implements:
//! - X25519 key exchange
//! - ChaCha20-Poly1305 AEAD encryption
//! - BLAKE3 hashing and HMAC
//! - Resonance Tag generation

use blake3::Hasher;
use chacha20poly1305::{
    aead::{Aead, KeyInit, OsRng},
    ChaCha20Poly1305, Nonce, Key as ChachaKey,
};
use hmac::Hmac;
use rand::RngCore;
use sha2::Sha256;
use subtle::ConstantTimeEq;
use x25519_dalek;
use zeroize::{Zeroize, ZeroizeOnDrop};

use crate::error::{Error, Result};

/// Size of resonance tag in bytes
pub const TAG_SIZE: usize = 8;

/// Size of X25519 public key in bytes
pub const X25519_PUBLIC_KEY_SIZE: usize = 32;

/// Size of X25519 private key in bytes
pub const X25519_PRIVATE_KEY_SIZE: usize = 32;

/// Size of ChaCha20-Poly1305 key in bytes
pub const CHACHA20_KEY_SIZE: usize = 32;

/// Size of Poly1305 tag in bytes
pub const POLY1305_TAG_SIZE: usize = 16;

/// Size of nonce in bytes
pub const NONCE_SIZE: usize = 12;

/// Default time window for tag rotation in milliseconds (optimized: increased from 5s to 10s)
pub const DEFAULT_WINDOW_MS: u64 = 10_000;

/// HKDF context strings
const HKDF_SESSION_KEY_CONTEXT: &str = "aivpn-session-key-v1";
const HKDF_TAG_SECRET_CONTEXT: &str = "aivpn-tag-secret-v1";
const HKDF_PRNG_SEED_CONTEXT: &str = "aivpn-prng-seed-v1";

/// Session keys derived from key exchange
#[derive(Clone, Zeroize, ZeroizeOnDrop)]
pub struct SessionKeys {
    pub session_key: [u8; CHACHA20_KEY_SIZE],
    pub tag_secret: [u8; 32],
    pub prng_seed: [u8; 32],
}

/// X25519 keypair for key exchange
#[derive(Debug, Clone)]
pub struct KeyPair {
    private_key_bytes: [u8; X25519_PRIVATE_KEY_SIZE],
    public_key_bytes: [u8; X25519_PUBLIC_KEY_SIZE],
}

impl Drop for KeyPair {
    fn drop(&mut self) {
        self.private_key_bytes.zeroize();
    }
}
impl KeyPair {
    /// Generate a new ephemeral keypair
    pub fn generate() -> Self {
        let mut private_key_bytes = [0u8; 32];
        OsRng.fill_bytes(&mut private_key_bytes);
        
        // X25519 clamping (RFC 7748)
        private_key_bytes[0] &= 248;
        private_key_bytes[31] &= 127;
        private_key_bytes[31] |= 64;
        
        let public_key_bytes = x25519_dalek::x25519(private_key_bytes, x25519_dalek::X25519_BASEPOINT_BYTES);
        
        Self { 
            private_key_bytes,
            public_key_bytes,
        }
    }

    /// Create keypair from existing private key bytes (loaded from file)
    pub fn from_private_key(mut key_bytes: [u8; 32]) -> Self {
        // X25519 clamping (RFC 7748)
        key_bytes[0] &= 248;
        key_bytes[31] &= 127;
        key_bytes[31] |= 64;
        let public_key_bytes = x25519_dalek::x25519(key_bytes, x25519_dalek::X25519_BASEPOINT_BYTES);
        Self {
            private_key_bytes: key_bytes,
            public_key_bytes,
        }
    }

    /// Get the public key as bytes
    pub fn public_key_bytes(&self) -> [u8; X25519_PUBLIC_KEY_SIZE] {
        self.public_key_bytes
    }

    /// Compute shared secret with remote public key
    /// Returns error if the result is all-zero (small subgroup attack)
    pub fn compute_shared(&self, remote_public: &[u8; X25519_PUBLIC_KEY_SIZE]) -> Result<[u8; 32]> {
        let shared = x25519_dalek::x25519(self.private_key_bytes, *remote_public);
        // Reject all-zero shared secret (small subgroup / identity point attack)
        if shared.ct_eq(&[0u8; 32]).into() {
            return Err(Error::Crypto("DH result is all-zero (possible small subgroup attack)".into()));
        }
        Ok(shared)
    }
}

/// Derive session keys from DH result using HKDF-BLAKE3
pub fn derive_session_keys(
    dh_result: &[u8; 32],
    preshared_key: Option<&[u8; 32]>,
    eph_pub: &[u8; X25519_PUBLIC_KEY_SIZE],
) -> SessionKeys {
    // IKM = dh_result || preshared_key (or just dh_result if no PSK)
    let ikm: Vec<u8> = if let Some(psk) = preshared_key {
        let mut buf = [0u8; 64];
        buf[..32].copy_from_slice(dh_result);
        buf[32..].copy_from_slice(psk);
        buf.to_vec()
    } else {
        dh_result.to_vec()
    };

    // Derive keys using BLAKE3 derive_key with different contexts
    // Context strings are combined with key material for domain separation
    let session_key_input: Vec<u8> = [ikm.clone(), eph_pub.to_vec()].concat();
    let tag_secret_input: Vec<u8> = [ikm.clone(), eph_pub.to_vec()].concat();
    let prng_seed_input: Vec<u8> = [ikm, eph_pub.to_vec()].concat();
    
    let session_key_hash = blake3::derive_key(HKDF_SESSION_KEY_CONTEXT, &session_key_input);
    let tag_secret_hash = blake3::derive_key(HKDF_TAG_SECRET_CONTEXT, &tag_secret_input);
    let prng_seed_hash = blake3::derive_key(HKDF_PRNG_SEED_CONTEXT, &prng_seed_input);

    SessionKeys {
        session_key: session_key_hash[..CHACHA20_KEY_SIZE].try_into().unwrap(),
        tag_secret: tag_secret_hash[..32].try_into().unwrap(),
        prng_seed: prng_seed_hash[..32].try_into().unwrap(),
    }
}

/// Encrypt payload using ChaCha20-Poly1305
pub fn encrypt_payload(
    key: &[u8; CHACHA20_KEY_SIZE],
    nonce: &[u8; NONCE_SIZE],
    plaintext: &[u8],
) -> Result<Vec<u8>> {
    let cipher = ChaCha20Poly1305::new(ChachaKey::from_slice(key));
    let nonce = Nonce::from_slice(nonce);
    
    let ciphertext = cipher.encrypt(nonce, plaintext)?;
    Ok(ciphertext)
}

/// Decrypt payload using ChaCha20-Poly1305
pub fn decrypt_payload(
    key: &[u8; CHACHA20_KEY_SIZE],
    nonce: &[u8; NONCE_SIZE],
    ciphertext: &[u8],
) -> Result<Vec<u8>> {
    let cipher = ChaCha20Poly1305::new(ChachaKey::from_slice(key));
    let nonce = Nonce::from_slice(nonce);
    
    let plaintext = cipher.decrypt(nonce, ciphertext)?;
    Ok(plaintext)
}

/// Generate Resonance Tag using HMAC-BLAKE3
/// 
/// Tag = HMAC-BLAKE3(tag_secret, counter_bytes || time_window_bytes)
/// truncated to first 8 bytes.
/// The first byte is guaranteed NOT to be 1–4 (WireGuard message types),
/// preventing heuristic WireGuard detection by Wireshark / DPI (Issue #30).
pub fn generate_resonance_tag(
    tag_secret: &[u8; 32],
    counter: u64,
    time_window: u64,
) -> [u8; TAG_SIZE] {
    let mut hasher = Hasher::new_keyed(tag_secret);
    hasher.update(&counter.to_le_bytes());
    hasher.update(&time_window.to_le_bytes());
    
    let hash = hasher.finalize();
    let mut tag = [0u8; TAG_SIZE];
    tag.copy_from_slice(&hash.as_bytes()[..TAG_SIZE]);
    // Avoid WireGuard message type signatures: 0x01 (Initiation), 0x02 (Response),
    // 0x03 (Cookie), 0x04 (Transport).  DPI/Wireshark checks byte[0] ∈ {1..4}
    // followed by three zero bytes.  Shifting byte[0] out of that range eliminates
    // the heuristic match without reducing tag entropy (the secret is still 256-bit).
    if tag[0] >= 1 && tag[0] <= 4 {
        tag[0] = tag[0].wrapping_add(5); // 1→6, 2→7, 3→8, 4→9
    }
    tag
}

/// Compute time window from timestamp
pub fn compute_time_window(timestamp_ms: u64, window_ms: u64) -> u64 {
    timestamp_ms / window_ms
}

/// Get current timestamp in milliseconds
pub fn current_timestamp_ms() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64
}

/// Generate random bytes
pub fn random_bytes(len: usize) -> Vec<u8> {
    let mut buf = vec![0u8; len];
    OsRng.fill_bytes(&mut buf);
    buf
}

/// Compute BLAKE3 hash
pub fn blake3_hash(data: &[u8]) -> [u8; 32] {
    blake3::hash(data).into()
}

/// Obfuscate/deobfuscate ephemeral public key using server's static public key.
/// XOR with BLAKE3-derived mask makes eph_pub indistinguishable from random. (HIGH-9)
pub fn obfuscate_eph_pub(eph_pub: &mut [u8; 32], server_static_pub: &[u8; 32]) {
    let mask = blake3::derive_key("aivpn-eph-obfuscation-v1", server_static_pub);
    for i in 0..32 {
        eph_pub[i] ^= mask[i];
    }
}

/// Compute HMAC-SHA256
pub fn hmac_sha256(key: &[u8], data: &[u8]) -> [u8; 32] {
    use hmac::Mac;
    type HmacSha256 = Hmac<Sha256>;
    let mut mac = <HmacSha256 as Mac>::new_from_slice(key)
        .expect("HMAC can take key of any size");
    mac.update(data);
    let result = mac.finalize();
    result.into_bytes().into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_key_exchange() {
        let client_keys = KeyPair::generate();
        let server_keys = KeyPair::generate();

        let client_shared = client_keys.compute_shared(&server_keys.public_key_bytes()).unwrap();
        let server_shared = server_keys.compute_shared(&client_keys.public_key_bytes()).unwrap();

        assert_eq!(client_shared, server_shared);
    }

    #[test]
    fn test_encrypt_decrypt() {
        let key = [1u8; CHACHA20_KEY_SIZE];
        let nonce = [2u8; NONCE_SIZE];
        let plaintext = b"Hello, AIVPN!";

        let ciphertext = encrypt_payload(&key, &nonce, plaintext).unwrap();
        let decrypted = decrypt_payload(&key, &nonce, &ciphertext).unwrap();

        assert_eq!(plaintext.to_vec(), decrypted);
    }

    #[test]
    fn test_resonance_tag() {
        let tag_secret = [3u8; 32];
        let tag1 = generate_resonance_tag(&tag_secret, 1, 100);
        let tag2 = generate_resonance_tag(&tag_secret, 2, 100);
        let tag3 = generate_resonance_tag(&tag_secret, 1, 100);

        assert_ne!(tag1, tag2); // Different counter
        assert_eq!(tag1, tag3); // Same counter and window
    }
}
