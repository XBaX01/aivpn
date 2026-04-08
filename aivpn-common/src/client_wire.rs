use rand::RngCore;

use crate::crypto::{
    self, current_timestamp_ms, compute_time_window, decrypt_payload, derive_session_keys,
    encrypt_payload, generate_resonance_tag, KeyPair, SessionKeys, DEFAULT_WINDOW_MS,
    NONCE_SIZE, TAG_SIZE,
};
use crate::error::{Error, Result};
use crate::protocol::{ControlPayload, InnerHeader, InnerType};

pub const DEFAULT_ZERO_MDH: [u8; 4] = [0u8; 4];

pub struct DecodedPacket {
    pub counter: u64,
    pub header: InnerHeader,
    pub payload: Vec<u8>,
}

const RECV_REORDER_WINDOW: usize = 256;
const RECV_FUTURE_SEARCH_WINDOW: usize = 4096;

#[derive(Clone, Copy)]
struct Bitset256 {
    lo: u128,
    hi: u128,
}

impl Bitset256 {
    fn new() -> Self {
        Self { lo: 0, hi: 0 }
    }

    fn clear(&mut self) {
        self.lo = 0;
        self.hi = 0;
    }

    fn shl(self, shift: u64) -> Self {
        if shift >= 256 {
            return Self::new();
        }
        if shift == 0 {
            return self;
        }
        if shift >= 128 {
            return Self {
                lo: 0,
                hi: self.lo << (shift - 128),
            };
        }

        Self {
            lo: self.lo << shift,
            hi: (self.hi << shift) | (self.lo >> (128 - shift)),
        }
    }

    fn set_bit(&mut self, bit: usize) {
        if bit < 128 {
            self.lo |= 1u128 << bit;
        } else if bit < 256 {
            self.hi |= 1u128 << (bit - 128);
        }
    }

    fn get_bit(&self, bit: usize) -> bool {
        if bit < 128 {
            (self.lo >> bit) & 1 == 1
        } else if bit < 256 {
            (self.hi >> (bit - 128)) & 1 == 1
        } else {
            false
        }
    }
}

pub struct RecvWindow {
    highest: i64,
    bitmap: Bitset256,
}

impl Default for RecvWindow {
    fn default() -> Self {
        Self::new()
    }
}

impl RecvWindow {
    pub fn new() -> Self {
        Self {
            highest: -1,
            bitmap: Bitset256::new(),
        }
    }

    pub fn reset(&mut self) {
        self.highest = -1;
        self.bitmap.clear();
    }

    pub fn find_counter(&self, tag: &[u8; TAG_SIZE], keys: &SessionKeys) -> Option<u64> {
        let base_tw = compute_time_window(current_timestamp_ms(), DEFAULT_WINDOW_MS);
        let start = if self.highest < 0 {
            0
        } else {
            (self.highest as u64).saturating_sub((RECV_REORDER_WINDOW - 1) as u64)
        };
        let end = if self.highest < 0 {
            RECV_FUTURE_SEARCH_WINDOW as u64
        } else {
            std::cmp::max(
                RECV_FUTURE_SEARCH_WINDOW as u64,
                self.highest as u64 + RECV_FUTURE_SEARCH_WINDOW as u64 + 1,
            )
        };

        for tw_offset in [0i64, -1, 1] {
            let tw = (base_tw as i64 + tw_offset) as u64;
            for counter in start..end {
                if !self.is_new(counter) {
                    continue;
                }
                let expected = generate_resonance_tag(&keys.tag_secret, counter, tw);
                if &expected == tag {
                    return Some(counter);
                }
            }
        }

        None
    }

    pub fn mark(&mut self, counter: u64) {
        if self.highest < 0 || counter > self.highest as u64 {
            let shift = if self.highest < 0 {
                RECV_REORDER_WINDOW as u64
            } else {
                counter - self.highest as u64
            };
            self.bitmap = if shift >= RECV_REORDER_WINDOW as u64 {
                let mut bitmap = Bitset256::new();
                bitmap.set_bit(0);
                bitmap
            } else {
                let mut bitmap = self.bitmap.shl(shift);
                bitmap.set_bit(0);
                bitmap
            };
            self.highest = counter as i64;
        } else {
            let diff = (self.highest as u64 - counter) as usize;
            if diff < RECV_REORDER_WINDOW {
                self.bitmap.set_bit(diff);
            }
        }
    }

    fn is_new(&self, counter: u64) -> bool {
        if self.highest < 0 {
            return true;
        }

        let highest = self.highest as u64;
        if counter > highest {
            return true;
        }

        let diff = highest - counter;
        if diff >= RECV_REORDER_WINDOW as u64 {
            return false;
        }

        !self.bitmap.get_bit(diff as usize)
    }
}

pub fn build_inner_packet(inner_type: InnerType, seq_num: u16, payload: &[u8]) -> Vec<u8> {
    let mut inner = Vec::with_capacity(4 + payload.len());
    inner.extend_from_slice(&(inner_type as u16).to_le_bytes());
    inner.extend_from_slice(&seq_num.to_le_bytes());
    inner.extend_from_slice(payload);
    inner
}

