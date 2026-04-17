//! AIVPN Client Binary - Full Implementation

use aivpn_client::AivpnClient;
use aivpn_client::client::ClientConfig;
use aivpn_client::tunnel::TunnelConfig;
use aivpn_common::mask::preset_masks::webrtc_zoom_v3;
use aivpn_common::network_config::{ClientNetworkConfig, DEFAULT_VPN_MTU, LEGACY_SERVER_VPN_IP};
use clap::Parser;
use tracing::{info, error, warn};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;
use base64::Engine;
use std::net::Ipv4Addr;

/// AIVPN Client - Censorship-resistant VPN client
#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
pub struct ClientArgs {
    /// Server address (e.g., 1.2.3.4:443)
    #[arg(short, long)]
    pub server: Option<String>,

    /// Server public key (base64, 32 bytes)
    #[arg(long)]
    pub server_key: Option<String>,

    /// Connection key (aivpn://...) — contains server, key, PSK, VPN IP
    #[arg(short = 'k', long)]
    pub connection_key: Option<String>,

    /// TUN device name (random if not specified)
    #[arg(long)]
    pub tun_name: Option<String>,

    /// TUN device address
    #[arg(long, default_value = "10.0.0.2")]
    pub tun_addr: String,

    /// Route all traffic through VPN tunnel
    #[arg(long, default_value_t = false)]
    pub full_tunnel: bool,

    /// Config file path (JSON)
    #[arg(long)]
    pub config: Option<String>,

    #[command(subcommand)]
    pub command: Option<ClientCommand>,
}

#[derive(clap::Subcommand, Debug)]
pub enum ClientCommand {
    /// Recording CLI commands
    Record {
        #[command(subcommand)]
        action: RecordAction,
    },
}

#[derive(clap::Subcommand, Debug)]
pub enum RecordAction {
    /// Start a new traffic recording session
    Start {
        #[arg(short, long)]
        service: String,
    },
    /// Stop the current recording and generate a mask
    Stop,
    /// Show the last known recording capability/state from the running client daemon
    Status,
}

// Global shutdown flag
static SHUTDOWN: AtomicBool = AtomicBool::new(false);

