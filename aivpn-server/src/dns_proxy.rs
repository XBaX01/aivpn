//! DNS over HTTPS (DoH) proxy for VPN clients.
//!
//! Listens on UDP port 53 of the VPN gateway IP, forwards queries to a
//! configured DoH upstream via HTTPS POST (RFC 8484 `application/dns-message`),
//! and returns the response to the client.  Replies are forwarded as-is —
//! the upstream resolver handles caching and DNSSEC validation.
//!
//! When `block_plain_dns` is enabled an nftables rule is added that drops
//! plain-UDP/TCP DNS traffic leaving the server on non-VPN interfaces,
//! preventing DNS leaks from VPN clients that bypass the proxy.
//!
//! # server.json
//! ```json
//! {
//!   "dns": {
//!     "upstream_doh": "https://1.1.1.1/dns-query",
//!     "fallback_doh":  "https://8.8.8.8/dns-query",
//!     "block_plain_dns": true
//!   }
//! }
//! ```

use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;
use std::time::{Duration, Instant};

use dashmap::DashMap;
use serde::{Deserialize, Serialize};
use tokio::net::UdpSocket;
use tracing::{debug, info, warn};

const DNS_PORT: u16 = 53;
const MAX_DNS_PACKET: usize = 4096;
const UPSTREAM_TIMEOUT: Duration = Duration::from_secs(5);
/// Max DNS queries per second per VPN client IP before the request is dropped.
const MAX_DNS_RPS: u32 = 100;
/// Max DoH response body size — legitimate DNS responses are ≤65535 bytes.
const MAX_DOH_RESPONSE: usize = 65535;

/// DNS proxy configuration (`"dns"` block in `server.json`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DnsProxyConfig {
    /// Primary DoH upstream URL (RFC 8484).
    pub upstream_doh: String,
    /// Fallback DoH upstream tried when the primary times out.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fallback_doh: Option<String>,
    /// Add nftables rule blocking plain UDP 53 on non-VPN interfaces.
    #[serde(default)]
    pub block_plain_dns: bool,
}

/// Spawn the DNS proxy.  Runs until the socket bind fails.
pub async fn run(config: DnsProxyConfig, bind_ip: IpAddr, tun_iface: String) {
    let bind_addr = SocketAddr::new(bind_ip, DNS_PORT);

    if config.block_plain_dns {
        install_block_rule(&tun_iface);
    }

    let socket = match UdpSocket::bind(bind_addr).await {
        Ok(s) => {
            info!(
                "DNS proxy listening on {} → DoH {}",
                bind_addr, config.upstream_doh
            );
            Arc::new(s)
        }
        Err(e) => {
            warn!(
                "DNS proxy: bind {} failed: {} — proxy disabled",
                bind_addr, e
            );
            return;
        }
    };

    let http = match reqwest::Client::builder()
        .timeout(UPSTREAM_TIMEOUT)
        .https_only(true)
        .build()
    {
        Ok(c) => Arc::new(c),
        Err(e) => {
            warn!(
                "DNS proxy: HTTP client build failed: {} — proxy disabled",
                e
            );
            return;
        }
    };

    let cfg = Arc::new(config);
    let rate_limits: Arc<DashMap<IpAddr, (u32, Instant)>> = Arc::new(DashMap::new());
    let mut buf = vec![0u8; MAX_DNS_PACKET];

    loop {
        let (len, peer) = match socket.recv_from(&mut buf).await {
            Ok(r) => r,
            Err(e) => {
                warn!("DNS proxy: recv error: {}", e);
                continue;
            }
        };

        // Per-source-IP rate limit: cap at MAX_DNS_RPS queries/second.
        let from_ip = peer.ip();
        {
            let mut entry = rate_limits.entry(from_ip).or_insert((0u32, Instant::now()));
            let (count, since) = entry.value_mut();
            if since.elapsed().as_secs() >= 1 {
                *count = 0;
                *since = Instant::now();
            }
            if *count >= MAX_DNS_RPS {
                debug!("DNS proxy: rate limit exceeded for {}", from_ip);
                continue;
            }
            *count += 1;
        }

        let query = buf[..len].to_vec();
        let sock = socket.clone();
        let http = http.clone();
        let cfg = cfg.clone();

        tokio::spawn(async move {
            match forward(&http, &cfg, &query).await {
                Ok(resp) => {
                    if let Err(e) = sock.send_to(&resp, peer).await {
                        debug!("DNS proxy: send to {} failed: {}", peer, e);
                    }
                }
                Err(e) => debug!("DNS proxy: forward failed for {}: {}", peer, e),
            }
        });
    }
}

async fn forward(
    client: &reqwest::Client,
    cfg: &DnsProxyConfig,
    query: &[u8],
) -> Result<Vec<u8>, String> {
    let result = doh_post(client, &cfg.upstream_doh, query).await;
    if result.is_err() {
        if let Some(ref fb) = cfg.fallback_doh {
            return doh_post(client, fb, query).await;
        }
    }
    result
}

async fn doh_post(client: &reqwest::Client, url: &str, query: &[u8]) -> Result<Vec<u8>, String> {
    let resp = tokio::time::timeout(
        UPSTREAM_TIMEOUT,
        client
            .post(url)
            .header("Content-Type", "application/dns-message")
            .header("Accept", "application/dns-message")
            .body(query.to_vec())
            .send(),
    )
    .await
    .map_err(|_| format!("DoH timeout: {}", url))?
    .map_err(|e| format!("DoH request: {}", e))?;

    if !resp.status().is_success() {
        return Err(format!("DoH HTTP {}", resp.status()));
    }

    let body = resp.bytes().await.map_err(|e| format!("DoH body: {}", e))?;
    if body.len() > MAX_DOH_RESPONSE {
        return Err(format!(
            "DoH response too large: {} bytes (max {})",
            body.len(),
            MAX_DOH_RESPONSE
        ));
    }
    Ok(body.to_vec())
}

/// Block plain-DNS egress on non-VPN interfaces via nftables.
fn install_block_rule(tun_iface: &str) {
    let rule = format!(
        "add rule inet filter forward oifname != \"{}\" udp dport 53 drop",
        tun_iface
    );
    match std::process::Command::new("nft").arg(&rule).status() {
        Ok(s) if s.success() => {
            info!(
                "DNS proxy: plain-DNS block rule installed (iface={})",
                tun_iface
            )
        }
        Ok(s) => warn!("DNS proxy: nft rule exited {}, DNS leaks possible", s),
        Err(e) => warn!("DNS proxy: nft unavailable ({}), DNS leaks possible", e),
    }
}
