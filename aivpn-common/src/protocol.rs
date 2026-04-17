//! AIVPN Wire Protocol
//! 
//! Implements packet format, inner payload encoding, and control messages

use bytes::{BufMut, BytesMut};
use rand::RngCore;
use serde::{Deserialize, Serialize};

use crate::crypto::{POLY1305_TAG_SIZE, TAG_SIZE};
use crate::error::{Error, Result};
use crate::network_config::ClientNetworkConfig;

/// Maximum UDP packet size (optimized for VPN MTU 1420 + overhead)
pub const MAX_PACKET_SIZE: usize = 1500;

/// Minimum header overhead (tag + pad_len + inner_header + poly1305)
pub const MIN_HEADER_OVERHEAD: usize = TAG_SIZE + 2 + 4 + POLY1305_TAG_SIZE;

/// Maximum payload size
pub const MAX_PAYLOAD_SIZE: usize = MAX_PACKET_SIZE - MIN_HEADER_OVERHEAD;

/// Inner payload types
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[repr(u16)]
pub enum InnerType {
    Data = 0x0001,
    Control = 0x0002,
    Fragment = 0x0003,
    Ack = 0x0004,
}

impl InnerType {
    pub fn from_u16(value: u16) -> Option<Self> {
        match value {
            0x0001 => Some(Self::Data),
            0x0002 => Some(Self::Control),
            0x0003 => Some(Self::Fragment),
            0x0004 => Some(Self::Ack),
            _ => None,
        }
    }
}

/// Control message subtypes
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[repr(u8)]
pub enum ControlSubtype {
    KeyRotate = 0x01,
    MaskUpdate = 0x02,
    Keepalive = 0x03,
    TelemetryRequest = 0x04,
    TelemetryResponse = 0x05,
    TimeSync = 0x06,
    Shutdown = 0x07,
    ControlAck = 0x08,
    ServerHello = 0x09,
    RecordingStart = 0x0A,
    RecordingAck = 0x0B,
    RecordingStop = 0x0C,
    RecordingComplete = 0x0D,
    RecordingFailed = 0x0E,
    RecordingStatusRequest = 0x0F,
    RecordingStatus = 0x10,
}

impl ControlSubtype {
    pub fn from_u8(value: u8) -> Option<Self> {
        match value {
            0x01 => Some(Self::KeyRotate),
            0x02 => Some(Self::MaskUpdate),
            0x03 => Some(Self::Keepalive),
            0x04 => Some(Self::TelemetryRequest),
            0x05 => Some(Self::TelemetryResponse),
            0x06 => Some(Self::TimeSync),
            0x07 => Some(Self::Shutdown),
            0x08 => Some(Self::ControlAck),
            0x09 => Some(Self::ServerHello),
            0x0A => Some(Self::RecordingStart),
            0x0B => Some(Self::RecordingAck),
            0x0C => Some(Self::RecordingStop),
            0x0D => Some(Self::RecordingComplete),
            0x0E => Some(Self::RecordingFailed),
            0x0F => Some(Self::RecordingStatusRequest),
            0x10 => Some(Self::RecordingStatus),
            _ => None,
        }
    }
}

/// Inner payload header (after decryption)
#[derive(Debug, Clone)]
pub struct InnerHeader {
    pub inner_type: InnerType,
    pub seq_num: u16,
}

impl InnerHeader {
    pub fn encode(&self) -> [u8; 4] {
        let mut buf = [0u8; 4];
        buf[0..2].copy_from_slice(&(self.inner_type as u16).to_le_bytes());
        buf[2..4].copy_from_slice(&self.seq_num.to_le_bytes());
        buf
    }

    pub fn decode(data: &[u8]) -> Result<Self> {
        if data.len() < 4 {
            return Err(Error::InvalidPacket("Inner header too short"));
        }
        let inner_type = InnerType::from_u16(u16::from_le_bytes([data[0], data[1]]))
            .ok_or(Error::InvalidPacket("Unknown inner type"))?;
        let seq_num = u16::from_le_bytes([data[2], data[3]]);
        Ok(Self { inner_type, seq_num })
    }
}