#[tokio::main]
async fn main() {
    // Initialize logging — default to INFO level when RUST_LOG is not set
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info"))
        )
        .init();

    // Setup Ctrl+C handler in a separate task
    let shutdown = Arc::new(AtomicBool::new(false));
    let shutdown_clone = shutdown.clone();
    tokio::spawn(async move {
        tokio::signal::ctrl_c().await.expect("Failed to setup signal handler");
        info!("Received Ctrl+C, shutting down...");
        shutdown_clone.store(true, Ordering::SeqCst);
        SHUTDOWN.store(true, Ordering::SeqCst);
    });
    
    // Parse arguments
    let args = ClientArgs::parse();
    
    // Handle subcommands
    if let Some(command) = args.command {
        match command {
            ClientCommand::Record { action } => {
                match action {
                    RecordAction::Start { service } => {
                        aivpn_client::record_cmd::handle_recording_status(true, Some(&service));
                        let socket = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();
                        socket.send_to(format!("record_start:{}", service).as_bytes(), "127.0.0.1:44301").unwrap();
                        println!("Recording start requested for '{}'.", service);
                        println!("Run 'aivpn-client record status' to inspect progress.");
                    }
                    RecordAction::Stop => {
                        let prior = aivpn_client::record_cmd::read_local_status();
                        aivpn_client::record_cmd::mark_recording_stop_requested(prior.as_ref().and_then(|status| status.service.as_deref()));
                        let socket = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();
                        socket.send_to(b"record_stop", "127.0.0.1:44301").unwrap();
                        println!("Recording stop requested.");
                        println!("Run 'aivpn-client record status' to inspect progress.");
                    }
                    RecordAction::Status => {
                        let before = aivpn_client::record_cmd::read_local_status().map(|status| status.updated_at_ms).unwrap_or(0);
                        if let Ok(socket) = std::net::UdpSocket::bind("127.0.0.1:0") {
                            let _ = socket.send_to(b"record_status", "127.0.0.1:44301");
                        }
                        let start = std::time::Instant::now();
                        let mut latest = None;
                        while start.elapsed() < std::time::Duration::from_secs(2) {
                            if let Some(status) = aivpn_client::record_cmd::read_local_status() {
                                if status.updated_at_ms >= before {
                                    latest = Some(status);
                                    break;
                                }
                            }
                            std::thread::sleep(std::time::Duration::from_millis(100));
                        }
                        if let Some(status) = latest.or_else(aivpn_client::record_cmd::read_local_status) {
                            aivpn_client::record_cmd::print_local_status(&status);
                        } else {
                            println!("No recording status is available yet.");
                        }
                    }
                }
                return;
            }
        }
    }

    // Parse connection key or individual args
    let (server_addr, server_key_b64, psk_bytes, network_config) = if let Some(ref conn_key) = args.connection_key {
        let payload = conn_key.trim().strip_prefix("aivpn://").unwrap_or(conn_key.trim());
        let json_bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .decode(payload)
            .unwrap_or_else(|e| {
                error!("Invalid connection key: {}", e);
                std::process::exit(1);
            });
        let json: serde_json::Value = serde_json::from_slice(&json_bytes)
            .unwrap_or_else(|e| {
                error!("Malformed connection key JSON: {}", e);
                std::process::exit(1);
            });
        let s = json["s"].as_str().unwrap_or_else(|| {
            error!("Connection key missing server address (\"s\")");
            std::process::exit(1);
        }).to_string();
        let k = json["k"].as_str().unwrap_or_else(|| {
            error!("Connection key missing server key (\"k\")");
            std::process::exit(1);
        }).to_string();
        let psk: Option<Vec<u8>> = json["p"].as_str().and_then(|p| {
            base64::engine::general_purpose::STANDARD.decode(p).ok()
        });
        let network_config = json
            .get("n")
            .cloned()
            .and_then(|value| serde_json::from_value::<ClientNetworkConfig>(value).ok())
            .or_else(|| {
                json["i"].as_str().and_then(|ip| {
                    ip.parse::<Ipv4Addr>().ok().map(|client_ip| ClientNetworkConfig {
                        client_ip,
                        server_vpn_ip: LEGACY_SERVER_VPN_IP,
                        prefix_len: 24,
                        mtu: DEFAULT_VPN_MTU,
                        mdh_len: 20,
                    })
                })
            })
            .unwrap_or_else(|| fallback_network_config(&args.tun_addr));
        (s, k, psk, network_config)
    } else {
        let server = args.server.clone().unwrap_or_else(|| {
            error!("Either --connection-key or --server + --server-key required");
            std::process::exit(1);
        });
        let key = args.server_key.clone().unwrap_or_else(|| {
            error!("Either --connection-key or --server + --server-key required");
            std::process::exit(1);
        });
        (server, key, None, fallback_network_config(&args.tun_addr))
    };
    
    info!("AIVPN Client v{}", env!("CARGO_PKG_VERSION"));
    info!("Connecting to server: {}", server_addr);
    
    // Parse server key
    let server_key_decoded = base64::engine::general_purpose::STANDARD
        .decode(&server_key_b64)
        .unwrap_or_else(|e| {
            error!("Invalid server key: {}", e);
            std::process::exit(1);
        });
    
    let mut server_public_key = [0u8; 32];
    if server_key_decoded.len() != 32 {
        error!("Server key must be 32 bytes, got {}", server_key_decoded.len());
        std::process::exit(1);
    }
    server_public_key.copy_from_slice(&server_key_decoded);

    // Parse PSK
    let preshared_key: Option<[u8; 32]> = psk_bytes.and_then(|v| {
        if v.len() == 32 {
            let mut arr = [0u8; 32];
            arr.copy_from_slice(&v);
            Some(arr)
        } else {
            None
        }
    });
    
    let tun_name_fixed = args.tun_name.clone();
    let full_tunnel = args.full_tunnel;
    let network_config = network_config;

    let mut backoff = Duration::from_secs(1);
    let max_backoff = Duration::from_secs(60);

    loop {
        if shutdown.load(Ordering::SeqCst) {
            info!("Shutdown requested, stopping client loop");
            break;
        }

        // Use stable TUN name when user asked for it; otherwise generate fresh.
        // Fresh name avoids rare conflicts when previous TUN wasn't fully torn down yet.
        let tun_name = tun_name_fixed.clone().unwrap_or_else(|| {
            use rand::Rng;
            format!("tun{:04x}", rand::thread_rng().gen::<u16>())
        });

        let config = ClientConfig {
            server_addr: server_addr.clone(),
            server_public_key,
            preshared_key,
            initial_mask: webrtc_zoom_v3(),
            server_signing_pub: None,
            tun_config: TunnelConfig::from_network_config(
                tun_name.clone(),
                network_config,
                full_tunnel,
            ),
        };

        match AivpnClient::new(config) {
            Ok(mut client) => {
                info!("Client initialized successfully (TUN: {})", tun_name);
                
                // Write initial stats file
                let _ = std::fs::write("/var/run/aivpn/traffic.stats", "sent:0,received:0");
                let _ = std::fs::write("/tmp/aivpn-traffic.stats", "sent:0,received:0");
                aivpn_client::record_cmd::reset_local_status();

                match client.run(shutdown.clone()).await {
                    Ok(()) => break,
                    Err(e) => {
                        warn!("Client run failed: {}. Reconnecting in {}s", e, backoff.as_secs());
                    }
                }
            }
            Err(e) => {
                error!("Failed to create client: {}. Reconnecting in {}s", e, backoff.as_secs());
            }
        }

        if shutdown.load(Ordering::SeqCst) {
            info!("Shutdown requested after failure");
            break;
        }

        tokio::time::sleep(backoff).await;
        backoff = std::cmp::min(backoff * 2, max_backoff);
    }
}

fn fallback_network_config(tun_addr: &str) -> ClientNetworkConfig {
    let client_ip = tun_addr.parse::<Ipv4Addr>().unwrap_or_else(|_| {
        error!("Invalid TUN address '{}': expected IPv4 address", tun_addr);
        std::process::exit(1);
    });

    ClientNetworkConfig {
        client_ip,
        server_vpn_ip: LEGACY_SERVER_VPN_IP,
        prefix_len: 24,
        mtu: DEFAULT_VPN_MTU,
        mdh_len: 20,
    }
}
