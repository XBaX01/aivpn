//! eBPF observability — polls the XDP/TC ring buffer and exports stats.
//!
//! When xdp_prog.o is absent the observer is a no-op; the server runs fully.
//! Ring buffer protocol (defined alongside xdp_prog.c):
//!   struct bpf_event { u32 type; u32 session_id; u64 count; u32 drop_reason; };
//!   type: 1=xdp_drop  2=session_bytes  3=anomaly_hint

use std::sync::Arc;
use std::time::Duration;

use tracing::{debug, info};

use aivpn_common::event_log::{AivpnEvent, EventBus, XdpDropReason};

/// Per-session stats read from the BPF LRU hash map.
#[derive(Debug, Default, Clone)]
pub struct BpfSessionStats {
    pub packets_rx: u64,
    pub bytes_rx: u64,
    pub packets_tx: u64,
    pub bytes_tx: u64,
    pub drops: u64,
}

pub struct EbpfObserver {
    events: EventBus,
    poll_interval: Duration,
}

impl EbpfObserver {
    pub fn new(events: EventBus) -> Self {
        Self {
            events,
            poll_interval: Duration::from_millis(100),
        }
    }

    /// Spawn background polling task.  No-op if BPF maps are unavailable.
    pub fn start(self: Arc<Self>) {
        tokio::spawn(async move {
            self.run_loop().await;
        });
    }

    async fn run_loop(&self) {
        #[cfg(target_os = "linux")]
        {
            let rb_path = "/sys/fs/bpf/aivpn_events";
            if !std::path::Path::new(rb_path).exists() {
                debug!(
                    "eBPF ring buffer not found at {} — observer inactive",
                    rb_path
                );
                return;
            }
            info!(
                "eBPF observer active (poll interval {:?})",
                self.poll_interval
            );
            loop {
                self.drain_stats().await;
                tokio::time::sleep(self.poll_interval).await;
            }
        }
        #[cfg(not(target_os = "linux"))]
        {
            debug!("eBPF observer not supported on this platform");
        }
    }

    #[cfg(target_os = "linux")]
    async fn drain_stats(&self) {
        // Read cumulative XDP drop counter exported via BPF map → procfs bridge.
        // A full libbpf ring_buffer__poll() integration requires a C extension;
        // that is wired in when xdp_prog.o is loaded via kernel_accel.
        let stats_path = "/sys/fs/bpf/aivpn_drop_count";
        if let Ok(content) = std::fs::read_to_string(stats_path) {
            if let Ok(count) = content.trim().parse::<u64>() {
                if count > 0 {
                    self.events.emit(AivpnEvent::XdpDrop {
                        reason: XdpDropReason::WindowExpired,
                        count,
                    });
                }
            }
        }
    }
}

/// Read per-session stats from BPF LRU hash map.
/// Returns None when BPF is unavailable (graceful degradation).
pub fn read_session_stats(_session_id: u32) -> Option<BpfSessionStats> {
    // Placeholder — real impl calls bpf_map_lookup_elem() via fd from kernel_accel.
    None
}
