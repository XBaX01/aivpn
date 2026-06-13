//! AIVPN Server Binary

use aivpn_common::crypto;
use aivpn_common::event_log::{EventBus, EventSinkConfig};
use aivpn_common::mask::{IATDistType, MaskProfile, SizeDistType};
use aivpn_common::network_config::{
    netmask_to_prefix_len, ClientNetworkConfig, VpnNetworkConfig, DEFAULT_VPN_MTU,
};
use aivpn_server::audit_log::AuditLogger;
use aivpn_server::backup::{export_server, import_server, ExportOptions};
use aivpn_server::gateway::GatewayConfig;
use aivpn_server::neural::NeuralConfig;
use aivpn_server::pool_sync::{PeerSyncer, PoolSyncConfig};
use aivpn_server::qos::{dscp_by_name, parse_bandwidth, ClientQos, QosEnforcer};
use aivpn_server::{AivpnServer, ClientDatabase, ServerArgs};
use clap::Parser;
use serde::{Deserialize, Deserializer};
use std::net::{Ipv4Addr, SocketAddr};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tracing::{error, info};

const DEFAULT_SERVER_CONFIG_PATH: &str = "/etc/aivpn/server.json";
const LOCAL_SERVER_CONFIG_PATH: &str = "config/server.json";
const DEFAULT_LISTEN_ADDR: &str = "0.0.0.0:443";

/// `"auto"` or a fixed number in `server.json` `tun_mtu` field.
#[derive(Debug, Clone)]
enum MtuSetting {
    Auto,
    Fixed(u16),
}

impl<'de> Deserialize<'de> for MtuSetting {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        use serde::de::Error;
        let v = serde_json::Value::deserialize(d)?;
        match v {
            serde_json::Value::String(s) if s == "auto" => Ok(MtuSetting::Auto),
            serde_json::Value::Number(n) => n
                .as_u64()
                .and_then(|n| if n <= 65535 { Some(n as u16) } else { None })
                .map(MtuSetting::Fixed)
                .ok_or_else(|| D::Error::custom("tun_mtu must be 0–65535")),
            _ => Err(D::Error::custom("tun_mtu must be a number or \"auto\"")),
        }
    }
}

/// Probe the outbound-interface MTU via `/sys/class/net` and subtract VPN overhead.
/// Falls back to `DEFAULT_TUN_MTU` on any error.
fn detect_mtu() -> u16 {
    let iface = (|| -> Option<String> {
        let out = std::process::Command::new("ip")
            .args(["route", "get", "1.1.1.1"])
            .output()
            .ok()?;
        let text = String::from_utf8_lossy(&out.stdout);
        // "1.1.1.1 via … dev eth0 …" — extract the token after "dev"
        let mut it = text.split_whitespace();
        while let Some(tok) = it.next() {
            if tok == "dev" {
                return it.next().map(|s| s.to_string());
            }
        }
        None
    })();

    let physical_mtu: Option<u16> = iface.as_deref().and_then(|dev| {
        let path = format!("/sys/class/net/{dev}/mtu");
        std::fs::read_to_string(&path)
            .ok()
            .and_then(|s| s.trim().parse::<u16>().ok())
    });

    match physical_mtu {
        Some(mtu) => {
            // 20 IP + 8 UDP + 8 tag + 1 pad_len + 2 inner_hdr + 16 poly1305 = 55; round to 64
            let overhead: u16 = 64;
            let effective = mtu.saturating_sub(overhead).clamp(1200, 1420);
            info!(
                "MTU auto-detected: physical={} (dev={}) → tun={}",
                mtu,
                iface.as_deref().unwrap_or("?"),
                effective
            );
            effective
        }
        None => {
            info!(
                "MTU auto-detection failed (iface={:?}), using default {}",
                iface,
                aivpn_server::nat::DEFAULT_TUN_MTU
            );
            aivpn_server::nat::DEFAULT_TUN_MTU
        }
    }
}

#[derive(Debug, Clone, Default, Deserialize)]
struct ServerFileConfig {
    listen_addr: Option<String>,
    tun_name: Option<String>,
    tun_addr: Option<Ipv4Addr>,
    tun_netmask: Option<Ipv4Addr>,
    network_config: Option<VpnNetworkConfig>,
    mask_dir: Option<String>,
    bootstrap_mask_files: Option<Vec<String>>,
    session_timeout_secs: Option<u64>,
    idle_timeout_secs: Option<u64>,
    tun_mtu: Option<MtuSetting>,
    #[serde(default)]
    pool: Option<PoolSyncConfig>,
}

