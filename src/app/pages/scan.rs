//! 闪电扫描 / 全盘扫描页面与共用的 `scan_page` 渲染。

use super::super::core::{AppCore, ScanPageState, ScanPhase};
use super::super::util::{format_duration, truncate};
use super::super::App;
use crate::config::AppConfig;
use crate::icons;
use crate::scan::ScanKind;
use crate::theme::colors;
use crate::widgets::{self, action_button, action_button_width, ThreatAction, Toast};
use eframe::egui;
use egui::{Color32, Stroke, Vec2};
use std::time::Duration;

pub(in crate::app) fn quick_scan_page(ui: &mut egui::Ui, app: &mut App) {
    let other_running = app.core.full.is_running();
    let AppCore { quick, config, .. } = &mut app.core;
    scan_page(
        ui,
        quick,
        config,
        &mut app.toasts,
        "Quick Scan",
        colors::ACCENT_BLUE,
        |p, r, s| icons::bolt(p, r, s.color),
        true,
        other_running,
    );
}

pub(in crate::app) fn full_scan_page(ui: &mut egui::Ui, app: &mut App) {
    let other_running = app.core.quick.is_running();
    let AppCore { full, config, .. } = &mut app.core;
    scan_page(
        ui,
        full,
        config,
        &mut app.toasts,
        "Full Scan",
        colors::ACCENT_BLUE,
        icons::computer,
        true,
        other_running,
    );
}

