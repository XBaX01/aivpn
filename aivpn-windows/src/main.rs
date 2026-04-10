//! AIVPN Windows GUI Application
//!
//! Native Windows app using egui/eframe with system tray support.
//! Manages aivpn-client.exe as a subprocess — no console window visible.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod vpn_manager;
mod key_storage;
mod localization;
mod tray;
mod ui;

use eframe::egui;
use key_storage::KeyStorage;
use localization::Lang;
use vpn_manager::VpnManager;

const APP_VERSION: &str = env!("CARGO_PKG_VERSION");
const WINDOW_WIDTH: f32 = 360.0;
const WINDOW_HEIGHT: f32 = 480.0;

fn main() -> eframe::Result<()> {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([WINDOW_WIDTH, WINDOW_HEIGHT])
            .with_min_inner_size([WINDOW_WIDTH, WINDOW_HEIGHT])
            .with_max_inner_size([WINDOW_WIDTH, WINDOW_HEIGHT])
            .with_resizable(false)
            .with_decorations(true)
            .with_title("AIVPN"),
        ..Default::default()
    };

    eframe::run_native(
        "AIVPN",
        options,
        Box::new(|cc| {
            cc.egui_ctx.set_visuals(dark_visuals());
            Ok(Box::new(AivpnApp::new()))
        }),
    )
}

fn dark_visuals() -> egui::Visuals {
    let mut visuals = egui::Visuals::dark();
    visuals.window_corner_radius = egui::CornerRadius::same(8);
    visuals.widgets.noninteractive.bg_fill = egui::Color32::from_rgb(30, 30, 35);
    visuals.panel_fill = egui::Color32::from_rgb(25, 25, 30);
    visuals
}

/// Main application state
pub struct AivpnApp {
    vpn: VpnManager,
    keys: KeyStorage,
    lang: Lang,
    // UI state
    show_add_key: bool,
    new_key_name: String,
    new_key_value: String,
    editing_key_idx: Option<usize>,
    error_message: Option<String>,
    error_timer: Option<std::time::Instant>,
}

impl AivpnApp {
    fn new() -> Self {
        let keys = KeyStorage::load();
        Self {
            vpn: VpnManager::new(),
            keys,
            lang: Lang::load(),
            show_add_key: false,
            new_key_name: String::new(),
            new_key_value: String::new(),
            editing_key_idx: None,
            error_message: None,
            error_timer: None,
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

impl eframe::App for AivpnApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.clear_old_error();
        self.vpn.poll_status();

        // Request repaint every second for live stats
        ctx.request_repaint_after(std::time::Duration::from_secs(1));

        egui::CentralPanel::default().show(ctx, |ui| {
            ui::draw_main_ui(ui, self);
        });
    }
}