#[tokio::main]
async fn main() {
    // Parse arguments first (before logging for CLI commands)
    let args = ServerArgs::parse_from(std::env::args());

    // Mask validation doesn't need the server config or client DB.
    if let Some(ref path) = args.validate_mask {
        handle_validate_mask(path);
        return;
    }

    let config_path = resolve_config_path(&args);
    let file_config = load_server_file_config(config_path.as_deref());
    let network_config = resolve_network_config(file_config.as_ref()).unwrap_or_else(|e| {
        eprintln!("Failed to resolve VPN network config: {}", e);
        std::process::exit(1);
    });
    let bootstrap_masks = load_bootstrap_masks(file_config.as_ref()).unwrap_or_else(|e| {
        eprintln!("Failed to load bootstrap masks: {}", e);
        std::process::exit(1);
    });

    // Load client database
    let clients_db_path = Path::new(&args.clients_db);
    let client_db = match ClientDatabase::load(clients_db_path, network_config.clone()) {
        Ok(db) => Arc::new(db),
        Err(e) => {
            eprintln!("Failed to load client database: {}", e);
            std::process::exit(1);
        }
    };

    // Handle CLI management commands (no logging needed)
    if let Some(ref name) = args.add_client {
        handle_add_client(&client_db, name, &args);
        return;
    }
    if let Some(ref id) = args.remove_client {
        handle_remove_client(&client_db, id);
        return;
    }
    if args.list_clients {
        handle_list_clients(&client_db);
        return;
    }
    if let Some(ref id) = args.show_client {
        handle_show_client(&client_db, id, &args);
        return;
    }
    if let Some(ref peer_addr) = args.enroll.clone() {
        handle_enroll(&client_db, peer_addr, &args);
        return;
    }
    if let Some(ref output_path) = args.export.clone() {
        handle_export(&args, output_path);
        return;
    }
    if let Some(ref archive_path) = args.import.clone() {
        handle_import(archive_path, args.dry_run, &args);
        return;
    }
    if let Some(ref name_or_id) = args.set_client_qos.clone() {
        handle_set_client_qos(&client_db, name_or_id, &args);
        return;
    }

    // Initialize logging (only for server mode)
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::from_default_env()
                .add_directive("aivpn_server=debug".parse().unwrap())
                .add_directive("aivpn_common=debug".parse().unwrap()),
        )
        .init();

    info!("AIVPN Server v{}", env!("CARGO_PKG_VERSION"));
    info!("Starting server...");
    info!("Listening on: {}", args.listen);
    info!("Registered clients: {}", client_db.list_clients().len());
    info!(
        "Authoritative VPN subnet: {} (server {}, mtu {})",
        network_config.cidr_string(),
        network_config.server_vpn_ip,
        network_config.mtu,
    );

    // Load server private key from file if provided (HIGH-11)
    let server_private_key = if let Some(ref key_file) = args.key_file {
        let key_data = std::fs::read(key_file).unwrap_or_else(|e| {
            error!("Failed to read key file '{}': {}", key_file, e);
            std::process::exit(1);
        });
        if key_data.len() != 32 {
            error!("Key file must be exactly 32 bytes, got {}", key_data.len());
            std::process::exit(1);
        }
        let mut key = [0u8; 32];
        key.copy_from_slice(&key_data);
        info!("Loaded server key from file");
        let kp = crypto::KeyPair::from_private_key(key);
        let pub_bytes = kp.public_key_bytes();
        info!(
            "Server public key (hex): {}",
            pub_bytes
                .iter()
                .map(|b| format!("{:02x}", b))
                .collect::<String>()
        );
        key
    } else {
        info!("No --key-file provided, server key will be ephemeral");
        [0u8; 32]
    };

    // Generate random TUN name if not specified (MED-1: avoids fingerprinting)
    let tun_name = args
        .tun_name
        .clone()
        .or_else(|| {
            file_config
                .as_ref()
                .and_then(|config| config.tun_name.clone())
        })
        .unwrap_or_else(|| {
            use rand::Rng;
            format!("tun{:04x}", rand::thread_rng().gen::<u16>())
        });

    let listen_addr = resolve_listen_addr(&args, file_config.as_ref());

    // Clone client_db for management API before moving into GatewayConfig
    #[cfg(all(feature = "management-api", unix))]
    let mgmt_db = client_db.clone();
    #[cfg(all(feature = "management-api", unix))]
    let mgmt_socket = args.management_socket.clone();
    #[cfg(all(feature = "management-api", unix))]
    let mgmt_pub_key = if server_private_key != [0u8; 32] {
        Some(crypto::KeyPair::from_private_key(server_private_key).public_key_bytes())
    } else {
        None
    };
    #[cfg(all(feature = "management-api", unix))]
    let mgmt_server_addr = args.server_ip.as_ref().map(|ip| {
        if ip.parse::<SocketAddr>().is_ok() {
            ip.clone()
        } else {
            let port = listen_addr
                .parse::<SocketAddr>()
                .map(|a| a.port())
                .unwrap_or(443);
            format!("{}:{}", ip, port)
        }
    });

    // Build structured event bus (stdout JSONL sink)
    let event_bus = EventBus::new(EventSinkConfig {
        stdout: true,
        webhook_url: None,
    });

    // Audit logger
    let audit_logger = AuditLogger::new(std::path::Path::new(&args.audit_log));

    // Pool sync — start listener + outbound tasks if pool is configured
    let pool_sync_config: Option<PoolSyncConfig> = args
        .pool_config
        .as_deref()
        .and_then(|p| std::fs::read_to_string(p).ok())
        .and_then(|s| serde_json::from_str(&s).ok())
        .or_else(|| file_config.as_ref().and_then(|c| c.pool.clone()));

    // Clone client_db for pool sync before it is consumed by GatewayConfig.
    let client_db_for_sync: Option<Arc<ClientDatabase>> =
        pool_sync_config.as_ref().map(|_| client_db.clone());

    // Build per-client QoS enforcer, pre-loaded from the client DB
    let qos_enforcer = {
        let enforcer = Arc::new(QosEnforcer::new());
        for client in client_db.list_clients() {
            if let Some(qos) = client.qos {
                enforcer.set_client(&client.id, &qos);
            }
        }
        enforcer
    };

    // Create config
    let config = GatewayConfig {
        listen_addr,
        per_ip_pps_limit: args.per_ip_pps_limit,
        tun_name,
        tun_addr: network_config.server_ip_string(),
        tun_netmask: network_config.netmask_string(),
        network_config,
        server_private_key,
        signing_key: [0u8; 64],
        enable_nat: true,
        enable_neural: true,
        neural_config: NeuralConfig::default(),
        client_db: Some(client_db),
        mask_dir: resolve_mask_dir(&args, file_config.as_ref()),
        session_timeout_secs: file_config.as_ref().and_then(|c| c.session_timeout_secs),
        idle_timeout_secs: file_config.as_ref().and_then(|c| c.idle_timeout_secs),
        bootstrap_masks,
        tun_mtu: match file_config.as_ref().and_then(|c| c.tun_mtu.as_ref()) {
            Some(MtuSetting::Fixed(v)) => *v,
            Some(MtuSetting::Auto) | None => detect_mtu(),
        },
        event_bus: event_bus.clone(),
        qos_enforcer,
    };
    let _ = audit_logger; // used by management subcommands; suppress unused warning

    // Spawn management API (Unix socket, optional)
    #[cfg(all(feature = "management-api", unix))]
    {
        if mgmt_socket.is_some() {
            let db = mgmt_db.clone();
            let socket = mgmt_socket.clone();
            let handle = tokio::spawn(async move {
                aivpn_server::management_api::serve(
                    Some(db),
                    socket,
                    mgmt_pub_key,
                    mgmt_server_addr,
                )
                .await;
            });
            // Keep handle alive; log if the task exits unexpectedly
            tokio::spawn(async move {
                if handle.await.is_err() {
                    error!("Management API task exited unexpectedly");
                }
            });
        }

        // SIGHUP → reload client database
        {
            let db = mgmt_db;
            tokio::spawn(async move {
                use tokio::signal::unix::{signal, SignalKind};
                let mut sighup = match signal(SignalKind::hangup()) {
                    Ok(s) => s,
                    Err(e) => {
                        tracing::warn!("Failed to register SIGHUP handler: {}", e);
                        return;
                    }
                };
                loop {
                    sighup.recv().await;
                    info!("SIGHUP received — reloading client database");
                    let db = db.clone();
                    let _ = tokio::task::spawn_blocking(move || db.reload_if_changed()).await;
                }
            });
        }
    }

    // Create and run server
    match AivpnServer::new(config) {
        Ok(server) => {
            // Start pool sync after session_manager and mask catalog are initialised.
            // Sync packets ride the existing VPN UDP port — no extra TCP port needed.
            if let (Some(ref pool_cfg), Some(db)) = (&pool_sync_config, client_db_for_sync) {
                if let Some(syncer) =
                    PeerSyncer::new(db, pool_cfg, server.catalog_mdh(), event_bus.clone())
                {
                    syncer.start(server.session_manager());
                    info!(
                        "Pool sync active ({} peers, in-protocol UDP)",
                        pool_cfg.peers.len()
                    );
                }
            }
            info!("Server initialized successfully");
            if let Err(e) = server.run().await {
                error!("Server error: {}", e);
                std::process::exit(1);
            }
        }
        Err(e) => {
            error!("Failed to create server: {}", e);
            std::process::exit(1);
        }
    }
}

