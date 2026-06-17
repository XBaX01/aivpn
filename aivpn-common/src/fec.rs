//! XOR Forward Error Correction (FEC) encoder/decoder (0.9.0)
//!
//! Every N data packets one repair packet is emitted = XOR of those N packets.
//! If exactly one packet from the group is lost, it can be recovered.
//! Overhead: 1/N extra packets. N configured via AdaptiveLevel::fec_n().

/// Encodes a stream of data packets into groups, emitting a repair per group.
#[derive(Debug, Clone)]
pub struct FecEncoder {
    xor_buf: Vec<u8>,
    max_len: usize,
    count: u8,
    group_size: u8,
    group_seq: u16,
}

impl FecEncoder {
    pub fn new(group_size: u8, max_payload: usize) -> Self {
        assert!(group_size > 0);
        Self {
            xor_buf: vec![0u8; max_payload],
            max_len: 0,
            count: 0,
            group_size,
            group_seq: 0,
        }
    }

    /// Feed a data payload. Returns `Some(repair)` when the group is complete.
    pub fn feed(&mut self, payload: &[u8]) -> Option<FecRepair> {
        let len = payload.len().min(self.xor_buf.len());
        for (a, b) in self.xor_buf[..len].iter_mut().zip(&payload[..len]) {
            *a ^= b;
        }
        if len > self.max_len {
            self.max_len = len;
        }
        self.count += 1;
        if self.count >= self.group_size {
            let repair = FecRepair {
                group_seq: self.group_seq,
                group_size: self.group_size,
                xor_data: self.xor_buf[..self.max_len].to_vec(),
            };
            self.xor_buf.iter_mut().for_each(|b| *b = 0);
            self.max_len = 0;
            self.count = 0;
            self.group_seq = self.group_seq.wrapping_add(1);
            Some(repair)
        } else {
            None
        }
    }

    pub fn reset(&mut self) {
        self.xor_buf.iter_mut().for_each(|b| *b = 0);
        self.max_len = 0;
        self.count = 0;
    }
}

/// A FEC repair packet carrying the XOR of a group of N data packets.
#[derive(Debug, Clone)]
pub struct FecRepair {
    pub group_seq: u16,
    pub group_size: u8,
    pub xor_data: Vec<u8>,
}

impl FecRepair {
    pub fn encode(&self) -> Vec<u8> {
        let mut buf = Vec::with_capacity(3 + self.xor_data.len());
        buf.extend_from_slice(&self.group_seq.to_le_bytes());
        buf.push(self.group_size);
        buf.extend_from_slice(&self.xor_data);
        buf
    }

    pub fn decode(data: &[u8]) -> Option<Self> {
        if data.len() < 3 { return None; }
        Some(Self {
            group_seq: u16::from_le_bytes([data[0], data[1]]),
            group_size: data[2],
            xor_data: data[3..].to_vec(),
        })
    }

    /// Recover a missing packet given the XOR of all received siblings.
    pub fn recover(&self, received_xor: &[u8]) -> Vec<u8> {
        let len = self.xor_data.len().max(received_xor.len());
        let mut out = vec![0u8; len];
        for i in 0..len {
            out[i] = self.xor_data.get(i).copied().unwrap_or(0)
                ^ received_xor.get(i).copied().unwrap_or(0);
        }
        out
    }
}

/// Tracks received packets in a group and recovers the missing one on repair receipt.
#[derive(Debug, Clone, Default)]
pub struct FecDecoder {
    group: Vec<Option<Vec<u8>>>,
    group_seq: u16,
}

impl FecDecoder {
    pub fn new() -> Self { Self::default() }

    pub fn record(&mut self, group_seq: u16, group_size: u8, idx: u8, payload: Vec<u8>) {
        if self.group_seq != group_seq || self.group.len() != group_size as usize {
            self.group = vec![None; group_size as usize];
            self.group_seq = group_seq;
        }
        let i = idx as usize % group_size as usize;
        self.group[i] = Some(payload);
    }

    /// Attempt recovery when repair arrives. Returns (missing_idx, payload) if successful.
    pub fn recover(&self, repair: &FecRepair) -> Option<(u8, Vec<u8>)> {
        if self.group_seq != repair.group_seq { return None; }
        let missing: Vec<u8> = self.group.iter().enumerate()
            .filter(|(_, p)| p.is_none())
            .map(|(i, _)| i as u8)
            .collect();
        if missing.len() != 1 { return None; }

        let mut xor_recv = vec![0u8; repair.xor_data.len()];
        for pkt in self.group.iter().flatten() {
            for (a, b) in xor_recv.iter_mut().zip(pkt.iter()) {
                *a ^= b;
            }
        }
        Some((missing[0], repair.recover(&xor_recv)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn xor_recovery_works() {
        let pkts: Vec<Vec<u8>> = vec![
            vec![1, 2, 3, 4],
            vec![5, 6, 7, 8],
            vec![9, 10, 11, 12],
        ];
        let mut enc = FecEncoder::new(3, 64);
        let mut repair = None;
        for p in &pkts { repair = enc.feed(p); }
        let repair = repair.unwrap();

        let mut dec = FecDecoder::new();
        dec.record(0, 3, 0, pkts[0].clone());
        dec.record(0, 3, 2, pkts[2].clone());

        let (idx, recovered) = dec.recover(&repair).unwrap();
        assert_eq!(idx, 1);
        assert_eq!(recovered, pkts[1]);
    }
}
