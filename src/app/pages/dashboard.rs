//! 仪表盘页面。

use super::super::{App, Page};
use crate::icons;
use crate::localtime::Timestamp;
use crate::theme::colors;
use crate::widgets::{self, action_button, action_button_width};
use eframe::egui;
use egui::{Stroke, Vec2};

pub(in crate::app) fn dashboard_page(ui: &mut egui::Ui, _ctx: &egui::Context, app: &mut App) {
    let today = Timestamp::now();
    let has_threats = app
        .core
        .config
        .last_full_scan
        .as_ref()
        .map(|r| r.threats_found > 0)
        .unwrap_or(false)
        || app
            .core
            .config
            .last_quick_scan
            .as_ref()
            .map(|r| r.threats_found > 0)
            .unwrap_or(false);

    let mut content_height = app.core.dashboard_content_height;
    widgets::vertically_centered(ui, &mut content_height, |ui| {
        let (color, title) = if has_threats {
            (colors::RED, "System Status: At Risk")
        } else {
            (colors::GREEN, "System Status: Secure")
        };

        const DIAMETER: f32 = 180.0;
        const GLOW_MARGIN: f32 = 60.0;
        let (response, painter) = ui.allocate_painter(
            Vec2::splat(DIAMETER + GLOW_MARGIN * 2.0),
            egui::Sense::hover(),
        );
        let center = response.rect.center();
        let radius = DIAMETER / 2.0 - 4.0;
        widgets::paint_glow(&painter, center, radius, color);
        painter.circle_filled(center, radius, colors::BG_CARD);
        painter.circle_stroke(center, radius, Stroke::new(3.0, color));
        let glyph_rect = egui::Rect::from_center_size(center, Vec2::splat(DIAMETER * 0.46));
        if has_threats {
            icons::status_glyph_at_risk(&painter, glyph_rect, color);
        } else {
            icons::status_glyph_secure(&painter, glyph_rect, color);
        }

        ui.add_space(20.0);
        widgets::bold_label(ui, title, 20.0, colors::TEXT_PRIMARY);
        ui.add_space(6.0);
        let sub = match &app.core.config.last_full_scan {
            Some(r) if r.threats_found == 0 => {
                format!(
                    "Last Full Scan · {} · No threats found",
                    r.time.display_relative_to(&today)
                )
            }
            Some(r) => format!(
                "Last Full Scan · {} · {} threat(s) found",
                r.time.display_relative_to(&today),
                r.threats_found
            ),
            None => "No full scan performed yet".to_string(),
        };
        ui.label(egui::RichText::new(sub).color(colors::TEXT_SECONDARY));

        ui.add_space(28.0);
        const BTN_GAP: f32 = 12.0;
        let row_width =
            action_button_width(ui, "Quick Scan") + BTN_GAP + action_button_width(ui, "Full Scan");
        ui.allocate_ui_with_layout(
            Vec2::new(row_width, 42.0),
            egui::Layout::left_to_right(egui::Align::Center),
            |ui| {
                if action_button(ui, "Quick Scan", |p, r, s| icons::bolt(p, r, s.color)) {
                    app.navigate(Page::QuickScan);
                    if app.core.any_scan_running() {
                        app.toast("Finish the current scan before starting another");
                    } else {
                        let removable = app.core.config.scan_removable_drives;
                        app.core.quick.start(removable);
                    }
                }
                ui.add_space(BTN_GAP);
                if action_button(ui, "Full Scan", icons::computer) {
                    app.navigate(Page::FullScan);
                    if app.core.any_scan_running() {
                        app.toast("Finish the current scan before starting another");
                    } else {
                        let removable = app.core.config.scan_removable_drives;
                        app.core.full.start(removable);
                    }
                }
            },
        );
    });
    app.core.dashboard_content_height = content_height;
}
