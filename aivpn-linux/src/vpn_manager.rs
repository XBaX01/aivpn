#[derive(Debug, Clone, PartialEq)]
pub enum VpnStatus {
    Disconnected,
    Connecting,
    Connected { vpn_ip: String },
    Error(String),
}

pub fn find_client_binary() -> Result<std::path::PathBuf, String> {
    // 1. Same directory as this executable
    if let Ok(exe) = std::env::current_exe() {
        let candidate = exe
            .parent()
            .unwrap_or(std::path::Path::new("/"))
            .join("aivpn-client");
        if candidate.exists() {
            return Ok(candidate);
        }
    }
    // 2. PATH
    let path_var = std::env::var("PATH").unwrap_or_default();
    for dir in path_var.split(':') {
        let p = std::path::Path::new(dir).join("aivpn-client");
        if p.exists() {
            return Ok(p);
        }
    }
    Err("'aivpn-client' not found in PATH or next to aivpn-linux binary".to_string())
}
