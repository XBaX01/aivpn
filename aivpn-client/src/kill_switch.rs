//! Kill-switch and leak-protection for all platforms.
//!
//! When active, all outbound traffic is blocked except:
//!   - traffic on the VPN TUN interface
//!   - traffic to the physical VPN server IP (so the tunnel stays alive)
//!   - loopback traffic
//!
//! Rules are intentionally NOT removed on unexpected process death (SIGKILL),
//! keeping the user protected until they explicitly run `kill-switch clear`.

use aivpn_common::error::{Error, Result};
use std::io;
use tracing::info;
#[allow(unused_imports)]
use tracing::warn;

pub struct KillSwitch {
    tun_name: String,
    server_ip: String,
    active: bool,
}

impl KillSwitch {
    pub fn new(tun_name: String, server_ip: String) -> Self {
        Self {
            tun_name,
            server_ip,
            active: false,
        }
    }

    pub fn is_active(&self) -> bool {
        self.active
    }

    /// Activate kill-switch: block all traffic except VPN tunnel + server bypass.
    pub fn activate(&mut self) -> Result<()> {
        if self.active {
            return Ok(());
        }
        self.activate_impl()?;
        self.active = true;
        info!("Kill-switch activated — non-VPN traffic blocked");
        Ok(())
    }

    /// Remove kill-switch rules (called on graceful disconnect).
    pub fn deactivate(&mut self) {
        if !self.active {
            return;
        }
        self.deactivate_impl();
        self.active = false;
        info!("Kill-switch deactivated");
    }

    /// Remove any stale rules left by a previous session (e.g. after SIGKILL).
    /// Safe to call when no rules are present.
    pub fn clear_stale() {
        Self::clear_stale_impl();
        info!("Kill-switch stale rules cleared");
    }

    // ──────────────────── Linux ────────────────────

    #[cfg(target_os = "linux")]
    fn activate_impl(&self) -> Result<()> {
        use std::process::Command;

        // Try nftables first
        if Command::new("nft")
            .arg("--version")
            .status()
            .map(|s| s.success())
            .unwrap_or(false)
        {
            let ok = Command::new("nft")
                .args(["add", "table", "inet", "aivpn_ks"])
                .status()
                .map(|s| s.success())
                .unwrap_or(false);
            if !ok {
                return Err(Error::Io(io::Error::new(
                    io::ErrorKind::Other,
                    "kill-switch: nft failed to create aivpn_ks table",
                )));
            }
            let chain_spec = "{ type filter hook output priority 0 ; policy drop ; }";
            Command::new("nft")
                .args(["add", "chain", "inet", "aivpn_ks", "output", chain_spec])
                .status()
                .ok();
            for rule in &[
                vec![
                    "add", "rule", "inet", "aivpn_ks", "output", "oifname", "lo", "accept",
                ],
                vec![
                    "add",
                    "rule",
                    "inet",
                    "aivpn_ks",
                    "output",
                    "oifname",
                    self.tun_name.as_str(),
                    "accept",
                ],
                // Server IP bypass — pass as distinct argv, never through a shell
                vec![
                    "add",
                    "rule",
                    "inet",
                    "aivpn_ks",
                    "output",
                    "ip",
                    "daddr",
                    self.server_ip.as_str(),
                    "accept",
                ],
            ] {
                Command::new("nft").args(rule.as_slice()).status().ok();
            }
            return Ok(());
        }

        // Fallback: iptables
        let tun = self.tun_name.as_str();
        let sip = self.server_ip.as_str();
        for cmd in &[
            vec!["iptables", "-N", "AIVPN_KS"],
            vec!["iptables", "-F", "AIVPN_KS"],
            vec!["iptables", "-D", "OUTPUT", "-j", "AIVPN_KS"],
            vec!["iptables", "-I", "OUTPUT", "1", "-j", "AIVPN_KS"],
            vec!["iptables", "-A", "AIVPN_KS", "-o", "lo", "-j", "ACCEPT"],
            vec!["iptables", "-A", "AIVPN_KS", "-o", tun, "-j", "ACCEPT"],
            vec!["iptables", "-A", "AIVPN_KS", "-d", sip, "-j", "ACCEPT"],
            vec!["iptables", "-A", "AIVPN_KS", "-j", "DROP"],
        ] {
            let _ = Command::new(cmd[0]).args(&cmd[1..]).status();
        }
        Ok(())
    }

