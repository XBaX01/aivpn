#![windows_subsystem = "windows"]

//! AIVPN Windows GUI Application
//!
//! Native Windows app using egui/eframe with system tray support.
//! Manages aivpn-client.exe as a subprocess — no console window visible.
//!
//! Window behavior:
//! - Close (×) or Minimize (_) → window hides to system tray
//! - Left-click tray icon → restore window
//! - Right-click tray icon → context menu (Show / Quit)
//! - Quit button in UI or tray menu → disconnect VPN and exit
//!
//! Architecture:
//! A background thread continuously polls tray events even when the
//! window is hidden (SW_HIDE stops eframe's update loop). The thread
//! calls Win32 ShowWindow directly to wake up eframe, and communicates
//! the action type via an atomic flag.

mod vpn_manager;
mod key_storage;
mod localization;
mod tray;
mod ui;

use eframe::egui;
use key_storage::KeyStorage;
use localization::Lang;
use vpn_manager::VpnManager;
use std::fs;
use std::path::PathBuf;

const APP_VERSION: &str = env!("CARGO_PKG_VERSION");
const WINDOW_WIDTH: f32 = 360.0;
const WINDOW_HEIGHT: f32 = 480.0;

// ── Win32 window management ────────────────────────────────────────────────

/// Hide the AIVPN window (removes from taskbar)
#[cfg(windows)]
fn win32_hide_window() {
    use winapi::um::winuser::{FindWindowW, ShowWindow, SW_HIDE};
    unsafe {
        let title: Vec<u16> = "AIVPN\0".encode_utf16().collect();
        let hwnd = FindWindowW(std::ptr::null(), title.as_ptr());
        if !hwnd.is_null() {
            ShowWindow(hwnd, SW_HIDE);
        }
    }
}

#[cfg(not(windows))]
fn win32_hide_window() {}

// ── Entry point ────────────────────────────────────────────────────────────

fn main() -> eframe::Result<()> {
    let log_path = startup_log_path();
    write_startup_log(&log_path, "startup: entered main");

    // Decode embedded app icon (shared between window title bar and tray)
    let (icon_rgba, icon_w, icon_h) = decode_embedded_icon();

    let egui_icon = if !icon_rgba.is_empty() {
        Some(Box::new(egui::IconData {
            rgba: icon_rgba.clone(),
            width: icon_w,
            height: icon_h,
        }))
    } else {
        None
    };

    let tray_icon_rgba = icon_rgba;
    let tray_icon_w = icon_w;
    let tray_icon_h = icon_h;

    let mut viewport = egui::ViewportBuilder::default()
        .with_inner_size([WINDOW_WIDTH, WINDOW_HEIGHT])
        .with_min_inner_size([WINDOW_WIDTH, WINDOW_HEIGHT])
        .with_max_inner_size([WINDOW_WIDTH, WINDOW_HEIGHT])
        .with_resizable(false)
        .with_decorations(true)
        .with_transparent(false)
        .with_title("AIVPN");

    if let Some(icon_data) = egui_icon {
        viewport = viewport.with_icon(icon_data);
    }

    let options = eframe::NativeOptions {
        viewport,
        renderer: eframe::Renderer::Wgpu,
        wgpu_options: egui_wgpu::WgpuConfiguration {
            wgpu_setup: egui_wgpu::WgpuSetup::CreateNew(egui_wgpu::WgpuSetupCreateNew {
                instance_descriptor: wgpu::InstanceDescriptor {
                    backends: wgpu::Backends::DX12 | wgpu::Backends::VULKAN | wgpu::Backends::GL,
                    flags: wgpu::InstanceFlags::from_env_or_default()
                        .union(wgpu::InstanceFlags::ALLOW_UNDERLYING_NONCOMPLIANT_ADAPTER),
                    ..Default::default()
                },
                native_adapter_selector: Some(std::sync::Arc::new(
                    |adapters: &[wgpu::Adapter], _surface| -> Result<wgpu::Adapter, String> {
                        for adapter in adapters {
                            let info = adapter.get_info();
                            if info.device_type != wgpu::DeviceType::Cpu {
                                return Ok(adapter.clone());
                            }
                        }
                        adapters.first().cloned().ok_or_else(|| "No adapters found".to_string())
                    },
                )),
                ..Default::default()
            }),
            ..Default::default()
        },
        ..Default::default()
    };

    let result = eframe::run_native(
        "AIVPN",
        options,
        Box::new(move |cc| {
            cc.egui_ctx.set_visuals(dark_visuals());

            let lang = Lang::load();
            let tray_mgr = tray::TrayManager::new(
                tray_icon_rgba,
                tray_icon_w,
                tray_icon_h,
                localization::t(lang, "show"),
                localization::t(lang, "quit"),
            ).ok();

            if tray_mgr.is_none() {
                eprintln!("Warning: failed to create tray icon");
            }

            Ok(Box::new(AivpnApp::new(tray_mgr)))
        }),
    );

    match &result {
        Ok(()) => write_startup_log(&log_path, "startup: run_native returned ok"),
        Err(err) => write_startup_log(&log_path, &format!("startup error: {err}")),
    }

    result
}

