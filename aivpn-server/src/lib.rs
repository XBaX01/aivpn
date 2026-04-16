//! AIVPN Server Implementation - Production v0.3
//! 
//! Gateway server with:
//! - UDP listener with O(1) tag validation
//! - Session management
//! - TUN device and NAT forwarding
//! - Mimicry decoding
//! - Neural Resonance Module (Patent 1 — Signal Reconstruction Resonance)
//! - Automatic Mask Rotation (Patent 3 — Self-Expanding Cognitive System)
//! - Mask Catalog with Neural Unpack signatures (Patent 9 — Skill Discovery)
//! - Automatic Key Rotation
//! - Passive Mask Distribution
//! - Prometheus Metrics

pub mod gateway;
pub mod session;
pub mod server;
pub mod nat;
pub mod client_db;

// Phase 3-5 modules
pub mod neural;
pub mod key_rotation;
pub mod passive_distribution;
pub mod metrics;

// Auto Mask Recording modules
pub mod recording;
pub mod mask_gen;
pub mod mask_store;

pub use server::AivpnServer;
pub use server::ServerArgs;
pub use gateway::{Gateway, GatewayConfig};
pub use session::SessionManager;
pub use nat::NatForwarder;
pub use client_db::ClientDatabase;

// Phase 3-5 exports
pub use neural::{NeuralResonanceModule, NeuralConfig, ResonanceStatus, ResonanceResult};
pub use key_rotation::{KeyRotator, KeyRotationConfig};
pub use passive_distribution::{PassiveMaskReceiver, PassiveDistributionConfig};
pub use metrics::MetricsCollector;

// Auto Mask Recording exports
pub use recording::RecordingManager;
pub use mask_store::MaskStore;