/// AIVPN Packet structure
#[derive(Debug, Clone)]
pub struct AivpnPacket {
    pub resonance_tag: [u8; TAG_SIZE],
    pub mask_dependent_header: Vec<u8>,
    pub pad_len: u16,
    pub encrypted_payload: Vec<u8>,
    pub random_padding: Vec<u8>,
}

impl AivpnPacket {
    pub fn new(
        resonance_tag: [u8; TAG_SIZE],
        mask_dependent_header: Vec<u8>,
        encrypted_payload: Vec<u8>,
        padding_len: u16,
    ) -> Self {
        Self {
            resonance_tag,
            mask_dependent_header,
            pad_len: padding_len,
            encrypted_payload,
            random_padding: {
                let mut pad = vec![0u8; padding_len as usize];
                rand::thread_rng().fill_bytes(&mut pad);
                pad
            },
        }
    }

    /// Serialize packet to bytes
    pub fn to_bytes(&self) -> BytesMut {
        let total_len = TAG_SIZE 
            + self.mask_dependent_header.len() 
            + 2 // pad_len
            + self.encrypted_payload.len() 
            + self.random_padding.len();
        
        let mut buf = BytesMut::with_capacity(total_len);
        buf.put_slice(&self.resonance_tag);
        buf.put_slice(&self.mask_dependent_header);
        buf.put_u16_le(self.pad_len);
        buf.put_slice(&self.encrypted_payload);
        buf.put_slice(&self.random_padding);
        buf
    }

    /// Deserialize packet from bytes
    pub fn from_bytes(data: &[u8]) -> Result<Self> {
        if data.len() < TAG_SIZE + 2 {
            return Err(Error::InvalidPacket("Packet too short"));
        }

        let mut cursor = 0;

        // Resonance tag
        let mut resonance_tag = [0u8; TAG_SIZE];
        resonance_tag.copy_from_slice(&data[cursor..cursor + TAG_SIZE]);
        cursor += TAG_SIZE;

        // We need to know the mask-dependent header length to parse correctly
        // This is determined by the active Mask profile
        // For now, we'll parse it in the server/client with mask context
        // Return raw data for upper layers to parse
        let _remaining = &data[cursor..];
        
        Ok(Self {
            resonance_tag,
            mask_dependent_header: Vec::new(),
            pad_len: 0,
            encrypted_payload: Vec::new(),
            random_padding: Vec::new(),
        })
    }

    /// Parse with mask context (knowing MDH length)
    pub fn from_bytes_with_mdh_len(data: &[u8], mdh_len: usize) -> Result<Self> {
        if data.len() < TAG_SIZE + mdh_len + 2 {
            return Err(Error::InvalidPacket("Packet too short"));
        }

        let mut cursor = 0;

        // Resonance tag
        let mut resonance_tag = [0u8; TAG_SIZE];
        resonance_tag.copy_from_slice(&data[cursor..cursor + TAG_SIZE]);
        cursor += TAG_SIZE;

        // Mask-dependent header
        let mask_dependent_header = data[cursor..cursor + mdh_len].to_vec();
        cursor += mdh_len;

        // Pad length
        let pad_len = u16::from_le_bytes([data[cursor], data[cursor + 1]]);
        cursor += 2;

        // Encrypted payload (everything except padding)
        let payload_len = data.len() - cursor - pad_len as usize;
        let encrypted_payload = data[cursor..cursor + payload_len].to_vec();
        cursor += payload_len;

        // Random padding
        let random_padding = data[cursor..].to_vec();

        Ok(Self {
            resonance_tag,
            mask_dependent_header,
            pad_len,
            encrypted_payload,
            random_padding,
        })
    }
}