    #[cfg(target_os = "linux")]
    fn deactivate_impl(&self) {
        use std::process::Command;
        // Try nftables
        if Command::new("sh")
            .args(["-c", "nft list table inet aivpn_ks 2>/dev/null"])
            .status()
            .map(|s| s.success())
            .unwrap_or(false)
        {
            let _ = Command::new("nft")
                .args(["delete", "table", "inet", "aivpn_ks"])
                .status();
            return;
        }
        // Fallback: iptables
        let _ = Command::new("iptables")
            .args(["-D", "OUTPUT", "-j", "AIVPN_KS"])
            .status();
        let _ = Command::new("iptables").args(["-F", "AIVPN_KS"]).status();
        let _ = Command::new("iptables").args(["-X", "AIVPN_KS"]).status();
    }

    #[cfg(target_os = "linux")]
    fn clear_stale_impl() {
        use std::process::Command;
        let _ = Command::new("nft")
            .args(["delete", "table", "inet", "aivpn_ks"])
            .status();
        let _ = Command::new("iptables")
            .args(["-D", "OUTPUT", "-j", "AIVPN_KS"])
            .status();
        let _ = Command::new("iptables").args(["-F", "AIVPN_KS"]).status();
        let _ = Command::new("iptables").args(["-X", "AIVPN_KS"]).status();
    }

    // ──────────────────── macOS ────────────────────

    #[cfg(target_os = "macos")]
    fn activate_impl(&self) -> Result<()> {
        use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
        use std::process::Command;

        let rules = format!(
            "block out all\npass out on lo0 all\npass out on {tun} all\npass out proto {{tcp,udp}} from any to {sip}\n",
            tun = self.tun_name,
            sip = self.server_ip,
        );

        // Write anchor rules to a root-only directory, not world-writable /tmp.
        // O_NOFOLLOW ensures we fail if the path is a symlink (symlink attack prevention).
        let run_dir = "/var/run/aivpn";
        std::fs::DirBuilder::new()
            .mode(0o700)
            .recursive(true)
            .create(run_dir)
            .map_err(|e| {
                Error::Io(io::Error::new(
                    io::ErrorKind::Other,
                    format!("kill-switch: failed to create {}: {}", run_dir, e),
                ))
            })?;

        let anchor_file = "/var/run/aivpn/aivpn_ks.conf";
        let _ = std::fs::remove_file(anchor_file); // remove stale file; ignore error
        std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW)
            .open(anchor_file)
            .and_then(|mut f| {
                use std::io::Write;
                f.write_all(rules.as_bytes())
            })
            .map_err(|e| {
                Error::Io(io::Error::new(
                    io::ErrorKind::Other,
                    format!("kill-switch: failed to write pf rules: {}", e),
                ))
            })?;

        let load = Command::new("pfctl")
            .args(["-a", "aivpn_ks", "-f", anchor_file])
            .status()
            .map_err(Error::Io)?;
        if !load.success() {
            let _ = std::fs::remove_file(anchor_file);
            return Err(Error::Io(io::Error::new(
                io::ErrorKind::Other,
                "kill-switch: pfctl failed to load anchor",
            )));
        }

        // Enable pf if not already running (best-effort)
        let _ = Command::new("pfctl").args(["-e"]).status();

        // Inject anchor reference into running pf config if not present
        let already = Command::new("sh")
            .args(["-c", "pfctl -s all 2>/dev/null | grep -q aivpn_ks"])
            .status()
            .map(|s| s.success())
            .unwrap_or(false);
        if !already {
            let existing = std::fs::read_to_string("/etc/pf.conf").unwrap_or_default();
            let with_anchor = format!("{}\nanchor \"aivpn_ks\"\n", existing);
            let ref_file = "/var/run/aivpn/aivpn_ks_ref.conf";
            let _ = std::fs::remove_file(ref_file);
            let wrote = std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode(0o600)
                .custom_flags(libc::O_NOFOLLOW)
                .open(ref_file)
                .and_then(|mut f| {
                    use std::io::Write;
                    f.write_all(with_anchor.as_bytes())
                })
                .is_ok();
            if wrote {
                let _ = Command::new("pfctl").args(["-f", ref_file]).status();
            }
        }

