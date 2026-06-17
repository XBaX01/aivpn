//! Lightweight DNS proxy (0.9.0)
//!
//! Listens on a local UDP port, forwards queries to an upstream resolver
//! through the active VPN tunnel, and returns the response to the caller.
//! This prevents DNS leaks on platforms that don't support per-app DNS
//! configuration (e.g., desktop Linux without systemd-resolved).
//!
//! Usage: `--dns-proxy 127.0.0.1:5300 --dns-upstream 1.1.1.1:53`
//!
//! Point /etc/resolv.conf at 127.0.0.1:5300 (or use a stub resolver) and all
//! DNS traffic will flow through the VPN-protected path.

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;
use tokio::net::UdpSocket;
use tracing::{debug, warn};

const DNS_BUF: usize = 512;
const UPSTREAM_TIMEOUT: Duration = Duration::from_secs(5);

/// Configuration for the embedded DNS proxy.
#[derive(Debug, Clone)]
pub struct DnsProxyConfig {
    /// Address to bind the local listener (e.g., "127.0.0.1:5300").
    pub listen_addr: SocketAddr,
    /// Upstream DNS resolver reachable via the VPN (e.g., "1.1.1.1:53").
    pub upstream_addr: SocketAddr,
}

impl Default for DnsProxyConfig {
    fn default() -> Self {
        Self {
            listen_addr: "127.0.0.1:5300".parse().unwrap(),
            upstream_addr: "1.1.1.1:53".parse().unwrap(),
        }
    }
}

/// Spawn the DNS proxy as a background task. Returns a JoinHandle.
pub fn spawn_dns_proxy(cfg: DnsProxyConfig) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        if let Err(e) = run_dns_proxy(cfg).await {
            warn!("DNS proxy stopped: {}", e);
        }
    })
}

async fn run_dns_proxy(cfg: DnsProxyConfig) -> std::io::Result<()> {
    let listener = Arc::new(UdpSocket::bind(cfg.listen_addr).await?);
    tracing::info!(
        "DNS proxy listening on {} → upstream {}",
        cfg.listen_addr,
        cfg.upstream_addr
    );

    let mut buf = vec![0u8; DNS_BUF];
    loop {
        let (len, client_addr) = match listener.recv_from(&mut buf).await {
            Ok(v) => v,
            Err(e) => {
                warn!("DNS proxy recv error: {}", e);
                continue;
            }
        };
        let query = buf[..len].to_vec();
        let upstream = cfg.upstream_addr;
        let sock = listener.clone();
        tokio::spawn(async move {
            match forward_query(&query, upstream, client_addr, &sock).await {
                Ok(()) => {}
                Err(e) => debug!("DNS forward error for {}: {}", client_addr, e),
            }
        });
    }
}

async fn forward_query(
    query: &[u8],
    upstream: SocketAddr,
    client: SocketAddr,
    out: &UdpSocket,
) -> std::io::Result<()> {
    let bind_addr = if upstream.is_ipv4() { "0.0.0.0:0" } else { "[::]:0" };
    let up_sock = UdpSocket::bind(bind_addr).await?;
    up_sock.send_to(query, upstream).await?;

    let mut resp = vec![0u8; DNS_BUF];
    let (n, _) = tokio::time::timeout(UPSTREAM_TIMEOUT, up_sock.recv_from(&mut resp))
        .await
        .map_err(|_| std::io::Error::new(std::io::ErrorKind::TimedOut, "DNS upstream timeout"))??;

    out.send_to(&resp[..n], client).await?;
    Ok(())
}
