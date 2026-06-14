//! Structured event logging — shared event types for server and client.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::io::Write;
use std::sync::{Arc, Mutex};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AivpnEvent {
    Connection {
        client_id: String,
        vpn_ip: String,
        action: ConnectionAction,
        server_node: Option<String>,
    },
    MaskRotation {
        old_mask: String,
        new_mask: String,
        reason: RotationReason,
    },
    KillSwitch {
        action: KillSwitchAction,
        platform: String,
    },
    Anomaly {
        mse: f32,
        threshold: f32,
        mask_id: String,
    },
    XdpDrop {
        reason: XdpDropReason,
        count: u64,
    },
    PeerSync {
        peer: String,
        action: PeerSyncAction,
        clients_synced: u32,
    },
    Bench {
        latency_p50_ms: f64,
        latency_p95_ms: f64,
        throughput_up_mbps: f64,
        throughput_down_mbps: f64,
        packet_loss_pct: f64,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConnectionAction {
    Connect,
    Disconnect,
    Failover {
        from_server: String,
        to_server: String,
    },
    Reconnect,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RotationReason {
    Neural,
    Manual,
    Scheduled,
    PeerSync,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum KillSwitchAction {
    Enabled,
    Disabled,
    Cleared,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum XdpDropReason {
    TooShort,
    WindowExpired,
    Malformed,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PeerSyncAction {
    Connected,
    Disconnected,
    FullSync,
    Delta,
}

/// Wrapper carrying an event with a UTC timestamp.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LoggedEvent {
    pub ts: DateTime<Utc>,
    #[serde(flatten)]
    pub event: AivpnEvent,
}

/// Sink configuration parsed from environment.
#[derive(Debug, Clone)]
pub struct EventSinkConfig {
    pub stdout: bool,
    pub webhook_url: Option<String>,
}

impl Default for EventSinkConfig {
    fn default() -> Self {
        Self {
            stdout: true,
            webhook_url: None,
        }
    }
}

impl EventSinkConfig {
    pub fn from_env() -> Self {
        Self {
            stdout: true,
            webhook_url: std::env::var("AIVPN_EVENT_WEBHOOK").ok(),
        }
    }
}

/// Thread-safe event bus.  Clone-cheap (Arc inside).
#[derive(Clone)]
pub struct EventBus {
    inner: Arc<EventBusInner>,
}

struct EventBusInner {
    stdout: bool,
    stdout_lock: Mutex<()>,
}

impl EventBus {
    pub fn new(cfg: EventSinkConfig) -> Self {
        Self {
            inner: Arc::new(EventBusInner {
                stdout: cfg.stdout,
                stdout_lock: Mutex::new(()),
            }),
        }
    }

    pub fn disabled() -> Self {
        Self::new(EventSinkConfig {
            stdout: false,
            webhook_url: None,
        })
    }

    pub fn emit(&self, event: AivpnEvent) {
        if !self.inner.stdout {
            return;
        }
        let logged = LoggedEvent {
            ts: Utc::now(),
            event,
        };
        if let Ok(line) = serde_json::to_string(&logged) {
            let _guard = self.inner.stdout_lock.lock();
            let _ = writeln!(std::io::stdout(), "{}", line);
        }
    }
}
