//! AIVPN Common Library
//!
//! Shared cryptographic primitives, protocol structures, and utilities
//! for AIVPN client and server implementations.

pub mod client_wire;
pub mod crypto;
pub mod error;
pub mod event_log;
pub mod mask;
pub mod network_config;
pub mod protocol;
pub mod recording;

#[cfg(feature = "client-upload")]
pub mod upload_pipeline;

#[cfg(target_os = "linux")]
pub mod kernel_accel;

pub use client_wire::*;
pub use crypto::*;
pub use error::*;
pub use mask::*;
pub use network_config::*;
pub use protocol::*;
pub use recording::*;