#[allow(clippy::too_many_arguments)]
fn scan_page(
    ui: &mut egui::Ui,
    state: &mut ScanPageState,
    config: &mut AppConfig,
    toasts: &mut Vec<Toast>,
    title: &str,
    ring_color: Color32,
    icon: fn(&egui::Painter, egui::Rect, Stroke),
    show_start_button_when_idle: bool,
    other_running: bool,
) {
    let mut content_height = state.content_height;
    widgets::vertically_centered(ui, &mut content_height, |ui| {
        match &state.phase {
            ScanPhase::Idle => {
                const IDLE_RING_GLYPH: f32 = 52.0;
                const FULL_SCAN_IDLE_RING_GLYPH_SCALE: f32 = 1.25;
                let glyph_size = if state.kind == ScanKind::Full {
                    IDLE_RING_GLYPH * FULL_SCAN_IDLE_RING_GLYPH_SCALE
                } else {
                    IDLE_RING_GLYPH
                };

                let (deco_resp, painter) =
                    ui.allocate_painter(Vec2::splat(120.0), egui::Sense::hover());
                let deco_center = deco_resp.rect.center();
                let deco_radius = 56.0;
                painter.circle_filled(deco_center, deco_radius, colors::BG_CARD);
                painter.circle_stroke(deco_center, deco_radius, Stroke::new(2.0, ring_color));
                let deco_glyph = egui::Rect::from_center_size(deco_center, Vec2::splat(glyph_size));
                icon(&painter, deco_glyph, Stroke::new(2.0, ring_color));

                ui.add_space(14.0);
                ui.label(
                    egui::RichText::new(format!("Ready for {title}")).color(colors::TEXT_SECONDARY),
                );
                ui.add_space(16.0);
                const START_BTN_SHIFT_LEFT: f32 = 2.0;
                ui.horizontal(|ui| {
                    let label = format!("Start {title}");
                    let btn_w = action_button_width(ui, &label);
                    let left = (ui.available_width() - btn_w) / 2.0 - START_BTN_SHIFT_LEFT;
                    ui.add_space(left.max(0.0));
                    if show_start_button_when_idle && action_button(ui, &label, icon) {
                        if other_running {
                            toasts.push(Toast::new(
                                "Finish the current scan before starting another",
                            ));
                        } else {
                            state.start(config.scan_removable_drives);
                        }
                    }
                });
            }
            ScanPhase::Enumerating {
                done,
                total,
                files_found,
            } => {
                widgets::progress_ring(
                    ui,
                    220.0,
                    None,
                    ring_color,
                    "Enumerating",
                    &format!("{done}/{total} processes"),
                );
                ui.add_space(16.0);
                widgets::centered_stat_pills(
                    ui,
                    &[
                        (format!("{done} / {total}"), "processes"),
                        (files_found.to_string(), "files"),
                    ],
                );
                ui.add_space(10.0);
                if ui.link("Cancel Scan").clicked() {
                    state.request_cancel();
                }
            }
            ScanPhase::Scanning {
                total,
                scanned,
                current_path,
            } => {
                let disk_walking = state.kind == ScanKind::Full
                    && total.is_none()
                    && *scanned == 0
                    && current_path.is_empty();

                let percent = if disk_walking {
                    None
                } else {
                    total.map(|t| {
                        if t == 0 {
                            1.0
                        } else {
                            *scanned as f32 / t as f32
                        }
                    })
                };

                let title_text = if disk_walking {
                    if state.walk_files_found > 0 {
                        state.walk_files_found.to_string()
                    } else {
                        "…".to_string()
                    }
                } else {
                    percent
                        .map(|p| format!("{:.0}%", p * 100.0))
                        .unwrap_or_else(|| format!("{scanned}"))
                };

                let heading = if disk_walking {
                    format!("Preparing {title}")
                } else if *scanned == 0 && current_path.is_empty() && state.engine_loading {
                    format!("Starting {title}")
                } else {
                    format!("Running {title}")
                };

                widgets::bold_label(ui, &heading, 14.0, colors::TEXT_PRIMARY);
                ui.add_space(6.0);
                widgets::progress_ring(
                    ui,
                    220.0,
                    percent,
                    ring_color,
                    &title_text,
                    if disk_walking {
                        "inspect files"
                    } else if *scanned == 0 && current_path.is_empty() && state.engine_loading {
                        "scan engine"
                    } else {
                        ""
                    },
                );
                ui.add_space(4.0);
                let status_line = if let Some(p) = &state.engine_scanning_path {
                    truncate(p, 60)
                } else if disk_walking {
                    if state.walk_files_found > 0 {
                        format!(
                            "Finding key files on disk… {n} to scan",
                            n = state.walk_files_found
                        )
                    } else {
                        "Finding executables on disk…".to_string()
                    }
                } else if state.engine_loading && state.engine_loading_remaining > 0 {
                    format!(
                        "Loading scan engine ({remaining} files)…",
                        remaining = state.engine_loading_remaining
                    )
                } else if state.engine_loading && !current_path.is_empty() {
                    format!("{} — loading engine…", truncate(current_path, 48))
                } else if state.engine_loading {
                    "Loading scan engine…".to_string()
                } else if current_path.is_empty() {
                    "Scanning…".to_string()
                } else {
                    truncate(current_path, 60)
                };
                ui.label(
                    egui::RichText::new(status_line)
                        .color(colors::TEXT_SECONDARY)
                        .small(),
                );
                ui.add_space(4.0);
                if let Some(started) = state.started_at.as_ref() {
                    ui.label(
                        egui::RichText::new(format!(
                            "Elapsed {}",
                            format_duration(started.elapsed())
                        ))
                        .color(colors::TEXT_SECONDARY)
                        .small(),
                    );
                }
                ui.add_space(16.0);
                let first_pill = if disk_walking {
                    (state.walk_files_found.to_string(), "to scan")
                } else {
                    match total {
                        Some(t) => (format!("{scanned} / {t}"), "files"),
                        None => (scanned.to_string(), "scanned"),
                    }
                };
                widgets::centered_stat_pills(
                    ui,
                    &[first_pill, (state.threats.len().to_string(), "threats")],
                );
                ui.add_space(10.0);
                if ui.link("Cancel Scan").clicked() {
                    state.request_cancel();
                }
                ui.ctx().request_repaint_after(Duration::from_millis(33));
            }
            ScanPhase::Done {
                scanned,
                elapsed,
                cancelled,
            } => {
                let has_threats = !state.threats.is_empty();
                let color = if has_threats {
                    colors::RED
                } else {
                    colors::GREEN
                };
                const DIAMETER: f32 = 140.0;
                const GLOW_MARGIN: f32 = 50.0;
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
                ui.add_space(14.0);
                let heading = if *cancelled {
                    "Scan cancelled".to_string()
                } else if has_threats {
                    format!("{} threat(s) found", state.threats.len())
                } else {
                    "No threats found".to_string()
                };
                widgets::bold_label(ui, &heading, 18.0, colors::TEXT_PRIMARY);
                ui.label(
                    egui::RichText::new(format!(
                        "{title} · Duration {} · {scanned} files scanned",
                        format_duration(*elapsed)
                    ))
                    .color(colors::TEXT_SECONDARY)
                    .small(),
                );
                ui.add_space(16.0);
                if action_button(ui, &format!("Run {title} Again"), icon) {
                    if other_running {
                        toasts.push(Toast::new(
                            "Finish the current scan before starting another",
                        ));
                    } else {
                        state.start(config.scan_removable_drives);
                    }
                }
            }
        }

        if let Some(err) = &state.last_error {
            ui.add_space(10.0);
            ui.label(egui::RichText::new(err).color(colors::RED));
        }

        ui.add_space(20.0);
        let mut ignore_target: Option<usize> = None;
        let mut quarantine_target: Option<usize> = None;
        for (i, threat) in state.threats.iter().enumerate() {
            ui.horizontal(|ui| {
                ui.add_space((ui.available_width() - 700.0).max(0.0) / 2.0);
                ui.vertical(|ui| {
                    ui.set_width(700.0_f32.min(ui.available_width()));
                    let path_str = threat.path.display().to_string();
                    let action = widgets::threat_card(ui, &threat.virus_name, &path_str);
                    match action {
                        ThreatAction::Ignore => ignore_target = Some(i),
                        ThreatAction::Quarantine => quarantine_target = Some(i),
                        ThreatAction::None => {}
                    }
                    ui.add_space(8.0);
                });
            });
        }
        if let Some(i) = ignore_target {
            let t = state.threats.remove(i);
            config.add_ignored(t.path.display().to_string(), t.virus_name);
        }
        if let Some(i) = quarantine_target {
            let t = &state.threats[i];
            match crate::quarantine::quarantine_file(&t.path, &t.virus_name) {
                Ok(entry) => {
                    config.add_quarantined(entry);
                    state.threats.remove(i);
                    toasts.push(Toast::new("File moved to quarantine"));
                }
                Err(e) => {
                    #[cfg(windows)]
                    {
                        if state.pending_force_quarantine.is_none()
                            && !state.is_force_quarantining()
                        {
                            state.pending_force_quarantine = Some(
                                super::super::core::PendingForceQuarantine {
                                path: t.path.clone(),
                                virus_name: t.virus_name.clone(),
                            });
                        } else {
                            toasts.push(Toast::new(e));
                        }
                    }
                    #[cfg(not(windows))]
                    {
                        toasts.push(Toast::new(e));
                    }
                }
            }
        }
    });
    state.content_height = content_height;

    #[cfg(windows)]
    {
        if state.pending_force_quarantine.is_some() {
            let path_display = state
                .pending_force_quarantine
                .as_ref()
                .unwrap()
                .path
                .display()
                .to_string();
            let mut confirmed = false;
            let mut cancelled = false;
            egui::Window::new("Force Quarantine")
                .collapsible(false)
                .resizable(false)
                .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
                .show(ui.ctx(), |ui| {
                    ui.add_space(4.0);
                    ui.label("The file is currently in use.");
                    ui.add_space(2.0);
                    ui.label(
                        egui::RichText::new(truncate(&path_display, 56))
                            .color(colors::TEXT_SECONDARY)
                            .small(),
                    );
                    ui.add_space(8.0);
                    ui.label("Do you want to force quarantine?");
                    ui.add_space(10.0);
                    ui.horizontal(|ui| {
                        ui.add_space(80.0);
                        if ui.button("Force Quarantine").clicked() {
                            confirmed = true;
                        }
                        ui.add_space(8.0);
                        if ui.button("Cancel").clicked() {
                            cancelled = true;
                        }
                    });
                });
            if confirmed {
                let pending = state.pending_force_quarantine.take().unwrap();
                state.start_force_quarantine(ui.ctx(), pending);
                toasts.push(Toast::new("Force quarantining…"));
            } else if cancelled {
                state.pending_force_quarantine = None;
            }
        }

        if let Some(result) = state.poll_force_quarantine() {
            match result {
                Ok((entry, path)) => {
                    config.add_quarantined(entry);
                    state.threats.retain(|t| t.path != path);
                    toasts.push(Toast::new("File moved to quarantine"));
                }
                Err(e) => {
                    toasts.push(Toast::new(e));
                }
            }
        }

        if state.is_force_quarantining() {
            ui.ctx()
                .request_repaint_after(std::time::Duration::from_millis(100));
        }
    }
}
