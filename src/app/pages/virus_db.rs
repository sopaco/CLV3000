//! 病毒库页面。

use super::super::App;
use crate::icons;
use crate::paths;
use crate::theme::colors;
use crate::widgets::{self, action_button, action_button_width};
use eframe::egui;
use egui::{Stroke, Vec2};

pub(in crate::app) fn virus_db_page(ui: &mut egui::Ui, app: &mut App) {
    ui.add_space(28.0);
    ui.columns(2, |columns| {
        virus_db_status_column(&mut columns[0], app);
        virus_db_about_column(&mut columns[1], app);
    });
}

fn virus_db_status_column(ui: &mut egui::Ui, app: &mut App) {
    let core = &mut app.core;
    let mut content_height = core.virus_db.status_col_height;
    let mut pending_toast: Option<String> = None;
    widgets::vertically_centered(ui, &mut content_height, |ui| {
        let (response, painter) = ui.allocate_painter(Vec2::splat(96.0), egui::Sense::hover());
        icons::database(
            &painter,
            response.rect.shrink(6.0),
            Stroke::new(2.0, colors::ACCENT_BLUE),
        );
        ui.add_space(14.0);
        widgets::bold_label(ui, "Virus Database", 18.0, colors::TEXT_PRIMARY);
        ui.add_space(14.0);

        let (available, detail_dir) = {
            let probe = core.virus_db.engine_probe();
            (probe.available, probe.detail_dir.clone())
        };
        if core.virus_db.db_version.is_none() {
            core.virus_db.refresh_db_version(ui.ctx().clone());
        }
        let status = if available {
            "Built-in database ready"
        } else {
            "Scan engine not found"
        };
        ui.label(egui::RichText::new(status).color(colors::TEXT_SECONDARY));

        if let Some(ver) = &core.virus_db.db_version {
            ui.label(
                egui::RichText::new(format!("Version: {ver}"))
                    .color(colors::TEXT_MUTED)
                    .small(),
            );
        }

        ui.add_space(16.0);
        let update_label = if core.virus_db.updating {
            "Updating…"
        } else {
            "Update Database"
        };
        const BTN_GAP: f32 = 12.0;
        let row_width = action_button_width(ui, "Open Folder")
            + BTN_GAP
            + action_button_width(ui, update_label);
        ui.allocate_ui_with_layout(
            Vec2::new(row_width, 42.0),
            egui::Layout::left_to_right(egui::Align::Center),
            |ui| {
                if action_button(ui, "Open Folder", icons::folder) {
                    paths::ensure_dir(&detail_dir);
                    if let Err(e) = paths::open_in_file_explorer(&detail_dir) {
                        pending_toast = Some(e);
                    }
                }
                ui.add_space(BTN_GAP);
                if action_button(ui, update_label, icons::database) && !core.virus_db.updating {
                    core.virus_db.start_update(ui.ctx().clone());
                    pending_toast = Some("Updating database…".to_string());
                }
            },
        );
    });
    core.virus_db.status_col_height = content_height;
    if let Some(msg) = pending_toast {
        app.toast(msg);
    }
}

fn virus_db_about_column(ui: &mut egui::Ui, app: &mut App) {
    let mut content_height = app.core.virus_db.about_col_height;
    widgets::vertically_centered(ui, &mut content_height, |ui| {
        const LOGO_DISPLAY_PT: f32 = 120.0;
        if let Some(tex) = &app.app_icon_texture {
            ui.add(
                egui::Image::new((tex.id(), tex.size_vec2()))
                    .fit_to_exact_size(Vec2::splat(LOGO_DISPLAY_PT))
                    .corner_radius(16.0),
            );
        }
        ui.add_space(12.0);
        widgets::bold_label(ui, "CLV3000", 17.0, colors::TEXT_PRIMARY);
        ui.add_space(4.0);
        ui.label(
            egui::RichText::new(format!("Version {}", env!("CARGO_PKG_VERSION")))
                .color(colors::TEXT_SECONDARY)
                .small(),
        );
        ui.add_space(10.0);
        ui.label(
            egui::RichText::new("Fast, reliable virus protection for even older PCs")
                .color(colors::TEXT_MUTED)
                .small(),
        );
    });
    app.core.virus_db.about_col_height = content_height;
}
