//! Append-only admin audit log.
//!
//! Each administrative action is recorded as a JSON line in the audit log path.
//! Rotation is handled externally via logrotate.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use chrono::Utc;
use serde::{Deserialize, Serialize};
use tracing::warn;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AuditActor {
    Cli,
    Api,
    System,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuditEntry {
    pub ts: String,
    pub actor: AuditActor,
    pub action: String,
    pub target: String,
    pub result: String,
}

/// Thread-safe append-only audit logger.
#[derive(Clone)]
pub struct AuditLogger {
    inner: Arc<Mutex<AuditLoggerInner>>,
}

struct AuditLoggerInner {
    path: PathBuf,
}

impl AuditLogger {
    pub fn new(path: &Path) -> Self {
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        Self {
            inner: Arc::new(Mutex::new(AuditLoggerInner {
                path: path.to_path_buf(),
            })),
        }
    }

    pub fn disabled() -> Self {
        Self::new(Path::new("/dev/null"))
    }

    pub fn log(&self, actor: AuditActor, action: &str, target: &str, result: &str) {
        let entry = AuditEntry {
            ts: Utc::now().to_rfc3339(),
            actor,
            action: action.to_string(),
            target: target.to_string(),
            result: result.to_string(),
        };
        let line = match serde_json::to_string(&entry) {
            Ok(s) => s,
            Err(e) => {
                warn!("audit_log serialize: {}", e);
                return;
            }
        };
        let inner = self.inner.lock().unwrap();
        match std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&inner.path)
        {
            Ok(mut f) => {
                let _ = writeln!(f, "{}", line);
            }
            Err(e) => {
                warn!("audit_log write {:?}: {}", inner.path, e);
            }
        }
    }
}