fn load_server_public_key(args: &ServerArgs) -> Option<[u8; 32]> {
    args.key_file.as_ref().and_then(|key_file| {
        let key_data = std::fs::read(key_file).ok()?;
        if key_data.len() != 32 {
            return None;
        }
        let mut key = [0u8; 32];
        key.copy_from_slice(&key_data);
        let kp = crypto::KeyPair::from_private_key(key);
        Some(kp.public_key_bytes())
    })
}

/// Build a connection key: aivpn://BASE64({"s":"host:port","k":"...","p":"...","i":"...","n":{...}})
fn build_connection_key(
    args: &ServerArgs,
    server_ip: &str,
    server_pub_b64: &str,
    psk_b64: &str,
    client_network_config: ClientNetworkConfig,
) -> String {
    use base64::Engine;
    let server_addr = build_connection_server_addr(args, server_ip);
    let json = serde_json::json!({
        "s": server_addr,
        "k": server_pub_b64,
        "p": psk_b64,
        "i": client_network_config.client_ip,
        "n": client_network_config,
    });
    let json_bytes = serde_json::to_string(&json).unwrap();
    let encoded = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(json_bytes.as_bytes());
    format!("aivpn://{}", encoded)
}

fn build_connection_server_addr(args: &ServerArgs, server_ip: &str) -> String {
    if server_ip.parse::<SocketAddr>().is_ok() {
        return server_ip.to_string();
    }

    let config_path = resolve_config_path(args);
    let file_config = load_server_file_config(config_path.as_deref());
    let listen_addr = resolve_listen_addr(args, file_config.as_ref());

    let port = listen_addr
        .parse::<SocketAddr>()
        .map(|addr| addr.port())
        .unwrap_or(443);

    format!("{}:{}", server_ip, port)
}

fn handle_add_client(db: &ClientDatabase, name: &str, args: &ServerArgs) {
    match db.add_client(name) {
        Ok(client) => {
            use base64::Engine;
            let psk_b64 = base64::engine::general_purpose::STANDARD.encode(&client.psk);
            let server_pub = load_server_public_key(args);
            let client_network_config = db.network_config().client_config(client.vpn_ip).unwrap();

            println!("✅ Client '{}' created!", name);
            println!("   ID:     {}", client.id);
            println!("   VPN IP: {}", client.vpn_ip);
            println!();

            if let (Some(pub_key), Some(ref server_ip)) = (server_pub, &args.server_ip) {
                let pub_b64 = base64::engine::general_purpose::STANDARD.encode(&pub_key);
                let conn_key = build_connection_key(
                    args,
                    server_ip,
                    &pub_b64,
                    &psk_b64,
                    client_network_config,
                );
                println!("══ Connection Key (paste into app) ══");
                println!();
                println!("{}", conn_key);
                println!();
            } else {
                if server_pub.is_none() {
                    eprintln!("⚠  --key-file not provided, cannot generate connection key");
                }
                if args.server_ip.is_none() {
                    eprintln!("⚠  --server-ip not provided, cannot generate connection key");
                    eprintln!("   Use: --server-ip YOUR_PUBLIC_IP or set AIVPN_SERVER_IP env var");
                }
            }
        }
        Err(e) => {
            eprintln!("❌ Failed to add client: {}", e);
            std::process::exit(1);
        }
    }
}

fn handle_remove_client(db: &ClientDatabase, id: &str) {
    // Allow removal by name too
    let actual_id = db
        .list_clients()
        .iter()
        .find(|c| c.id == id || c.name == id)
        .map(|c| c.id.clone());

    match actual_id {
        Some(cid) => match db.remove_client(&cid) {
            Ok(()) => println!("✅ Client '{}' removed.", id),
            Err(e) => {
                eprintln!("❌ Failed to remove: {}", e);
                std::process::exit(1);
            }
        },
        None => {
            eprintln!("❌ Client '{}' not found.", id);
            std::process::exit(1);
        }
    }
}

