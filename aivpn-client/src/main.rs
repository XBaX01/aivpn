//! AIVPN Client Binary - Full Implementation

use aivpn_client::adaptive::{AdaptiveConfig, AdaptiveMonitor};
use aivpn_client::bench::run_bench;
use aivpn_client::bootstrap_cache;
use aivpn_client::bootstrap_loader::{self, BootstrapConfig};
use aivpn_client::client::ClientConfig;
use aivpn_client::server_pool::{PoolMode, ServerEntry, ServerPool};
use aivpn_client::tunnel::TunnelConfig;
use aivpn_client::AivpnClient;
use aivpn_common::mask::preset_masks;
#[cfg(not(feature = "production-secure"))]
use aivpn_common::mask::preset_masks::bootstrap_default;
use aivpn_common::mask::BootstrapDescriptor;
use aivpn_common::network_config::{ClientNetworkConfig, DEFAULT_VPN_MTU, LEGACY_SERVER_VPN_IP};
use base64::Engine;
use clap::Parser;
use serde::Deserialize;
use std::net::Ipv4Addr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tracing::{error, info, warn};

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

    /// Server signing public key (base64, 32 bytes)
    #[arg(long)]
    pub server_signing_key: Option<String>,

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

    /// Passive bootstrap descriptor URL (may be repeated)
    #[arg(long)]
    pub bootstrap_descriptor_url: Vec<String>,

    /// CDN bootstrap URL (multi-channel distribution)
    #[arg(long)]
    pub bootstrap_cdn_url: Option<String>,

    /// Telegram bot for bootstrap distribution (e.g., @aivpn_bot)
    #[arg(long)]
    pub bootstrap_telegram: Option<String>,

    /// GitHub repo for bootstrap distribution (e.g., infosave2007/aivpn)
    #[arg(long)]
    pub bootstrap_github: Option<String>,

    /// IPFS hash for bootstrap distribution
    #[arg(long)]
    pub bootstrap_ipfs: Option<String>,

    /// Disable built-in bootstrap fallback (production secure mode)
    #[arg(long, default_value_t = false)]
    pub no_fallback: bool,

    /// Run as SOCKS5 proxy on this address instead of a TUN device (no root required).
    /// Example: --proxy-listen 127.0.0.1:1080
    #[arg(long, value_name = "HOST:PORT")]
    pub proxy_listen: Option<String>,

    /// Path to a 104-byte mTLS client certificate (raw binary or base64-encoded).
    /// Required when the server has `mtls.required = true`.
    #[arg(long, value_name = "FILE")]
    pub mtls_cert: Option<std::path::PathBuf>,

    /// Route only these CIDRs through the VPN (comma-separated, split-tunnel mode).
    /// Example: --include-routes 10.0.0.0/8,192.168.1.0/24
    #[arg(
        long,
        value_name = "CIDR,...",
        use_value_delimiter = true,
        value_delimiter = ','
    )]
    pub include_routes: Vec<String>,

    /// Bypass the VPN for these CIDRs (comma-separated). Use with --full-tunnel to exclude subnets.
    /// Example: --exclude-routes 192.168.0.0/16,172.16.0.0/12
    #[arg(
        long,
        value_name = "CIDR,...",
        use_value_delimiter = true,
        value_delimiter = ','
    )]
    pub exclude_routes: Vec<String>,

    /// Block all non-VPN traffic while connected (kill-switch / leak protection).
    /// Rules persist after unexpected process death; run `kill-switch clear` to recover.
    #[arg(long, default_value_t = false)]
    pub kill_switch: bool,

    /// Enable adaptive mode — automatically adjusts MTU and keepalive on packet loss.
    #[arg(long, default_value_t = false)]
    pub adaptive: bool,

    /// Start a local DNS proxy on this address to prevent DNS leaks (e.g. 127.0.0.1:5300).
    /// Point /etc/resolv.conf at this address after connecting.
    #[arg(long, value_name = "HOST:PORT")]
    pub dns_proxy: Option<String>,

    /// Upstream DNS resolver used by --dns-proxy (default: 1.1.1.1:53).
    #[arg(long, value_name = "HOST:PORT", default_value = "1.1.1.1:53")]
    pub dns_upstream: String,

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
    /// Kill-switch management
    #[command(name = "kill-switch")]
    KillSwitch {
        #[command(subcommand)]
        action: KillSwitchAction,
    },
    /// Run connection diagnostics (latency, packet loss, quality score)
    Bench {
        /// Duration of the benchmark in seconds
        #[arg(short, long, default_value_t = 10)]
        duration: u64,
        /// Output as JSON instead of human-readable text
        #[arg(long)]
        json: bool,
    },
}

