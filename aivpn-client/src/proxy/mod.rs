pub mod device;
pub mod socks5;

use std::collections::VecDeque;
use std::net::{Ipv4Addr, SocketAddr};
use std::sync::atomic::{AtomicBool, AtomicU16, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use smoltcp::iface::{Config, Interface, SocketHandle, SocketSet};
use smoltcp::socket::tcp::{Socket as TcpSocket, SocketBuffer as TcpSocketBuffer, State};
use smoltcp::time::Instant as SmolInstant;
use smoltcp::wire::{HardwareAddress, IpAddress, IpCidr, IpEndpoint, Ipv4Address};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::mpsc;
use tracing::{debug, error, info, warn};

use crate::proxy::device::VpnDevice;
use crate::proxy::socks5::{Socks5Session, REP_GENERAL_FAILURE, REP_HOST_UNREACHABLE, REP_SUCCESS};

/// MTU matching WAN_SAFE_TUN_MTU in tunnel.rs
const PROXY_MTU: usize = 1346;
const TCP_BUF: usize = 65536;
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

static SRC_PORT: AtomicU16 = AtomicU16::new(49152);

pub struct ProxyConfig {
    pub listen_addr: SocketAddr,
    pub vpn_ip: Ipv4Addr,
    pub gateway_ip: Ipv4Addr,
    pub prefix_len: u8,
}

pub struct ProxyHandle {
    pub rx_queue: Arc<Mutex<VecDeque<Vec<u8>>>>,
}

struct ManagedConn {
    handle: SocketHandle,
    inbound: Arc<Mutex<VecDeque<Vec<u8>>>>,
    outbound_tx: tokio::sync::mpsc::Sender<Vec<u8>>,
    close_flag: Arc<AtomicBool>,
    /// Present until TCP handshake completes, then cleared.
    ready_tx: Option<std::sync::mpsc::SyncSender<bool>>,
}

struct NewConn {
    target: IpEndpoint,
    src_port: u16,
    inbound: Arc<Mutex<VecDeque<Vec<u8>>>>,
    outbound_tx: tokio::sync::mpsc::Sender<Vec<u8>>,
    close_flag: Arc<AtomicBool>,
    ready_tx: std::sync::mpsc::SyncSender<bool>,
}

/// Start the smoltcp stack thread and SOCKS5 listener.
pub async fn spawn_proxy(
    config: ProxyConfig,
    tun_to_udp_tx: mpsc::Sender<Vec<u8>>,
) -> std::io::Result<ProxyHandle> {
    let rx_queue: Arc<Mutex<VecDeque<Vec<u8>>>> = Arc::new(Mutex::new(VecDeque::new()));
    let tx_queue: Arc<Mutex<VecDeque<Vec<u8>>>> = Arc::new(Mutex::new(VecDeque::new()));

    let (cmd_tx, cmd_rx) = std::sync::mpsc::channel::<NewConn>();

    let vpn_ip = config.vpn_ip;
    let gateway_ip = config.gateway_ip;
    let prefix_len = config.prefix_len;
    let rx_clone = Arc::clone(&rx_queue);
    let tx_clone = Arc::clone(&tx_queue);

    std::thread::spawn(move || {
        run_stack(
            rx_clone,
            tx_clone,
            tun_to_udp_tx,
            cmd_rx,
            vpn_ip,
            gateway_ip,
            prefix_len,
        );
    });

    let listener = tokio::net::TcpListener::bind(config.listen_addr).await?;
    info!("SOCKS5 proxy listening on {}", config.listen_addr);

    tokio::spawn(async move {
        loop {
            match listener.accept().await {
                Ok((stream, peer)) => {
                    debug!("Proxy: connection from {}", peer);
                    let tx = cmd_tx.clone();
                    tokio::spawn(handle_socks5(stream, tx));
                }
                Err(e) => {
                    error!("Proxy accept error: {}", e);
                    tokio::time::sleep(Duration::from_millis(100)).await;
                }
            }
        }
    });

    Ok(ProxyHandle { rx_queue })
}

fn smoltcp_now() -> SmolInstant {
    let d = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    SmolInstant::from_millis(d.as_millis() as i64)
}

fn alloc_src_port() -> u16 {
    let p = SRC_PORT.fetch_add(1, Ordering::Relaxed);
    if p > 65000 {
        SRC_PORT.store(49152, Ordering::Relaxed);
    }
    p
}

fn run_stack(
    rx_queue: Arc<Mutex<VecDeque<Vec<u8>>>>,
    tx_queue: Arc<Mutex<VecDeque<Vec<u8>>>>,
    tun_to_udp_tx: mpsc::Sender<Vec<u8>>,
    cmd_rx: std::sync::mpsc::Receiver<NewConn>,
    vpn_ip: Ipv4Addr,
    gateway_ip: Ipv4Addr,
    prefix_len: u8,
) {
    let mut device = VpnDevice::new(Arc::clone(&rx_queue), Arc::clone(&tx_queue), PROXY_MTU);

    let cfg = Config::new(HardwareAddress::Ip);
    let mut iface = Interface::new(cfg, &mut device, smoltcp_now());

    let [a, b, c, d] = vpn_ip.octets();
    iface.update_ip_addrs(|addrs| {
        let _ = addrs.push(IpCidr::new(IpAddress::v4(a, b, c, d), prefix_len));
    });

    let [ga, gb, gc, gd] = gateway_ip.octets();
    let _ = iface
        .routes_mut()
        .add_default_ipv4_route(Ipv4Address::new(ga, gb, gc, gd));

    let mut sockets = SocketSet::new(vec![]);
    let mut conns: Vec<ManagedConn> = Vec::new();

    loop {
        // Accept new connection requests from the async SOCKS5 handlers
        loop {
            match cmd_rx.try_recv() {
                Ok(nc) => {
                    let rx_buf = TcpSocketBuffer::new(vec![0u8; TCP_BUF]);
                    let tx_buf = TcpSocketBuffer::new(vec![0u8; TCP_BUF]);
                    let mut socket = TcpSocket::new(rx_buf, tx_buf);
                    socket.set_ack_delay(None);

                    let mut cx = iface.context();
                    match socket.connect(&mut cx, nc.target, nc.src_port) {
                        Ok(()) => {
                            let handle = sockets.add(socket);
                            conns.push(ManagedConn {
                                handle,
                                inbound: nc.inbound,
                                outbound_tx: nc.outbound_tx,
                                close_flag: nc.close_flag,
                                ready_tx: Some(nc.ready_tx),
                            });
                        }
                        Err(e) => {
                            error!("smoltcp connect error: {}", e);
                            let _ = nc.ready_tx.send(false);
                        }
                    }
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => break,
                Err(std::sync::mpsc::TryRecvError::Disconnected) => return,
            }
        }

        // Drive the network stack
        iface.poll(smoltcp_now(), &mut device, &mut sockets);

        // Forward smoltcp TX packets to the VPN upload pipeline
        {
            let mut q = tx_queue.lock().unwrap();
            while let Some(pkt) = q.pop_front() {
                if tun_to_udp_tx.blocking_send(pkt).is_err() {
                    return;
                }
            }
        }

        // Service each active connection
        let mut remove: Vec<usize> = Vec::new();
        for (i, conn) in conns.iter_mut().enumerate() {
            if service_conn(conn, &mut sockets) {
                remove.push(i);
            }
        }
        for i in remove.into_iter().rev() {
            let conn = conns.remove(i);
            sockets.remove(conn.handle);
        }

        std::thread::sleep(Duration::from_millis(1));
    }
}

/// Returns true when the connection should be removed from the active list.
fn service_conn(conn: &mut ManagedConn, sockets: &mut SocketSet) -> bool {
    let socket = sockets.get_mut::<TcpSocket>(conn.handle);

    // While TCP handshake is pending, check socket state
    if let Some(ready_tx) = conn.ready_tx.take() {
        match socket.state() {
            State::Established => {
                let _ = ready_tx.send(true);
                // Fall through to service any data that arrived in parallel
            }
            State::Closed | State::TimeWait | State::CloseWait => {
                let _ = ready_tx.send(false);
                return true;
            }
            _ => {
                conn.ready_tx = Some(ready_tx);
                return false;
            }
        }
    }

    // SOCKS5 client disconnected
    if conn.close_flag.load(Ordering::Relaxed) {
        socket.abort();
        return true;
    }

    // Write queued data from SOCKS5 client → smoltcp TCP socket
    if socket.may_send() {
        let mut q = conn.inbound.lock().unwrap();
        while !q.is_empty() && socket.can_send() {
            let chunk = q.pop_front().unwrap();
            match socket.send_slice(&chunk) {
                Ok(n) if n < chunk.len() => {
                    // Partial write — put remainder back at the front
                    q.push_front(chunk[n..].to_vec());
                    break;
                }
                Ok(_) => {}
                Err(_) => break,
            }
        }
    }

    // Read data from smoltcp TCP socket → SOCKS5 client
    if socket.can_recv() {
        let mut tmp = vec![0u8; 4096];
        if let Ok(n) = socket.recv_slice(&mut tmp) {
            if n > 0 {
                tmp.truncate(n);
                if conn.outbound_tx.try_send(tmp).is_err() {
                    socket.abort();
                    return true;
                }
            }
        }
    }

    matches!(
        socket.state(),
        State::Closed | State::TimeWait | State::FinWait2
    )
}

async fn handle_socks5(stream: tokio::net::TcpStream, cmd_tx: std::sync::mpsc::Sender<NewConn>) {
    let mut session = Socks5Session::new(stream);

    let target_addr = match session.negotiate().await {
        Ok(a) => a,
        Err(e) => {
            warn!("SOCKS5 negotiate: {}", e);
            let _ = session.send_reply(REP_GENERAL_FAILURE).await;
            return;
        }
    };

    let target_ipv4 = match target_addr.ip() {
        std::net::IpAddr::V4(ip) => ip,
        std::net::IpAddr::V6(_) => {
            warn!("Proxy: IPv6 targets not supported: {}", target_addr);
            let _ = session.send_reply(REP_HOST_UNREACHABLE).await;
            return;
        }
    };

    let [ta, tb, tc, td] = target_ipv4.octets();
    let target = IpEndpoint::new(IpAddress::v4(ta, tb, tc, td), target_addr.port());
    let src_port = alloc_src_port();

    let inbound: Arc<Mutex<VecDeque<Vec<u8>>>> = Arc::new(Mutex::new(VecDeque::new()));
    let close_flag = Arc::new(AtomicBool::new(false));
    let (outbound_tx, mut outbound_rx) = tokio::sync::mpsc::channel::<Vec<u8>>(128);
    let (ready_tx, ready_rx) = std::sync::mpsc::sync_channel(1);

    if cmd_tx
        .send(NewConn {
            target,
            src_port,
            inbound: Arc::clone(&inbound),
            outbound_tx,
            close_flag: Arc::clone(&close_flag),
            ready_tx,
        })
        .is_err()
    {
        let _ = session.send_reply(REP_GENERAL_FAILURE).await;
        return;
    }

    // Block a threadpool thread while the smoltcp TCP handshake completes
    let connected = tokio::task::spawn_blocking(move || {
        ready_rx.recv_timeout(CONNECT_TIMEOUT).unwrap_or(false)
    })
    .await
    .unwrap_or(false);

    if !connected {
        let _ = session.send_reply(REP_HOST_UNREACHABLE).await;
        return;
    }

    if session.send_reply(REP_SUCCESS).await.is_err() {
        close_flag.store(true, Ordering::Relaxed);
        return;
    }

    // Bidirectional bridge: SOCKS5 TCP stream ↔ smoltcp per-socket queues
    let (mut socks_rd, mut socks_wr) = session.stream.into_split();
    let inbound_clone = Arc::clone(&inbound);
    let close_flag_rd = Arc::clone(&close_flag);

    let read_half = tokio::spawn(async move {
        let mut buf = vec![0u8; 8192];
        loop {
            match socks_rd.read(&mut buf).await {
                Ok(0) | Err(_) => break,
                Ok(n) => {
                    inbound_clone.lock().unwrap().push_back(buf[..n].to_vec());
                }
            }
        }
        close_flag_rd.store(true, Ordering::Relaxed);
    });

    let write_half = tokio::spawn(async move {
        while let Some(data) = outbound_rx.recv().await {
            if socks_wr.write_all(&data).await.is_err() {
                break;
            }
        }
    });

    tokio::select! {
        _ = read_half => {}
        _ = write_half => {}
    }

    close_flag.store(true, Ordering::Relaxed);
}
