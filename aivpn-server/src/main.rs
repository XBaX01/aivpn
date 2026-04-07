//! AIVPN Server Binary

use aivpn_server::{AivpnServer, ServerArgs, ClientDatabase};
use aivpn_server::gateway::GatewayConfig;
use aivpn_server::neural::NeuralConfig;
use aivpn_common::crypto;
use tracing::{info, error};
use clap::Parser;
use std::net::SocketAddr;
use std::path::Path;
use std::sync::Arc;

#[tokio::main]
async fn main() {
    // Parse arguments first (before logging for CLI commands)
    let args = ServerArgs::parse_from(std::env::args());

    // Load client database
    let clients_db_path = Path::new(&args.clients_db);
    let client_db = match ClientDatabase::load(clients_db_path) {
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

    // Initialize logging (only for server mode)
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::from_default_env()
        )
        .init();

    info!("AIVPN Server v{}", env!("CARGO_PKG_VERSION"));
    info!("Starting server...");
    info!("Listening on: {}", args.listen);
    info!("Registered clients: {}", client_db.list_clients().len());

    // Load server private key from file if provided (HIGH-11)
    let server_private_key = if let Some(ref key_file) = args.key_file {
        let key_data = std::fs::read(key_file)
            .unwrap_or_else(|e| {
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
        info!("Server public key (hex): {}", pub_bytes.iter().map(|b| format!("{:02x}", b)).collect::<String>());
        key
    } else {
        info!("No --key-file provided, server key will be ephemeral");
        [0u8; 32]
    };

    // Generate random TUN name if not specified (MED-1: avoids fingerprinting)
    let tun_name = args.tun_name.unwrap_or_else(|| {
        use rand::Rng;
        format!("tun{:04x}", rand::thread_rng().gen::<u16>())
    });

    // Create config
    let config = GatewayConfig {
        listen_addr: args.listen,
        tun_name,
        tun_addr: "10.0.0.1".to_string(),
        tun_netmask: "255.255.255.0".to_string(),
        server_private_key,
        signing_key: [0u8; 64],
        enable_nat: true,
        enable_neural: true,
        neural_config: NeuralConfig::default(),
        client_db: Some(client_db),
    };

    // Create and run server
    match AivpnServer::new(config) {
        Ok(mut server) => {
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
        if key_data.len() != 32 { return None; }
        let mut key = [0u8; 32];
        key.copy_from_slice(&key_data);
        let kp = crypto::KeyPair::from_private_key(key);
        Some(kp.public_key_bytes())
    })
}

/// Build a connection key: aivpn://BASE64({"s":"host:port","k":"...","p":"...","i":"..."})
fn build_connection_key(args: &ServerArgs, server_ip: &str, server_pub_b64: &str, psk_b64: &str, vpn_ip: &str) -> String {
    use base64::Engine;
    let server_addr = build_connection_server_addr(args, server_ip);
    let json = serde_json::json!({
        "s": server_addr,
        "k": server_pub_b64,
        "p": psk_b64,
        "i": vpn_ip
    });
    let json_bytes = serde_json::to_string(&json).unwrap();
    let encoded = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(json_bytes.as_bytes());
    format!("aivpn://{}", encoded)
}

fn build_connection_server_addr(args: &ServerArgs, server_ip: &str) -> String {
    if server_ip.parse::<SocketAddr>().is_ok() {
        return server_ip.to_string();
    }

    let port = args
        .listen
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

            println!("✅ Client '{}' created!", name);
            println!("   ID:     {}", client.id);
            println!("   VPN IP: {}", client.vpn_ip);
            println!();

            if let (Some(pub_key), Some(ref server_ip)) = (server_pub, &args.server_ip) {
                let pub_b64 = base64::engine::general_purpose::STANDARD.encode(&pub_key);
                let conn_key = build_connection_key(args, server_ip, &pub_b64, &psk_b64, &client.vpn_ip.to_string());
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
    let actual_id = db.list_clients()
        .iter()
        .find(|c| c.id == id || c.name == id)
        .map(|c| c.id.clone());

    match actual_id {
        Some(cid) => {
            match db.remove_client(&cid) {
                Ok(()) => println!("✅ Client '{}' removed.", id),
                Err(e) => {
                    eprintln!("❌ Failed to remove: {}", e);
                    std::process::exit(1);
                }
            }
        }
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
        println!("Add a client: aivpn-server --add-client \"Phone\" --key-file /etc/aivpn/server.key");
        return;
    }

    println!("{:<18} {:<20} {:<12} {:<8} {:<12} {:<12} {}",
        "ID", "NAME", "VPN IP", "STATUS", "UPLOAD", "DOWNLOAD", "LAST SEEN");
    println!("{}", "-".repeat(100));

    for client in &clients {
        let status = if client.enabled { "active" } else { "disabled" };
        let upload = format_bytes(client.stats.bytes_out);
        let download = format_bytes(client.stats.bytes_in);
        let last_seen = client.stats.last_connected
            .map(|t| t.format("%Y-%m-%d %H:%M").to_string())
            .unwrap_or_else(|| "never".to_string());

        println!("{:<18} {:<20} {:<12} {:<8} {:<12} {:<12} {}",
            client.id, client.name, client.vpn_ip, status, upload, download, last_seen);
    }
    println!();
    println!("Total: {} client(s)", clients.len());
}

fn handle_show_client(db: &ClientDatabase, id: &str, args: &ServerArgs) {
    let client = db.list_clients()
        .into_iter()
        .find(|c| c.id == id || c.name == id);

    match client {
        Some(client) => {
            use base64::Engine;
            let psk_b64 = base64::engine::general_purpose::STANDARD.encode(&client.psk);
            let server_pub = load_server_public_key(args);

            println!("Client: {} ({})", client.name, client.id);
            println!("  VPN IP:      {}", client.vpn_ip);
            println!("  Status:      {}", if client.enabled { "active" } else { "disabled" });
            println!("  Created:     {}", client.created_at.format("%Y-%m-%d %H:%M"));
            println!("  Connections: {}", client.stats.total_connections);
            println!("  Upload:      {}", format_bytes(client.stats.bytes_out));
            println!("  Download:    {}", format_bytes(client.stats.bytes_in));
            println!("  Last seen:   {}",
                client.stats.last_connected
                    .map(|t| t.format("%Y-%m-%d %H:%M:%S").to_string())
                    .unwrap_or_else(|| "never".to_string()));

            if let (Some(pub_key), Some(ref server_ip)) = (server_pub, &args.server_ip) {
                let pub_b64 = base64::engine::general_purpose::STANDARD.encode(&pub_key);
                let conn_key = build_connection_key(args, server_ip, &pub_b64, &psk_b64, &client.vpn_ip.to_string());
                println!();
                println!("══ Connection Key ══");
                println!();
                println!("{}", conn_key);
                println!();
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
        }
    }

    #[test]
    fn build_connection_server_addr_keeps_explicit_port() {
        let args = test_args("0.0.0.0:443");
        assert_eq!(build_connection_server_addr(&args, "203.0.113.10:8443"), "203.0.113.10:8443");
    }

    #[test]
    fn build_connection_server_addr_adds_listen_port_once() {
        let args = test_args("0.0.0.0:443");
        assert_eq!(build_connection_server_addr(&args, "203.0.113.10"), "203.0.113.10:443");
    }

    #[test]
    fn build_connection_key_embeds_normalized_server_addr() {
        let args = test_args("0.0.0.0:443");
        let key = build_connection_key(&args, "203.0.113.10:8443", "server-key", "psk", "10.0.0.2");
        let payload = key.strip_prefix("aivpn://").unwrap();
        let json_bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .decode(payload)
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&json_bytes).unwrap();

        assert_eq!(json["s"], "203.0.113.10:8443");
    }
}
