//! Adaptive connection behavior for unstable networks.
//!
//! All parameters are opt-in via CLI/UI — no hardcoded behavior.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use tracing::info;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AdaptiveConfig {
    pub enabled: bool,
    /// Loss % threshold to trigger adaptation (default 5.0).
    pub loss_threshold_pct: f32,
    /// MTU reduction step in bytes (default 50).
    pub mtu_step_down: u16,
    /// Keepalive multiplier when adapting (0.5 = twice as frequent).
    pub keepalive_factor: f32,
    /// Neural cooldown multiplier during adaptation.
    pub neural_cooldown_factor: f32,
    /// Minimum MTU the adapter will set.
    pub min_mtu: u16,
}

impl Default for AdaptiveConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            loss_threshold_pct: 5.0,
            mtu_step_down: 50,
            keepalive_factor: 0.5,
            neural_cooldown_factor: 1.5,
            min_mtu: 576,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum AdaptiveMode {
    Normal,
    Adapting,
    MtuReduced,
}

struct Inner {
    mode: AdaptiveMode,
    mtu_delta: i32,
    window: VecDeque<(u64, u64)>,
    packets_sent: u64,
    packets_lost: u64,
    last_probe: Instant,
}

impl Inner {
    fn loss_pct(&self) -> f32 {
        let sent: u64 = self.window.iter().map(|(s, _)| s).sum();
        if sent == 0 {
            return 0.0;
        }
        let lost: u64 = self.window.iter().map(|(_, l)| l).sum();
        lost as f32 / sent as f32 * 100.0
    }
}

/// Thread-safe adaptive monitor.
#[derive(Clone)]
pub struct AdaptiveMonitor {
    config: AdaptiveConfig,
    inner: Arc<Mutex<Inner>>,
}

impl AdaptiveMonitor {
    pub fn new(config: AdaptiveConfig) -> Self {
        Self {
            inner: Arc::new(Mutex::new(Inner {
                mode: AdaptiveMode::Normal,
                mtu_delta: 0,
                window: VecDeque::with_capacity(20),
                packets_sent: 0,
                packets_lost: 0,
                last_probe: Instant::now(),
            })),
            config,
        }
    }

    pub fn is_enabled(&self) -> bool {
        self.config.enabled
    }

    pub fn on_packet_sent(&self) {
        if !self.config.enabled {
            return;
        }
        self.inner.lock().unwrap().packets_sent += 1;
    }

    pub fn on_packet_loss(&self, count: u64) {
        if !self.config.enabled {
            return;
        }
        {
            let mut g = self.inner.lock().unwrap();
            g.packets_lost += count;
        }
        self.maybe_adapt();
    }

    fn maybe_adapt(&self) {
        let mut g = self.inner.lock().unwrap();
        if g.last_probe.elapsed() < Duration::from_secs(5) {
            return;
        }
        g.last_probe = Instant::now();

        if g.window.len() >= 20 {
            g.window.pop_front();
        }
        let sent = g.packets_sent;
        let lost = g.packets_lost;
        g.window.push_back((sent, lost));
        g.packets_sent = 0;
        g.packets_lost = 0;

        let loss = g.loss_pct();
        if loss >= self.config.loss_threshold_pct {
            g.mode = AdaptiveMode::Adapting;
            g.mtu_delta = (g.mtu_delta - self.config.mtu_step_down as i32)
                .max(-(1500 - self.config.min_mtu as i32));
            info!("Adaptive: loss={:.1}% → mtu_delta={}", loss, g.mtu_delta);
        } else if loss < self.config.loss_threshold_pct / 2.0 && g.mtu_delta < 0 {
            g.mtu_delta = (g.mtu_delta + self.config.mtu_step_down as i32).min(0);
            if g.mtu_delta == 0 {
                g.mode = AdaptiveMode::Normal;
                info!("Adaptive: recovered to normal MTU");
            }
        }
    }

    /// Bytes to subtract from the configured MTU.
    pub fn mtu_delta(&self) -> i32 {
        if !self.config.enabled {
            return 0;
        }
        self.inner.lock().unwrap().mtu_delta
    }

    /// Keepalive multiplier (< 1.0 means more frequent pings).
    pub fn keepalive_factor(&self) -> f32 {
        if !self.config.enabled {
            return 1.0;
        }
        if self.inner.lock().unwrap().mode != AdaptiveMode::Normal {
            self.config.keepalive_factor
        } else {
            1.0
        }
    }

    pub fn current_mode(&self) -> AdaptiveMode {
        self.inner.lock().unwrap().mode.clone()
    }
}
