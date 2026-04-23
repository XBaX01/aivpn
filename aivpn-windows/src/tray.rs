//! System tray integration — background-thread event processing.
//!
//! The tray icon uses the real AIVPN application icon.
//! A dedicated background thread monitors tray events (clicks, menu)
//! and directly calls Win32 ShowWindow to restore the window —
//! this is critical because eframe stops calling update() when
//! the window is hidden via SW_HIDE.
//!
//! Communication with the main eframe thread is via an atomic flag.

use tray_icon::menu::{Menu, MenuEvent, MenuItem, PredefinedMenuItem};
use tray_icon::{Icon, TrayIcon, TrayIconBuilder, TrayIconEvent};

use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::Arc;

use crate::vpn_manager::{format_bytes, ConnectionState, TrafficStats};

// Menu item IDs
const MENU_SHOW_ID: &str = "aivpn_show";
const MENU_QUIT_ID: &str = "aivpn_quit";

// Atomic action values
const ACTION_NONE: u8 = 0;
const ACTION_SHOW: u8 = 1;
const ACTION_QUIT: u8 = 2;

pub struct TrayManager {
    _tray_icon: TrayIcon,
    /// Shared atomic flag: background thread writes, main thread reads
    pub action: Arc<AtomicU8>,
}

impl TrayManager {
    /// Create the system tray icon and start the background event thread.
    pub fn new(
        icon_rgba: Vec<u8>,
        icon_w: u32,
        icon_h: u32,
        show_label: &str,
        quit_label: &str,
    ) -> Result<Self, String> {
        let icon = Icon::from_rgba(icon_rgba, icon_w, icon_h)
            .map_err(|e| format!("Tray icon error: {}", e))?;

        // Context menu: Show / ─── / Quit
        let menu = Menu::new();
        let show_item = MenuItem::with_id(MENU_SHOW_ID, show_label, true, None);
        let quit_item = MenuItem::with_id(MENU_QUIT_ID, quit_label, true, None);
        let _ = menu.append(&show_item);
        let _ = menu.append(&PredefinedMenuItem::separator());
        let _ = menu.append(&quit_item);

        let tray_icon = TrayIconBuilder::new()
            .with_menu(Box::new(menu))
            .with_tooltip("AIVPN")
            .with_icon(icon)
            .build()
            .map_err(|e| format!("Tray build error: {}", e))?;

        let action = Arc::new(AtomicU8::new(ACTION_NONE));

        // Spawn background thread to monitor tray events.
        // This thread runs even when the main window is hidden (SW_HIDE),
        // which is when eframe stops calling update().
        let action_clone = action.clone();
        std::thread::Builder::new()
            .name("tray-events".into())
            .spawn(move || {
                tray_event_loop(action_clone);
            })
            .map_err(|e| format!("Failed to spawn tray thread: {}", e))?;

        Ok(Self {
            _tray_icon: tray_icon,
            action,
        })
    }

    /// Update the tooltip with current VPN status and traffic stats.
    pub fn update_tooltip(&self, state: ConnectionState, stats: &TrafficStats) {
        let tooltip = build_tooltip(state, stats);
        let _ = self._tray_icon.set_tooltip(Some(tooltip));
    }

    /// Read the pending action (set by background thread) and clear it.
    /// Returns ACTION_SHOW or ACTION_QUIT, or ACTION_NONE if nothing pending.
    pub fn take_action(&self) -> u8 {
        self.action.swap(ACTION_NONE, Ordering::SeqCst)
    }
}

// ── Background thread ──────────────────────────────────────────────────────

/// Continuously poll tray icon and menu events.
/// When an action is detected, set the atomic flag AND call Win32 ShowWindow
/// to make the hidden window visible again (waking up eframe).
fn tray_event_loop(action: Arc<AtomicU8>) {
    let menu_rx = MenuEvent::receiver();
    let icon_rx = TrayIconEvent::receiver();

    loop {
        let mut did_something = false;

        // Process all queued menu events
        while let Ok(event) = menu_rx.try_recv() {
            did_something = true;
            match event.id.as_ref() {
                MENU_SHOW_ID => {
                    action.store(ACTION_SHOW, Ordering::SeqCst);
                    wake_and_show_window();
                }
                MENU_QUIT_ID => {
                    action.store(ACTION_QUIT, Ordering::SeqCst);
                    // Show window briefly so eframe can process the quit
                    wake_and_show_window();
                }
                _ => {}
            }
        }

        // Process all queued tray icon events
        while let Ok(event) = icon_rx.try_recv() {
            did_something = true;
            match event {
                // Left-click release → show window
                TrayIconEvent::Click {
                    button: tray_icon::MouseButton::Left,
                    button_state: tray_icon::MouseButtonState::Up,
                    ..
                } => {
                    action.store(ACTION_SHOW, Ordering::SeqCst);
                    wake_and_show_window();
                }
                // Double-click → show window (Windows-only)
                TrayIconEvent::DoubleClick { .. } => {
                    action.store(ACTION_SHOW, Ordering::SeqCst);
                    wake_and_show_window();
                }
                _ => {}
            }
        }

        // Sleep longer if idle, shorter if busy
        if did_something {
            std::thread::sleep(std::time::Duration::from_millis(50));
        } else {
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
    }
}

/// Use Win32 API to make the hidden AIVPN window visible again.
/// This is called from the background thread to wake up eframe.
#[cfg(windows)]
fn wake_and_show_window() {
    use winapi::um::winuser::{
        FindWindowW, IsIconic, IsWindowVisible,
        ShowWindow, SetForegroundWindow,
        SW_SHOW, SW_RESTORE,
    };
    unsafe {
        let title: Vec<u16> = "AIVPN\0".encode_utf16().collect();
        let hwnd = FindWindowW(std::ptr::null(), title.as_ptr());
        if hwnd.is_null() {
            return;
        }
        // If minimized, restore first
        if IsIconic(hwnd) != 0 {
            ShowWindow(hwnd, SW_RESTORE);
        }
        // If hidden (not visible), show
        if IsWindowVisible(hwnd) == 0 {
            ShowWindow(hwnd, SW_SHOW);
        }
        ShowWindow(hwnd, SW_RESTORE);
        SetForegroundWindow(hwnd);
    }
}

#[cfg(not(windows))]
fn wake_and_show_window() {}

// ── Tooltip ────────────────────────────────────────────────────────────────

fn build_tooltip(state: ConnectionState, stats: &TrafficStats) -> String {
    match state {
        ConnectionState::Connected => {
            if stats.bytes_sent > 0 || stats.bytes_received > 0 {
                format!(
                    "AIVPN \u{2714} Connected\n\u{2193} {}  \u{2191} {}",
                    format_bytes(stats.bytes_received),
                    format_bytes(stats.bytes_sent),
                )
            } else {
                "AIVPN \u{2714} Connected".to_string()
            }
        }
        ConnectionState::Connecting => "AIVPN \u{2026} Connecting".to_string(),
        ConnectionState::Disconnecting => "AIVPN \u{2026} Disconnecting".to_string(),
        ConnectionState::Disconnected => "AIVPN \u{2014} Disconnected".to_string(),
    }
}
