use std::net::Ipv4Addr;

use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};

pub const DEFAULT_VPN_MTU: u16 = 1346;
pub const LEGACY_VPN_PREFIX_LEN: u8 = 24;
pub const LEGACY_SERVER_VPN_IP: Ipv4Addr = Ipv4Addr::new(10, 0, 0, 1);

fn default_server_vpn_ip() -> Ipv4Addr {
    LEGACY_SERVER_VPN_IP
}

fn default_prefix_len() -> u8 {
    LEGACY_VPN_PREFIX_LEN
}

fn default_mtu() -> u16 {
    DEFAULT_VPN_MTU
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct VpnNetworkConfig {
    #[serde(default = "default_server_vpn_ip")]
    pub server_vpn_ip: Ipv4Addr,
    #[serde(default = "default_prefix_len")]
    pub prefix_len: u8,
    #[serde(default = "default_mtu")]
    pub mtu: u16,
}

impl Default for VpnNetworkConfig {
    fn default() -> Self {
        Self {
            server_vpn_ip: default_server_vpn_ip(),
            prefix_len: default_prefix_len(),
            mtu: default_mtu(),
        }
    }
}

impl VpnNetworkConfig {
    pub fn validate(&self) -> Result<()> {
        if !(1..=30).contains(&self.prefix_len) {
            return Err(Error::InvalidPacket("VPN prefix length must be in range 1..=30"));
        }
        if self.server_vpn_ip == self.network_addr() || self.server_vpn_ip == self.broadcast_addr() {
            return Err(Error::InvalidPacket("Server VPN IP must be a usable host address"));
        }
        Ok(())
    }

    pub fn netmask(&self) -> Ipv4Addr {
        prefix_len_to_netmask(self.prefix_len)
    }

    pub fn network_addr(&self) -> Ipv4Addr {
        Ipv4Addr::from(ipv4_to_u32(self.server_vpn_ip) & self.mask_u32())
    }

    pub fn broadcast_addr(&self) -> Ipv4Addr {
        Ipv4Addr::from(self.network_u32() | !self.mask_u32())
    }

    pub fn contains(&self, ip: Ipv4Addr) -> bool {
        (ipv4_to_u32(ip) & self.mask_u32()) == self.network_u32()
    }

    pub fn cidr_string(&self) -> String {
        format!("{}/{}", self.network_addr(), self.prefix_len)
    }

    pub fn server_ip_string(&self) -> String {
        self.server_vpn_ip.to_string()
    }

    pub fn netmask_string(&self) -> String {
        self.netmask().to_string()
    }

    pub fn host_offset(&self, ip: Ipv4Addr) -> u32 {
        ipv4_to_u32(ip) & !self.mask_u32()
    }

    pub fn max_host_offset(&self) -> u32 {
        let host_mask = !self.mask_u32();
        host_mask.saturating_sub(1)
    }

    pub fn is_usable_host(&self, ip: Ipv4Addr) -> bool {
        self.contains(ip) && ip != self.network_addr() && ip != self.broadcast_addr()
    }

    pub fn ip_for_host_offset(&self, host_offset: u32) -> Option<Ipv4Addr> {
        if host_offset == 0 || host_offset > self.max_host_offset() {
            return None;
        }
        Some(Ipv4Addr::from(self.network_u32() | host_offset))
    }

    pub fn client_config(&self, client_ip: Ipv4Addr) -> Result<ClientNetworkConfig> {
        if !self.is_usable_host(client_ip) {
            return Err(Error::InvalidPacket("Client VPN IP is outside configured VPN subnet"));
        }
        if client_ip == self.server_vpn_ip {
            return Err(Error::InvalidPacket("Client VPN IP cannot equal server VPN IP"));
        }
        Ok(ClientNetworkConfig {
            client_ip,
            server_vpn_ip: self.server_vpn_ip,
            prefix_len: self.prefix_len,
            mtu: self.mtu,
        })
    }

    fn network_u32(&self) -> u32 {
        ipv4_to_u32(self.server_vpn_ip) & self.mask_u32()
    }

    fn mask_u32(&self) -> u32 {
        if self.prefix_len == 0 {
            0
        } else {
            u32::MAX << (32 - self.prefix_len)
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClientNetworkConfig {
    pub client_ip: Ipv4Addr,
    pub server_vpn_ip: Ipv4Addr,
    pub prefix_len: u8,
    #[serde(default = "default_mtu")]
    pub mtu: u16,
}

impl ClientNetworkConfig {
    pub const WIRE_SIZE: usize = 12;
    const WIRE_VERSION: u8 = 1;

    pub fn validate(&self) -> Result<()> {
        VpnNetworkConfig {
            server_vpn_ip: self.server_vpn_ip,
            prefix_len: self.prefix_len,
            mtu: self.mtu,
        }
        .client_config(self.client_ip)
        .map(|_| ())
    }

    pub fn netmask(&self) -> Ipv4Addr {
        prefix_len_to_netmask(self.prefix_len)
    }

    pub fn cidr_string(&self) -> String {
        format!("{}/{}", self.client_ip, self.prefix_len)
    }

    pub fn netmask_string(&self) -> String {
        self.netmask().to_string()
    }

    pub fn encode_wire(&self) -> [u8; Self::WIRE_SIZE] {
        let mut buf = [0u8; Self::WIRE_SIZE];
        buf[0] = Self::WIRE_VERSION;
        buf[1] = self.prefix_len;
        buf[2..4].copy_from_slice(&self.mtu.to_le_bytes());
        buf[4..8].copy_from_slice(&self.server_vpn_ip.octets());
        buf[8..12].copy_from_slice(&self.client_ip.octets());
        buf
    }

    pub fn decode_wire(data: &[u8]) -> Result<Self> {
        if data.len() != Self::WIRE_SIZE {
            return Err(Error::InvalidPacket("Client network config has invalid wire length"));
        }
        if data[0] != Self::WIRE_VERSION {
            return Err(Error::InvalidPacket("Unsupported client network config wire version"));
        }

        let config = Self {
            prefix_len: data[1],
            mtu: u16::from_le_bytes([data[2], data[3]]),
            server_vpn_ip: Ipv4Addr::new(data[4], data[5], data[6], data[7]),
            client_ip: Ipv4Addr::new(data[8], data[9], data[10], data[11]),
        };
        config.validate()?;
        Ok(config)
    }
}

pub fn prefix_len_to_netmask(prefix_len: u8) -> Ipv4Addr {
    if prefix_len == 0 {
        return Ipv4Addr::new(0, 0, 0, 0);
    }

    Ipv4Addr::from(u32::MAX << (32 - prefix_len))
}

pub fn netmask_to_prefix_len(netmask: Ipv4Addr) -> Result<u8> {
    let mask = ipv4_to_u32(netmask);
    let prefix_len = mask.leading_ones() as u8;
    let expected = if prefix_len == 0 {
        0
    } else {
        u32::MAX << (32 - prefix_len)
    };
    if mask != expected {
        return Err(Error::InvalidPacket("VPN netmask must be contiguous"));
    }
    Ok(prefix_len)
}

fn ipv4_to_u32(ip: Ipv4Addr) -> u32 {
    u32::from(ip)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wire_roundtrip_preserves_client_network_config() {
        let config = ClientNetworkConfig {
            client_ip: Ipv4Addr::new(10, 150, 0, 2),
            server_vpn_ip: Ipv4Addr::new(10, 150, 0, 1),
            prefix_len: 24,
            mtu: 1346,
        };

        let decoded = ClientNetworkConfig::decode_wire(&config.encode_wire()).unwrap();
        assert_eq!(decoded, config);
    }

    #[test]
    fn network_helpers_compute_addresses() {
        let config = VpnNetworkConfig {
            server_vpn_ip: Ipv4Addr::new(10, 150, 0, 1),
            prefix_len: 24,
            mtu: 1346,
        };

        assert_eq!(config.network_addr(), Ipv4Addr::new(10, 150, 0, 0));
        assert_eq!(config.broadcast_addr(), Ipv4Addr::new(10, 150, 0, 255));
        assert_eq!(config.netmask(), Ipv4Addr::new(255, 255, 255, 0));
        assert!(config.is_usable_host(Ipv4Addr::new(10, 150, 0, 10)));
        assert!(!config.is_usable_host(Ipv4Addr::new(10, 150, 0, 0)));
    }
}