fn handle_list_clients(db: &ClientDatabase) {
    let clients = db.list_clients();
    if clients.is_empty() {
        println!("No registered clients.");
        println!();
        println!(
            "Add a client: aivpn-server --add-client \"Phone\" --key-file /etc/aivpn/server.key"
        );
        return;
    }

    println!(
        "{:<18} {:<20} {:<12} {:<8} {:<12} {:<12} {}",
        "ID", "NAME", "VPN IP", "STATUS", "UPLOAD", "DOWNLOAD", "LAST SEEN"
    );
    println!("{}", "-".repeat(100));

    for client in &clients {
        let status = if client.enabled { "active" } else { "disabled" };
        let upload = format_bytes(client.stats.bytes_out);
        let download = format_bytes(client.stats.bytes_in);
        let last_seen = client
            .stats
            .last_connected
            .map(|t| t.format("%Y-%m-%d %H:%M").to_string())
            .unwrap_or_else(|| "never".to_string());

        println!(
            "{:<18} {:<20} {:<12} {:<8} {:<12} {:<12} {}",
            client.id, client.name, client.vpn_ip, status, upload, download, last_seen
        );
    }
    println!();
    println!("Total: {} client(s)", clients.len());
}

fn handle_show_client(db: &ClientDatabase, id: &str, args: &ServerArgs) {
    let client = db
        .list_clients()
        .into_iter()
        .find(|c| c.id == id || c.name == id);

    match client {
        Some(client) => {
            use base64::Engine;
            let psk_b64 = base64::engine::general_purpose::STANDARD.encode(&client.psk);
            let server_pub = load_server_public_key(args);
            let client_network_config = db.network_config().client_config(client.vpn_ip);

            println!("Client: {} ({})", client.name, client.id);
            println!("  VPN IP:      {}", client.vpn_ip);
            println!(
                "  Status:      {}",
                if client.enabled { "active" } else { "disabled" }
            );
            println!(
                "  Created:     {}",
                client.created_at.format("%Y-%m-%d %H:%M")
            );
            println!("  Connections: {}", client.stats.total_connections);
            println!("  Upload:      {}", format_bytes(client.stats.bytes_out));
            println!("  Download:    {}", format_bytes(client.stats.bytes_in));
            println!(
                "  Last seen:   {}",
                client
                    .stats
                    .last_connected
                    .map(|t| t.format("%Y-%m-%d %H:%M:%S").to_string())
                    .unwrap_or_else(|| "never".to_string())
            );

            if let (Some(pub_key), Some(ref server_ip)) = (server_pub, &args.server_ip) {
                match client_network_config {
                    Ok(client_network_config) => {
                        let pub_b64 = base64::engine::general_purpose::STANDARD.encode(&pub_key);
                        let conn_key = build_connection_key(
                            args,
                            server_ip,
                            &pub_b64,
                            &psk_b64,
                            client_network_config,
                        );
                        println!();
                        println!("══ Connection Key ══");
                        println!();
                        println!("{}", conn_key);
                        println!();
                    }
                    Err(err) => {
                        eprintln!("⚠  Cannot generate connection key for this client under the current VPN subnet: {}", err);
                        eprintln!("   Client VPN IP: {}", client.vpn_ip);
                        eprintln!(
                            "   Current server subnet: {}",
                            db.network_config().cidr_string()
                        );
                        eprintln!("   Reissue this client in the active subnet to get a new key.");
                    }
                }
            } else if args.server_ip.is_none() {
                eprintln!("⚠  --server-ip not provided, cannot generate connection key");
            }
        }
        None => {
            eprintln!("Client '{}' not found.", id);
            std::process::exit(1);
        }
    }
}

fn format_bytes(bytes: u64) -> String {
    if bytes < 1024 {
        format!("{} B", bytes)
    } else if bytes < 1024 * 1024 {
        format!("{:.1} KB", bytes as f64 / 1024.0)
    } else if bytes < 1024 * 1024 * 1024 {
        format!("{:.1} MB", bytes as f64 / (1024.0 * 1024.0))
    } else {
        format!("{:.1} GB", bytes as f64 / (1024.0 * 1024.0 * 1024.0))
    }
}

fn load_server_file_config(path: Option<&str>) -> Option<ServerFileConfig> {
    let path = path?;
    let content = std::fs::read_to_string(path).unwrap_or_else(|e| {
        eprintln!("Failed to read config file '{}': {}", path, e);
        std::process::exit(1);
    });
    Some(serde_json::from_str(&content).unwrap_or_else(|e| {
        eprintln!("Failed to parse config file '{}': {}", path, e);
        std::process::exit(1);
    }))
}

fn resolve_config_path(args: &ServerArgs) -> Option<String> {
    if let Some(path) = &args.config {
        return Some(path.clone());
    }

    // Only auto-select a config file that can actually be opened; an existing
    // but unreadable file (e.g. /etc/aivpn/server.json owned by root) must not
    // trigger a hard exit — the server falls back to defaults instead.
    [DEFAULT_SERVER_CONFIG_PATH, LOCAL_SERVER_CONFIG_PATH]
        .iter()
        .find(|path| std::fs::File::open(path).is_ok())
        .map(|path| path.to_string())
}

fn resolve_network_config(
    file_config: Option<&ServerFileConfig>,
) -> aivpn_common::error::Result<VpnNetworkConfig> {
    let config = if let Some(file_config) = file_config {
        if let Some(network_config) = file_config.network_config.clone() {
            network_config
        } else {
            VpnNetworkConfig {
                server_vpn_ip: file_config.tun_addr.unwrap_or(Ipv4Addr::new(10, 0, 0, 1)),
                prefix_len: netmask_to_prefix_len(
                    file_config
                        .tun_netmask
                        .unwrap_or(Ipv4Addr::new(255, 255, 255, 0)),
                )?,
                mtu: DEFAULT_VPN_MTU,
                keepalive_secs: None,
                ipv6_enabled: false,
                ipv6_prefix: "fd10:cafe::/48".to_string(),
            }
        }
    } else {
        VpnNetworkConfig::default()
    };

    config.validate()?;
    Ok(config)
}

fn resolve_listen_addr(args: &ServerArgs, file_config: Option<&ServerFileConfig>) -> String {
    if args.listen == DEFAULT_LISTEN_ADDR {
        file_config
            .and_then(|config| config.listen_addr.clone())
            .unwrap_or_else(|| args.listen.clone())
    } else {
        args.listen.clone()
    }
}

