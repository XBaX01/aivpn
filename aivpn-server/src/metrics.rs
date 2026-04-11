//! Prometheus Metrics (Phase 5)
//! 
//! Implements monitoring and metrics export for AIVPN
//! 
//! Features:
//! - Session count and state
//! - Packet processing rates
//! - Bandwidth usage
//! - Mask rotation events
//! - Neural module health
//! - DPI attack detection

#[cfg(feature = "metrics")]
use prometheus::{Registry, Counter, Gauge, Histogram, Opts, TextEncoder};
#[cfg(feature = "metrics")]
use tracing::warn;

/// Metrics collector
pub struct MetricsCollector {
    #[cfg(feature = "metrics")]
    registry: Registry,
    
    #[cfg(feature = "metrics")]
    sessions_total: Gauge,
    
    #[cfg(feature = "metrics")]
    sessions_active: Gauge,
    
    #[cfg(feature = "metrics")]
    packets_received: Counter,
    
    #[cfg(feature = "metrics")]
    packets_sent: Counter,
    
    #[cfg(feature = "metrics")]
    bytes_received: Counter,
    
    #[cfg(feature = "metrics")]
    bytes_sent: Counter,
    
    #[cfg(feature = "metrics")]
    packet_processing_time: Histogram,
    
    #[cfg(feature = "metrics")]
    tag_validation_time: Histogram,
    
    #[cfg(feature = "metrics")]
    mask_rotations: Counter,
    
    #[cfg(feature = "metrics")]
    key_rotations: Counter,
    
    #[cfg(feature = "metrics")]
    neural_checks_total: Counter,
    
    #[cfg(feature = "metrics")]
    neural_checks_failed: Counter,
    
    #[cfg(feature = "metrics")]
    dpi_attacks_detected: Counter,
}

impl MetricsCollector {
    /// Create new metrics collector
    pub fn new() -> Self {
        #[cfg(feature = "metrics")]
        {
            let registry = Registry::new();
            
            // Session metrics
            let sessions_total = Gauge::with_opts(Opts::new(
                "aivpn_sessions_total",
                "Total number of sessions"
            )).unwrap();
            registry.register(Box::new(sessions_total.clone())).unwrap();
            
            let sessions_active = Gauge::with_opts(Opts::new(
                "aivpn_sessions_active",
                "Number of active sessions"
            )).unwrap();
            registry.register(Box::new(sessions_active.clone())).unwrap();
            
            // Packet metrics
            let packets_received = Counter::with_opts(Opts::new(
                "aivpn_packets_received_total",
                "Total packets received"
            )).unwrap();
            registry.register(Box::new(packets_received.clone())).unwrap();
            
            let packets_sent = Counter::with_opts(Opts::new(
                "aivpn_packets_sent_total",
                "Total packets sent"
            )).unwrap();
            registry.register(Box::new(packets_sent.clone())).unwrap();
            
            // Bandwidth metrics
            let bytes_received = Counter::with_opts(Opts::new(
                "aivpn_bytes_received_total",
                "Total bytes received"
            )).unwrap();
            registry.register(Box::new(bytes_received.clone())).unwrap();
            
            let bytes_sent = Counter::with_opts(Opts::new(
                "aivpn_bytes_sent_total",
                "Total bytes sent"
            )).unwrap();
            registry.register(Box::new(bytes_sent.clone())).unwrap();
            
            // Performance metrics
            let packet_processing_time = Histogram::with_opts(Opts::new(
                "aivpn_packet_processing_seconds",
                "Packet processing time"
            )).unwrap();
            registry.register(Box::new(packet_processing_time.clone())).unwrap();
            
            let tag_validation_time = Histogram::with_opts(Opts::new(
                "aivpn_tag_validation_seconds",
                "Tag validation time"
            )).unwrap();
            registry.register(Box::new(tag_validation_time.clone())).unwrap();
            
            // Rotation metrics
            let mask_rotations = Counter::with_opts(Opts::new(
                "aivpn_mask_rotations_total",
                "Total mask rotations"
            )).unwrap();
            registry.register(Box::new(mask_rotations.clone())).unwrap();
            
            let key_rotations = Counter::with_opts(Opts::new(
                "aivpn_key_rotations_total",
                "Total key rotations"
            )).unwrap();
            registry.register(Box::new(key_rotations.clone())).unwrap();
            
            // Neural module metrics
            let neural_checks_total = Counter::with_opts(Opts::new(
                "aivpn_neural_checks_total",
                "Total neural resonance checks"
            )).unwrap();
            registry.register(Box::new(neural_checks_total.clone())).unwrap();
            
            let neural_checks_failed = Counter::with_opts(Opts::new(
                "aivpn_neural_checks_failed_total",
                "Failed neural resonance checks"
            )).unwrap();
            registry.register(Box::new(neural_checks_failed.clone())).unwrap();
            
            // Security metrics
            let dpi_attacks_detected = Counter::with_opts(Opts::new(
                "aivpn_dpi_attacks_detected_total",
                "DPI attacks detected"
            )).unwrap();
            registry.register(Box::new(dpi_attacks_detected.clone())).unwrap();
            
            Self {
                registry,
                sessions_total,
                sessions_active,
                packets_received,
                packets_sent,
                bytes_received,
                bytes_sent,
                packet_processing_time,
                tag_validation_time,
                mask_rotations,
                key_rotations,
                neural_checks_total,
                neural_checks_failed,
                dpi_attacks_detected,
            }
        }
        
        #[cfg(not(feature = "metrics"))]
        Self {}
    }
    