pub fn build_zero_mdh_packet(
    keys: &SessionKeys,
    counter: &mut u64,
    inner: &[u8],
    obfuscated_eph_pub: Option<&[u8; 32]>,
) -> Result<Vec<u8>> {
    let pad_len: u16 = 8 + rand::thread_rng().next_u32() as u16 % 16;
    let mut plaintext = Vec::with_capacity(2 + inner.len() + pad_len as usize);
    plaintext.extend_from_slice(&pad_len.to_le_bytes());
    plaintext.extend_from_slice(inner);
    plaintext.resize(2 + inner.len() + pad_len as usize, 0);
    rand::thread_rng().fill_bytes(&mut plaintext[2 + inner.len()..]);

    let current_counter = *counter;
    *counter += 1;

    let nonce = counter_to_nonce(current_counter);
    let ciphertext = encrypt_payload(&keys.session_key, &nonce, &plaintext)?;
    let time_window = compute_time_window(current_timestamp_ms(), DEFAULT_WINDOW_MS);
    let tag = generate_resonance_tag(&keys.tag_secret, current_counter, time_window);

    let eph_len = if obfuscated_eph_pub.is_some() { 32 } else { 0 };
    let mut packet = Vec::with_capacity(TAG_SIZE + DEFAULT_ZERO_MDH.len() + eph_len + ciphertext.len());
    packet.extend_from_slice(&tag);
    packet.extend_from_slice(&DEFAULT_ZERO_MDH);
    if let Some(eph) = obfuscated_eph_pub {
        packet.extend_from_slice(eph);
    }
    packet.extend_from_slice(&ciphertext);

    Ok(packet)
}

pub fn decode_packet_with_mdh_len(
    packet: &[u8],
    keys: &SessionKeys,
    recv_window: &mut RecvWindow,
    mdh_len: usize,
) -> Result<DecodedPacket> {
    if packet.len() < TAG_SIZE + mdh_len + 16 {
        return Err(Error::InvalidPacket("Packet too short"));
    }

    let tag: [u8; TAG_SIZE] = packet[..TAG_SIZE]
        .try_into()
        .map_err(|_| Error::InvalidPacket("Packet tag malformed"))?;
    let counter = recv_window
        .find_counter(&tag, keys)
        .ok_or(Error::InvalidPacket("Invalid resonance tag"))?;

    let nonce = counter_to_nonce(counter);
    let ciphertext = &packet[TAG_SIZE + mdh_len..];
    let padded = decrypt_payload(&keys.session_key, &nonce, ciphertext)?;
    recv_window.mark(counter);

    if padded.len() < 2 {
        return Err(Error::InvalidPacket("Decrypted payload too short"));
    }

    let pad_len = u16::from_le_bytes([padded[0], padded[1]]) as usize;
    let end = padded
        .len()
        .checked_sub(pad_len)
        .ok_or(Error::InvalidPacket("Invalid padding length"))?;
    if end < 2 {
        return Err(Error::InvalidPacket("Invalid padding length"));
    }

    let inner = &padded[2..end];
    if inner.len() < 4 {
        return Err(Error::InvalidPacket("Inner payload too short"));
    }

    let header = InnerHeader::decode(inner)?;
    let payload = inner[4..].to_vec();

    Ok(DecodedPacket {
        counter,
        header,
        payload,
    })
}

pub fn process_server_hello_with_mdh_len(
    packet: &[u8],
    keys: &mut SessionKeys,
    keypair: &KeyPair,
    recv_window: &mut RecvWindow,
    send_counter: &mut u64,
    mdh_len: usize,
) -> Result<()> {
    let decoded = decode_packet_with_mdh_len(packet, keys, recv_window, mdh_len)?;

    if decoded.header.inner_type != InnerType::Control {
        return Err(Error::InvalidPacket("Expected control packet for ServerHello"));
    }

    match ControlPayload::decode(&decoded.payload)? {
        ControlPayload::ServerHello { server_eph_pub, .. } => {
            let dh2 = keypair.compute_shared(&server_eph_pub)?;
            let old_session_key = keys.session_key;
            *keys = derive_session_keys(&dh2, Some(&old_session_key), &keypair.public_key_bytes());
            *send_counter = 0;
            recv_window.reset();
            Ok(())
        }
        _ => Err(Error::InvalidPacket("Expected ServerHello control payload")),
    }
}

pub fn obfuscate_client_eph_pub(
    keypair: &KeyPair,
    server_public_key: &[u8; 32],
) -> [u8; 32] {
    let mut obfuscated = keypair.public_key_bytes();
    crypto::obfuscate_eph_pub(&mut obfuscated, server_public_key);
    obfuscated
}

pub fn counter_to_nonce(counter: u64) -> [u8; NONCE_SIZE] {
    let mut nonce = [0u8; NONCE_SIZE];
    nonce[..8].copy_from_slice(&counter.to_le_bytes());
    nonce
}