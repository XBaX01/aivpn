//! NAT Forwarder Module
//!
//! Handles TUN device creation, packet forwarding, and NAT masquerading.
//! Detects nftables vs iptables at runtime; manages rules idempotently so
//! that server restarts never accumulate duplicate firewall entries.

use std::io;
use std::sync::Arc;
use tokio::io::AsyncWriteExt;
use tokio::sync::Mutex;
#[cfg(target_os = "linux")]
use tracing::warn;
use tracing::{debug, info};

use aivpn_common::error::{Error, Result};
use aivpn_common::network_config::VpnNetworkConfig;

/// Default server TUN MTU. Conservative enough to avoid fragmentation on
/// most paths (outer UDP ≈ inner + 57 bytes, so outer ≈ 1477 on a 1500 MTU link).
/// Lower to 1380 for tunnelled uplinks (L2TP, VXLAN, GRE).
pub const DEFAULT_TUN_MTU: u16 = 1420;

// ── Firewall backend ───────────────────────────────────────────────────────

#[cfg(target_os = "linux")]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FwBackend {
    Nftables,
    Iptables,
}

/// Detect which firewall backend is available. Prefers nftables.
#[cfg(target_os = "linux")]
fn detect_fw_backend() -> FwBackend {
    use std::process::Command;
    let ok = Command::new("nft")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false);
    if ok {
        FwBackend::Nftables
    } else {
        FwBackend::Iptables
    }
}

/// Add an iptables rule only if an identical rule does not already exist.
#[cfg(target_os = "linux")]
fn ipt_ensure(table: &str, chain: &str, rule: &[&str]) {
    use std::process::Command;
    let mut check: Vec<&str> = vec!["-t", table, "-C", chain];
    check.extend_from_slice(rule);
    let exists = Command::new("iptables")
        .args(&check)
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false);
    if !exists {
        let mut add: Vec<&str> = vec!["-t", table, "-A", chain];
        add.extend_from_slice(rule);
        if let Ok(out) = Command::new("iptables").args(&add).output() {
            if !out.status.success() {
                warn!(
                    "iptables add failed: {}",
                    String::from_utf8_lossy(&out.stderr)
                );
            }
        }
    }
}

/// Delete an iptables rule (best-effort; silently ignores "not found").
#[cfg(target_os = "linux")]
fn ipt_delete(table: &str, chain: &str, rule: &[&str]) {
    use std::process::Command;
    let mut del: Vec<&str> = vec!["-t", table, "-D", chain];
    del.extend_from_slice(rule);
    let _ = Command::new("iptables").args(&del).output();
}

// ── NatForwarder ───────────────────────────────────────────────────────────

/// NAT Forwarder for routing VPN client traffic to the internet.
pub struct NatForwarder {
    tun_name: String,
    tun_addr: String,
    tun_netmask: String,
    tun_mtu: u16,
    network_config: VpnNetworkConfig,
    writer: Option<Arc<Mutex<tun::DeviceWriter>>>,
    writer_taken: Option<Mutex<Option<tun::DeviceWriter>>>,
    reader: Option<Mutex<Option<tun::DeviceReader>>>,
    #[cfg(target_os = "linux")]
    fw_backend: Option<FwBackend>,
}

impl NatForwarder {
    pub fn new(
        tun_name: &str,
        tun_addr: &str,
        tun_netmask: &str,
        tun_mtu: u16,
        network_config: VpnNetworkConfig,
    ) -> Result<Self> {
        Ok(Self {
            tun_name: tun_name.to_string(),
            tun_addr: tun_addr.to_string(),
            tun_netmask: tun_netmask.to_string(),
            tun_mtu,
            network_config,
            writer: None,
            writer_taken: None,
            reader: None,
            #[cfg(target_os = "linux")]
            fw_backend: None,
        })
    }

    /// Create TUN device and install firewall rules.
    pub fn create(&mut self) -> Result<()> {
        let mut config = tun::Configuration::default();

        config
            .tun_name(&self.tun_name)
            .address(&self.tun_addr)
            .netmask(&self.tun_netmask)
            .mtu(self.tun_mtu)
            .up();

        #[cfg(target_os = "linux")]
        config.platform_config(|config| {
            config.ensure_root_privileges(true);
        });

        let dev = tun::create_as_async(&config)
            .map_err(|e| Error::Io(io::Error::new(io::ErrorKind::Other, e.to_string())))?;

        let (writer, reader) = dev
            .split()
            .map_err(|e| Error::Io(io::Error::new(io::ErrorKind::Other, e.to_string())))?;
        self.writer_taken = Some(Mutex::new(Some(writer)));
        self.reader = Some(Mutex::new(Some(reader)));

        info!(
            "Created NAT TUN device: {} ({}/{}, subnet {}, mtu {})",
            self.tun_name,
            self.tun_addr,
            self.tun_netmask,
            self.network_config.cidr_string(),
            self.tun_mtu,
        );

        #[cfg(target_os = "linux")]
        {
            self.enable_ip_forwarding()?;
            let backend = detect_fw_backend();
            info!("Firewall backend: {:?}", backend);
            match backend {
                FwBackend::Nftables => self.setup_nftables()?,
                FwBackend::Iptables => self.setup_iptables()?,
            }
            self.fw_backend = Some(backend);
        }

        Ok(())
    }

