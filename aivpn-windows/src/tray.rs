//! System tray icon (placeholder for tray-icon integration)
//!
//! On Windows, the system tray allows the app to run in background.
//! This module provides tray icon creation and menu handling.
//! Full tray integration requires the event loop — used from main.

// Tray integration is handled directly in main.rs via tray-icon crate.
// This module provides icon generation helpers.

use image::{Rgba, RgbaImage};

/// Generate a simple colored circle icon for the tray
pub fn generate_tray_icon(connected: bool) -> Vec<u8> {
    let size = 32u32;
    let mut img = RgbaImage::new(size, size);
    let center = (size / 2) as f32;
    let radius = (size / 2 - 2) as f32;

    let color = if connected {
        Rgba([0x4C, 0xD9, 0x64, 0xFF]) // Green
    } else {
        Rgba([0xFF, 0x3B, 0x30, 0xFF]) // Red
    };

    for y in 0..size {
        for x in 0..size {
            let dx = x as f32 - center;
            let dy = y as f32 - center;
            let dist = (dx * dx + dy * dy).sqrt();

            if dist <= radius {
                img.put_pixel(x, y, color);
            } else if dist <= radius + 1.0 {
                // Anti-alias edge
                let alpha = ((radius + 1.0 - dist) * 255.0) as u8;
                img.put_pixel(
                    x,
                    y,
                    Rgba([color[0], color[1], color[2], alpha]),
                );
            } else {
                img.put_pixel(x, y, Rgba([0, 0, 0, 0]));
            }
        }
    }

    // Return RGBA bytes
    img.into_raw()
}

pub const TRAY_ICON_SIZE: u32 = 32;