/// Control message payload
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ControlPayload {
    KeyRotate {
        new_eph_pub: [u8; 32],
    },
    MaskUpdate {
        mask_data: Vec<u8>,
        #[serde(with = "serde_bytes")]
        signature: [u8; 64],
    },
    Keepalive,
    TelemetryRequest {
        metric_flags: u8,
    },
    TelemetryResponse {
        packet_loss: u16,
        rtt_ms: u16,
        jitter_ms: u16,
        buffer_pct: u8,
    },
    TimeSync {
        server_ts_ms: u64,
    },
    Shutdown {
        reason: u8,
    },
    ControlAck {
        ack_seq: u16,
        ack_for_subtype: u8,
    },
    ServerHello {
        server_eph_pub: [u8; 32],
        #[serde(with = "serde_bytes")]
        signature: [u8; 64],
        network_config: Option<ClientNetworkConfig>,
    },
    /// Admin requests recording start for a service
    RecordingStart {
        service: String,
    },
    /// Server acknowledges recording start
    RecordingAck {
        session_id: [u8; 16],
        status: String,
    },
    /// Admin requests recording stop
    RecordingStop {
        session_id: [u8; 16],
    },
    /// Server reports mask generation complete
    RecordingComplete {
        service: String,
        mask_id: String,
        confidence: f32,
    },
    /// Server reports recording/generation failure
    RecordingFailed {
        reason: String,
    },
    /// Client asks whether the current authenticated session may record masks.
    RecordingStatusRequest,
    /// Server reports recording capability and current active recording state.
    RecordingStatus {
        can_record: bool,
        active_service: Option<String>,
    },
}

impl ControlPayload {
    pub fn encode(&self) -> Result<Vec<u8>> {
        let mut buf = Vec::new();
        
        match self {
            Self::KeyRotate { new_eph_pub } => {
                buf.push(ControlSubtype::KeyRotate as u8);
                buf.push(0); // reserved
                buf.extend_from_slice(&(new_eph_pub.len() as u16).to_le_bytes());
                buf.extend_from_slice(new_eph_pub);
            }
            Self::MaskUpdate { mask_data, signature } => {
                buf.push(ControlSubtype::MaskUpdate as u8);
                buf.extend_from_slice(&(mask_data.len() as u16).to_le_bytes());
                buf.extend_from_slice(mask_data);
                buf.extend_from_slice(signature);
            }
            Self::Keepalive => {
                buf.push(ControlSubtype::Keepalive as u8);
            }
            Self::TelemetryRequest { metric_flags } => {
                buf.push(ControlSubtype::TelemetryRequest as u8);
                buf.push(*metric_flags);
            }
            Self::TelemetryResponse { packet_loss, rtt_ms, jitter_ms, buffer_pct } => {
                buf.push(ControlSubtype::TelemetryResponse as u8);
                buf.push(0); // flags
                buf.extend_from_slice(&packet_loss.to_le_bytes());
                buf.extend_from_slice(&rtt_ms.to_le_bytes());
                buf.extend_from_slice(&jitter_ms.to_le_bytes());
                buf.push(*buffer_pct);
                buf.extend_from_slice(&[0u8; 3]); // reserved
            }
            Self::TimeSync { server_ts_ms } => {
                buf.push(ControlSubtype::TimeSync as u8);
                buf.extend_from_slice(&server_ts_ms.to_le_bytes());
            }
            Self::Shutdown { reason } => {
                buf.push(ControlSubtype::Shutdown as u8);
                buf.push(*reason);
            }
            Self::ControlAck { ack_seq, ack_for_subtype } => {
                buf.push(ControlSubtype::ControlAck as u8);
                buf.extend_from_slice(&ack_seq.to_le_bytes());
                buf.push(*ack_for_subtype);
            }
            Self::ServerHello { server_eph_pub, signature, network_config } => {
                buf.push(ControlSubtype::ServerHello as u8);
                buf.extend_from_slice(server_eph_pub);
                buf.extend_from_slice(signature);
                if let Some(network_config) = network_config {
                    buf.extend_from_slice(&network_config.encode_wire());
                }
            }
            Self::RecordingStart { service } => {
                buf.push(ControlSubtype::RecordingStart as u8);
                let service_bytes = service.as_bytes();
                buf.extend_from_slice(&(service_bytes.len() as u16).to_le_bytes());
                buf.extend_from_slice(service_bytes);
            }
            Self::RecordingAck { session_id, status } => {
                buf.push(ControlSubtype::RecordingAck as u8);
                buf.extend_from_slice(session_id);
                let status_bytes = status.as_bytes();
                buf.extend_from_slice(&(status_bytes.len() as u16).to_le_bytes());
                buf.extend_from_slice(status_bytes);
            }
            Self::RecordingStop { session_id } => {
                buf.push(ControlSubtype::RecordingStop as u8);
                buf.extend_from_slice(session_id);
            }
            Self::RecordingComplete { service, mask_id, confidence } => {
                buf.push(ControlSubtype::RecordingComplete as u8);
                let service_bytes = service.as_bytes();
                buf.extend_from_slice(&(service_bytes.len() as u16).to_le_bytes());
                buf.extend_from_slice(service_bytes);
                let mask_id_bytes = mask_id.as_bytes();
                buf.extend_from_slice(&(mask_id_bytes.len() as u16).to_le_bytes());
                buf.extend_from_slice(mask_id_bytes);
                buf.extend_from_slice(&confidence.to_le_bytes());
            }
            Self::RecordingFailed { reason } => {
                buf.push(ControlSubtype::RecordingFailed as u8);
                let reason_bytes = reason.as_bytes();
                buf.extend_from_slice(&(reason_bytes.len() as u16).to_le_bytes());
                buf.extend_from_slice(reason_bytes);
            }
            Self::RecordingStatusRequest => {
                buf.push(ControlSubtype::RecordingStatusRequest as u8);
            }
            Self::RecordingStatus { can_record, active_service } => {
                buf.push(ControlSubtype::RecordingStatus as u8);
                let mut flags = 0u8;
                if *can_record {
                    flags |= 0x01;
                }
                if active_service.is_some() {
                    flags |= 0x02;
                }
                buf.push(flags);
                if let Some(service) = active_service {
                    let service_bytes = service.as_bytes();
                    buf.extend_from_slice(&(service_bytes.len() as u16).to_le_bytes());
                    buf.extend_from_slice(service_bytes);
                }
            }
        }
        
        Ok(buf)
    }

