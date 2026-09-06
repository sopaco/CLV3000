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
        .interact(bg_rect, response.id.with("threats_entry"), egui::Sense::click())
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
                        ui.with_layout(
                            egui::Layout::right_to_left(egui::Align::Center),
                            |ui| {
                                let (resp, painter) =
                                    ui.allocate_painter(Vec2::splat(30.0), egui::Sense::click());
                                let resp = resp.on_hover_cursor(egui::CursorIcon::PointingHand);
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
                            },
                        );
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
                    if state.pending_force_quarantine.is_none()
                        && !state.is_force_quarantining()
                    {
                        state.pending_force_quarantine = Some(
                            super::super::core::PendingForceQuarantine {
                                path: t.path.clone(),
                                virus_name: t.virus_name.clone(),
                            },
                        );
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
    use crate::theme::colors;
    use egui::{Align2, Margin, Sense, Stroke, Vec2};

    /// 回归测试：威胁弹窗的两个关闭入口（头部 × 图标按钮 + 底部 Close 按钮）
    /// hover 时必须切换为手型光标，与全应用其他按钮（`action_button`、
    /// `pill_button`、标题栏/侧边栏按钮）的交互惯例一致。
    ///
    /// egui 标准 `Button` 默认不改光标，自定义 `allocate_painter` 按钮的
    /// `on_hover_cursor` 依赖 hit-test 注册（`interaction.rs` 的 hovered
    /// 集合），两者都可能悄悄失效，这里用模拟 `PointerMoved` 事件直接断言
    /// 帧输出的 `cursor_icon` 兜底。
    #[test]
    fn modal_close_buttons_show_pointing_hand_cursor() {
        let ctx = egui::Context::default();
        crate::theme::apply(&ctx);

        let size = Vec2::new(720.0, 537.0);
        let mut close_x_rect = egui::Rect::NOTHING;
        let mut close_btn_rect = egui::Rect::NOTHING;

        // anchor 居中的 Window 首帧位置未稳定（frame 0 → 1 整窗漂移），鼠标
        // 事件从布局稳定后的 frame 2 才开始发，rect 取上一稳定帧的值。
        for frame in 0..4 {
            let mut raw = egui::RawInput::default();
            raw.screen_rect = Some(egui::Rect::from_min_size(
                egui::pos2(0.0, 0.0),
                egui::vec2(1121.0, 747.0),
            ));
            raw.time = Some(frame as f64 * 0.1);
            if frame == 2 {
                raw.events.push(egui::Event::PointerMoved(close_x_rect.center()));
            } else if frame == 3 {
                raw.events
                    .push(egui::Event::PointerMoved(close_btn_rect.center()));
            }

            let output = ctx.run_ui(raw, |ui| {
                let ctx = ui.ctx().clone();
                egui::Window::new("Threats Found")
                    .title_bar(false)
                    .collapsible(false)
                    .resizable(false)
                    .fixed_size(size)
                    .anchor(Align2::CENTER_CENTER, [0.0, 0.0])
                    .frame(
                        egui::Frame::default()
                            .fill(colors::BG_CARD)
                            .stroke(Stroke::new(1.0, colors::RED_BORDER))
                            .corner_radius(16.0),
                    )
                    .show(&ctx, |ui| {
                        // 头部（复刻）：× 按钮与真实 threats_modal 相同的实现。
                        egui::Frame::default()
                            .inner_margin(Margin::symmetric(20, 14))
                            .show(ui, |ui| {
                                ui.set_min_width(ui.available_width());
                                ui.horizontal(|ui| {
                                    let (_r, _p) =
                                        ui.allocate_painter(Vec2::splat(36.0), Sense::hover());
                                    ui.with_layout(
                                        egui::Layout::right_to_left(egui::Align::Center),
                                        |ui| {
                                            let (resp, _p) =
                                                ui.allocate_painter(Vec2::splat(30.0), Sense::click());
                                            let resp =
                                                resp.on_hover_cursor(egui::CursorIcon::PointingHand);
                                            close_x_rect = resp.rect;
                                        },
                                    );
                                });
                            });
                        // 底部（复刻）：Close 按钮，与修复后的真实实现一致。
                        ui.add_space(40.0);
                        ui.vertical_centered(|ui| {
                            let resp = ui
                                .add(
                                    egui::Button::new(
                                        egui::RichText::new("Close").color(colors::TEXT_PRIMARY),
                                    )
                                    .fill(colors::ACCENT_BLUE_BG)
                                    .stroke(Stroke::new(1.0, colors::BORDER))
                                    .min_size(Vec2::new(120.0, 32.0)),
                                )
                                .on_hover_cursor(egui::CursorIcon::PointingHand);
                            close_btn_rect = resp.rect;
                        });
                    });
            });

            if frame >= 2 {
                assert_eq!(
                    output.platform_output.cursor_icon,
                    egui::CursorIcon::PointingHand,
                    "frame {frame}: close button hover should show pointing-hand cursor"
                );
            }
            output.drop_without_applying_deltas();
        }
    }

    /// 回归测试：威胁列表弹窗（`threats_modal` 的结构复刻 + 真实
    /// `widgets::threat_card`）必须满足——
    /// 1. 所有威胁卡片宽度一致（按钮因此右对齐到同一条竖线）；
    /// 2. 弹窗保持 `fixed_size`，不被内容撑宽；
    /// 3. 红色头部覆盖弹窗内容区完整宽度。
    ///
    /// 历史缺陷：`pill_button` 用 13px 量宽却按 Body 字号渲染 label，实际
    /// 内容比申请的 `desired` 宽 ~13px 并在 right_to_left 布局里向右溢出，
    /// 级联后果是——每张卡比上一张宽 13px（按钮错位）、ScrollArea 内容
    /// 变宽、窗口从 720 被撑到 746+、头部红色区域盖不满弹窗右缘。
    #[test]
    fn modal_cards_equal_width_and_header_full_width() {
        let ctx = egui::Context::default();
        crate::theme::apply(&ctx);
        // 首帧 anchor 位置未稳定，断言从第 2 帧开始；多跑几帧确认布局不再漂移。
        for frame in 0..4 {
            let mut raw = egui::RawInput::default();
            raw.screen_rect = Some(egui::Rect::from_min_size(
                egui::pos2(0.0, 0.0),
                egui::vec2(1121.0, 747.0),
            ));
            raw.time = Some(frame as f64 * 0.1);

            let mut card_widths: Vec<f32> = Vec::new();
            let mut header_width = 0.0;
            let mut window_width = 0.0;
            // 与 threats_modal 相同的自适应尺寸公式（raw.screen_rect = 1121x747）。
            let size = Vec2::new(
                (1121.0_f32 * 0.72).clamp(420.0, 720.0),
                (747.0_f32 * 0.72).clamp(320.0, 580.0),
            );

            let output = ctx.run_ui(raw, |ui| {
                let ctx = ui.ctx().clone();
                let win = egui::Window::new("Threats Found")
                    .title_bar(false)
                    .collapsible(false)
                    .resizable(false)
                    .fixed_size(size)
                    .anchor(Align2::CENTER_CENTER, [0.0, 0.0])
                    .frame(
                        egui::Frame::default()
                            .fill(colors::BG_CARD)
                            .stroke(Stroke::new(1.0, colors::RED_BORDER))
                            .corner_radius(16.0),
                    )
                    .show(&ctx, |ui| {
                        // ── 头部（复刻，含 set_min_width 全宽保障）
                        let header = egui::Frame::default()
                            .fill(colors::RED_BG)
                            .corner_radius(egui::CornerRadius {
                                nw: 15,
                                ne: 15,
                                sw: 0,
                                se: 0,
                            })
                            .inner_margin(Margin::symmetric(20, 14))
                            .show(ui, |ui| {
                                ui.set_min_width(ui.available_width());
                                ui.horizontal(|ui| {
                                    let (_r, _p) =
                                        ui.allocate_painter(Vec2::splat(36.0), Sense::hover());
                                    ui.add_space(12.0);
                                    ui.vertical(|ui| {
                                        crate::widgets::bold_label(
                                            ui,
                                            "2 threats found",
                                            17.0,
                                            colors::TEXT_PRIMARY,
                                        );
                                        ui.label("subtitle");
                                    });
                                    ui.with_layout(
                                        egui::Layout::right_to_left(egui::Align::Center),
                                        |ui| {
                                            let (_r, _p) =
                                                ui.allocate_painter(Vec2::splat(30.0), Sense::click());
                                        },
                                    );
                                });
                            });
                        header_width = header.response.rect.width();

                        // ── 列表 + 底部按钮（复刻），卡片用真实 threat_card
                        egui::Frame::default()
                            .inner_margin(Margin::symmetric(20, 12))
                            .show(ui, |ui| {
                                const FOOTER_RESERVE: f32 = 12.0 + 40.0;
                                egui::ScrollArea::vertical()
                                    .auto_shrink([false, false])
                                    .max_height(
                                        (ui.available_height() - FOOTER_RESERVE).max(60.0),
                                    )
                                    .show(ui, |ui| {
                                        for i in 0..2 {
                                            let rect = ui
                                                .vertical(|ui| {
                                                    ui.set_width(ui.available_width());
                                                    crate::widgets::threat_card(
                                                        ui,
                                                        "Eicar-Test-Signature",
                                                        &format!(
                                                            "C:\\Users\\x\\Downloads\\EICAR\\E{i}.exe"
                                                        ),
                                                    );
                                                })
                                                .response
                                                .rect;
                                            card_widths.push(rect.width());
                                            ui.add_space(8.0);
                                        }
                                    });
                                ui.add_space(12.0);
                                ui.vertical_centered(|ui| {
                                    let _ = ui.button("Close");
                                });
                            });
                    });
                if let Some(win) = win {
                    window_width = win.response.rect.width();
                }
            });
            output.drop_without_applying_deltas();

            if frame >= 1 {
                let (a, b) = (card_widths[0], card_widths[1]);
                assert!(
                    (a - b).abs() < 0.5,
                    "threat cards differ in width: {a} vs {b}"
                );
                assert!(
                    (window_width - size.x).abs() < 0.5,
                    "modal window grew to {window_width} (fixed_size = {})",
                    size.x
                );
                // 头部应盖满内容区：窗口宽 - 左右各 1px 描边。
                assert!(
                    (header_width - (size.x - 2.0)).abs() < 0.5,
                    "header width {header_width} doesn't cover full modal width"
                );
            }
        }
    }
}

