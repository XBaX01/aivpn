//! Shell-based eBPF TC loader for per-client QoS enforcement.
//!
//! Attaches the pre-compiled `tc_qos_prog.o` eBPF program to a TUN interface
//! via `tc` (traffic control) and updates per-client rules via `bpftool map`.
//!
//! The eBPF TC egress hook marks DSCP on outbound packets and enforces
//! per-client bandwidth limits using a BPF LRU_HASH map `qos_rules`.
//! Falls back gracefully when `tc` or `bpftool` are unavailable.

use std::path::{Path, PathBuf};
use std::process::Command;
use tracing::{debug, warn};

/// eBPF TC loader — attaches a compiled BPF program to a network interface.
pub struct TcQosLoader {
    iface: String,
    prog_path: PathBuf,
    attached: bool,
}

/// Per-client QoS rule stored in the BPF `qos_rules` map.
pub struct TcQosRule {
    /// VPN IPv4 address as big-endian u32
    pub vpn_ip: u32,
    /// Upstream bandwidth limit in bytes/s (0 = unlimited)
    pub rate_up_bps: u64,
    /// Downstream bandwidth limit in bytes/s (0 = unlimited)
    pub rate_down_bps: u64,
    /// DSCP value 0–63 to stamp on outgoing packets
    pub dscp: u8,
}

impl TcQosLoader {
    /// Create a new loader for `iface`, using the compiled BPF object at `prog_path`.
    pub fn new(iface: &str, prog_path: &Path) -> Self {
        Self {
            iface: iface.to_string(),
            prog_path: prog_path.to_path_buf(),
            attached: false,
        }
    }

    /// Attach the BPF TC egress program to the interface.
    ///
    /// Runs:
    /// ```sh
    /// tc qdisc add dev <iface> clsact
    /// tc filter add dev <iface> egress bpf da obj <prog> sec tc
    /// ```
    pub fn attach(&mut self) -> bool {
        if !self.prog_path.exists() {
            warn!(
                "tc_loader: BPF object not found at {} — skipping attach",
                self.prog_path.display()
            );
            return false;
        }

        // Add clsact qdisc (idempotent — ignore error if already present)
        let qdisc = Command::new("tc")
            .args(["qdisc", "add", "dev", &self.iface, "clsact"])
            .output();
        match qdisc {
            Ok(o) if o.status.success() || already_exists(&o.stderr) => {}
            Ok(o) => {
                warn!(
                    "tc_loader: qdisc add failed: {}",
                    String::from_utf8_lossy(&o.stderr).trim()
                );
                return false;
            }
            Err(e) => {
                warn!("tc_loader: tc not available: {}", e);
                return false;
            }
        }

        // Attach BPF program as TC egress filter
        let filter = Command::new("tc")
            .args([
                "filter",
                "add",
                "dev",
                &self.iface,
                "egress",
                "bpf",
                "da",
                "obj",
                self.prog_path.to_str().unwrap_or(""),
                "sec",
                "tc",
            ])
            .output();
        match filter {
            Ok(o) if o.status.success() => {
                debug!("tc_loader: BPF TC filter attached to {}", self.iface);
                self.attached = true;
                true
            }
            Ok(o) => {
                warn!(
                    "tc_loader: filter add failed: {}",
                    String::from_utf8_lossy(&o.stderr).trim()
                );
                false
            }
            Err(e) => {
                warn!("tc_loader: tc filter add error: {}", e);
                false
            }
        }
    }

    /// Update or insert a per-client QoS rule in the BPF `qos_rules` map.
    ///
    /// Value layout (17 bytes): `[rate_up_bps: u64 BE][rate_down_bps: u64 BE][dscp: u8]`
    /// Map pinned at `/sys/fs/bpf/aivpn_qos_rules`.
    pub fn update_rule(&self, rule: &TcQosRule) -> bool {
        if !self.attached {
            return false;
        }

        // Pack value: 8 + 8 + 1 = 17 bytes, hex-encoded for bpftool
        let mut value = [0u8; 17];
        value[0..8].copy_from_slice(&rule.rate_up_bps.to_be_bytes());
        value[8..16].copy_from_slice(&rule.rate_down_bps.to_be_bytes());
        value[16] = rule.dscp;

        let key_hex = format!("{:08x}", rule.vpn_ip);
        let val_hex: String = value.iter().map(|b| format!("{:02x}", b)).collect();

        let out = Command::new("bpftool")
            .args([
                "map",
                "update",
                "pinned",
                "/sys/fs/bpf/aivpn_qos_rules",
                "key",
                "hex",
                &key_hex,
                "value",
                "hex",
                &val_hex,
            ])
            .output();

        match out {
            Ok(o) if o.status.success() => {
                debug!("tc_loader: QoS rule updated for VPN IP {:08x}", rule.vpn_ip);
                true
            }
            Ok(o) => {
                warn!(
                    "tc_loader: bpftool map update failed: {}",
                    String::from_utf8_lossy(&o.stderr).trim()
                );
                false
            }
            Err(e) => {
                warn!("tc_loader: bpftool not available: {}", e);
                false
            }
        }
    }

    /// Remove a client's rule from the BPF map.
    pub fn delete_rule(&self, vpn_ip: u32) -> bool {
        if !self.attached {
            return false;
        }
        let key_hex = format!("{:08x}", vpn_ip);
        let out = Command::new("bpftool")
            .args([
                "map",
                "delete",
                "pinned",
                "/sys/fs/bpf/aivpn_qos_rules",
                "key",
                "hex",
                &key_hex,
            ])
            .output();
        matches!(out, Ok(o) if o.status.success())
    }

    /// Detach the BPF TC program and remove the clsact qdisc.
    pub fn detach(&mut self) {
        if !self.attached {
            return;
        }
        let out = Command::new("tc")
            .args(["qdisc", "del", "dev", &self.iface, "clsact"])
            .output();
        match out {
            Ok(o) if o.status.success() => {
                debug!("tc_loader: clsact qdisc removed from {}", self.iface);
            }
            Ok(o) => {
                warn!(
                    "tc_loader: qdisc del failed: {}",
                    String::from_utf8_lossy(&o.stderr).trim()
                );
            }
            Err(e) => {
                warn!("tc_loader: tc not available during detach: {}", e);
            }
        }
        self.attached = false;
    }

    pub fn is_attached(&self) -> bool {
        self.attached
    }
}

impl Drop for TcQosLoader {
    fn drop(&mut self) {
        self.detach();
    }
}

fn already_exists(stderr: &[u8]) -> bool {
    let msg = String::from_utf8_lossy(stderr);
    msg.contains("already exists") || msg.contains("RTEXIST")
}