    pub fn decode(data: &[u8]) -> Result<Self> {
        if data.is_empty() {
            return Err(Error::InvalidPacket("Empty control payload"));
        }

        let subtype = ControlSubtype::from_u8(data[0])
            .ok_or(Error::InvalidPacket("Unknown control subtype"))?;

        match subtype {
            ControlSubtype::KeyRotate => {
                if data.len() < 6 {
                    return Err(Error::InvalidPacket("KeyRotate too short"));
                }
                let new_eph_pub_len = u16::from_le_bytes([data[2], data[3]]) as usize;
                if data.len() < 4 + new_eph_pub_len {
                    return Err(Error::InvalidPacket("KeyRotate invalid length"));
                }
                let mut new_eph_pub = [0u8; 32];
                new_eph_pub.copy_from_slice(&data[4..4 + 32]);
                Ok(Self::KeyRotate { new_eph_pub })
            }
            ControlSubtype::MaskUpdate => {
                if data.len() < 4 {
                    return Err(Error::InvalidPacket("MaskUpdate too short"));
                }
                let mask_len = u16::from_le_bytes([data[1], data[2]]) as usize;
                if data.len() < 3 + mask_len + 64 {
                    return Err(Error::InvalidPacket("MaskUpdate invalid length"));
                }
                let mask_data = data[3..3 + mask_len].to_vec();
                let mut signature = [0u8; 64];
                signature.copy_from_slice(&data[3 + mask_len..3 + mask_len + 64]);
                Ok(Self::MaskUpdate { mask_data, signature })
            }
            ControlSubtype::Keepalive => Ok(Self::Keepalive),
            ControlSubtype::TelemetryRequest => {
                if data.len() < 2 {
                    return Err(Error::InvalidPacket("TelemetryRequest too short"));
                }
                Ok(Self::TelemetryRequest { metric_flags: data[1] })
            }
            ControlSubtype::TelemetryResponse => {
                if data.len() < 12 {
                    return Err(Error::InvalidPacket("TelemetryResponse too short"));
                }
                Ok(Self::TelemetryResponse {
                    packet_loss: u16::from_le_bytes([data[2], data[3]]),
                    rtt_ms: u16::from_le_bytes([data[4], data[5]]),
                    jitter_ms: u16::from_le_bytes([data[6], data[7]]),
                    buffer_pct: data[8],
                })
            }
            ControlSubtype::TimeSync => {
                if data.len() < 9 {
                    return Err(Error::InvalidPacket("TimeSync too short"));
                }
                Ok(Self::TimeSync { 
                    server_ts_ms: u64::from_le_bytes(data[1..9].try_into().unwrap()) 
                })
            }
            ControlSubtype::Shutdown => {
                if data.len() < 2 {
                    return Err(Error::InvalidPacket("Shutdown too short"));
                }
                Ok(Self::Shutdown { reason: data[1] })
            }
            ControlSubtype::ControlAck => {
                if data.len() < 4 {
                    return Err(Error::InvalidPacket("ControlAck too short"));
                }
                Ok(Self::ControlAck {
                    ack_seq: u16::from_le_bytes([data[1], data[2]]),
                    ack_for_subtype: data[3],
                })
            }
            ControlSubtype::ServerHello => {
                if data.len() < 1 + 32 + 64 {
                    return Err(Error::InvalidPacket("ServerHello too short"));
                }
                let mut server_eph_pub = [0u8; 32];
                server_eph_pub.copy_from_slice(&data[1..33]);
                let mut signature = [0u8; 64];
                signature.copy_from_slice(&data[33..97]);
                let network_config = if data.len() >= 97 + ClientNetworkConfig::WIRE_SIZE {
                    Some(ClientNetworkConfig::decode_wire(
                        &data[97..97 + ClientNetworkConfig::WIRE_SIZE],
                    )?)
                } else {
                    None
                };
                Ok(Self::ServerHello {
                    server_eph_pub,
                    signature,
                    network_config,
                })
            }
            ControlSubtype::RecordingStart => {
                if data.len() < 3 {
                    return Err(Error::InvalidPacket("RecordingStart too short"));
                }
                let svc_len = u16::from_le_bytes([data[1], data[2]]) as usize;
                if data.len() < 3 + svc_len {
                    return Err(Error::InvalidPacket("RecordingStart invalid length"));
                }
                let service = String::from_utf8_lossy(&data[3..3 + svc_len]).to_string();
                Ok(Self::RecordingStart { service })
            }
            ControlSubtype::RecordingAck => {
                if data.len() < 1 + 16 + 2 {
                    return Err(Error::InvalidPacket("RecordingAck too short"));
                }
                let mut session_id = [0u8; 16];
                session_id.copy_from_slice(&data[1..17]);
                let status_len = u16::from_le_bytes([data[17], data[18]]) as usize;
                if data.len() < 19 + status_len {
                    return Err(Error::InvalidPacket("RecordingAck invalid length"));
                }
                let status = String::from_utf8_lossy(&data[19..19 + status_len]).to_string();
                Ok(Self::RecordingAck { session_id, status })
            }
            ControlSubtype::RecordingStop => {
                if data.len() < 17 {
                    return Err(Error::InvalidPacket("RecordingStop too short"));
                }
                let mut session_id = [0u8; 16];
                session_id.copy_from_slice(&data[1..17]);
                Ok(Self::RecordingStop { session_id })
            }
            ControlSubtype::RecordingComplete => {
                if data.len() < 3 {
                    return Err(Error::InvalidPacket("RecordingComplete too short"));
                }
                let mut cursor = 1;
                let svc_len = u16::from_le_bytes([data[cursor], data[cursor + 1]]) as usize;
                cursor += 2;
                if data.len() < cursor + svc_len + 2 + 4 {
                    return Err(Error::InvalidPacket("RecordingComplete invalid length"));
                }
                let service = String::from_utf8_lossy(&data[cursor..cursor + svc_len]).to_string();
                cursor += svc_len;
                let mid_len = u16::from_le_bytes([data[cursor], data[cursor + 1]]) as usize;
                cursor += 2;
                if data.len() < cursor + mid_len + 4 {
                    return Err(Error::InvalidPacket("RecordingComplete invalid mask_id"));
                }
                let mask_id = String::from_utf8_lossy(&data[cursor..cursor + mid_len]).to_string();
                cursor += mid_len;
                let confidence = f32::from_le_bytes([
                    data[cursor], data[cursor + 1], data[cursor + 2], data[cursor + 3],
                ]);
                Ok(Self::RecordingComplete { service, mask_id, confidence })
            }
            ControlSubtype::RecordingFailed => {
                if data.len() < 3 {
                    return Err(Error::InvalidPacket("RecordingFailed too short"));
                }
                let reason_len = u16::from_le_bytes([data[1], data[2]]) as usize;
                if data.len() < 3 + reason_len {
                    return Err(Error::InvalidPacket("RecordingFailed invalid length"));
                }
                let reason = String::from_utf8_lossy(&data[3..3 + reason_len]).to_string();
                Ok(Self::RecordingFailed { reason })
            }
            ControlSubtype::RecordingStatusRequest => Ok(Self::RecordingStatusRequest),
            ControlSubtype::RecordingStatus => {
                if data.len() < 2 {
                    return Err(Error::InvalidPacket("RecordingStatus too short"));
                }
                let flags = data[1];
                let can_record = (flags & 0x01) != 0;
                let has_service = (flags & 0x02) != 0;
                let active_service = if has_service {
                    if data.len() < 4 {
                        return Err(Error::InvalidPacket("RecordingStatus missing service length"));
                    }
                    let service_len = u16::from_le_bytes([data[2], data[3]]) as usize;
                    if data.len() < 4 + service_len {
                        return Err(Error::InvalidPacket("RecordingStatus invalid service length"));
                    }
                    Some(String::from_utf8_lossy(&data[4..4 + service_len]).to_string())
                } else {
                    None
                };
                Ok(Self::RecordingStatus { can_record, active_service })
            }
        }
    }
}

