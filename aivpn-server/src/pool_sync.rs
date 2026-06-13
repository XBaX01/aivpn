//! Server pool synchronization — keeps clients.json in sync across pool nodes.
//!
//! Pool nodes share the same `server.key`.  Each node runs both a listener
//! (accepts inbound pushes) and outbound sync tasks (pushes to peers).
//! Protocol: length-prefixed JSON frames over TCP, authenticated with
//! BLAKE3-keyed hash of the shared `sync_key`.

use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tracing::{debug, error, info, warn};

use crate::client_db::ClientDatabase;
use aivpn_common::error::{Error, Result};
use aivpn_common::event_log::{AivpnEvent, EventBus, PeerSyncAction};
use base64::Engine as _;

const SYNC_INTERVAL: Duration = Duration::from_secs(5);
const SYNC_TIMEOUT: Duration = Duration::from_secs(10);
const MAX_FRAME_SIZE: usize = 16 * 1024 * 1024;

/// Pool configuration stored in server.json under `"pool"`.
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct PoolSyncConfig {
    /// Peer server addresses (host:port using the VPN port; sync uses port+1).
    #[serde(default)]
    pub peers: Vec<String>,
    /// Override the sync TCP port.  Default = main listen port + 1.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sync_port: Option<u16>,
    /// 32-byte BLAKE3 key (base64).  All pool nodes must share this value.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sync_key: Option<String>,
}

#[derive(Debug, serde::Serialize, serde::Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
enum SyncMsg {
    FullSync { clients_json: String },
    Ping,
    Pong,
    Err { message: String },
}

/// Manages inbound and outbound peer synchronization.
pub struct PeerSyncer {
    db: Arc<ClientDatabase>,
    config: PoolSyncConfig,
    sync_key: [u8; 32],
    events: EventBus,
}

impl PeerSyncer {
    pub fn new(db: Arc<ClientDatabase>, config: PoolSyncConfig, events: EventBus) -> Arc<Self> {
        let sync_key: [u8; 32] = config
            .sync_key
            .as_deref()
            .and_then(|k| base64::engine::general_purpose::STANDARD.decode(k).ok())
            .and_then(|b| b.try_into().ok())
            .unwrap_or([0u8; 32]);

        Arc::new(Self {
            db,
            config,
            sync_key,
            events,
        })
    }

    /// Spawn listener + outbound tasks.  Returns immediately.
    pub fn start(self: Arc<Self>, sync_port: u16) {
        {
            let me = self.clone();
            tokio::spawn(async move { me.run_listener(sync_port).await });
        }
        for peer in self.config.peers.clone() {
            let me = self.clone();
            tokio::spawn(async move { me.outbound_loop(peer).await });
        }
    }

    async fn run_listener(self: Arc<Self>, port: u16) {
        let addr = format!("0.0.0.0:{}", port);
        let listener = match TcpListener::bind(&addr).await {
            Ok(l) => {
                info!("Pool sync listener on {}", addr);
                l
            }
            Err(e) => {
                warn!("Pool sync bind {} failed: {} — peer sync disabled", addr, e);
                return;
            }
        };
        loop {
            match listener.accept().await {
                Ok((stream, peer_addr)) => {
                    debug!("Pool sync inbound from {}", peer_addr);
                    let me = self.clone();
                    tokio::spawn(async move {
                        if let Err(e) = me.handle_inbound(stream).await {
                            warn!("Pool sync inbound error: {}", e);
                        }
                    });
                }
                Err(e) => {
                    error!("Pool sync accept: {}", e);
                    tokio::time::sleep(Duration::from_secs(1)).await;
                }
            }
        }
    }

    async fn outbound_loop(self: Arc<Self>, peer: String) {
        let mut backoff = SYNC_INTERVAL;
        loop {
            match self.push_to_peer(&peer).await {
                Ok(n) => {
                    debug!("Pool sync pushed {} clients to {}", n, peer);
                    self.events.emit(AivpnEvent::PeerSync {
                        peer: peer.clone(),
                        action: PeerSyncAction::FullSync,
                        clients_synced: n as u32,
                    });
                    backoff = SYNC_INTERVAL;
                }
                Err(e) => {
                    warn!(
                        "Pool sync to {} failed: {} — retry in {:?}",
                        peer, e, backoff
                    );
                    backoff = (backoff * 2).min(Duration::from_secs(120));
                }
            }
            tokio::time::sleep(backoff).await;
        }
    }

