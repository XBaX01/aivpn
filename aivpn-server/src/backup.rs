//! Backup / export / import for server configuration.
//!
//! Creates a tar.gz archive with manifest.json, clients.json, server.json, masks/.

use std::io::Read;
use std::path::{Path, PathBuf};

use chrono::Utc;
use serde::{Deserialize, Serialize};
use tracing::{info, warn};

use aivpn_common::error::{Error, Result};

const MANIFEST_NAME: &str = "manifest.json";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BackupManifest {
    pub aivpn_version: String,
    pub created_at: String,
    pub components: Vec<String>,
}

impl BackupManifest {
    fn new(components: Vec<String>) -> Self {
        Self {
            aivpn_version: env!("CARGO_PKG_VERSION").to_string(),
            created_at: Utc::now().to_rfc3339(),
            components,
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct ExportOptions {
    pub include_clients: bool,
    pub include_masks: bool,
    pub include_config: bool,
    pub config_path: Option<PathBuf>,
    pub mask_dir: Option<PathBuf>,
    pub clients_db: Option<PathBuf>,
}

/// Export server data to `output_path` (.tar.gz).
pub fn export_server(opts: &ExportOptions, output_path: &Path) -> Result<()> {
    let mut components = Vec::new();
    let file = std::fs::File::create(output_path)
        .map_err(|e| Error::Session(format!("create backup: {}", e)))?;
    let gz = flate2::write::GzEncoder::new(file, flate2::Compression::default());
    let mut ar = tar::Builder::new(gz);

    if opts.include_clients {
        if let Some(ref p) = opts.clients_db {
            if p.exists() {
                ar.append_path_with_name(p, "clients.json")
                    .map_err(|e| Error::Session(format!("archive clients: {}", e)))?;
                components.push("clients".to_string());
            } else {
                warn!("clients.json not found at {:?}, skipping", p);
            }
        }
    }

    if opts.include_config {
        if let Some(ref p) = opts.config_path {
            if p.exists() {
                ar.append_path_with_name(p, "server.json")
                    .map_err(|e| Error::Session(format!("archive config: {}", e)))?;
                components.push("config".to_string());
            }
        }
    }

    if opts.include_masks {
        if let Some(ref dir) = opts.mask_dir {
            if dir.is_dir() {
                for entry in std::fs::read_dir(dir)
                    .map_err(|e| Error::Session(format!("read mask dir: {}", e)))?
                {
                    let entry = entry.map_err(|e| Error::Session(format!("mask entry: {}", e)))?;
                    let path = entry.path();
                    if path.extension().and_then(|e| e.to_str()) == Some("json") {
                        let rel = PathBuf::from("masks").join(entry.file_name());
                        ar.append_path_with_name(&path, &rel)
                            .map_err(|e| Error::Session(format!("archive mask: {}", e)))?;
                    }
                }
                components.push("masks".to_string());
            }
        }
    }

    // Always write manifest last
    let manifest = BackupManifest::new(components);
    let manifest_json = serde_json::to_vec_pretty(&manifest)
        .map_err(|e| Error::Session(format!("serialize manifest: {}", e)))?;
    let mut header = tar::Header::new_gnu();
    header.set_size(manifest_json.len() as u64);
    header.set_mode(0o644);
    header.set_cksum();
    ar.append_data(&mut header, MANIFEST_NAME, manifest_json.as_slice())
        .map_err(|e| Error::Session(format!("archive manifest: {}", e)))?;

    ar.finish()
        .map_err(|e| Error::Session(format!("finalize archive: {}", e)))?;

    info!(
        "Backup written to {:?} (components: {:?})",
        output_path, manifest.components
    );
    Ok(())
}

/// Import from a backup archive.  `dry_run = true` prints diff without writing.
pub fn import_server(archive_path: &Path, target_dir: &Path, dry_run: bool) -> Result<()> {
    // First pass: read manifest
    let manifest = {
        let file = std::fs::File::open(archive_path)
            .map_err(|e| Error::Session(format!("open backup: {}", e)))?;
        let gz = flate2::read::GzDecoder::new(file);
        let mut ar = tar::Archive::new(gz);
        let mut found: Option<BackupManifest> = None;
        for entry in ar
            .entries()
            .map_err(|e| Error::Session(format!("read archive: {}", e)))?
        {
            let mut entry = entry.map_err(|e| Error::Session(format!("entry: {}", e)))?;
            let path = entry
                .path()
                .map_err(|e| Error::Session(format!("entry path: {}", e)))?
                .to_path_buf();
            if path.to_str() == Some(MANIFEST_NAME) {
                let mut buf = String::new();
                entry
                    .read_to_string(&mut buf)
                    .map_err(|e| Error::Session(format!("read manifest: {}", e)))?;
                found = serde_json::from_str(&buf).ok();
                break;
            }
        }
        found.ok_or_else(|| Error::Session("backup missing manifest.json".to_string()))?
    };

    let backup_major = semver_major(&manifest.aivpn_version);
    let current_major = semver_major(env!("CARGO_PKG_VERSION"));
    if backup_major != current_major {
        warn!(
            "Version mismatch: backup={} current={} — import may not be fully compatible",
            manifest.aivpn_version,
            env!("CARGO_PKG_VERSION")
        );
    }

    if dry_run {
        println!("DRY RUN — no files will be written.");
        println!("Backup created:  {}", manifest.created_at);
        println!("Backup version:  {}", manifest.aivpn_version);
        println!("Components:      {:?}", manifest.components);
        println!("Restore target:  {:?}", target_dir);
        return Ok(());
    }

    std::fs::create_dir_all(target_dir)
        .map_err(|e| Error::Session(format!("create target dir: {}", e)))?;

    // Second pass: extract
    let file = std::fs::File::open(archive_path)
        .map_err(|e| Error::Session(format!("open backup: {}", e)))?;
    let gz = flate2::read::GzDecoder::new(file);
    let mut ar = tar::Archive::new(gz);
    for entry in ar
        .entries()
        .map_err(|e| Error::Session(format!("read archive: {}", e)))?
    {
        let mut entry = entry.map_err(|e| Error::Session(format!("entry: {}", e)))?;
        let rel = entry
            .path()
            .map_err(|e| Error::Session(format!("entry path: {}", e)))?
            .to_path_buf();
        if rel.to_str() == Some(MANIFEST_NAME) {
            continue;
        }
        let dest = target_dir.join(&rel);
        if let Some(parent) = dest.parent() {
            std::fs::create_dir_all(parent).map_err(|e| Error::Session(format!("mkdir: {}", e)))?;
        }
        let tmp = dest.with_extension("tmp");
        let mut buf = Vec::new();
        entry
            .read_to_end(&mut buf)
            .map_err(|e| Error::Session(format!("read entry: {}", e)))?;
        std::fs::write(&tmp, &buf)
            .map_err(|e| Error::Session(format!("write {:?}: {}", tmp, e)))?;
        std::fs::rename(&tmp, &dest)
            .map_err(|e| Error::Session(format!("rename {:?}: {}", dest, e)))?;
        info!("Restored {:?}", rel);
    }

    info!("Import complete from {:?}", archive_path);
    Ok(())
}

fn semver_major(v: &str) -> u64 {
    v.split('.')
        .next()
        .and_then(|s| s.parse().ok())
        .unwrap_or(0)
}
