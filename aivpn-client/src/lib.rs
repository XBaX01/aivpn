//! AIVPN Client Implementation
//!
//! Client with:
//! - TUN device for packet capture
//! - Mimicry Engine for traffic shaping
//! - Key exchange and session management
//! - Auto Mask Recording CLI support

pub mod adaptive;
pub mod bench;
pub mod bootstrap_cache;
pub mod bootstrap_loader;
pub mod client;
pub mod dns_proxy;
pub mod kill_switch;
pub mod proxy;
pub mod record_cmd;
pub mod server_pool;
pub mod tunnel;

pub use aivpn_common::mimicry::MimicryEngine;
pub use client::AivpnClient;
pub use tunnel::Tunnel;
