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

pub mod client_db;
pub mod gateway;
pub mod nat;
pub mod server;
pub mod session;

#[cfg(all(feature = "management-api", unix))]
pub mod management_api;

// Phase 3-5 modules
pub mod key_rotation;
pub mod metrics;
pub mod neural;
pub mod passive_distribution;

// Auto Mask Recording modules
pub mod mask_gen;
pub mod mask_store;
pub mod recording;

pub use client_db::ClientDatabase;
pub use gateway::{Gateway, GatewayConfig};
pub use nat::NatForwarder;
pub use server::AivpnServer;
pub use server::ServerArgs;
pub use session::SessionManager;

// Phase 3-5 exports
pub use key_rotation::{KeyRotationConfig, KeyRotator};
pub use metrics::MetricsCollector;
pub use neural::{NeuralConfig, NeuralResonanceModule, ResonanceResult, ResonanceStatus};
pub use passive_distribution::{PassiveDistributionConfig, PassiveMaskReceiver};

// Auto Mask Recording exports
pub use mask_store::MaskStore;
pub use recording::RecordingManager;
