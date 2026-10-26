//! 闪电扫描 / 全盘扫描页面与共用的 `scan_page` 渲染。

use super::super::App;
use super::super::core::{AppCore, ScanPageState, ScanPhase};
use super::super::util::{format_duration, truncate};
use crate::config::AppConfig;
use crate::icons;
use crate::scan::ScanKind;
use crate::theme::colors;
use crate::widgets::{self, ThreatAction, Toast, action_button, action_button_width};
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
                if !state.threats.is_empty() {
                    ui.add_space(12.0);
                    if threats_entry_button(ui, state.threats.len()) {
                        state.threats_modal_open = true;
                    }
                }
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
                if has_threats && threats_entry_button(ui, state.threats.len()) {
                    state.threats_modal_open = true;
                }
                if has_threats {
                    ui.add_space(10.0);
                }
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
    });
    state.content_height = content_height;

    // 威胁列表不再内嵌页面：原来只占页面剩余高度的一小条滚动区，看不全也点不准。
    // 页面里只留「数量 + View list」入口按钮（见上方各 phase 分支），点开在
    // 居中模态弹窗里查看完整列表并直接处理隔离/忽略。
    if state.threats_modal_open {
        threats_modal(ui.ctx(), state, config, toasts);
    }

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
            // Foreground 层：保证强制隔离确认框始终盖在威胁列表弹窗（Middle 层）之上。
            egui::Window::new("Force Quarantine")
                .collapsible(false)
                .resizable(false)
                .order(egui::Order::Foreground)
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

/// 主页面上的威胁入口按钮：红色警示胶囊「⚠ N threats found · View list」，
/// 点击打开威胁列表弹窗（`threats_modal`）。沿用项目里"量尺寸 + 占位 Shape
/// 回填"的自定义胶囊按钮标准实现（同 `widgets::pill_button`、`tab_button`）。
fn threats_entry_button(ui: &mut egui::Ui, count: usize) -> bool {
    const H_PAD: f32 = 16.0;
    const V_PAD: f32 = 9.0;
    const ICON_SIZE: f32 = 17.0;
    const ICON_GAP: f32 = 9.0;
    const HINT_GAP: f32 = 12.0;

    let label = format!("{count} threat{} found", if count == 1 { "" } else { "s" });
    let hint = "View list";
    let label_w = widgets::measure_text_width(ui, &label, 14.0);
    let hint_w = widgets::measure_text_width(ui, hint, 14.0);
    let desired = Vec2::new(
        H_PAD * 2.0 + ICON_SIZE + ICON_GAP + label_w + HINT_GAP + hint_w,
        V_PAD * 2.0 + ICON_SIZE.max(ui.text_style_height(&egui::TextStyle::Body)),
    );

    let bg_idx = ui.painter().add(egui::Shape::Noop);
    let response = ui
        .allocate_ui_with_layout(
            desired,
            egui::Layout::left_to_right(egui::Align::Center),
            |ui| {
                ui.add_space(H_PAD);
                let (icon_resp, painter) =
                    ui.allocate_painter(Vec2::splat(ICON_SIZE), egui::Sense::hover());
                icons::warning_triangle(
                    &painter,
                    icon_resp.rect,
                    Stroke::new(1.7, colors::RED),
                    None,
                );
                ui.add_space(ICON_GAP);
                ui.label(egui::RichText::new(label).color(colors::TEXT_PRIMARY));
                ui.add_space(HINT_GAP);
                ui.label(egui::RichText::new(hint).color(colors::RED));
                ui.add_space(H_PAD);
            },
        )
        .response;

    let bg_rect = response.rect;
    let interact = ui
        .interact(
            bg_rect,
            response.id.with("threats_entry"),
            egui::Sense::click(),
        )
        .on_hover_cursor(egui::CursorIcon::PointingHand);
    let fill = if interact.hovered() {
        Color32::from_rgb(56, 27, 30)
    } else {
        colors::RED_BG
    };
    let shape = egui::epaint::RectShape::new(
        bg_rect,
        egui::CornerRadius::same(255),
        fill,
        Stroke::new(1.0, colors::RED_BORDER),
        egui::epaint::StrokeKind::Inside,
    );
    ui.painter().set(bg_idx, egui::Shape::Rect(shape));

    interact.clicked()
}

