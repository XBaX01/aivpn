//! UI drawing — egui layout matching macOS AIVPN style
//!
//! Dark theme, compact, 360×480 window.

use eframe::egui::{self, Color32, CornerRadius, RichText, Vec2};
use crate::vpn_manager::{ConnectionState, format_bytes};
use crate::localization::{Lang, t};
use crate::AivpnApp;
use crate::APP_VERSION;

// Color palette
const GREEN: Color32 = Color32::from_rgb(0x4C, 0xD9, 0x64);
const RED: Color32 = Color32::from_rgb(0xFF, 0x3B, 0x30);
const ORANGE: Color32 = Color32::from_rgb(0xFF, 0x9F, 0x0A);
const BLUE: Color32 = Color32::from_rgb(0x00, 0x7A, 0xFF);
const PURPLE: Color32 = Color32::from_rgb(0x9B, 0x59, 0xB6);
const DIM: Color32 = Color32::from_rgb(0x8E, 0x8E, 0x93);
const CARD_BG: Color32 = Color32::from_rgb(0x2C, 0x2C, 0x30);
const SELECTED_BG: Color32 = Color32::from_rgb(0x1C, 0x3A, 0x5C);

pub fn draw_main_ui(ui: &mut egui::Ui, app: &mut AivpnApp) {
    ui.spacing_mut().item_spacing = Vec2::new(8.0, 6.0);

    // Header: title + language toggle
    draw_header(ui, app);

    ui.add_space(4.0);

    // Connection status card
    draw_status_card(ui, app);

    ui.add_space(4.0);

    // Traffic stats (when connected)
    if app.vpn.is_connected() {
        draw_traffic_stats(ui, app);
        ui.add_space(4.0);
    }

    // Keys section
    draw_keys_section(ui, app);

    ui.add_space(4.0);

    // Connect/Disconnect button
    draw_connect_button(ui, app);

    // Error message
    if let Some(ref msg) = app.error_message {
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            ui.label(RichText::new("⚠").color(ORANGE));
            ui.label(RichText::new(msg).color(ORANGE).size(12.0));
        });
    }

    // Footer
    ui.with_layout(egui::Layout::bottom_up(egui::Align::Center), |ui| {
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            ui.label(
                RichText::new(format!("{} {}", t(app.lang, "version"), APP_VERSION))
                    .color(DIM)
                    .size(11.0),
            );
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui
                    .small_button(RichText::new(t(app.lang, "quit")).color(DIM).size(11.0))
                    .clicked()
                {
                    if app.vpn.is_connected() {
                        app.vpn.disconnect();
                    }
                    std::process::exit(0);
                }
            });
        });
    });
}

fn draw_header(ui: &mut egui::Ui, app: &mut AivpnApp) {
    ui.horizontal(|ui| {
        ui.label(RichText::new("AIVPN").size(20.0).strong().color(PURPLE));

        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let lang_btn = ui.button(
                RichText::new(app.lang.label())
                    .size(12.0)
                    .color(BLUE),
            );
            if lang_btn.clicked() {
                app.lang.toggle();
            }
        });
    });
}

fn draw_status_card(ui: &mut egui::Ui, app: &AivpnApp) {
    egui::Frame::new()
        .fill(CARD_BG)
        .corner_radius(CornerRadius::same(8))
        .inner_margin(12.0)
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                // Status dot
                let (color, label_key) = match app.vpn.state() {
                    ConnectionState::Connected => (GREEN, "connected"),
                    ConnectionState::Connecting => (ORANGE, "connecting"),
                    ConnectionState::Disconnecting => (ORANGE, "disconnecting"),
                    ConnectionState::Disconnected => (RED, "disconnected"),
                };

                let (rect, _) = ui.allocate_exact_size(Vec2::new(12.0, 12.0), egui::Sense::hover());
                ui.painter().circle_filled(rect.center(), 6.0, color);

                ui.label(
                    RichText::new(t(app.lang, label_key))
                        .size(15.0)
                        .color(color),
                );
            });

            // Show selected key info
            if let Some(key) = app.keys.selected_key() {
                ui.add_space(4.0);
                if !key.server_addr.is_empty() {
                    ui.label(
                        RichText::new(format!("Server: {}", key.server_addr))
                            .size(11.0)
                            .color(DIM),
                    );
                }
            }
        });
}