        Ok(())
    }

    #[cfg(target_os = "macos")]
    fn deactivate_impl(&self) {
        use std::process::Command;
        let _ = Command::new("pfctl")
            .args(["-a", "aivpn_ks", "-F", "rules"])
            .status();
        let _ = std::fs::remove_file("/var/run/aivpn/aivpn_ks.conf");
        let _ = std::fs::remove_file("/var/run/aivpn/aivpn_ks_ref.conf");
    }

    #[cfg(target_os = "macos")]
    fn clear_stale_impl() {
        use std::process::Command;
        let _ = Command::new("pfctl")
            .args(["-a", "aivpn_ks", "-F", "rules"])
            .status();
        let _ = std::fs::remove_file("/var/run/aivpn/aivpn_ks.conf");
        let _ = std::fs::remove_file("/var/run/aivpn/aivpn_ks_ref.conf");
    }

    // ──────────────────── Windows ────────────────────

    #[cfg(target_os = "windows")]
    fn activate_impl(&self) -> Result<()> {
        use std::process::Command;

        let status = Command::new("netsh")
            .args([
                "advfirewall",
                "firewall",
                "add",
                "rule",
                "name=AIVPN_KS_BLOCK",
                "dir=out",
                "action=block",
                "profile=any",
            ])
            .status()
            .map_err(Error::Io)?;
        if !status.success() {
            warn!("kill-switch: failed to add block rule — Windows Firewall may be disabled");
        }

        for (name, extra) in &[
            ("AIVPN_KS_ALLOW_VPN", format!("interface={}", self.tun_name)),
            (
                "AIVPN_KS_ALLOW_SERVER",
                format!("remoteip={}", self.server_ip),
            ),
            ("AIVPN_KS_ALLOW_LOCAL", "remoteip=127.0.0.0/8".to_string()),
        ] {
            let _ = Command::new("netsh")
                .args([
                    "advfirewall",
                    "firewall",
                    "add",
                    "rule",
                    &format!("name={}", name),
                    "dir=out",
                    "action=allow",
                    extra.as_str(),
                ])
                .status();
        }

        Ok(())
    }

    #[cfg(target_os = "windows")]
    fn deactivate_impl(&self) {
        use std::process::Command;
        for name in &[
            "AIVPN_KS_BLOCK",
            "AIVPN_KS_ALLOW_VPN",
            "AIVPN_KS_ALLOW_SERVER",
            "AIVPN_KS_ALLOW_LOCAL",
        ] {
            let _ = Command::new("netsh")
                .args([
                    "advfirewall",
                    "firewall",
                    "delete",
                    "rule",
                    &format!("name={}", name),
                ])
                .status();
        }
    }

    #[cfg(target_os = "windows")]
    fn clear_stale_impl() {
        use std::process::Command;
        for name in &[
            "AIVPN_KS_BLOCK",
            "AIVPN_KS_ALLOW_VPN",
            "AIVPN_KS_ALLOW_SERVER",
            "AIVPN_KS_ALLOW_LOCAL",
        ] {
            let _ = Command::new("netsh")
                .args([
                    "advfirewall",
                    "firewall",
                    "delete",
                    "rule",
                    &format!("name={}", name),
                ])
                .status();
        }
    }

    // ──────────────────── Unsupported platforms ────────────────────

    #[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
    fn activate_impl(&self) -> Result<()> {
        warn!("Kill-switch not supported on this platform");
        Ok(())
    }

    #[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
    fn deactivate_impl(&self) {}

    #[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
    fn clear_stale_impl() {}
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_new_not_active() {
        let ks = KillSwitch::new("tun0".to_string(), "1.2.3.4".to_string());
        assert!(!ks.is_active());
    }

    #[test]
    fn test_deactivate_when_not_active_is_noop() {
        let mut ks = KillSwitch::new("tun0".to_string(), "1.2.3.4".to_string());
        ks.deactivate();
        assert!(!ks.is_active());
    }

    #[test]
    fn test_fields_stored() {
        let ks = KillSwitch::new("utun5".to_string(), "198.51.100.1".to_string());
        assert_eq!(ks.server_ip, "198.51.100.1");
        assert_eq!(ks.tun_name, "utun5");
        assert!(!ks.is_active());
    }

    #[test]
    fn test_double_activate_skips_second() {
        // Verify the guard condition: second activate() on an already-active
        // KillSwitch returns Ok without re-running the platform commands.
        // We simulate by setting active = true manually via a helper method.
        let mut ks = KillSwitch::new("tun9".to_string(), "10.0.0.1".to_string());
        ks.active = true; // pretend it's already on
                          // Should not panic or fail
        let result = ks.activate();
        assert!(result.is_ok());
    }
}
