use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use smoltcp::phy::{Device, DeviceCapabilities, Medium, RxToken, TxToken};
use smoltcp::time::Instant;

pub struct VpnDevice {
    pub rx_queue: Arc<Mutex<VecDeque<Vec<u8>>>>,
    tx_queue: Arc<Mutex<VecDeque<Vec<u8>>>>,
    mtu: usize,
}

impl VpnDevice {
    pub fn new(
        rx_queue: Arc<Mutex<VecDeque<Vec<u8>>>>,
        tx_queue: Arc<Mutex<VecDeque<Vec<u8>>>>,
        mtu: usize,
    ) -> Self {
        Self {
            rx_queue,
            tx_queue,
            mtu,
        }
    }
}

pub struct VpnRxToken {
    packet: Vec<u8>,
}

pub struct VpnTxToken {
    tx_queue: Arc<Mutex<VecDeque<Vec<u8>>>>,
}

impl RxToken for VpnRxToken {
    fn consume<R, F>(mut self, f: F) -> R
    where
        F: FnOnce(&mut [u8]) -> R,
    {
        f(&mut self.packet)
    }
}

impl TxToken for VpnTxToken {
    fn consume<R, F>(self, len: usize, f: F) -> R
    where
        F: FnOnce(&mut [u8]) -> R,
    {
        let mut buf = vec![0u8; len];
        let result = f(&mut buf);
        self.tx_queue.lock().unwrap().push_back(buf);
        result
    }
}

impl Device for VpnDevice {
    type RxToken<'a>
        = VpnRxToken
    where
        Self: 'a;
    type TxToken<'a>
        = VpnTxToken
    where
        Self: 'a;

    fn receive(&mut self, _timestamp: Instant) -> Option<(Self::RxToken<'_>, Self::TxToken<'_>)> {
        let packet = self.rx_queue.lock().unwrap().pop_front()?;
        Some((
            VpnRxToken { packet },
            VpnTxToken {
                tx_queue: Arc::clone(&self.tx_queue),
            },
        ))
    }

    fn transmit(&mut self, _timestamp: Instant) -> Option<Self::TxToken<'_>> {
        Some(VpnTxToken {
            tx_queue: Arc::clone(&self.tx_queue),
        })
    }

    fn capabilities(&self) -> DeviceCapabilities {
        let mut caps = DeviceCapabilities::default();
        caps.medium = Medium::Ip;
        caps.max_transmission_unit = self.mtu;
        caps
    }
}