fn draw_traffic_stats(ui: &mut egui::Ui, app: &AivpnApp) {
    let stats = app.vpn.stats();
    let up_to_date = stats.bytes_sent > 0 || stats.bytes_received > 0;
    
    egui::Frame::new()
        .fill(CARD_BG)
        .corner_radius(CornerRadius::same(8))
        .inner_margin(12.0)
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(
                    RichText::new(t(app.lang, "traffic"))
                        .size(13.0)
                        .color(DIM),
                );
                if !up_to_date {
                    ui.add_space(8.0);
                    ui.label(
                        RichText::new("waiting...")
                            .size(11.0)
                            .color(DIM),
                    );
                }
            });
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                // Download - smooth arrow
                ui.label(
                    RichText::new("⬇")
                        .size(18.0)
                        .color(GREEN),
                );
                ui.add_space(4.0);
                ui.label(
                    RichText::new(format_bytes(stats.bytes_received))
                        .size(15.0)
                        .color(Color32::WHITE)
                        .strong(),
                );
                
                ui.add_space(32.0);
                
                // Upload - smooth arrow
                ui.label(
                    RichText::new("⬆")
                        .size(18.0)
                        .color(BLUE),
                );
                ui.add_space(4.0);
                ui.label(
                    RichText::new(format_bytes(stats.bytes_sent))
                        .size(15.0)
                        .color(Color32::WHITE)
                        .strong(),
                );
            });
        });
}

