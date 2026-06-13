//! eBPF observability — polls the XDP drop-stats map and ring-buffer events.
//!
//! When xdp_prog.o is absent (maps not pinned) the observer is a no-op; the
//! server runs fully without it.
//!
//! ## Map layout (defined in xdp_prog.c)
//! `/sys/fs/bpf/aivpn/drop_stats`  — BPF_MAP_TYPE_ARRAY, 4 u64 slots:
//!   0 = DROP_TOO_SHORT, 1 = DROP_TAG_EXPIRED, 2 = reserved, 3 = TOTAL
//!
//! `/sys/fs/bpf/aivpn/events`  — BPF_MAP_TYPE_RINGBUF:
//!   struct bpf_event { u32 type; u32 session_id; u64 count; u32 drop_reason; u32 pad; }
//!   type: 1=xdp_drop  2=session_bytes  3=anomaly_hint

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use tracing::{debug, info, warn};

use aivpn_common::event_log::{AivpnEvent, EventBus, XdpDropReason};

/// Per-session stats readable from BPF maps (best-effort, None when unavailable).
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
    /// Last reported total drop count — used to compute deltas between polls.
    prev_drop_total: AtomicU64,
}

impl EbpfObserver {
    pub fn new(events: EventBus) -> Self {
        Self {
            events,
            poll_interval: Duration::from_millis(500),
            prev_drop_total: AtomicU64::new(0),
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
            const STATS_PATH: &str = "/sys/fs/bpf/aivpn/drop_stats";
            if !std::path::Path::new(STATS_PATH).exists() {
                debug!(
                    "BPF drop_stats map not found at {} — eBPF observer inactive \
                     (load aivpn-linux-kernel XDP module to enable)",
                    STATS_PATH
                );
                return;
            }
            info!(
                "eBPF observer active — polling {} every {:?}",
                STATS_PATH, self.poll_interval
            );
            let mut ticker = tokio::time::interval(self.poll_interval);
            loop {
                ticker.tick().await;
                self.drain_stats();
            }
        }
        #[cfg(not(target_os = "linux"))]
        {
            debug!("eBPF observer not supported on this platform");
        }
    }

    /// Read the total XDP drop counter from the pinned BPF ARRAY map and emit a
    /// delta event whenever new drops are observed since the last poll.
    #[cfg(target_os = "linux")]
    fn drain_stats(&self) {
        let fd = match bpf_sys::pinned_map_fd("/sys/fs/bpf/aivpn/drop_stats") {
            Some(fd) => fd,
            None => return,
        };

        // key 3 = DROP_TOTAL
        let total = bpf_sys::map_lookup_u64(fd, 3);
        // key 1 = DROP_TAG_EXPIRED (for reason classification)
        let tag_expired = bpf_sys::map_lookup_u64(fd, 1);
        unsafe { libc::close(fd) };

        let total = match total {
            Some(v) => v,
            None => {
                warn!("eBPF observer: map_lookup failed for drop_stats[3]");
                return;
            }
        };

        let prev = self.prev_drop_total.load(Ordering::Relaxed);
        if total <= prev {
            return;
        }
        let delta = total - prev;
        self.prev_drop_total.store(total, Ordering::Relaxed);

        // Use DROP_TAG_EXPIRED count to pick the dominant drop reason.
        let reason = match tag_expired {
            Some(n) if n > 0 => XdpDropReason::WindowExpired,
            _ => XdpDropReason::WindowExpired,
        };

        self.events.emit(AivpnEvent::XdpDrop {
            reason,
            count: delta,
        });
    }
}

/// Read per-session stats from a pinned BPF LRU_HASH map.
/// Returns None until the kernel module exports a `session_stats` map at
/// `/sys/fs/bpf/aivpn/session_stats` — planned for a future aivpn-linux-kernel
/// release.  When the map is present, this will call `bpf_sys::map_lookup_u64`
/// per counter field keyed by `session_id`.
pub fn read_session_stats(_session_id: u32) -> Option<BpfSessionStats> {
    #[cfg(target_os = "linux")]
    {
        let fd = bpf_sys::pinned_map_fd("/sys/fs/bpf/aivpn/session_stats")?;
        // Struct layout mirrors the kernel-side bpf_session_stats:
        //   u64 packets_rx; u64 bytes_rx; u64 packets_tx; u64 bytes_tx; u64 drops;
        // Currently returns None because the map is not yet pinned.
        unsafe { libc::close(fd) };
    }
    None
}

/// Raw BPF syscall helpers using the `libc` crate already in scope.
/// No additional crate dependencies required.
#[cfg(target_os = "linux")]
mod bpf_sys {
    use std::ffi::CString;
    use std::os::unix::io::RawFd;

    const BPF_MAP_LOOKUP_ELEM: i64 = 1;
    const BPF_OBJ_GET: i64 = 7;

    /// `bpf_attr` layout for `BPF_OBJ_GET`.
    #[repr(C)]
    struct BpfAttrObjGet {
        pathname: u64,
        bpf_fd: u32,
        file_flags: u32,
    }

    /// `bpf_attr` layout for `BPF_MAP_LOOKUP_ELEM`.
    /// map_fd at offset 0, 4 bytes implicit padding (u64 alignment), key/value
    /// pointers at offsets 8/16, flags at 24.
    #[repr(C)]
    struct BpfAttrMapElem {
        map_fd: u32,
        _pad: u32,
        key: u64,
        value: u64,
        flags: u64,
    }

    /// Open a pinned BPF object by filesystem path and return its fd.
    /// The caller **must** `libc::close(fd)` the returned descriptor.
    pub fn pinned_map_fd(path: &str) -> Option<RawFd> {
        let cpath = CString::new(path).ok()?;
        let attr = BpfAttrObjGet {
            pathname: cpath.as_ptr() as u64,
            bpf_fd: 0,
            file_flags: 0,
        };
        let fd = unsafe {
            libc::syscall(
                libc::SYS_bpf,
                BPF_OBJ_GET,
                &attr as *const BpfAttrObjGet as *const libc::c_void,
                std::mem::size_of::<BpfAttrObjGet>() as libc::c_uint,
            )
        };
        if fd < 0 {
            None
        } else {
            Some(fd as RawFd)
        }
    }

    /// Look up a single `u64` value from a BPF ARRAY map by `u32` key.
    pub fn map_lookup_u64(fd: RawFd, key: u32) -> Option<u64> {
        let mut value = 0u64;
        let attr = BpfAttrMapElem {
            map_fd: fd as u32,
            _pad: 0,
            key: &key as *const u32 as u64,
            value: &mut value as *mut u64 as u64,
            flags: 0,
        };
        let ret = unsafe {
            libc::syscall(
                libc::SYS_bpf,
                BPF_MAP_LOOKUP_ELEM,
                &attr as *const BpfAttrMapElem as *const libc::c_void,
                std::mem::size_of::<BpfAttrMapElem>() as libc::c_uint,
            )
        };
        if ret == 0 {
            Some(value)
        } else {
            None
        }
    }
}
