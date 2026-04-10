//! VPN Manager — subprocess control for aivpn-client.exe
//!
//! Launches the Rust client binary in the background (no console window),
//! monitors its process state, and reads traffic stats from file.

use std::path::PathBuf;
use std::process::{Child, Command};
use std::time::Instant;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConnectionState {
    Disconnected,
    Connecting,
    Connected,
    Disconnecting,
}

pub struct TrafficStats {
    pub bytes_sent: u64,
    pub bytes_received: u64,
}

pub struct VpnManager {
    state: ConnectionState,
    child: Option<Child>,
    stats: TrafficStats,
    last_poll: Option<Instant>,
    client_binary: PathBuf,
}

impl VpnManager {
    pub fn new() -> Self {
        Self {
            state: ConnectionState::Disconnected,
            child: None,
            stats: TrafficStats {
                bytes_sent: 0,
                bytes_received: 0,
            },
            last_poll: None,
            client_binary: Self::find_client_binary(),
        }
    }

    pub fn state(&self) -> ConnectionState {
        self.state
    }

    pub fn stats(&self) -> &TrafficStats {
        &self.stats
    }

    pub fn is_connected(&self) -> bool {
        self.state == ConnectionState::Connected
    }

    pub fn is_busy(&self) -> bool {
        matches!(
            self.state,
            ConnectionState::Connecting | ConnectionState::Disconnecting
        )
    }

    /// Connect using a connection key
    pub fn connect(&mut self, connection_key: &str, full_tunnel: bool) -> Result<(), String> {
        if self.child.is_some() {
            return Err("Already running".to_string());
        }

        if !self.client_binary.exists() {
            return Err(format!(
                "Client binary not found: {}",
                self.client_binary.display()
            ));
        }

        self.state = ConnectionState::Connecting;

        let mut cmd = Command::new(&self.client_binary);
        cmd.arg("--connection-key").arg(connection_key);

        if full_tunnel {
            cmd.arg("--full-tunnel");
        }

        // Hide console window on Windows
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            const CREATE_NO_WINDOW: u32 = 0x08000000;
            cmd.creation_flags(CREATE_NO_WINDOW);
        }

        match cmd.spawn() {
            Ok(child) => {
                self.child = Some(child);
                self.state = ConnectionState::Connected;
                self.stats.bytes_sent = 0;
                self.stats.bytes_received = 0;
                Ok(())
            }
            Err(e) => {
                self.state = ConnectionState::Disconnected;
                Err(format!("Failed to start client: {}", e))
            }
        }
    }

    /// Disconnect — kill the client process
    pub fn disconnect(&mut self) {
        self.state = ConnectionState::Disconnecting;

        if let Some(ref mut child) = self.child {
            let _ = child.kill();
            let _ = child.wait();
        }
        self.child = None;
        self.state = ConnectionState::Disconnected;
        self.stats.bytes_sent = 0;
        self.stats.bytes_received = 0;
    }

    /// Poll process status and read traffic stats
    pub fn poll_status(&mut self) {
        // Only poll every 500ms
        if let Some(last) = self.last_poll {
            if last.elapsed() < std::time::Duration::from_millis(500) {
                return;
            }
        }
        self.last_poll = Some(Instant::now());

        // Check if process is still alive
        if let Some(ref mut child) = self.child {
            match child.try_wait() {
                Ok(Some(_status)) => {
                    // Process exited
                    self.child = None;
                    self.state = ConnectionState::Disconnected;
                    return;
                }
                Ok(None) => {
                    // Still running
                    if self.state == ConnectionState::Connecting {
                        self.state = ConnectionState::Connected;
                    }
                }
                Err(_) => {
                    self.child = None;
                    self.state = ConnectionState::Disconnected;
                    return;
                }
            }
        }

        // Read traffic stats from file
        if self.state == ConnectionState::Connected {
            self.read_traffic_stats();
        }
    }

    fn read_traffic_stats(&mut self) {
        // Try standard path first, then fallback
        let paths = [
            Self::stats_file_path(),
            std::env::temp_dir().join("aivpn-traffic.stats"),
        ];

        for path in &paths {
            if let Ok(content) = std::fs::read_to_string(path) {
                if let Some((sent, recv)) = Self::parse_stats(&content) {
                    self.stats.bytes_sent = sent;
                    self.stats.bytes_received = recv;
                    return;
                }
            }
        }
    }

    fn parse_stats(content: &str) -> Option<(u64, u64)> {
        // Format: "sent:12345,received:67890"
        let content = content.trim();
        let mut sent = None;
        let mut recv = None;

        for part in content.split(',') {
            let mut kv = part.splitn(2, ':');
            match (kv.next(), kv.next()) {
                (Some("sent"), Some(v)) => sent = v.trim().parse().ok(),
                (Some("received"), Some(v)) => recv = v.trim().parse().ok(),
                _ => {}
            }
        }

        match (sent, recv) {
            (Some(s), Some(r)) => Some((s, r)),
            _ => None,
        }
    }

    fn stats_file_path() -> PathBuf {
        if cfg!(windows) {
            dirs::data_local_dir()
                .unwrap_or_else(|| PathBuf::from("."))
                .join("AIVPN")
                .join("traffic.stats")
        } else {
            PathBuf::from("/var/run/aivpn/traffic.stats")
        }
    }

    fn find_client_binary() -> PathBuf {
        // 1. Next to this executable
        if let Ok(exe) = std::env::current_exe() {
            let dir = exe.parent().unwrap_or(std::path::Path::new("."));
            let candidate = dir.join("aivpn-client.exe");
            if candidate.exists() {
                return candidate;
            }
            // Also check without .exe for non-windows dev
            let candidate = dir.join("aivpn-client");
            if candidate.exists() {
                return candidate;
            }
        }

        // 2. Program Files
        if cfg!(windows) {
            let pf = PathBuf::from(r"C:\Program Files\AIVPN\aivpn-client.exe");
            if pf.exists() {
                return pf;
            }
        }

        // 3. Fallback — expect it in PATH
        PathBuf::from(if cfg!(windows) {
            "aivpn-client.exe"
        } else {
            "aivpn-client"
        })
    }
}

/// Format byte count for display
pub fn format_bytes(bytes: u64) -> String {
    const KB: u64 = 1024;
    const MB: u64 = 1024 * 1024;
    const GB: u64 = 1024 * 1024 * 1024;

    if bytes >= GB {
        format!("{:.1} GB", bytes as f64 / GB as f64)
    } else if bytes >= MB {
        format!("{:.1} MB", bytes as f64 / MB as f64)
    } else if bytes >= KB {
        format!("{:.1} KB", bytes as f64 / KB as f64)
    } else {
        format!("{} B", bytes)
    }
}