fn draw_keys_section(ui: &mut egui::Ui, app: &mut AivpnApp) {
    ui.horizontal(|ui| {
        ui.label(RichText::new(t(app.lang, "keys")).size(13.0).color(DIM));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui
                .button(RichText::new("+").size(14.0).strong().color(BLUE))
                .clicked()
            {
                app.show_add_key = true;
                app.editing_key_idx = None;
                app.new_key_name.clear();
                app.new_key_value.clear();
            }
        });
    });

    // Add/Edit key form
    if app.show_add_key {
        draw_key_form(ui, app);
    }

    // Key list
    egui::Frame::new()
        .fill(CARD_BG)
        .corner_radius(CornerRadius::same(8))
        .inner_margin(8.0)
        .show(ui, |ui| {
            if app.keys.keys.is_empty() {
                ui.add_space(8.0);
                ui.vertical(|ui| {
                    ui.allocate_ui(Vec2::new(ui.available_width(), 60.0), |ui| {
                        ui.with_layout(egui::Layout::top_down(egui::Align::Center), |ui| {
                            ui.label(
                                RichText::new("📋")
                                    .size(28.0)
                                    .color(DIM),
                            );
                            ui.add_space(4.0);
                            ui.label(
                                RichText::new(t(app.lang, "no_keys"))
                                    .size(12.0)
                                    .color(DIM),
                            );
                            ui.add_space(4.0);
                            if ui
                                .button(
                                    RichText::new(t(app.lang, "add_key"))
                                        .size(12.0)
                                        .color(BLUE),
                                )
                                .clicked()
                            {
                                app.show_add_key = true;
                                app.editing_key_idx = None;
                                app.new_key_name.clear();
                                app.new_key_value.clear();
                            }
                        });
                    });
                });
            } else {
                let selected = app.keys.selected;
                let mut action: Option<KeyAction> = None;

                egui::ScrollArea::vertical()
                    .max_height(140.0)
                    .show(ui, |ui| {
                        for (idx, key) in app.keys.keys.iter().enumerate() {
                            let is_selected = selected == Some(idx);
                            
                            // Allocate space for the row
                            let desired_height = if key.full_tunnel { 44.0 } else { 36.0 };
                            let (rect, response) = ui.allocate_exact_size(
                                Vec2::new(ui.available_width(), desired_height),
                                egui::Sense::click(),
                            );
                            
                            // Draw background
                            let connected = app.vpn.is_connected();
                            let bg_fill = if is_selected {
                                SELECTED_BG
                            } else if connected {
                                Color32::from_rgba_premultiplied(44, 44, 48, 100)
                            } else {
                                Color32::TRANSPARENT
                            };
                            if bg_fill != Color32::TRANSPARENT {
                                ui.painter().rect_filled(
                                    rect.expand(1.0),
                                    egui::CornerRadius::same(4),
                                    bg_fill,
                                );
                            }
                            
                            // Handle click for selection (only when disconnected)
                            if response.clicked() && !app.vpn.is_connected() {
                                action = Some(KeyAction::Select(idx));
                            }
                            
                            // Draw content manually with painter
                            let text_color = if is_selected { Color32::WHITE } else { Color32::from_rgb(0xC0, 0xC0, 0xC0) };
                            let name_font_id = egui::FontId::new(13.0, egui::FontFamily::Proportional);
                            let addr_font_id = egui::FontId::new(10.0, egui::FontFamily::Proportional);
                            
                            // Key name
                            let name_pos = rect.left_top() + Vec2::new(8.0, 6.0);
                            ui.painter().text(name_pos, egui::Align2::LEFT_TOP, &key.name, name_font_id, text_color);
                            
                            // Full tunnel indicator
                            if key.full_tunnel {
                                let ft_pos = rect.left_top() + Vec2::new(8.0, 22.0);
                                ui.painter().text(ft_pos, egui::Align2::LEFT_TOP, t(app.lang, "full_tunnel"), egui::FontId::new(9.0, egui::FontFamily::Proportional), GREEN);
                            }
                            
                            // Server address
                            if !key.server_addr.is_empty() {
                                let addr_y = if key.full_tunnel { 32.0 } else { 22.0 };
                                let addr_pos = rect.left_top() + Vec2::new(8.0, addr_y);
                                ui.painter().text(addr_pos, egui::Align2::LEFT_TOP, &key.server_addr, addr_font_id, DIM);
                            }
                            
                            // Edit button (right side)
                            let edit_text = t(app.lang, "edit");
                            let edit_btn_rect = egui::Rect::from_min_size(
                                rect.right_top() - Vec2::new(80.0, 0.0),
                                Vec2::new(36.0, 20.0),
                            );
                            let edit_btn_response = ui.interact(edit_btn_rect, ui.id().with(("edit", idx)), egui::Sense::click());
                            if edit_btn_response.hovered() {
                                ui.painter().rect_filled(edit_btn_rect, egui::CornerRadius::same(3), Color32::from_rgb(0x50, 0x50, 0x55));
                            }
                            if edit_btn_response.clicked() {
                                action = Some(KeyAction::Edit(idx));
                            }
                            ui.painter().text(
                                edit_btn_rect.center(),
                                egui::Align2::CENTER_CENTER,
                                edit_text,
                                egui::FontId::new(11.0, egui::FontFamily::Proportional),
                                ORANGE,
                            );
                            
                            // Delete button
                            let del_text = t(app.lang, "delete");
                            let del_btn_rect = egui::Rect::from_min_size(
                                rect.right_top() - Vec2::new(40.0, 0.0),
                                Vec2::new(36.0, 20.0),
                            );
                            let del_btn_response = ui.interact(del_btn_rect, ui.id().with(("del", idx)), egui::Sense::click());
                            if del_btn_response.hovered() {
                                ui.painter().rect_filled(del_btn_rect, egui::CornerRadius::same(3), Color32::from_rgb(0x50, 0x30, 0x30));
                            }
                            if del_btn_response.clicked() {
                                action = Some(KeyAction::Delete(idx));
                            }
                            ui.painter().text(
                                del_btn_rect.center(),
                                egui::Align2::CENTER_CENTER,
                                del_text,
                                egui::FontId::new(11.0, egui::FontFamily::Proportional),
                                RED,
                            );
                        }
                    });

                // Handle actions after iteration
                match action {
                    Some(KeyAction::Select(idx)) => {
                        app.keys.selected = Some(idx);
                        app.keys.save();
                    }
                    Some(KeyAction::Delete(idx)) => {
                        app.keys.remove_key(idx);
                    }
                    Some(KeyAction::Edit(idx)) => {
                        let key = &app.keys.keys[idx];
                        app.new_key_name = key.name.clone();
                        app.new_key_value = key.key.clone();
                        app.new_key_full_tunnel = key.full_tunnel;
                        app.editing_key_idx = Some(idx);
                        app.show_add_key = true;
                    }
                    None => {}
                }
            }
        });
}

