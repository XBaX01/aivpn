//! AIVPN Client Implementation
//! 
//! Client with:
//! - TUN device for packet capture
//! - Mimicry Engine for traffic shaping
//! - Key exchange and session management
//! - Auto Mask Recording CLI support

pub mod client;
pub mod mimicry;
pub mod tunnel;
pub mod record_cmd;

pub use client::AivpnClient;
pub use mimicry::MimicryEngine;
pub use tunnel::Tunnel;
