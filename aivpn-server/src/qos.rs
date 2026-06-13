//! Per-client QoS — userspace token bucket rate limiting and DSCP marking.

use dashmap::DashMap;
use std::sync::Arc;
use std::time::Instant;

/// QoS settings for a single client (persisted in clients.json).
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct ClientQos {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bandwidth_limit_up: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bandwidth_limit_down: Option<u64>,
    /// DSCP value 0–63 applied to outgoing TUN packets.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dscp_class: Option<u8>,
    /// Priority hint: 0 = default, 1 = high, 2 = low.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub priority: Option<u8>,
}

struct TokenBucket {
    capacity: u64,
    tokens: f64,
    rate_bps: u64,
    last: Instant,
}

impl TokenBucket {
    fn new(rate_bps: u64) -> Self {
        let capacity = (rate_bps as f64 * 0.1).max(1500.0) as u64;
        Self {
            capacity,
            tokens: capacity as f64,
            rate_bps,
            last: Instant::now(),
        }
    }

    fn try_consume(&mut self, bytes: u64) -> bool {
        let now = Instant::now();
        let elapsed = now.duration_since(self.last).as_secs_f64();
        self.last = now;
        self.tokens = (self.tokens + elapsed * self.rate_bps as f64).min(self.capacity as f64);
        if self.tokens >= bytes as f64 {
            self.tokens -= bytes as f64;
            true
        } else {
            false
        }
    }
}

struct ClientBuckets {
    up: Option<TokenBucket>,
    down: Option<TokenBucket>,
    dscp: Option<u8>,
}

/// Thread-safe QoS enforcer, shared across gateway tasks.
#[derive(Clone)]
pub struct QosEnforcer {
    buckets: Arc<DashMap<String, parking_lot::Mutex<ClientBuckets>>>,
}

impl QosEnforcer {
    pub fn new() -> Self {
        Self {
            buckets: Arc::new(DashMap::new()),
        }
    }

    pub fn set_client(&self, client_id: &str, qos: &ClientQos) {
        let entry = ClientBuckets {
            up: qos.bandwidth_limit_up.map(TokenBucket::new),
            down: qos.bandwidth_limit_down.map(TokenBucket::new),
            dscp: qos.dscp_class,
        };
        self.buckets
            .insert(client_id.to_string(), parking_lot::Mutex::new(entry));
    }

    pub fn remove_client(&self, client_id: &str) {
        self.buckets.remove(client_id);
    }

    /// Returns `true` if the packet should be forwarded (upstream: client→server).
    pub fn check_upstream(&self, client_id: &str, bytes: u64) -> bool {
        if let Some(entry) = self.buckets.get(client_id) {
            let mut b = entry.lock();
            if let Some(ref mut bucket) = b.up {
                return bucket.try_consume(bytes);
            }
        }
        true
    }

    /// Returns `true` if the packet should be forwarded (downstream: server→client).
    pub fn check_downstream(&self, client_id: &str, bytes: u64) -> bool {
        if let Some(entry) = self.buckets.get(client_id) {
            let mut b = entry.lock();
            if let Some(ref mut bucket) = b.down {
                return bucket.try_consume(bytes);
            }
        }
        true
    }

    pub fn get_dscp(&self, client_id: &str) -> Option<u8> {
        self.buckets.get(client_id).and_then(|e| e.lock().dscp)
    }
}

impl Default for QosEnforcer {
    fn default() -> Self {
        Self::new()
    }
}

/// Apply DSCP to an IPv4 packet payload (modifies TOS byte in-place).
pub fn apply_dscp_ipv4(pkt: &mut [u8], dscp: u8) -> bool {
    if pkt.len() < 20 {
        return false;
    }
    let ecn = pkt[1] & 0x03;
    pkt[1] = (dscp << 2) | ecn;
    pkt[10] = 0;
    pkt[11] = 0;
    let sum = ipv4_checksum(&pkt[..20]);
    pkt[10] = (sum >> 8) as u8;
    pkt[11] = (sum & 0xff) as u8;
    true
}

fn ipv4_checksum(header: &[u8]) -> u16 {
    let mut sum: u32 = 0;
    for i in (0..header.len()).step_by(2) {
        let word = (header[i] as u32) << 8 | header[i + 1] as u32;
        sum = sum.wrapping_add(word);
    }
    while sum >> 16 != 0 {
        sum = (sum & 0xffff) + (sum >> 16);
    }
    !(sum as u16)
}

/// Parse "10M", "500K", "2G" → bytes/sec.
pub fn parse_bandwidth(s: &str) -> Option<u64> {
    let s = s.trim();
    let (num, mul) = if let Some(r) = s.strip_suffix('G').or_else(|| s.strip_suffix('g')) {
        (r, 1_000_000_000u64)
    } else if let Some(r) = s.strip_suffix('M').or_else(|| s.strip_suffix('m')) {
        (r, 1_000_000u64)
    } else if let Some(r) = s.strip_suffix('K').or_else(|| s.strip_suffix('k')) {
        (r, 1_000u64)
    } else {
        (s, 1u64)
    };
    num.parse::<f64>().ok().map(|n| (n * mul as f64) as u64)
}

/// DSCP class name → numeric value.
pub fn dscp_by_name(name: &str) -> Option<u8> {
    match name.to_uppercase().as_str() {
        "DEFAULT" | "BE" => Some(0),
        "AF11" => Some(10),
        "AF12" => Some(12),
        "AF13" => Some(14),
        "AF21" => Some(18),
        "AF22" => Some(20),
        "AF23" => Some(22),
        "AF31" => Some(26),
        "AF32" => Some(28),
        "AF33" => Some(30),
        "AF41" => Some(34),
        "AF42" => Some(36),
        "AF43" => Some(38),
        "EF" => Some(46),
        "CS1" => Some(8),
        "CS2" => Some(16),
        "CS3" => Some(24),
        "CS4" => Some(32),
        "CS5" => Some(40),
        "CS6" => Some(48),
        "CS7" => Some(56),
        _ => name.parse::<u8>().ok(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn token_bucket_allows_within_capacity() {
        let mut b = TokenBucket::new(1_000_000);
        assert!(b.try_consume(100));
    }

    #[test]
    fn parse_bandwidth_units() {
        assert_eq!(parse_bandwidth("10M"), Some(10_000_000));
        assert_eq!(parse_bandwidth("500K"), Some(500_000));
        assert_eq!(parse_bandwidth("1G"), Some(1_000_000_000));
    }

    #[test]
    fn dscp_by_name_known() {
        assert_eq!(dscp_by_name("EF"), Some(46));
        assert_eq!(dscp_by_name("AF11"), Some(10));
        assert_eq!(dscp_by_name("46"), Some(46));
    }

    #[test]
    fn apply_dscp_sets_tos() {
        let mut pkt = vec![0u8; 20];
        pkt[0] = 0x45;
        apply_dscp_ipv4(&mut pkt, 46);
        assert_eq!(pkt[1] >> 2, 46);
    }
}