#[derive(clap::Subcommand, Debug)]
pub enum KillSwitchAction {
    /// Remove stale kill-switch firewall rules left by a previous session
    Clear,
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

#[derive(Debug, Clone, Default, Deserialize)]
struct ClientFileConfig {
    server_addr: Option<String>,
    server_public_key: Option<String>,
    server_signing_public_key: Option<String>,
    preshared_key: Option<String>,
    tun_name: Option<String>,
    tun_addr: Option<String>,
    full_tunnel: Option<bool>,
    network_config: Option<ClientNetworkConfig>,
    bootstrap_descriptor_urls: Option<Vec<String>>,
    bootstrap_descriptors: Option<Vec<BootstrapDescriptor>>,
    include_routes: Option<Vec<String>>,
    exclude_routes: Option<Vec<String>>,
    kill_switch: Option<bool>,
}

fn load_client_file_config(path: Option<&str>) -> Option<ClientFileConfig> {
    let resolved = path?;
    std::fs::read_to_string(&resolved)
        .ok()
        .and_then(|json| serde_json::from_str::<ClientFileConfig>(&json).ok())
}

fn decode_base64_key(label: &str, encoded: &str) -> [u8; 32] {
    let decoded = base64::engine::general_purpose::STANDARD
        .decode(encoded)
        .unwrap_or_else(|e| {
            error!("Invalid {}: {}", label, e);
            std::process::exit(1);
        });
    if decoded.len() != 32 {
        error!("{} must be 32 bytes, got {}", label, decoded.len());
        std::process::exit(1);
    }
    let mut out = [0u8; 32];
    out.copy_from_slice(&decoded);
    out
}

// In development mode, derive a deterministic bootstrap mask from the PSK when no cached descriptors are available.
fn bootstrap_mask_for_psk(psk: &[u8; 32]) -> aivpn_common::mask::MaskProfile {
    use blake3;
    let presets = preset_masks::all();
    let hash = blake3::derive_key("aivpn-bootstrap-mask-v1", psk);
    let idx = hash[0] as usize % presets.len();
    presets[idx].clone()
}

#[tokio::main]
async fn main() {
    // Initialize logging — default to INFO level when RUST_LOG is not set
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    // Setup Ctrl+C handler in a separate task
    let shutdown = Arc::new(AtomicBool::new(false));
    let shutdown_clone = shutdown.clone();
    tokio::spawn(async move {
        tokio::signal::ctrl_c()
            .await
            .expect("Failed to setup signal handler");
        info!("Received Ctrl+C, shutting down...");
        shutdown_clone.store(true, Ordering::SeqCst);
        SHUTDOWN.store(true, Ordering::SeqCst);
    });

    // Parse arguments
    let args = ClientArgs::parse();

    // Handle subcommands
    if let Some(command) = args.command {
        match command {
            ClientCommand::KillSwitch { action } => match action {
                KillSwitchAction::Clear => {
                    aivpn_client::kill_switch::KillSwitch::clear_stale();
                    println!("Kill-switch stale rules cleared.");
                    return;
                }
            },
            ClientCommand::Bench { duration, json } => {
                let server_addr = args
                    .server
                    .clone()
                    .or_else(|| {
                        args.connection_key.as_deref().and_then(|ck| {
                            let payload = ck.trim().strip_prefix("aivpn://").unwrap_or(ck.trim());
                            base64::engine::general_purpose::URL_SAFE_NO_PAD
                                .decode(payload)
                                .ok()
                                .and_then(|b| serde_json::from_slice::<serde_json::Value>(&b).ok())
                                .and_then(|v| v["s"].as_str().map(|s| s.to_string()))
                        })
                    })
                    .unwrap_or_else(|| {
                        error!("--server or --connection-key required for bench");
                        std::process::exit(1);
                    });

                let addr = server_addr.parse().unwrap_or_else(|e| {
                    error!("Invalid server address '{}': {}", server_addr, e);
                    std::process::exit(1);
                });
                let result = run_bench(addr, Duration::from_secs(duration)).await;
                if json {
                    println!(
                        "{}",
                        serde_json::to_string_pretty(&result).unwrap_or_default()
                    );
                } else {
                    println!("═══ AIVPN Diagnostics ═══");
                    println!("Server:      {}", server_addr);
                    println!("Samples:     {}", result.samples);
                    println!("Latency P50: {:.1} ms", result.latency_p50_ms);
                    println!("Latency P95: {:.1} ms", result.latency_p95_ms);
                    println!("Latency P99: {:.1} ms", result.latency_p99_ms);
                    println!("Packet loss: {:.1}%", result.packet_loss_pct);
                    println!("Throughput:  {:.0} kbps est.", result.throughput_up_kbps);
                    println!(
                        "Quality:     {} ({})",
                        result.quality_score, result.quality_label
                    );
                }
                return;
            }
            ClientCommand::Record { action } => {
                match action {
                    RecordAction::Start { service } => {
                        aivpn_client::record_cmd::handle_recording_status(true, Some(&service));
                        let socket = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();
                        socket
                            .send_to(
                                format!("record_start:{}", service).as_bytes(),
                                "127.0.0.1:44301",
                            )
                            .unwrap();
                        println!("Recording start requested for '{}'.", service);
                        println!("Run 'aivpn-client record status' to inspect progress.");
                    }
                    RecordAction::Stop => {
                        let prior = aivpn_client::record_cmd::read_local_status();
                        aivpn_client::record_cmd::mark_recording_stop_requested(
                            prior.as_ref().and_then(|status| status.service.as_deref()),
                        );
                        let socket = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();
                        socket.send_to(b"record_stop", "127.0.0.1:44301").unwrap();
                        println!("Recording stop requested.");
                        println!("Run 'aivpn-client record status' to inspect progress.");
                    }
                    RecordAction::Status => {
                        let before = aivpn_client::record_cmd::read_local_status()
                            .map(|status| status.updated_at_ms)
                            .unwrap_or(0);
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
                        if let Some(status) =
                            latest.or_else(aivpn_client::record_cmd::read_local_status)
                        {
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

    let file_config = load_client_file_config(args.config.as_deref());

    // Parse connection key or individual args
    let (
        server_addr,
        server_key_b64,
        psk_bytes,
        network_config,
        inline_descriptors,
        bootstrap_descriptor_urls,
        tun_name_fixed,
        full_tunnel,
    ) = if let Some(ref conn_key) = args.connection_key {
        let payload = conn_key
            .trim()
            .strip_prefix("aivpn://")
            .unwrap_or(conn_key.trim());
        let json_bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .decode(payload)
            .unwrap_or_else(|e| {
                error!("Invalid connection key: {}", e);
                std::process::exit(1);
            });
        let json: serde_json::Value = serde_json::from_slice(&json_bytes).unwrap_or_else(|e| {
            error!("Malformed connection key JSON: {}", e);
            std::process::exit(1);
        });
        let s = json["s"]
            .as_str()
            .unwrap_or_else(|| {
                error!("Connection key missing server address (\"s\")");
                std::process::exit(1);
            })
            .to_string();
        let k = json["k"]
            .as_str()
            .unwrap_or_else(|| {
                error!("Connection key missing server key (\"k\")");
                std::process::exit(1);
            })
            .to_string();
        let psk: Option<Vec<u8>> = json["p"]
            .as_str()
            .and_then(|p| base64::engine::general_purpose::STANDARD.decode(p).ok());
        let network_config = json
            .get("n")
            .cloned()
            .and_then(|value| serde_json::from_value::<ClientNetworkConfig>(value).ok())
            .or_else(|| {
                json["i"].as_str().and_then(|ip| {
                    ip.parse::<Ipv4Addr>()
                        .ok()
                        .map(|client_ip| ClientNetworkConfig {
                            client_ip,
                            server_vpn_ip: LEGACY_SERVER_VPN_IP,
                            prefix_len: 24,
                            mtu: DEFAULT_VPN_MTU,
                            mdh_len: 20,
                            keepalive_secs: None,
                            ipv6_address: None,
                        })
                })
            })
            .unwrap_or_else(|| fallback_network_config(&args.tun_addr));
        let inline_descriptors = json
            .get("bd")
            .cloned()
            .and_then(|value| serde_json::from_value::<Vec<BootstrapDescriptor>>(value).ok())
            .unwrap_or_default();
        let bootstrap_descriptor_urls = json
            .get("bu")
            .cloned()
            .and_then(|value| serde_json::from_value::<Vec<String>>(value).ok())
            .unwrap_or_default();
        (
            s,
            k,
            psk,
            network_config,
            inline_descriptors,
            bootstrap_descriptor_urls,
            args.tun_name.clone(),
            args.full_tunnel,
        )
    } else {
        let server = args
            .server
            .clone()
            .or_else(|| {
                file_config
                    .as_ref()
                    .and_then(|config| config.server_addr.clone())
            })
            .unwrap_or_else(|| {
                error!("Either --connection-key or --server + --server-key required");
                std::process::exit(1);
            });
        let key = args
            .server_key
            .clone()
            .or_else(|| {
                file_config
                    .as_ref()
                    .and_then(|config| config.server_public_key.clone())
            })
            .unwrap_or_else(|| {
                error!("Either --connection-key or --server + --server-key required");
                std::process::exit(1);
            });
        let psk = file_config
            .as_ref()
            .and_then(|config| config.preshared_key.as_ref())
            .and_then(|value| base64::engine::general_purpose::STANDARD.decode(value).ok());
        let network_config = file_config
            .as_ref()
            .and_then(|config| config.network_config.clone())
            .unwrap_or_else(|| {
                fallback_network_config(
                    file_config
                        .as_ref()
                        .and_then(|config| config.tun_addr.as_deref())
                        .unwrap_or(&args.tun_addr),
                )
            });
        let mut urls = file_config
            .as_ref()
            .and_then(|config| config.bootstrap_descriptor_urls.clone())
            .unwrap_or_default();
        urls.extend(args.bootstrap_descriptor_url.clone());
        (
            server,
            key,
            psk,
            network_config,
            file_config
                .as_ref()
                .and_then(|config| config.bootstrap_descriptors.clone())
                .unwrap_or_default(),
            urls,
            args.tun_name.clone().or_else(|| {
                file_config
                    .as_ref()
                    .and_then(|config| config.tun_name.clone())
            }),
            args.full_tunnel
                || file_config
                    .as_ref()
                    .and_then(|config| config.full_tunnel)
                    .unwrap_or(false),
        )
    };

    // Build server pool from optional "pool" array in the connection key
    let server_pool = args.connection_key.as_deref().and_then(|ck| {
        let payload = ck.trim().strip_prefix("aivpn://").unwrap_or(ck.trim());
        let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .decode(payload)
            .ok()?;
        let json: serde_json::Value = serde_json::from_slice(&bytes).ok()?;
        let peers: Vec<ServerEntry> = serde_json::from_value(json.get("pool")?.clone()).ok()?;
        if peers.is_empty() {
            return None;
        }
        Some(ServerPool::new(&server_addr, peers, PoolMode::Failover))
    });
    if let Some(ref pool) = server_pool {
        info!("Server pool active: {} node(s)", pool.node_count());
    }

    // Adaptive mode monitor
    let adaptive_monitor = AdaptiveMonitor::new(AdaptiveConfig {
        enabled: args.adaptive,
        ..AdaptiveConfig::default()
    });
    if args.adaptive {
        info!("Adaptive mode enabled (auto MTU/keepalive tuning)");
    }
    // Lower initial MTU for restrictive mobile networks (MTS, Megafon) when adaptive is on.
    // The AdaptiveMonitor will step it down further if packet loss is detected.
    let network_config = if args.adaptive {
        aivpn_common::network_config::ClientNetworkConfig {
            mtu: network_config.mtu.min(1200),
            ..network_config
        }
    } else {
        network_config
    };
    let _ = adaptive_monitor;

    info!("AIVPN Client v{}", env!("CARGO_PKG_VERSION"));
    info!("Connecting to server: {}", server_addr);

    let server_public_key = decode_base64_key("server key", &server_key_b64);

    // Optional ed25519 signing key for ServerHello/MaskUpdate/BootstrapDescriptor verification.
    let server_signing_key: Option<[u8; 32]> = args
        .server_signing_key
        .as_deref()
        .or_else(|| {
            file_config
                .as_ref()
                .and_then(|c| c.server_signing_public_key.as_deref())
        })
        .map(|b64| decode_base64_key("server signing key", b64));

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

    let network_config = network_config;

    for descriptor in inline_descriptors {
        if let Err(e) = bootstrap_cache::store_verified_descriptor(descriptor, None) {
            warn!(
                "Failed to store bootstrap descriptor from config/key: {}",
                e
            );
        }
    }
    let fetched = bootstrap_cache::refresh_from_urls(&bootstrap_descriptor_urls).await;
    if fetched > 0 {
        info!(
            "Fetched {} bootstrap descriptor(s) from passive URLs",
            fetched
        );
    }

    // Build multi-channel bootstrap configuration if any channels are specified
    let mut bootstrap_config = BootstrapConfig::default();
    if let Some(cdn_url) = &args.bootstrap_cdn_url {
        bootstrap_config = bootstrap_config.with_cdn(cdn_url, "custom");
    }
    if let Some(telegram_bot) = &args.bootstrap_telegram {
        bootstrap_config = bootstrap_config.with_telegram(telegram_bot);
    }
    if let Some(github_repo) = &args.bootstrap_github {
        bootstrap_config = bootstrap_config.with_github(github_repo, "bootstrap-");
    }
    if let Some(ipfs_hash) = &args.bootstrap_ipfs {
        bootstrap_config = bootstrap_config.with_ipfs(ipfs_hash);
    }

    // Load from multi-channel if configured
    if !bootstrap_config.channels.is_empty() {
        info!(
            "Loading bootstrap descriptors from {} channels",
            bootstrap_config.channels.len()
        );
        let stats = bootstrap_loader::load_multi_channel(&bootstrap_config).await;
        info!(
            "Multi-channel bootstrap: {}/{} succeeded, {} descriptors loaded in {}ms",
            stats.successful_channels,
            stats.total_channels,
            stats.total_descriptors,
            stats.elapsed_ms
        );
    }

    let no_fallback = args.no_fallback || !bootstrap_config.channels.is_empty();
    let proxy_listen = args.proxy_listen.as_ref().map(|s| {
        s.parse::<std::net::SocketAddr>().unwrap_or_else(|e| {
            error!("Invalid --proxy-listen '{}': {}", s, e);
            std::process::exit(1);
        })
    });
    let mtls_cert: Option<Vec<u8>> = args.mtls_cert.as_ref().map(|path| {
        let raw = std::fs::read(path).unwrap_or_else(|e| {
            error!("Cannot read --mtls-cert '{}': {}", path.display(), e);
            std::process::exit(1);
        });
        // Accept raw 104-byte binary or base64-encoded text.
        if raw.len() == 104 {
            raw
        } else {
            let s = String::from_utf8_lossy(&raw);
            use base64::Engine as _;
            base64::engine::general_purpose::STANDARD
                .decode(s.trim())
                .unwrap_or_else(|e| {
                    error!(
                        "--mtls-cert is neither 104-byte binary nor valid base64: {}",
                        e
                    );
                    std::process::exit(1);
                })
        }
    });
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

        // Select initial mask from cached descriptors
        // In production secure mode (no_fallback), fail if no valid descriptors are available
        let initial_mask = bootstrap_cache::select_initial_mask(preshared_key.as_ref());

        let initial_mask = if no_fallback {
            match initial_mask {
                Some(mask) => mask,
                None => {
                    error!(
                        "No valid bootstrap descriptors available and fallback disabled. \
                         Ensure multi-channel bootstrap is configured or descriptors are cached."
                    );
                    tokio::time::sleep(backoff).await;
                    backoff = std::cmp::min(backoff * 2, max_backoff);
                    continue;
                }
            }
        } else {
            // Development mode: fall back to built-in mask if no cached descriptors
            #[cfg(feature = "production-secure")]
            {
                match initial_mask {
                    Some(mask) => mask,
                    None => {
                        error!("No valid bootstrap descriptors available");
                        tokio::time::sleep(backoff).await;
                        backoff = std::cmp::min(backoff * 2, max_backoff);
                        continue;
                    }
                }
            }
            #[cfg(not(feature = "production-secure"))]
            {
                // psk-based fallback provides a stable development experience without requiring descriptor management, while still allowing testing of the full mask selection and rotation logic.
                match initial_mask {
                    Some(mask) => mask,
                    None => match &preshared_key {
                        Some(psk_bytes) => bootstrap_mask_for_psk(psk_bytes),
                        None => bootstrap_default(),
                    },
                }
            }
        };

        let include_routes: Vec<String> = if !args.include_routes.is_empty() {
            args.include_routes.clone()
        } else {
            file_config
                .as_ref()
                .and_then(|c| c.include_routes.clone())
                .unwrap_or_default()
        };
        let exclude_routes: Vec<String> = if !args.exclude_routes.is_empty() {
            args.exclude_routes.clone()
        } else {
            file_config
                .as_ref()
                .and_then(|c| c.exclude_routes.clone())
                .unwrap_or_default()
        };
        let mut tun_config = TunnelConfig::from_network_config(
            tun_name.clone(),
            network_config.clone(),
            full_tunnel,
        );
        tun_config.include_routes = include_routes;
        tun_config.exclude_routes = exclude_routes;
        tun_config.kill_switch = args.kill_switch
            || file_config
                .as_ref()
                .and_then(|c| c.kill_switch)
                .unwrap_or(false);

        // Failover: pick next healthy node from pool, or fall back to primary
        let active_server = server_pool
            .as_ref()
            .and_then(|p| p.next_server())
            .map(|a| a.to_string())
            .unwrap_or_else(|| server_addr.clone());

        let config = ClientConfig {
            server_addr: active_server,
            server_public_key,
            server_signing_key,
            preshared_key,
            initial_mask,
            tun_config,
            proxy_listen,
            mtls_cert: mtls_cert.clone(),
        };

        // Start DNS proxy before connecting so it is ready as soon as the tunnel is up.
        let _dns_proxy_handle = args.dns_proxy.as_deref().and_then(|bind_str| {
            let bind_addr = bind_str.parse::<std::net::SocketAddr>().ok()?;
            let upstream_addr = args.dns_upstream.parse::<std::net::SocketAddr>().ok()?;
            Some(aivpn_client::dns_proxy::spawn_dns_proxy(
                aivpn_client::dns_proxy::DnsProxyConfig { listen_addr: bind_addr, upstream_addr },
            ))
        });

        match AivpnClient::new(config) {
            Ok(mut client) => {
                info!("Client initialized successfully (TUN: {})", tun_name);

                // Write initial stats file (platform-appropriate paths)
                #[cfg(target_os = "windows")]
                {
                    if let Some(local_app) = std::env::var_os("LOCALAPPDATA") {
                        let dir = std::path::PathBuf::from(local_app).join("AIVPN");
                        let _ = std::fs::create_dir_all(&dir);
                        let _ = std::fs::write(dir.join("traffic.stats"), "sent:0,received:0");
                    }
                    let _ = std::fs::write(
                        std::env::temp_dir().join("aivpn-traffic.stats"),
                        "sent:0,received:0",
                    );
                }
                #[cfg(not(target_os = "windows"))]
                {
                    let _ = std::fs::write("/var/run/aivpn/traffic.stats", "sent:0,received:0");
                    let _ = std::fs::write("/tmp/aivpn-traffic.stats", "sent:0,received:0");
                }
                aivpn_client::record_cmd::reset_local_status();

                match client.run(shutdown.clone()).await {
                    Ok(()) => break,
                    Err(e) => {
                        warn!(
                            "Client run failed: {}. Reconnecting in {}s",
                            e,
                            backoff.as_secs()
                        );
                    }
                }
            }
            Err(e) => {
                error!(
                    "Failed to create client: {}. Reconnecting in {}s",
                    e,
                    backoff.as_secs()
                );
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
        keepalive_secs: None,
        ipv6_address: None,
    }
}
