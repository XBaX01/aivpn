//! AIVPN Server
//! 
//! Main server entry point

use tracing::{info, error};
use tracing_subscriber::{self, EnvFilter};

use clap::Parser;

use aivpn_common::error::Result;
use crate::gateway::{Gateway, GatewayConfig};

/// AIVPN Server - Censorship-resistant VPN gateway
#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
pub struct ServerArgs {
    /// Listen address
    #[arg(short, long, default_value = "0.0.0.0:443")]
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
                .add_directive("aivpn_server=info".parse().unwrap())
                .add_directive("aivpn_common=info".parse().unwrap())
        )
        .init();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_server_creation() {
        let config = GatewayConfig::default();
        let server = AivpnServer::new(config);
        assert!(server.is_ok());
    }
}