fn load_bootstrap_masks(
    file_config: Option<&ServerFileConfig>,
) -> Result<Vec<MaskProfile>, String> {
    let Some(files) = file_config.and_then(|config| config.bootstrap_mask_files.clone()) else {
        return Ok(Vec::new());
    };

    let mut masks = Vec::new();
    for file in files {
        let content = std::fs::read_to_string(&file).map_err(|e| format!("{}: {}", file, e))?;

        // Trim whitespace to check if file is empty
        let trimmed = content.trim();
        if trimmed.is_empty() {
            // Skip empty files silently
            continue;
        }

        // Try to parse as a single MaskProfile first
        if let Ok(mask) = serde_json::from_str::<MaskProfile>(trimmed) {
            masks.push(mask);
            continue;
        }

        // Try to parse as an array of MaskProfile
        if let Ok(arr) = serde_json::from_str::<Vec<MaskProfile>>(trimmed) {
            masks.extend(arr);
            continue;
        }

        // If both fail, return an error
        return Err(format!(
            "{}: invalid JSON format, expected MaskProfile object or array of MaskProfile objects",
            file
        ));
    }
    Ok(masks)
}

/// Resolve mask directory: CLI --mask-dir / env AIVPN_MASK_DIR → server.json "mask_dir" → default
const DEFAULT_MASK_DIR: &str = "/var/lib/aivpn/masks";

fn resolve_mask_dir(args: &ServerArgs, file_config: Option<&ServerFileConfig>) -> PathBuf {
    // CLI/env already handled by clap (env = "AIVPN_MASK_DIR")
    if let Some(ref dir) = args.mask_dir {
        return PathBuf::from(dir);
    }
    // server.json
    if let Some(ref dir) = file_config.and_then(|c| c.mask_dir.clone()) {
        return PathBuf::from(dir);
    }
    PathBuf::from(DEFAULT_MASK_DIR)
}

fn handle_enroll(db: &ClientDatabase, peer_addr: &str, args: &ServerArgs) {
    use std::io::{Read, Write};
    use std::net::TcpStream;
    use std::time::Duration;

    let stream = TcpStream::connect_timeout(
        &peer_addr.parse().unwrap_or_else(|_| {
            eprintln!("❌ Invalid peer address: {}", peer_addr);
            std::process::exit(1);
        }),
        Duration::from_secs(10),
    );
    let mut stream = match stream {
        Ok(s) => s,
        Err(e) => {
            eprintln!("❌ Cannot connect to peer {}: {}", peer_addr, e);
            std::process::exit(1);
        }
    };

    // Send enroll probe: our server public key fingerprint
    let pub_key = load_server_public_key(args);
    let probe = serde_json::json!({
        "action": "enroll",
        "pub_key": pub_key.map(|k| hex::encode(k)).unwrap_or_default(),
    })
    .to_string();
    let len = (probe.len() as u32).to_le_bytes();
    let _ = stream.write_all(&len);
    let _ = stream.write_all(probe.as_bytes());

    // Read peer response
    let mut len_buf = [0u8; 4];
    if stream.read_exact(&mut len_buf).is_err() {
        eprintln!("❌ Peer did not respond to enroll probe");
        std::process::exit(1);
    }
    let msg_len = u32::from_le_bytes(len_buf) as usize;
    let mut msg_buf = vec![0u8; msg_len.min(1 << 20)];
    if stream.read_exact(&mut msg_buf).is_err() {
        eprintln!("❌ Failed to read peer response");
        std::process::exit(1);
    }
    let resp: serde_json::Value = serde_json::from_slice(&msg_buf).unwrap_or_default();

    if resp.get("status").and_then(|v| v.as_str()) != Some("ok") {
        eprintln!(
            "❌ Peer rejected enroll: {}",
            resp.get("message")
                .and_then(|v| v.as_str())
                .unwrap_or("unknown error")
        );
        std::process::exit(1);
    }

    // Push our client database to the peer
    let clients_json = match db.export_json() {
        Ok(j) => j,
        Err(e) => {
            eprintln!("❌ Failed to export client DB: {}", e);
            std::process::exit(1);
        }
    };
    let push = serde_json::json!({
        "action": "push_clients",
        "clients": serde_json::from_str::<serde_json::Value>(&clients_json).unwrap_or_default(),
    })
    .to_string();
    let len = (push.len() as u32).to_le_bytes();
    let _ = stream.write_all(&len);
    let _ = stream.write_all(push.as_bytes());

    println!(
        "✅ Peer {} enrolled. Clients pushed: {}",
        peer_addr,
        db.list_clients().len()
    );
    println!("   Add '{}' to your pool config to enable sync.", peer_addr);
}

fn handle_export(args: &ServerArgs, output_path: &str) {
    let opts = ExportOptions {
        include_clients: true,
        include_masks: true,
        include_config: true,
        config_path: Some(PathBuf::from(
            args.config.as_deref().unwrap_or("/etc/aivpn/server.json"),
        )),
        mask_dir: Some(PathBuf::from(
            args.mask_dir.as_deref().unwrap_or("/var/lib/aivpn/masks"),
        )),
        clients_db: Some(PathBuf::from(&args.clients_db)),
    };
    match export_server(&opts, std::path::Path::new(output_path)) {
        Ok(()) => println!("✅ Export complete: {}", output_path),
        Err(e) => {
            eprintln!("❌ Export failed: {}", e);
            std::process::exit(1);
        }
    }
}

fn handle_import(archive_path: &str, dry_run: bool, args: &ServerArgs) {
    let target_dir = args
        .config
        .as_deref()
        .and_then(|p| std::path::Path::new(p).parent())
        .unwrap_or(std::path::Path::new("/etc/aivpn"));
    match import_server(std::path::Path::new(archive_path), target_dir, dry_run) {
        Ok(()) => {
            if dry_run {
                println!("✅ Dry-run complete. No files written.");
            } else {
                println!("✅ Import complete.");
            }
        }
        Err(e) => {
            eprintln!("❌ Import failed: {}", e);
            std::process::exit(1);
        }
    }
}

