//! AIVPN Server
//!
//! Main server entry point

use std::sync::Arc;

use tracing_subscriber::{self, EnvFilter};

use clap::Parser;

use crate::gateway::{Gateway, GatewayConfig};
use aivpn_common::error::Result;

/// AIVPN Server - Censorship-resistant VPN gateway
#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
pub struct ServerArgs {
    /// Listen address (host:port). Overridden by listen_addr in server.json. Default: 0.0.0.0:443.
    #[arg(short, long, default_value = "0.0.0.0:443", env = "AIVPN_LISTEN")]
    pub listen: String,

    /// TUN device name (random if not specified — avoids fingerprinting)
    #[arg(long)]
    pub tun_name: Option<String>,

    /// Path to 32-byte server private key file
    #[arg(long)]
    pub key_file: Option<String>,

    /// Config file path
    #[arg(short, long)]
    pub config: Option<String>,

    /// Path to clients database file
    #[arg(long, default_value = "/etc/aivpn/clients.json")]
    pub clients_db: String,

    /// Add a new client with the given name and print config
    #[arg(long, value_name = "NAME")]
    pub add_client: Option<String>,

    /// Remove a client by ID
    #[arg(long, value_name = "ID")]
    pub remove_client: Option<String>,

    /// List all registered clients with stats
    #[arg(long)]
    pub list_clients: bool,

    /// Show client config by ID (for QR / import)
    #[arg(long, value_name = "ID")]
    pub show_client: Option<String>,

    /// Public IP of this server (embedded into connection keys).
    /// Required when using --add-client or --show-client to generate connection keys.
    #[arg(long, env = "AIVPN_SERVER_IP")]
    pub server_ip: Option<String>,

    /// Per-IP packet rate limit for incoming UDP traffic.
    #[arg(long, env = "AIVPN_PER_IP_PPS_LIMIT", default_value_t = 50000)]
    pub per_ip_pps_limit: u64,

    /// Directory for mask file storage.
    /// Resolved in order: CLI flag → env AIVPN_MASK_DIR → server.json "mask_dir" → default.
    #[arg(long, env = "AIVPN_MASK_DIR")]
    pub mask_dir: Option<String>,

    /// Unix socket path for the management HTTP API.
    /// If not specified, the management API is disabled.
    /// Example: /run/aivpn/api.sock
    #[cfg(all(feature = "management-api", unix))]
    #[arg(long, env = "AIVPN_MANAGEMENT_SOCKET")]
    pub management_socket: Option<String>,

    /// Validate a mask JSON file and print a quality report.
    /// Exits 0 on pass, 1 on structural errors.
    #[arg(long, value_name = "PATH")]
    pub validate_mask: Option<String>,

    // ── Pool / Enroll ──────────────────────────────────────────────────────────
    /// Enroll a peer server into the pool.
    /// Verifies that the peer shares the same server.key fingerprint, then
    /// pushes the full clients.json and adds the peer to the local pool config.
    #[arg(long, value_name = "PEER_ADDR")]
    pub enroll: Option<String>,

    /// Pool configuration JSON file path.
    /// Contains: {"peers": ["host:port", ...], "sync_port": 444, "sync_key": "hex"}
    #[arg(long, env = "AIVPN_POOL_CONFIG")]
    pub pool_config: Option<String>,

    // ── Backup / Restore ───────────────────────────────────────────────────────
    /// Export server state (clients DB, masks, config) to a tar.gz archive.
    #[arg(long, value_name = "OUTPUT_PATH")]
    pub export: Option<String>,

    /// Import server state from a tar.gz archive created by --export.
    #[arg(long, value_name = "ARCHIVE_PATH")]
    pub import: Option<String>,

    /// Dry-run mode for --import: print what would change without writing files.
    #[arg(long)]
    pub dry_run: bool,

    // ── Per-client QoS ─────────────────────────────────────────────────────────
    /// Set QoS for a client (by name or ID). Use with --bw-up, --bw-down, --dscp.
    #[arg(long, value_name = "NAME_OR_ID")]
    pub set_client_qos: Option<String>,

    /// Upstream (client→server) bandwidth limit. Example: 10M, 512K, 1G.
    #[arg(long, value_name = "BANDWIDTH")]
    pub bw_up: Option<String>,

    /// Downstream (server→client) bandwidth limit. Example: 50M, 1G.
    #[arg(long, value_name = "BANDWIDTH")]
    pub bw_down: Option<String>,

    /// DSCP traffic class name. Examples: EF, AF41, CS1, BE.
    #[arg(long, value_name = "CLASS")]
    pub dscp: Option<String>,

    // ── Audit Log ──────────────────────────────────────────────────────────────
    /// Path to the append-only admin audit log (JSONL format).
    #[arg(
        long,
        env = "AIVPN_AUDIT_LOG",
        default_value = "/var/log/aivpn/audit.log"
    )]
    pub audit_log: String,
}

/// AIVPN Server instance
pub struct AivpnServer {
    gateway: Gateway,
}

impl AivpnServer {
    /// Create new server instance
    pub fn new(config: GatewayConfig) -> Result<Self> {
        let gateway = Gateway::new(config)?;
        Ok(Self { gateway })
    }

    /// Return a shared reference to the session manager (for pool sync setup).
    pub fn session_manager(&self) -> Arc<crate::session::SessionManager> {
        self.gateway.session_manager()
    }

    /// Return the default MDH bytes from the mask catalog (for pool sync packets).
    pub fn catalog_mdh(&self) -> Vec<u8> {
        self.gateway.catalog_mdh()
    }

    /// Set multi-hop chain forwarder.  Must be called before `run()`.
    pub fn set_chain_forwarder(&mut self, cf: Arc<crate::chain_forwarder::ChainForwarder>) {
        self.gateway.set_chain_forwarder(cf);
    }

    /// Run the server
    pub async fn run(self) -> Result<()> {
        self.gateway.run().await
    }
}

/// Initialize logging
pub fn init_logging() {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::from_default_env()
                .add_directive("aivpn_server=debug".parse().unwrap())
                .add_directive("aivpn_common=debug".parse().unwrap()),
        )
        .init();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_server_creation() {
        // Create temp mask dir with a preset mask for the test
        let mask_dir = std::path::PathBuf::from("/tmp/aivpn-test-server-masks");
        let _ = std::fs::create_dir_all(&mask_dir);
        let mask = aivpn_common::mask::preset_masks::webrtc_zoom_v3();
        let json = serde_json::to_string_pretty(&mask).unwrap();
        std::fs::write(mask_dir.join(format!("{}.json", mask.mask_id)), &json).unwrap();
        std::fs::write(mask_dir.join(format!("{}.stats", mask.mask_id)), "{}").unwrap();

        let mut config = GatewayConfig::default();
        config.mask_dir = mask_dir;
        let server = AivpnServer::new(config);
        assert!(server.is_ok());
    }
}
