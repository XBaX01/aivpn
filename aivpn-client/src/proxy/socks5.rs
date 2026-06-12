use std::net::SocketAddr;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

const SOCKS5_VER: u8 = 5;
const METHOD_NO_AUTH: u8 = 0;
const METHOD_NO_ACCEPTABLE: u8 = 0xFF;
const CMD_CONNECT: u8 = 1;
const ATYP_IPV4: u8 = 1;
const ATYP_DOMAIN: u8 = 3;
const ATYP_IPV6: u8 = 4;

pub const REP_SUCCESS: u8 = 0;
pub const REP_GENERAL_FAILURE: u8 = 1;
pub const REP_HOST_UNREACHABLE: u8 = 4;
pub const REP_CMD_NOT_SUPPORTED: u8 = 7;

pub struct Socks5Session {
    pub stream: TcpStream,
}

impl Socks5Session {
    pub fn new(stream: TcpStream) -> Self {
        Self { stream }
    }

    /// Perform SOCKS5 handshake and return the CONNECT target address.
    pub async fn negotiate(&mut self) -> std::io::Result<SocketAddr> {
        // Client greeting: VER NMETHODS METHODS...
        let mut header = [0u8; 2];
        self.stream.read_exact(&mut header).await?;
        if header[0] != SOCKS5_VER {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "not SOCKS5",
            ));
        }
        let n = header[1] as usize;
        let mut methods = vec![0u8; n];
        self.stream.read_exact(&mut methods).await?;

        if !methods.contains(&METHOD_NO_AUTH) {
            self.stream
                .write_all(&[SOCKS5_VER, METHOD_NO_ACCEPTABLE])
                .await?;
            return Err(std::io::Error::new(
                std::io::ErrorKind::PermissionDenied,
                "no acceptable auth method",
            ));
        }
        self.stream.write_all(&[SOCKS5_VER, METHOD_NO_AUTH]).await?;

        // Request: VER CMD RSV ATYP ...
        let mut req = [0u8; 4];
        self.stream.read_exact(&mut req).await?;
        if req[0] != SOCKS5_VER || req[1] != CMD_CONNECT {
            self.send_reply(REP_CMD_NOT_SUPPORTED).await?;
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "only CONNECT is supported",
            ));
        }

        let ip: std::net::IpAddr = match req[3] {
            ATYP_IPV4 => {
                let mut a = [0u8; 4];
                self.stream.read_exact(&mut a).await?;
                std::net::IpAddr::V4(std::net::Ipv4Addr::from(a))
            }
            ATYP_DOMAIN => {
                let mut len_buf = [0u8; 1];
                self.stream.read_exact(&mut len_buf).await?;
                let mut name = vec![0u8; len_buf[0] as usize];
                self.stream.read_exact(&mut name).await?;
                let host = String::from_utf8_lossy(&name);
                // Resolve hostname via system DNS on the async runtime
                tokio::net::lookup_host(format!("{}:0", host))
                    .await?
                    .find(|a| a.is_ipv4())
                    .map(|a| a.ip())
                    .ok_or_else(|| {
                        std::io::Error::new(
                            std::io::ErrorKind::NotFound,
                            format!("DNS: no IPv4 result for {host}"),
                        )
                    })?
            }
            ATYP_IPV6 => {
                let mut a = [0u8; 16];
                self.stream.read_exact(&mut a).await?;
                std::net::IpAddr::V6(std::net::Ipv6Addr::from(a))
            }
            _ => {
                self.send_reply(REP_GENERAL_FAILURE).await?;
                return Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    "unknown ATYP",
                ));
            }
        };

        let mut port_buf = [0u8; 2];
        self.stream.read_exact(&mut port_buf).await?;
        let port = u16::from_be_bytes(port_buf);

        Ok(SocketAddr::new(ip, port))
    }

    /// Send SOCKS5 reply. Bound address is reported as 0.0.0.0:0.
    pub async fn send_reply(&mut self, rep: u8) -> std::io::Result<()> {
        // VER REP RSV ATYP BND.ADDR(4) BND.PORT(2)
        self.stream
            .write_all(&[SOCKS5_VER, rep, 0x00, ATYP_IPV4, 0, 0, 0, 0, 0, 0])
            .await
    }
}