    /// Enable IPv4 forwarding (idempotent — checks before writing).
    #[cfg(target_os = "linux")]
    fn enable_ip_forwarding(&self) -> Result<()> {
        use std::fs::{read_to_string, write};

        if let Ok(val) = read_to_string("/proc/sys/net/ipv4/ip_forward") {
            if val.trim() == "1" {
                info!("IPv4 forwarding already enabled");
                return Ok(());
            }
        }
        write("/proc/sys/net/ipv4/ip_forward", "1").map_err(|e| {
            Error::Io(io::Error::new(
                io::ErrorKind::PermissionDenied,
                format!("Failed to enable IP forwarding: {e}"),
            ))
        })?;
        info!("Enabled IPv4 forwarding");
        Ok(())
    }

    /// Install nftables rules inside a dedicated `aivpn` table.
    ///
    /// The table is deleted first (if it exists from a previous run) and then
    /// recreated atomically so there are never duplicate rules.
    #[cfg(target_os = "linux")]
    fn setup_nftables(&self) -> Result<()> {
        use std::io::Write as _;
        use std::process::{Command, Stdio};

        let cidr = self.network_config.cidr_string();
        let tun = &self.tun_name;

        // Remove stale table from a previous server run (idempotent).
        let _ = Command::new("nft")
            .args(["delete", "table", "ip", "aivpn"])
            .output();

        // Build and apply the full ruleset atomically via `nft -f -`.
        let ruleset = format!(
            "table ip aivpn {{\n\
             \tchain nat_out {{\n\
             \t\ttype nat hook postrouting priority srcnat;\n\
             \t\tip saddr {cidr} masquerade\n\
             \t}}\n\
             \tchain forward {{\n\
             \t\ttype filter hook forward priority filter;\n\
             \t\tiifname \"{tun}\" accept\n\
             \t\toifname \"{tun}\" ct state related,established accept\n\
             \t\toifname \"{tun}\" tcp flags syn / syn,rst tcp option maxseg size set rt mtu\n\
             \t\tiifname \"{tun}\" tcp flags syn / syn,rst tcp option maxseg size set rt mtu\n\
             \t}}\n\
             }}\n"
        );

        let mut child = Command::new("nft")
            .args(["-f", "-"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| {
                Error::Io(io::Error::new(
                    io::ErrorKind::Other,
                    format!("nft spawn: {e}"),
                ))
            })?;

        if let Some(stdin) = child.stdin.as_mut() {
            let _ = stdin.write_all(ruleset.as_bytes());
        }

        let out = child.wait_with_output().map_err(|e| {
            Error::Io(io::Error::new(
                io::ErrorKind::Other,
                format!("nft wait: {e}"),
            ))
        })?;

        if out.status.success() {
            info!("nftables: aivpn table installed (NAT + forward + MSS clamp)");
        } else {
            warn!(
                "nftables setup failed: {}",
                String::from_utf8_lossy(&out.stderr)
            );
        }

        Ok(())
    }

    /// Install iptables rules with check-before-add idempotency.
    ///
    /// Each rule is tagged with `-m comment --comment aivpn` so that Drop
    /// can remove only our rules without touching any pre-existing ones.
    #[cfg(target_os = "linux")]
    fn setup_iptables(&self) -> Result<()> {
        let cidr = self.network_config.cidr_string();
        let tun = &self.tun_name;
        let c = "aivpn";

        ipt_ensure(
            "nat",
            "POSTROUTING",
            &[
                "-s",
                &cidr,
                "-m",
                "comment",
                "--comment",
                c,
                "-j",
                "MASQUERADE",
            ],
        );
        ipt_ensure(
            "filter",
            "FORWARD",
            &["-i", tun, "-m", "comment", "--comment", c, "-j", "ACCEPT"],
        );
        ipt_ensure(
            "filter",
            "FORWARD",
            &[
                "-o",
                tun,
                "-m",
                "state",
                "--state",
                "RELATED,ESTABLISHED",
                "-m",
                "comment",
                "--comment",
                c,
                "-j",
                "ACCEPT",
            ],
        );
        ipt_ensure(
            "mangle",
            "FORWARD",
            &[
                "-o",
                tun,
                "-p",
                "tcp",
                "--tcp-flags",
                "SYN,RST",
                "SYN",
                "-m",
                "comment",
                "--comment",
                c,
                "-j",
                "TCPMSS",
                "--clamp-mss-to-pmtu",
            ],
        );
        ipt_ensure(
            "mangle",
            "FORWARD",
            &[
                "-i",
                tun,
                "-p",
                "tcp",
                "--tcp-flags",
                "SYN,RST",
                "SYN",
                "-m",
                "comment",
                "--comment",
                c,
                "-j",
                "TCPMSS",
                "--clamp-mss-to-pmtu",
            ],
        );

        info!("iptables: aivpn NAT + forward + MSS clamp rules installed");
        Ok(())
    }

    /// Forward a packet to the TUN interface (write path).
    pub async fn forward_packet(&self, packet: &[u8]) -> Result<()> {
        let writer = self.writer.as_ref().ok_or_else(|| {
            Error::Io(io::Error::new(
                io::ErrorKind::NotConnected,
                "TUN device not created",
            ))
        })?;

        let mut w = writer.lock().await;
        w.write_all(packet).await?;

        debug!("Forwarded {} bytes to TUN", packet.len());
        Ok(())
    }

    /// Take ownership of the TUN writer (for use in a dedicated writer task).
    pub async fn take_writer(&self) -> Option<tun::DeviceWriter> {
        if let Some(ref lock) = self.writer_taken {
            lock.lock().await.take()
        } else {
            None
        }
    }

    /// Take ownership of the TUN reader (for use in a spawned reader task).
    pub async fn take_reader(&self) -> Option<tun::DeviceReader> {
        if let Some(reader_lock) = &self.reader {
            reader_lock.lock().await.take()
        } else {
            None
        }
    }

    /// Return the TUN device name.
    pub fn tun_name(&self) -> &str {
        &self.tun_name
    }
}

impl Drop for NatForwarder {
    fn drop(&mut self) {
        if self.writer.is_some() {
            info!("Closing NAT TUN device: {}", self.tun_name);
        }

        #[cfg(target_os = "linux")]
        {
            match self.fw_backend {
                Some(FwBackend::Nftables) => {
                    use std::process::Command;
                    let _ = Command::new("nft")
                        .args(["delete", "table", "ip", "aivpn"])
                        .output();
                    info!("nftables: aivpn table removed");
                }
                Some(FwBackend::Iptables) => {
                    let cidr = self.network_config.cidr_string();
                    let tun = &self.tun_name;
                    let c = "aivpn";

                    ipt_delete(
                        "nat",
                        "POSTROUTING",
                        &[
                            "-s",
                            &cidr,
                            "-m",
                            "comment",
                            "--comment",
                            c,
                            "-j",
                            "MASQUERADE",
                        ],
                    );
                    ipt_delete(
                        "filter",
                        "FORWARD",
                        &["-i", tun, "-m", "comment", "--comment", c, "-j", "ACCEPT"],
                    );
                    ipt_delete(
                        "filter",
                        "FORWARD",
                        &[
                            "-o",
                            tun,
                            "-m",
                            "state",
                            "--state",
                            "RELATED,ESTABLISHED",
                            "-m",
                            "comment",
                            "--comment",
                            c,
                            "-j",
                            "ACCEPT",
                        ],
                    );
                    ipt_delete(
                        "mangle",
                        "FORWARD",
                        &[
                            "-o",
                            tun,
                            "-p",
                            "tcp",
                            "--tcp-flags",
                            "SYN,RST",
                            "SYN",
                            "-m",
                            "comment",
                            "--comment",
                            c,
                            "-j",
                            "TCPMSS",
                            "--clamp-mss-to-pmtu",
                        ],
                    );
                    ipt_delete(
                        "mangle",
                        "FORWARD",
                        &[
                            "-i",
                            tun,
                            "-p",
                            "tcp",
                            "--tcp-flags",
                            "SYN,RST",
                            "SYN",
                            "-m",
                            "comment",
                            "--comment",
                            c,
                            "-j",
                            "TCPMSS",
                            "--clamp-mss-to-pmtu",
                        ],
                    );
                    info!("iptables: aivpn rules removed");
                }
                None => {}
            }
        }
    }
}