    async fn push_to_peer(&self, peer: &str) -> Result<usize> {
        let sync_addr = self.peer_sync_addr(peer);
        let stream = tokio::time::timeout(SYNC_TIMEOUT, TcpStream::connect(&sync_addr))
            .await
            .map_err(|_| Error::Session("peer sync connect timeout".into()))?
            .map_err(|e| Error::Session(format!("peer sync connect: {}", e)))?;

        let (mut r, mut w) = stream.into_split();

        write_frame(&mut w, &self.auth_token()).await?;
        let ack = read_frame(&mut r).await?;
        if ack != b"ok" {
            return Err(Error::Session("peer sync auth rejected".into()));
        }

        let clients = self.db.list_clients();
        let n = clients.len();
        let clients_json = serde_json::to_string(&clients)
            .map_err(|e| Error::Session(format!("serialize clients: {}", e)))?;
        let msg = serde_json::to_vec(&SyncMsg::FullSync { clients_json })
            .map_err(|e| Error::Session(format!("serialize msg: {}", e)))?;
        write_frame(&mut w, &msg).await?;
        let _ = read_frame(&mut r).await?;
        Ok(n)
    }

    async fn handle_inbound(&self, stream: TcpStream) -> Result<()> {
        let (mut r, mut w) = stream.into_split();

        let tok = read_frame(&mut r).await?;
        if tok != self.auth_token() {
            let _ = write_frame(&mut w, b"auth_fail").await;
            return Err(Error::Session("peer sync: bad auth".into()));
        }
        write_frame(&mut w, b"ok").await?;

        let data = read_frame(&mut r).await?;
        let msg: SyncMsg = serde_json::from_slice(&data)
            .map_err(|e| Error::Session(format!("sync msg parse: {}", e)))?;

        match msg {
            SyncMsg::FullSync { clients_json } => {
                let n = self.db.merge_from_json(&clients_json)?;
                info!("Pool sync: merged {} clients from peer", n);
                self.events.emit(AivpnEvent::PeerSync {
                    peer: "inbound".into(),
                    action: PeerSyncAction::FullSync,
                    clients_synced: n as u32,
                });
            }
            SyncMsg::Ping => {}
            _ => {}
        }

        let ack = serde_json::to_vec(&SyncMsg::Pong).unwrap_or_default();
        write_frame(&mut w, &ack).await?;
        Ok(())
    }

    fn auth_token(&self) -> Vec<u8> {
        blake3::keyed_hash(&self.sync_key, b"aivpn-pool-sync-v1")
            .as_bytes()
            .to_vec()
    }

    /// Derive sync TCP address from a VPN peer address.
    fn peer_sync_addr(&self, peer: &str) -> String {
        if let Some(port) = self.config.sync_port {
            // host only or host:port — replace port
            let host = peer.rsplit_once(':').map(|(h, _)| h).unwrap_or(peer);
            return format!("{}:{}", host, port);
        }
        // Default: VPN port + 1
        if let Ok(addr) = peer.parse::<std::net::SocketAddr>() {
            return format!("{}:{}", addr.ip(), addr.port() + 1);
        }
        format!("{}:444", peer)
    }
}

async fn write_frame(w: &mut tokio::net::tcp::OwnedWriteHalf, data: &[u8]) -> Result<()> {
    let len = (data.len() as u32).to_le_bytes();
    w.write_all(&len)
        .await
        .map_err(|e| Error::Session(e.to_string()))?;
    w.write_all(data)
        .await
        .map_err(|e| Error::Session(e.to_string()))
}

async fn read_frame(r: &mut tokio::net::tcp::OwnedReadHalf) -> Result<Vec<u8>> {
    let mut lb = [0u8; 4];
    r.read_exact(&mut lb)
        .await
        .map_err(|e| Error::Session(e.to_string()))?;
    let len = u32::from_le_bytes(lb) as usize;
    if len > MAX_FRAME_SIZE {
        return Err(Error::Session("sync frame too large".into()));
    }
    let mut buf = vec![0u8; len];
    r.read_exact(&mut buf)
        .await
        .map_err(|e| Error::Session(e.to_string()))?;
    Ok(buf)
}
