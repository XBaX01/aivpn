//! Automatic Key Rotation (Phase 3)
//! 
//! Implements automatic session key rotation for enhanced security
//! 
//! Features:
//! - Time-based rotation (every 2 minutes)
//! - Data-based rotation (every 1 MB)
//! - In-band CONTROL message signaling
//! - Zero-downtime key transition

use std::time::{Duration, Instant};
use tracing::{info, debug};

use aivpn_common::crypto::{KeyPair, X25519_PUBLIC_KEY_SIZE};
use aivpn_common::error::Result;
use aivpn_common::protocol::ControlPayload;

/// Key rotation configuration
#[derive(Debug, Clone)]
pub struct KeyRotationConfig {
    /// Rotate keys every N seconds
    pub time_interval_secs: u64,
    
    /// Rotate keys every N bytes
    pub data_interval_bytes: u64,
    
    /// Enable automatic rotation
    pub enable_auto_rotation: bool,
}

impl Default for KeyRotationConfig {
    fn default() -> Self {
        Self {
            time_interval_secs: 120, // 2 minutes
            data_interval_bytes: 1_000_000, // 1 MB
            enable_auto_rotation: true,
        }
    }
}

/// Key rotation state
#[derive(Debug)]
pub struct KeyRotator {
    config: KeyRotationConfig,
    
    /// Current keypair
    current_keypair: KeyPair,
    
    /// Next keypair (pre-generated for smooth transition)
    next_keypair: Option<KeyPair>,
    
    /// Last rotation time
    last_rotation: Instant,
    
    /// Bytes transferred since last rotation
    bytes_since_rotation: u64,
    
    /// Rotation counter
    rotation_count: u64,
}

impl KeyRotator {
    /// Create new key rotator
    pub fn new(config: KeyRotationConfig) -> Result<Self> {
        let current_keypair = KeyPair::generate();
        
        Ok(Self {
            config,
            current_keypair,
            next_keypair: None,
            last_rotation: Instant::now(),
            bytes_since_rotation: 0,
            rotation_count: 0,
        })
    }
    
    /// Check if rotation is needed
    pub fn needs_rotation(&self) -> bool {
        if !self.config.enable_auto_rotation {
            return false;
        }
        
        // Time-based rotation
        let time_expired = self.last_rotation.elapsed() 
            >= Duration::from_secs(self.config.time_interval_secs);
        
        // Data-based rotation
        let data_expired = self.bytes_since_rotation 
            >= self.config.data_interval_bytes;
        
        time_expired || data_expired
    }
    
    /// Perform key rotation
    pub fn rotate_keys(&mut self) -> Result<KeyRotationEvent> {
        info!("Rotating session keys (rotation #{}).", self.rotation_count + 1);
        
        // Generate new keypair
        let new_keypair = KeyPair::generate();
        
        // Prepare rotation event
        let event = KeyRotationEvent {
            old_eph_pub: self.current_keypair.public_key_bytes(),
            new_eph_pub: new_keypair.public_key_bytes(),
            rotation_count: self.rotation_count,
        };
        
        // Update state
        self.next_keypair = Some(new_keypair);
        self.last_rotation = Instant::now();
        self.bytes_since_rotation = 0;
        self.rotation_count += 1;
        
        Ok(event)
    }
    
    /// Commit rotation (after ACK received)
    pub fn commit_rotation(&mut self) {
        if let Some(next) = self.next_keypair.take() {
            self.current_keypair = next;
            debug!("Key rotation committed successfully");
        }
    }
    
    /// Record bytes transferred
    pub fn record_bytes(&mut self, bytes: u64) {
        self.bytes_since_rotation += bytes;
    }
    
    /// Get current public key
    pub fn current_public_key(&self) -> [u8; X25519_PUBLIC_KEY_SIZE] {
        self.current_keypair.public_key_bytes()
    }
    
    /// Get next public key (if pre-generated)
    pub fn next_public_key(&self) -> Option<[u8; X25519_PUBLIC_KEY_SIZE]> {
        self.next_keypair.as_ref().map(|k| k.public_key_bytes())
    }
    
    /// Generate CONTROL message for key rotation
    pub fn create_rotation_message(&self) -> ControlPayload {
        ControlPayload::KeyRotate {
            new_eph_pub: self.next_public_key()
                .unwrap_or_else(|| self.current_public_key()),
        }
    }
    
    /// Get rotation statistics
    pub fn stats(&self) -> KeyRotationStats {
        KeyRotationStats {
            rotation_count: self.rotation_count,
            bytes_since_rotation: self.bytes_since_rotation,
            time_since_rotation: self.last_rotation.elapsed(),
            next_rotation_in: Duration::from_secs(self.config.time_interval_secs)
                .saturating_sub(self.last_rotation.elapsed()),
        }
    }
}

/// Key rotation event
#[derive(Debug, Clone)]
pub struct KeyRotationEvent {
    pub old_eph_pub: [u8; X25519_PUBLIC_KEY_SIZE],
    pub new_eph_pub: [u8; X25519_PUBLIC_KEY_SIZE],
    pub rotation_count: u64,
}

/// Key rotation statistics
#[derive(Debug, Clone)]
pub struct KeyRotationStats {
    pub rotation_count: u64,
    pub bytes_since_rotation: u64,
    pub time_since_rotation: Duration,
    pub next_rotation_in: Duration,
}
