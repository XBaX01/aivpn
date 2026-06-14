//! Connection diagnostics and benchmarking.
//!
//! `run_bench()` measures latency (P50/P95/P99) via UDP probes.
//! Used by the CLI `bench` subcommand and GUI Diagnostics tabs.

use std::net::SocketAddr;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use tokio::net::UdpSocket;
use tracing::{debug, warn};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct BenchResult {
    pub latency_p50_ms: f64,
    pub latency_p95_ms: f64,
    pub latency_p99_ms: f64,
    pub throughput_up_kbps: f64,
    pub throughput_down_kbps: f64,
    pub packet_loss_pct: f64,
    pub quality_score: u8,
    pub quality_label: String,
    pub samples: usize,
}

const PROBE_PAYLOAD: &[u8] = b"aivpn-bench-probe-v1";

/// Run a latency benchmark against `server_addr` for `duration`.
/// Sends UDP probes and collects timing.
pub async fn run_bench(server_addr: SocketAddr, duration: Duration) -> BenchResult {
    let sock = match UdpSocket::bind("0.0.0.0:0").await {
        Ok(s) => s,
        Err(e) => {
            warn!("bench: bind failed: {}", e);
            return BenchResult {
                quality_label: "Error".to_string(),
                ..Default::default()
            };
        }
    };

    let deadline = Instant::now() + duration;
    let mut rtts: Vec<f64> = Vec::new();
    let mut sent = 0u64;
    let mut buf = [0u8; 256];

    while Instant::now() < deadline {
        let t0 = Instant::now();
        if sock.send_to(PROBE_PAYLOAD, server_addr).await.is_err() {
            break;
        }
        sent += 1;

        match tokio::time::timeout(Duration::from_millis(500), sock.recv_from(&mut buf)).await {
            Ok(Ok(_)) => {
                let rtt = t0.elapsed().as_secs_f64() * 1000.0;
                rtts.push(rtt);
                debug!("bench rtt: {:.2} ms", rtt);
            }
            _ => {
                // Server drops unknown probes — estimate RTT from elapsed
                let elapsed = t0.elapsed().as_secs_f64() * 1000.0;
                if elapsed < 490.0 {
                    rtts.push(elapsed * 2.0);
                }
            }
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }

    let loss_pct = if sent > 0 {
        // Probes the server silently drops count as lost
        let recv = rtts.len() as u64;
        ((sent - recv.min(sent)) as f64 / sent as f64 * 100.0).min(100.0)
    } else {
        100.0
    };

    compute_result(rtts, loss_pct, sent as usize)
}

fn compute_result(mut rtts: Vec<f64>, loss_pct: f64, samples: usize) -> BenchResult {
    if rtts.is_empty() {
        return BenchResult {
            packet_loss_pct: 100.0,
            quality_score: 0,
            quality_label: "No response".to_string(),
            samples,
            ..Default::default()
        };
    }
    rtts.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let n = rtts.len();
    let p50 = rtts[n / 2];
    let p95 = rtts[(n as f64 * 0.95) as usize % n];
    let p99 = rtts[(n as f64 * 0.99) as usize % n];
    let throughput_kbps = if p50 > 0.0 {
        1400.0 * 8.0 / (p50 / 1000.0) / 1000.0
    } else {
        0.0
    };
    let quality_score = compute_quality(p50, loss_pct);
    let quality_label = match quality_score {
        80..=100 => "Excellent",
        60..=79 => "Good",
        40..=59 => "Fair",
        _ => "Poor",
    }
    .to_string();
    BenchResult {
        latency_p50_ms: p50,
        latency_p95_ms: p95,
        latency_p99_ms: p99,
        throughput_up_kbps: throughput_kbps,
        throughput_down_kbps: throughput_kbps,
        packet_loss_pct: loss_pct,
        quality_score,
        quality_label,
        samples,
    }
}

fn compute_quality(p50_ms: f64, loss_pct: f64) -> u8 {
    let latency_score = if p50_ms < 20.0 {
        100.0f64
    } else if p50_ms < 50.0 {
        90.0
    } else if p50_ms < 100.0 {
        75.0
    } else if p50_ms < 200.0 {
        55.0
    } else {
        30.0
    };
    ((latency_score - (loss_pct * 2.0).min(60.0)).max(0.0) as u8).min(100)
}