/// 威胁列表弹窗：把原先内嵌在扫描页底部的威胁卡片挪进居中模态窗，卡片占满
/// 弹窗宽度、列表内部滚动，空间比页面内嵌列表宽裕得多。隔离/忽略操作直接在
/// 弹窗内完成；列表清空（全部处理完）、点关闭或按 Esc 时自动收起。
///
/// 不做"独立原生窗口"（`show_viewport_deferred` 子视口）的原因同 `about_dialog.rs`：
/// eframe 在根视口隐藏等场景下对子视口的支持不可靠；居中模态弹窗同样能给出
/// "单独查看完整列表"的观感，且实现稳定。
fn threats_modal(
    ctx: &egui::Context,
    state: &mut ScanPageState,
    config: &mut AppConfig,
    toasts: &mut Vec<Toast>,
) {
    if state.threats.is_empty() {
        // 威胁已全部处理（隔离/忽略），自动收起。
        state.threats_modal_open = false;
        return;
    }

    let mut close_requested = false;
    let mut ignore_target: Option<usize> = None;
    let mut quarantine_target: Option<usize> = None;

    // 弹窗尺寸随屏幕自适应：约占 72%，同时限制上下限——小屏不裁切、大屏不空旷。
    // egui 0.36 里 `Context::screen_rect` 已移到 `InputState::viewport_rect`。
    let screen = ctx.input(|i| i.viewport_rect());
    let size = Vec2::new(
        (screen.width() * 0.72).clamp(420.0, 720.0),
        (screen.height() * 0.72).clamp(320.0, 580.0),
    );

    let count = state.threats.len();
    let heading = format!("{count} threat{} found", if count == 1 { "" } else { "s" });

    egui::Window::new("Threats Found")
        .title_bar(false)
        .collapsible(false)
        .resizable(false)
        .fixed_size(size)
        .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
        .frame(
            egui::Frame::default()
                .fill(colors::BG_CARD)
                .stroke(Stroke::new(1.0, colors::RED_BORDER))
                .corner_radius(16.0),
        )
        .show(ctx, |ui| {
            // ── 头部：红调底纹 + 警示图标 + 标题/副标题 + 右侧关闭按钮。
            //    上圆角 15 与外框的 16 对齐（留 1px 给外框描边），下圆角为 0。
            egui::Frame::default()
                .fill(colors::RED_BG)
                .corner_radius(egui::CornerRadius {
                    nw: 15,
                    ne: 15,
                    sw: 0,
                    se: 0,
                })
                .inner_margin(egui::Margin::symmetric(20, 14))
                .show(ui, |ui| {
                    // 显式撑满弹窗内容宽度：红色底纹必须覆盖弹窗完整横向区域，
                    // 不依赖内部行的自然宽度（否则内容窄时红色区域会缩成一条）。
                    ui.set_min_width(ui.available_width());
                    ui.horizontal(|ui| {
                        const ICON_BG: f32 = 36.0;
                        let (icon_resp, painter) =
                            ui.allocate_painter(Vec2::splat(ICON_BG), egui::Sense::hover());
                        painter.rect_filled(icon_resp.rect, 9.0, colors::RED);
                        icons::warning_triangle(
                            &painter,
                            icon_resp.rect.shrink(ICON_BG * 0.25),
                            Stroke::new(2.0, colors::RED_BG),
                            None,
                        );
                        ui.add_space(12.0);
                        ui.vertical(|ui| {
                            widgets::bold_label(ui, &heading, 17.0, colors::TEXT_PRIMARY);
                            ui.label(
                                egui::RichText::new(
                                    "Quarantine suspicious files, or ignore false positives.",
                                )
                                .color(colors::TEXT_SECONDARY)
                                .small(),
                            );
                        });
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            let (resp, painter) =
                                ui.allocate_painter(Vec2::splat(30.0), egui::Sense::click());
                            let resp = resp.on_hover_cursor(egui::CursorIcon::PointingHand);
                            // 自绘按钮不会自动进 accesskit 树（egui 只为标准控件调
                            // widget_info）——补上无障碍信息：屏幕阅读器能读出它，
                            // egui_kittest 也能 get_by_label 查到它（见 AGENTS.md
                            // 「UI 代码验证流程」）。取名 "Close window"（与 egui 内置
                            // 窗口关闭按钮同名）以区别于底部 "Close"，避免查询歧义。
                            resp.widget_info(|| {
                                egui::WidgetInfo::labeled(
                                    egui::WidgetType::Button,
                                    true,
                                    "Close window",
                                )
                            });
                            if resp.hovered() {
                                painter.rect_filled(resp.rect, 7.0, colors::RED_BORDER);
                            }
                            icons::close(
                                &painter,
                                resp.rect.shrink(9.0),
                                Stroke::new(1.5, colors::TEXT_SECONDARY),
                            );
                            if resp.clicked() {
                                close_requested = true;
                            }
                        });
                    });
                });

            // ── 列表 + 底部关闭按钮。ScrollArea 必须预留出底部按钮的高度，
            //    否则会占满剩余空间把按钮顶出弹窗外（按钮实际高度是文字高
            //    + 全局 button_padding，比 32 略高，预留量取 52 留出余量）。
            egui::Frame::default()
                .inner_margin(egui::Margin::symmetric(20, 12))
                .show(ui, |ui| {
                    const FOOTER_RESERVE: f32 = 12.0 + 40.0;
                    egui::ScrollArea::vertical()
                        .auto_shrink([false, false])
                        .max_height((ui.available_height() - FOOTER_RESERVE).max(60.0))
                        .show(ui, |ui| {
                            for (i, threat) in state.threats.iter().enumerate() {
                                ui.vertical(|ui| {
                                    ui.set_width(ui.available_width());
                                    let path_str = threat.path.display().to_string();
                                    let action =
                                        widgets::threat_card(ui, &threat.virus_name, &path_str);
                                    match action {
                                        ThreatAction::Ignore => ignore_target = Some(i),
                                        ThreatAction::Quarantine => quarantine_target = Some(i),
                                        ThreatAction::None => {}
                                    }
                                });
                                ui.add_space(8.0);
                            }
                        });
                    ui.add_space(12.0);
                    ui.vertical_centered(|ui| {
                        // 与全应用其他按钮一致：hover 时显示手型光标
                        // （egui 标准 Button 默认不改光标，需显式声明）。
                        if ui
                            .add(
                                egui::Button::new(
                                    egui::RichText::new("Close").color(colors::TEXT_PRIMARY),
                                )
                                .fill(colors::ACCENT_BLUE_BG)
                                .stroke(Stroke::new(1.0, colors::BORDER))
                                .min_size(Vec2::new(120.0, 32.0)),
                            )
                            .on_hover_cursor(egui::CursorIcon::PointingHand)
                            .clicked()
                        {
                            close_requested = true;
                        }
                    });
                });
        });

    if close_requested || ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
        state.threats_modal_open = false;
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
                    if state.pending_force_quarantine.is_none() && !state.is_force_quarantining() {
                        state.pending_force_quarantine =
                            Some(super::super::core::PendingForceQuarantine {
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
}

#[cfg(test)]
mod tests {
    //! egui_kittest 驱动**真实** `threats_modal` 的 UI 测试（见 AGENTS.md
    //! 「UI 代码验证流程」）。迁移自旧的"复刻 UI 结构"式测试——那种测试在测试里
    //! 手抄一遍弹窗布局，真实弹窗改了它照样绿，没有回归保护。

    use super::threats_modal;
    use crate::app::core::ScanPageState;
    use crate::config::AppConfig;
    use crate::scan::{ScanKind, Threat};
    use crate::widgets::Toast;
    use egui_kittest::Harness;
    use egui_kittest::kittest::Queryable;
    use std::path::PathBuf;

    /// 构造带两条假威胁、弹窗打开的扫描页状态。
    fn threats_state() -> ScanPageState {
        let mut state = ScanPageState::new(ScanKind::Quick);
        state.threats = (0..2)
            .map(|i| Threat {
                path: PathBuf::from(format!("C:\\Users\\x\\Downloads\\EICAR\\E{i}.exe")),
                virus_name: "Eicar-Test-Signature".to_string(),
            })
            .collect();
        state.threats_modal_open = true;
        state
    }

    /// 驱动真实 `threats_modal` 的 harness。状态由 harness 持有（元组），
    /// 测试用 `harness.state()` 断言交互结果。
    ///
    /// 屏幕尺寸沿用旧测试的 1121×747：弹窗尺寸公式据此取到 clamp 上限 720。
    /// `build_ui_state` 会立刻跑一帧（此时还是 egui 默认样式），所以 build 后先
    /// `theme::apply` 再 `run()` 重绘到稳定；max_steps 默认 4 帧对"锚定居中 +
    /// 主题切换"的稳定过程偏紧，放宽到 10。
    fn modal_harness(
        state: ScanPageState,
    ) -> Harness<'static, (ScanPageState, AppConfig, Vec<Toast>)> {
        let mut harness = Harness::builder()
            .with_size(egui::Vec2::new(1121.0, 747.0))
            .with_max_steps(10)
            .build_ui_state(
                |ui, (state, config, toasts)| threats_modal(ui.ctx(), state, config, toasts),
                (state, AppConfig::default(), Vec::new()),
            );
        crate::theme::apply(&harness.ctx);
        harness.run();
        harness
    }

    /// 回归测试：威胁弹窗的两个关闭入口（头部 × 图标按钮 + 底部 Close 按钮）
    /// hover 时必须切换为手型光标，与全应用其他按钮（`action_button`、
    /// `pill_button`、标题栏/侧边栏按钮）的交互惯例一致。
    ///
    /// egui 标准 `Button` 默认不改光标，自定义 `allocate_painter` 按钮的
    /// `on_hover_cursor` 依赖 hit-test 注册，两者都可能悄悄失效——这里驱动真实
    /// 弹窗、模拟 hover，直接断言帧输出里的 `cursor_icon`。
    #[test]
    fn threats_modal_close_buttons_show_pointing_hand_cursor() {
        let mut harness = modal_harness(threats_state());

        harness.get_by_label("Close window").hover();
        harness.run();
        assert_eq!(
            harness.output().platform_output.cursor_icon,
            egui::CursorIcon::PointingHand,
            "头部 × 按钮 hover 应显示手型光标"
        );

        harness.get_by_label("Close").hover();
        harness.run();
        assert_eq!(
            harness.output().platform_output.cursor_icon,
            egui::CursorIcon::PointingHand,
            "底部 Close 按钮 hover 应显示手型光标"
        );
    }

    /// 回归测试：威胁卡片必须等宽对齐、弹窗保持 fixed_size 不被内容撑宽。
    ///
    /// 历史缺陷：`pill_button` 用 13px 量宽却按 Body 字号渲染 label，实际内容
    /// 比申请的 `desired` 宽 ~13px 并在 right_to_left 布局里向右溢出，级联后果
    /// 是每张卡比上一张宽（按钮错位）、ScrollArea 内容变宽、窗口从 720 被撑到
    /// 746+。
    #[test]
    fn threats_modal_cards_equal_width_and_window_not_stretched() {
        let harness = modal_harness(threats_state());

        // 弹窗（egui::Window 以标题 "Threats Found" 进 accesskit 树）不被撑宽：
        // 1121×747 屏幕下 fixed_size 公式的 clamp 上限是 720。
        let window = harness.get_by_label("Threats Found");
        assert!(
            window.rect().width() <= 720.5,
            "modal window stretched to {} (fixed_size = 720)",
            window.rect().width()
        );

        // 每张卡里的 "High Risk" 徽标作为卡的定位锚点：左缘对齐 + 等宽。
        // （pill_button 溢出时卡片逐张变宽，锚点的左缘对齐与等宽性被破坏。）
        let badges: Vec<_> = harness.get_all_by_label("High Risk").collect();
        assert_eq!(badges.len(), 2, "两张威胁卡各有且仅有一个 High Risk 徽标");
        let (a, b) = (badges[0].rect(), badges[1].rect());
        assert!(
            (a.min.x - b.min.x).abs() < 0.5,
            "threat cards misaligned: left edges {} vs {}",
            a.min.x,
            b.min.x
        );
        assert!(
            (a.width() - b.width()).abs() < 0.5,
            "threat cards differ in width: {} vs {}",
            a.width(),
            b.width()
        );
    }

    /// 交互测试：两个关闭入口点击后真的关掉弹窗；Escape 同样生效。
    ///
    /// 注意：不要在 harness 里点击 "Quarantine"/"Ignore"——`quarantine_file`
    /// 动文件系统、`config.add_ignored` 会把测试机的真实 config.toml 落盘
    /// （`AppConfig::save` 没有测试隔离）。涉及文件系统/落盘副作用的路径不
    /// 在无头 UI 测试的覆盖范围。
    #[test]
    fn threats_modal_close_interactions() {
        let mut harness = modal_harness(threats_state());
        harness.get_by_label("Close").click();
        harness.run();
        assert!(
            !harness.state().0.threats_modal_open,
            "点击底部 Close 后弹窗应关闭"
        );

        let mut harness = modal_harness(threats_state());
        harness.get_by_label("Close window").click();
        harness.run();
        assert!(
            !harness.state().0.threats_modal_open,
            "点击头部 × 按钮后弹窗应关闭"
        );

        let mut harness = modal_harness(threats_state());
        harness.key_press(egui::Key::Escape);
        harness.run();
        assert!(!harness.state().0.threats_modal_open, "Escape 应关闭弹窗");
    }
}