    /// Update session count
    pub fn update_session_count(&self, total: usize, active: usize) {
        #[cfg(feature = "metrics")]
        {
            self.sessions_total.set(total as i64);
            self.sessions_active.set(active as i64);
        }

        #[cfg(not(feature = "metrics"))]
        let _ = (total, active);
    }
    
    /// Record packet received
    pub fn record_packet_received(&self, bytes: usize) {
        #[cfg(feature = "metrics")]
        {
            self.packets_received.inc();
            self.bytes_received.inc_by(bytes as u64);
        }

        #[cfg(not(feature = "metrics"))]
        let _ = bytes;
    }
    
    /// Record packet sent
    pub fn record_packet_sent(&self, bytes: usize) {
        #[cfg(feature = "metrics")]
        {
            self.packets_sent.inc();
            self.bytes_sent.inc_by(bytes as u64);
        }

        #[cfg(not(feature = "metrics"))]
        let _ = bytes;
    }
    
    /// Record packet processing time
    pub fn record_processing_time(&self, _seconds: f64) {
        #[cfg(feature = "metrics")]
        {
            self.packet_processing_time.observe(_seconds);
        }
    }

    /// Record tag validation time
    pub fn record_tag_validation_time(&self, _seconds: f64) {
        #[cfg(feature = "metrics")]
        {
            self.tag_validation_time.observe(_seconds);
        }
    }

    /// Record mask rotation
    pub fn record_mask_rotation(&self) {
        #[cfg(feature = "metrics")]
        {
            self.mask_rotations.inc();
        }
    }

    /// Record key rotation
    pub fn record_key_rotation(&self) {
        #[cfg(feature = "metrics")]
        {
            self.key_rotations.inc();
        }
    }

    /// Record neural check
    pub fn record_neural_check(&self, _failed: bool) {
        #[cfg(feature = "metrics")]
        {
            self.neural_checks_total.inc();
            if _failed {
                self.neural_checks_failed.inc();
            }
        }
    }
    
    /// Record DPI attack detection
    pub fn record_dpi_attack(&self) {
        #[cfg(feature = "metrics")]
        {
            self.dpi_attacks_detected.inc();
            warn!("DPI attack detected!");
        }
    }
    
    /// Export metrics in Prometheus format
    pub fn gather(&self) -> String {
        #[cfg(feature = "metrics")]
        {
            let encoder = TextEncoder::new();
            let metric_families = self.registry.gather();
            let mut output = String::new();
            encoder.encode(&metric_families, &mut output).unwrap();
            output
        }
        
        #[cfg(not(feature = "metrics"))]
        {
            String::new()
        }
    }
}

impl Default for MetricsCollector {
    fn default() -> Self {
        Self::new()
    }
}

/// Metrics HTTP handler (for integration with Prometheus)
#[cfg(feature = "metrics")]
pub async fn metrics_handler(
    collector: Arc<MetricsCollector>,
) -> Result<impl warp::Reply, warp::Rejection> {
    let metrics = collector.gather();
    Ok(warp::reply::with_header(
        metrics,
        "Content-Type",
        "text/plain; version=0.0.4",
    ))
}