fn handle_set_client_qos(db: &ClientDatabase, name_or_id: &str, args: &ServerArgs) {
    let client = db
        .list_clients()
        .into_iter()
        .find(|c| c.id == name_or_id || c.name == name_or_id);
    let client = match client {
        Some(c) => c,
        None => {
            eprintln!("❌ Client '{}' not found", name_or_id);
            std::process::exit(1);
        }
    };

    let bw_up = args.bw_up.as_deref().and_then(parse_bandwidth);
    let bw_down = args.bw_down.as_deref().and_then(parse_bandwidth);
    let dscp = args.dscp.as_deref().and_then(dscp_by_name);

    if bw_up.is_none() && bw_down.is_none() && dscp.is_none() && args.dscp.is_none() {
        eprintln!("⚠  No QoS parameters specified. Use --bw-up, --bw-down, and/or --dscp.");
        std::process::exit(1);
    }

    let qos = ClientQos {
        bandwidth_limit_up: bw_up,
        bandwidth_limit_down: bw_down,
        dscp_class: dscp,
        priority: None,
    };

    match db.set_client_qos(&client.id, qos) {
        Ok(()) => {
            println!("✅ QoS updated for '{}' ({})", client.name, client.id);
            if let Some(bw) = args.bw_up.as_deref() {
                println!("   Upload limit:   {}", bw);
            }
            if let Some(bw) = args.bw_down.as_deref() {
                println!("   Download limit: {}", bw);
            }
            if let Some(d) = args.dscp.as_deref() {
                println!("   DSCP class:     {}", d);
            }
        }
        Err(e) => {
            eprintln!("❌ Failed to set QoS: {}", e);
            std::process::exit(1);
        }
    }
}