enum KeyAction {
    Select(usize),
    Delete(usize),
    Edit(usize),
}

fn draw_key_form(ui: &mut egui::Ui, app: &mut AivpnApp) {
    egui::Frame::new()
        .fill(Color32::from_rgb(0x38, 0x38, 0x3C))
        .corner_radius(CornerRadius::same(8))
        .inner_margin(10.0)
        .show(ui, |ui| {
            ui.label(RichText::new(t(app.lang, "key_name")).size(11.0).color(DIM));
            ui.add(
                egui::TextEdit::singleline(&mut app.new_key_name)
                    .desired_width(f32::INFINITY)
                    .hint_text("My Server"),
            );

            ui.add_space(4.0);
            ui.label(RichText::new(t(app.lang, "key_value")).size(11.0).color(DIM));
            ui.add(
                egui::TextEdit::singleline(&mut app.new_key_value)
                    .desired_width(f32::INFINITY)
                    .hint_text("aivpn://..."),
            );

            ui.add_space(6.0);
            ui.horizontal(|ui| {
                ui.checkbox(&mut app.new_key_full_tunnel, "");
                ui.label(RichText::new(t(app.lang, "full_tunnel")).size(12.0).color(Color32::from_rgb(0x90, 0xEE, 0x90)));
            });
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                let save_clicked = ui
                    .button(RichText::new(t(app.lang, "save")).color(GREEN))
                    .clicked();
                let cancel_clicked = ui
                    .button(RichText::new(t(app.lang, "cancel")).color(DIM))
                    .clicked();

                if save_clicked {
                    let name = app.new_key_name.clone();
                    let value = app.new_key_value.clone();
                    let full_tunnel = app.new_key_full_tunnel;
                    let result = if let Some(idx) = app.editing_key_idx {
                        app.keys.update_key(idx, &name, &value, full_tunnel)
                    } else {
                        app.keys.add_key(&name, &value, full_tunnel)
                    };
                    match result {
                        Ok(()) => {
                            app.show_add_key = false;
                            app.editing_key_idx = None;
                            app.new_key_name.clear();
                            app.new_key_value.clear();
                            app.new_key_full_tunnel = false;
                        }
                        Err(e) => app.set_error(e),
                    }
                }
                if cancel_clicked {
                    app.show_add_key = false;
                    app.editing_key_idx = None;
                    app.new_key_name.clear();
                    app.new_key_value.clear();
                    app.new_key_full_tunnel = false;
                }
            });
        });
}

fn draw_connect_button(ui: &mut egui::Ui, app: &mut AivpnApp) {
    let is_connected = app.vpn.is_connected();
    let is_busy = app.vpn.is_busy();

    let (label, color) = if is_connected {
        (t(app.lang, "disconnect"), RED)
    } else {
        (t(app.lang, "connect"), GREEN)
    };

    ui.add_enabled_ui(!is_busy, |ui| {
        let btn = egui::Button::new(
            RichText::new(label)
                .size(16.0)
                .color(Color32::WHITE)
                .strong(),
        )
        .fill(color)
        .corner_radius(CornerRadius::same(8))
        .min_size(Vec2::new(ui.available_width(), 40.0));

        if ui.add(btn).clicked() {
            if is_connected {
                app.vpn.disconnect();
            } else {
                match app.keys.selected_key() {
                    Some(key) => {
                        let key_str = key.key.clone();
                        let full_tunnel = key.full_tunnel;
                        if let Err(e) = app.vpn.connect(&key_str, full_tunnel) {
                            app.set_error(e);
                        }
                    }
                    None => {
                        app.set_error(t(app.lang, "no_key_selected").to_string());
                    }
                }
            }
        }
    });
}