// ── Helpers ────────────────────────────────────────────────────────────────

fn decode_embedded_icon() -> (Vec<u8>, u32, u32) {
    let png_bytes = include_bytes!("../assets/aivpn_preview.png");
    match image::load_from_memory_with_format(png_bytes, image::ImageFormat::Png) {
        Ok(img) => {
            let rgba = img.to_rgba8();
            let (w, h) = rgba.dimensions();
            (rgba.into_raw(), w, h)
        }
        Err(e) => {
            eprintln!("Icon decode error: {}", e);
            (Vec::new(), 0, 0)
        }
    }
}

fn startup_log_path() -> PathBuf {
    dirs::data_local_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("AIVPN")
        .join("startup.log")
}

fn write_startup_log(path: &PathBuf, message: &str) {
    if let Some(parent) = path.parent() {
        let _ = fs::create_dir_all(parent);
    }
    let line = format!("{}\r\n", message);
    let _ = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .and_then(|mut file| std::io::Write::write_all(&mut file, line.as_bytes()));
}

fn dark_visuals() -> egui::Visuals {
    let mut visuals = egui::Visuals::dark();
    visuals.window_corner_radius = egui::CornerRadius::same(8);
    visuals.widgets.noninteractive.bg_fill = egui::Color32::from_rgb(30, 30, 35);
    visuals.panel_fill = egui::Color32::from_rgb(25, 25, 30);
    visuals
}

// ── Application state ──────────────────────────────────────────────────────

pub struct AivpnApp {
    vpn: VpnManager,
    keys: KeyStorage,
    lang: Lang,
    // UI state
    show_add_key: bool,
    new_key_name: String,
    new_key_value: String,
    new_key_full_tunnel: bool,
    editing_key_idx: Option<usize>,
    error_message: Option<String>,
    error_timer: Option<std::time::Instant>,
    // Recording UI state
    recording_service_name: String,
    // Tray
    tray: Option<tray::TrayManager>,
    pub should_quit: bool,
    window_visible: bool,
}

impl AivpnApp {
    fn new(tray: Option<tray::TrayManager>) -> Self {
        Self {
            vpn: VpnManager::new(),
            keys: KeyStorage::load(),
            lang: Lang::load(),
            show_add_key: false,
            new_key_name: String::new(),
            new_key_value: String::new(),
            new_key_full_tunnel: false,
            editing_key_idx: None,
            error_message: None,
            error_timer: None,
            recording_service_name: String::new(),
            tray,
            should_quit: false,
            window_visible: true,
        }
    }

    fn set_error(&mut self, msg: String) {
        self.error_message = Some(msg);
        self.error_timer = Some(std::time::Instant::now());
    }

    fn clear_old_error(&mut self) {
        if let Some(timer) = self.error_timer {
            if timer.elapsed() > std::time::Duration::from_secs(8) {
                self.error_message = None;
                self.error_timer = None;
            }
        }
    }
}

// ── Main loop ──────────────────────────────────────────────────────────────

impl eframe::App for AivpnApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.clear_old_error();
        self.vpn.poll_status();

        // Keep event loop ticking (for tooltip updates while window is visible)
        ctx.request_repaint_after(std::time::Duration::from_secs(1));

        // ── Intercept close (×) → hide to tray ────────────────────────
        if ctx.input(|i| i.viewport().close_requested()) {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            self.window_visible = false;
            win32_hide_window();
            // Note: after SW_HIDE, eframe may stop calling update().
            // The background tray thread handles events independently
            // and calls ShowWindow to restore when needed.
        }

        // ── Intercept minimize (_) → hide to tray ─────────────────────
        if ctx.input(|i| i.viewport().minimized.unwrap_or(false)) && self.window_visible {
            self.window_visible = false;
            win32_hide_window();
        }

        // ── Check for tray actions (set by background thread) ─────────
        if let Some(ref tm) = self.tray {
            let action = tm.take_action();
            match action {
                1 => {
                    // ACTION_SHOW: window was already restored by the tray thread
                    self.window_visible = true;
                }
                2 => {
                    // ACTION_QUIT
                    self.should_quit = true;
                }
                _ => {}
            }
        }

        // ── Handle quit ────────────────────────────────────────────────
        if self.should_quit {
            if self.vpn.is_connected() {
                self.vpn.disconnect();
            }
            // Drop tray icon to remove it from system tray
            self.tray = None;
            std::process::exit(0);
        }

        // ── Update tray tooltip ────────────────────────────────────────
        if let Some(ref tm) = self.tray {
            tm.update_tooltip(self.vpn.state(), self.vpn.stats());
        }

        // ── Draw UI ────────────────────────────────────────────────────
        egui::CentralPanel::default().show(ctx, |ui| {
            ui::draw_main_ui(ui, self);
        });
    }
}