fn handle_validate_mask(path: &str) {
    let content = match std::fs::read_to_string(path) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("error: cannot read {path}: {e}");
            std::process::exit(1);
        }
    };
    let profile: MaskProfile = match serde_json::from_str(&content) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("error: JSON parse failed in {path}: {e}");
            std::process::exit(1);
        }
    };

    let mut issues: Vec<String> = Vec::new();
    let mut warnings: Vec<String> = Vec::new();

    // signature_vector
    let sig_len = profile.signature_vector.len();
    if sig_len != 64 {
        issues.push(format!("signature_vector: {sig_len} floats (expected 64)"));
    } else if !profile.signature_vector.iter().all(|v| v.is_finite()) {
        issues.push("signature_vector: contains NaN or Inf".to_string());
    } else {
        let l2: f32 = profile
            .signature_vector
            .iter()
            .map(|v| v * v)
            .sum::<f32>()
            .sqrt();
        if l2 == 0.0 {
            warnings.push(
                "signature_vector is all-zeros — neural resonance inactive for this mask"
                    .to_string(),
            );
        }
    }

    // header_template vs eph_pub_offset
    let hdr_len = profile.header_template.len();
    if hdr_len != profile.eph_pub_offset as usize {
        issues.push(format!(
            "header_template length ({hdr_len}) != eph_pub_offset ({})",
            profile.eph_pub_offset
        ));
    }
    if profile.eph_pub_length != 32 {
        warnings.push(format!(
            "eph_pub_length = {} (expected 32 for X25519)",
            profile.eph_pub_length
        ));
    }
    let eph_end = profile.eph_pub_offset as u32 + profile.eph_pub_length as u32;
    if eph_end > 1350 {
        issues.push(format!(
            "eph region ends at byte {eph_end}, which exceeds 1350"
        ));
    }

    // size distribution bins sum
    if matches!(profile.size_distribution.dist_type, SizeDistType::Histogram) {
        let sum: f32 = profile.size_distribution.bins.iter().map(|b| b.2).sum();
        if (sum - 1.0).abs() > 0.02 {
            issues.push(format!(
                "size_distribution bins sum = {sum:.4} (expected 1.0 ± 0.02)"
            ));
        }
    }

    // FSM integrity
    let state_ids: std::collections::HashSet<u16> =
        profile.fsm_states.iter().map(|s| s.state_id).collect();
    if !state_ids.contains(&profile.fsm_initial_state) {
        issues.push(format!(
            "fsm_initial_state {} not found in fsm_states",
            profile.fsm_initial_state
        ));
    }
    for state in &profile.fsm_states {
        for t in &state.transitions {
            if !state_ids.contains(&t.next_state) {
                issues.push(format!(
                    "FSM state {}: transition to unknown state {}",
                    state.state_id, t.next_state
                ));
            }
        }
    }

    // expiry
    let now_secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let expires_str = if profile.expires_at == u64::MAX {
        "never".to_string()
    } else if profile.expires_at < now_secs {
        let days = (now_secs - profile.expires_at) / 86400;
        issues.push(format!("mask expired {days} day(s) ago"));
        format!("EXPIRED ({days} days ago)")
    } else {
        let days = (profile.expires_at - now_secs) / 86400;
        format!("{days} days remaining")
    };

    // ── Report ────────────────────────────────────────────────────────────
    println!("═══════════════════════════════════════════════════════");
    println!("Mask:     {} (v{})", profile.mask_id, profile.version);
    println!("Protocol: {:?}", profile.spoof_protocol);
    println!(
        "Header:   {} bytes, eph_pub @ {}..{}",
        hdr_len, profile.eph_pub_offset, eph_end
    );
    println!("Expires:  {expires_str}");

    let l2: f32 = if sig_len == 64 {
        profile
            .signature_vector
            .iter()
            .map(|v| v * v)
            .sum::<f32>()
            .sqrt()
    } else {
        0.0
    };
    println!("Sig vec:  {sig_len} floats, L2={l2:.3}");

    println!("───────────────────────────────────────────────────────");

    match profile.size_distribution.dist_type {
        SizeDistType::Histogram => {
            let bins = &profile.size_distribution.bins;
            let sum: f32 = bins.iter().map(|b| b.2).sum();
            println!("Size:     Histogram ({} bins), sum={sum:.3}", bins.len());
            for (lo, hi, p) in bins {
                println!("          [{lo}–{hi}]: {:.1}%", p * 100.0);
            }
        }
        SizeDistType::Parametric => {
            println!(
                "Size:     Parametric ({:?})",
                profile.size_distribution.parametric_type
            );
        }
    }

    let (jlo, jhi) = profile.iat_distribution.jitter_range_ms;
    let iat_type = match profile.iat_distribution.dist_type {
        IATDistType::Exponential => "Exponential",
        IATDistType::LogNormal => "LogNormal",
        IATDistType::Empirical => "Empirical",
        IATDistType::Gamma => "Gamma",
    };
    println!(
        "IAT:      {} params={:?} jitter=[{jlo:.1}, {jhi:.1}] ms",
        iat_type, profile.iat_distribution.params
    );

    println!(
        "FSM:      {} states, initial={}",
        profile.fsm_states.len(),
        profile.fsm_initial_state
    );
    println!("───────────────────────────────────────────────────────");

    for w in &warnings {
        println!("WARN:  {w}");
    }
    if issues.is_empty() {
        if warnings.is_empty() {
            println!("Result: PASS");
        } else {
            println!("Result: PASS (with warnings)");
        }
    } else {
        for issue in &issues {
            println!("FAIL:  {issue}");
        }
        println!("Result: FAIL ({} issue(s))", issues.len());
        std::process::exit(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::Engine;

    fn test_args(listen: &str) -> ServerArgs {
        ServerArgs {
            listen: listen.to_string(),
            tun_name: None,
            key_file: None,
            config: None,
            clients_db: "/tmp/clients.json".to_string(),
            add_client: None,
            remove_client: None,
            list_clients: false,
            show_client: None,
            server_ip: None,
            per_ip_pps_limit: 1000,
            mask_dir: None,
            validate_mask: None,
            #[cfg(all(feature = "management-api", unix))]
            management_socket: None,
            enroll: None,
            pool_config: None,
            export: None,
            import: None,
            dry_run: false,
            set_client_qos: None,
            bw_up: None,
            bw_down: None,
            dscp: None,
            audit_log: "/dev/null".to_string(),
        }
    }

    #[test]
    fn build_connection_server_addr_keeps_explicit_port() {
        let args = test_args("0.0.0.0:443");
        assert_eq!(
            build_connection_server_addr(&args, "203.0.113.10:8443"),
            "203.0.113.10:8443"
        );
    }

    #[test]
    fn build_connection_server_addr_adds_listen_port_once() {
        let args = test_args("0.0.0.0:443");
        assert_eq!(
            build_connection_server_addr(&args, "203.0.113.10"),
            "203.0.113.10:443"
        );
    }

    #[test]
    fn build_connection_key_embeds_normalized_server_addr() {
        let args = test_args("0.0.0.0:443");
        let key = build_connection_key(
            &args,
            "203.0.113.10:8443",
            "server-key",
            "psk",
            ClientNetworkConfig {
                client_ip: Ipv4Addr::new(10, 0, 0, 2),
                server_vpn_ip: Ipv4Addr::new(10, 0, 0, 1),
                prefix_len: 24,
                mtu: 1346,
                mdh_len: 20,
                keepalive_secs: None,
                ipv6_address: None,
            },
        );
        let payload = key.strip_prefix("aivpn://").unwrap();
        let json_bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .decode(payload)
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&json_bytes).unwrap();

        assert_eq!(json["s"], "203.0.113.10:8443");
        assert_eq!(json["n"]["prefix_len"], 24);
    }

    #[test]
    fn resolve_network_config_prefers_network_config_block() {
        let file_config = ServerFileConfig {
            listen_addr: None,
            tun_name: None,
            tun_addr: Some(Ipv4Addr::new(10, 0, 0, 1)),
            tun_netmask: Some(Ipv4Addr::new(255, 255, 255, 0)),
            network_config: Some(VpnNetworkConfig {
                server_vpn_ip: Ipv4Addr::new(10, 150, 0, 1),
                prefix_len: 24,
                mtu: 1400,
                keepalive_secs: None,
                ..Default::default()
            }),
            mask_dir: None,
            bootstrap_mask_files: None,
            session_timeout_secs: None,
            idle_timeout_secs: None,
            tun_mtu: None,
            pool: None,
        };

        let resolved = resolve_network_config(Some(&file_config)).unwrap();
        assert_eq!(resolved.server_vpn_ip, Ipv4Addr::new(10, 150, 0, 1));
        assert_eq!(resolved.mtu, 1400);
    }

    #[test]
    fn load_bootstrap_masks_handles_empty_file() {
        use std::io::Write;
        let temp_dir = std::env::temp_dir().join("aivpn_test_bootstrap_1");
        std::fs::create_dir_all(&temp_dir).unwrap();

        let empty_file = temp_dir.join("empty.json");
        std::fs::File::create(&empty_file)
            .unwrap()
            .write_all(b"")
            .unwrap();

        let file_config = ServerFileConfig {
            listen_addr: None,
            tun_name: None,
            tun_addr: None,
            tun_netmask: None,
            network_config: None,
            mask_dir: None,
            bootstrap_mask_files: Some(vec![empty_file.to_string_lossy().to_string()]),
            session_timeout_secs: None,
            idle_timeout_secs: None,
            tun_mtu: None,
            pool: None,
        };

        let result = load_bootstrap_masks(Some(&file_config));
        assert!(result.is_ok());
        assert!(result.unwrap().is_empty());

        std::fs::remove_dir_all(&temp_dir).ok();
    }

    #[test]
    fn load_bootstrap_masks_handles_empty_array() {
        use std::io::Write;
        let temp_dir = std::env::temp_dir().join("aivpn_test_bootstrap_2");
        std::fs::create_dir_all(&temp_dir).unwrap();

        let array_file = temp_dir.join("array.json");
        std::fs::File::create(&array_file)
            .unwrap()
            .write_all(b"[]")
            .unwrap();

        let file_config = ServerFileConfig {
            listen_addr: None,
            tun_name: None,
            tun_addr: None,
            tun_netmask: None,
            network_config: None,
            mask_dir: None,
            bootstrap_mask_files: Some(vec![array_file.to_string_lossy().to_string()]),
            session_timeout_secs: None,
            idle_timeout_secs: None,
            tun_mtu: None,
            pool: None,
        };

        let result = load_bootstrap_masks(Some(&file_config));
        assert!(result.is_ok());
        assert!(result.unwrap().is_empty());

        std::fs::remove_dir_all(&temp_dir).ok();
    }

    #[test]
    fn load_bootstrap_masks_handles_single_object() {
        use std::io::Write;
        let temp_dir = std::env::temp_dir().join("aivpn_test_bootstrap_3");
        std::fs::create_dir_all(&temp_dir).unwrap();

        let single_file = temp_dir.join("single.json");
        // Use a real mask profile from mask-assets (simplified but valid)
        let mask_json = r#"{
            "mask_id": "test_mask",
            "version": 2,
            "created_at": 0,
            "expires_at": 18446744073709551615,
            "spoof_protocol": "QUIC",
            "header_template": [192, 0, 0, 0, 1, 8, 73, 142, 56, 201, 15, 88, 197, 42],
            "eph_pub_offset": 14,
            "eph_pub_length": 32,
            "size_distribution": {
                "dist_type": "Histogram",
                "bins": [[64, 128, 0.3], [256, 512, 0.4], [768, 1200, 0.3]],
                "parametric_type": null,
                "parametric_params": null
            },
            "iat_distribution": {
                "dist_type": "Exponential",
                "params": [0.1],
                "jitter_range_ms": [0.0, 10.0]
            },
            "padding_strategy": "MatchDistribution",
            "fsm_states": [{"state_id": 0, "transitions": []}],
            "fsm_initial_state": 0,
            "signature_vector": [0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0],
            "reverse_profile": null,
            "signature": [0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0],
            "header_spec": {
                "type": "Structured",
                "fields": [
                    {"kind": "Fixed", "bytes": [192]},
                    {"kind": "Fixed", "bytes": [0, 0, 0, 1]},
                    {"kind": "Fixed", "bytes": [8]},
                    {"kind": "Id", "len": 8, "mode": "Random"}
                ]
            }
        }"#;
        std::fs::File::create(&single_file)
            .unwrap()
            .write_all(mask_json.as_bytes())
            .unwrap();

        let file_config = ServerFileConfig {
            listen_addr: None,
            tun_name: None,
            tun_addr: None,
            tun_netmask: None,
            network_config: None,
            mask_dir: None,
            bootstrap_mask_files: Some(vec![single_file.to_string_lossy().to_string()]),
            session_timeout_secs: None,
            idle_timeout_secs: None,
            tun_mtu: None,
            pool: None,
        };

        let result = load_bootstrap_masks(Some(&file_config));
        assert!(result.is_ok());
        let masks = result.unwrap();
        assert_eq!(masks.len(), 1);
        assert_eq!(masks[0].mask_id, "test_mask");

        std::fs::remove_dir_all(&temp_dir).ok();
    }

    #[test]
    fn load_bootstrap_masks_handles_array_of_objects() {
        use std::io::Write;
        let temp_dir = std::env::temp_dir().join("aivpn_test_bootstrap_4");
        std::fs::create_dir_all(&temp_dir).unwrap();

        let array_file = temp_dir.join("array.json");
        // Use a real mask profile from mask-assets (simplified but valid)
        let mask_json = r#"[
            {
                "mask_id": "mask1",
                "version": 2,
                "created_at": 0,
                "expires_at": 18446744073709551615,
                "spoof_protocol": "QUIC",
                "header_template": [192, 0, 0, 0, 1, 8, 73, 142, 56, 201, 15, 88, 197, 42],
                "eph_pub_offset": 14,
                "eph_pub_length": 32,
                "size_distribution": {
                    "dist_type": "Histogram",
                    "bins": [[64, 128, 0.3], [256, 512, 0.4], [768, 1200, 0.3]],
                    "parametric_type": null,
                    "parametric_params": null
                },
                "iat_distribution": {
                    "dist_type": "Exponential",
                    "params": [0.1],
                    "jitter_range_ms": [0.0, 10.0]
                },
                "padding_strategy": "MatchDistribution",
                "fsm_states": [{"state_id": 0, "transitions": []}],
                "fsm_initial_state": 0,
                "signature_vector": [0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0],
                "reverse_profile": null,
                "signature": [0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0],
                "header_spec": {
                    "type": "Structured",
                    "fields": [
                        {"kind": "Fixed", "bytes": [192]},
                        {"kind": "Fixed", "bytes": [0, 0, 0, 1]}
                    ]
                }
            },
            {
                "mask_id": "mask2",
                "version": 2,
                "created_at": 0,
                "expires_at": 18446744073709551615,
                "spoof_protocol": "WebRTC_STUN",
                "header_template": [0, 1, 0, 0],
                "eph_pub_offset": 4,
                "eph_pub_length": 32,
                "size_distribution": {
                    "dist_type": "Histogram",
                    "bins": [[256, 512, 0.5], [512, 1024, 0.5]],
                    "parametric_type": null,
                    "parametric_params": null
                },
                "iat_distribution": {
                    "dist_type": "Exponential",
                    "params": [0.2],
                    "jitter_range_ms": [0.0, 20.0]
                },
                "padding_strategy": "MatchDistribution",
                "fsm_states": [{"state_id": 0, "transitions": []}],
                "fsm_initial_state": 0,
                "signature_vector": [0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0],
                "reverse_profile": null,
                "signature": [0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0],
                "header_spec": null
            }
        ]"#;
        std::fs::File::create(&array_file)
            .unwrap()
            .write_all(mask_json.as_bytes())
            .unwrap();

        let file_config = ServerFileConfig {
            listen_addr: None,
            tun_name: None,
            tun_addr: None,
            tun_netmask: None,
            network_config: None,
            mask_dir: None,
            bootstrap_mask_files: Some(vec![array_file.to_string_lossy().to_string()]),
            session_timeout_secs: None,
            idle_timeout_secs: None,
            tun_mtu: None,
            pool: None,
        };

        let result = load_bootstrap_masks(Some(&file_config));
        assert!(result.is_ok());
        let masks = result.unwrap();
        assert_eq!(masks.len(), 2);
        assert_eq!(masks[0].mask_id, "mask1");
        assert_eq!(masks[1].mask_id, "mask2");

        std::fs::remove_dir_all(&temp_dir).ok();
    }
}