/// ACK packet for selective acknowledgment
#[derive(Debug, Clone)]
pub struct AckPacket {
    pub ack_seq: u16,
    pub ack_base: u16,
    pub bitmap: Vec<u8>,
}

impl AckPacket {
    pub fn new(ack_seq: u16, ack_base: u16, bitmap: Vec<u8>) -> Self {
        Self { ack_seq, ack_base, bitmap }
    }

    pub fn encode(&self) -> Vec<u8> {
        let mut buf = Vec::with_capacity(5 + self.bitmap.len());
        buf.extend_from_slice(&(InnerType::Ack as u16).to_le_bytes());
        buf.extend_from_slice(&self.ack_seq.to_le_bytes());
        buf.extend_from_slice(&self.ack_base.to_le_bytes());
        buf.push(self.bitmap.len() as u8);
        buf.extend_from_slice(&self.bitmap);
        buf
    }

    pub fn decode(data: &[u8]) -> Result<Self> {
        if data.len() < 5 {
            return Err(Error::InvalidPacket("ACK too short"));
        }
        let ack_seq = u16::from_le_bytes([data[2], data[3]]);
        let ack_base = u16::from_le_bytes([data[4], data[5]]);
        let bitmap_len = data[6] as usize;
        if data.len() < 7 + bitmap_len {
            return Err(Error::InvalidPacket("ACK invalid length"));
        }
        let bitmap = data[7..7 + bitmap_len].to_vec();
        Ok(Self { ack_seq, ack_base, bitmap })
    }
}
