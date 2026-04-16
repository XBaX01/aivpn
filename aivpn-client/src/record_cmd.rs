//! Recording CLI Commands — Client-side recording controls
//!
//! Handles `aivpn record start/stop/status` and `aivpn masks list/delete/retrain`
//! by sending appropriate ControlPayload messages to the server.

use tracing::info;

/// Display recording acknowledgment from server
pub fn handle_recording_ack(session_id: &[u8; 16], status: &str) {
    let sid_hex = session_id.iter()
        .take(4)
        .map(|b| format!("{:02x}", b))
        .collect::<String>();
    
    match status {
        "started" => {
            info!("📹 Recording started (session: {}...)", sid_hex);
            println!("Recording started. Use the service normally.");
            println!("Run 'aivpn record stop' when done.");
        }
        "analyzing" => {
            info!("🔍 Recording stopped, server analyzing...");
            println!("Recording stopped. Server is analyzing traffic...");
        }
        other => {
            info!("Recording status: {}", other);
            println!("Recording status: {}", other);
        }
    }
}

/// Display recording completion
pub fn handle_recording_complete(service: &str, mask_id: &str, confidence: f32) {
    info!("✅ Mask generated for '{}'", service);
    println!();
    println!("✅ Mask generated and tested!");
    println!();
    println!("   Mask ID:     {}", mask_id);
    println!("   Service:     {}", service);
    println!("   Confidence:  {:.2}", confidence);
    println!();
    println!("   Broadcasting to all clients...");
}

/// Display recording failure
pub fn handle_recording_failed(reason: &str) {
    info!("❌ Recording failed: {}", reason);
    println!();
    println!("❌ Recording failed!");
    println!("   Reason: {}", reason);
    println!();
    println!("   Tips:");
    println!("   - Use the service for at least 1 minute");
    println!("   - Ensure active traffic (not idle)");
    println!("   - Need at least 500 packets captured");
}
