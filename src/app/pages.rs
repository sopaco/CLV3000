//! 四个页面的渲染：仪表盘 / 闪电扫描 / 全盘扫描 / 病毒库，外加两个扫描页共用的
//! `scan_page` 状态机渲染、病毒库页两栏、以及"图标+文字"胶囊按钮 `action_button`
//! （四个页面都在用，独立出来避免每个页面各写一份）。

use super::core::{AppCore, ScanPageState, ScanPhase};
use super::util::{format_duration, truncate};
use super::{App, Page};
use crate::icons;
use crate::localtime::Timestamp;
use crate::paths;
use crate::scan::ScanKind;
use crate::theme::colors;
use crate::widgets::{self, ThreatAction, Toast};
use crate::config::AppConfig;
use eframe::egui;
use egui::{Color32, Stroke, Vec2};
use std::time::Duration;

pub(super) fn dashboard_page(ui: &mut egui::Ui, _ctx: &egui::Context, app: &mut App) {
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

        // 画布比圆环本身大一圈，专门留给外面那层光晕，不然会被裁掉。
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

const ACTION_BTN_ICON_SIZE: f32 = 21.0; // 原本 18，整体图标 +15% 的一部分
const ACTION_BTN_ICON_GAP: f32 = 10.0;
const ACTION_BTN_H_PAD: f32 = 16.0;
const ACTION_BTN_V_PAD: f32 = 10.0;

/// 量出 `action_button` 最终会占多宽——用来在外层把多个按钮组成的一行整体居中。
/// 之前有个地方（dashboard_page 的两个按钮）拍了个固定的"半宽"常数去居中，
/// 字体/字号一变常数就不准，按钮行跟着偏移——量出来才不会有这个问题，跟
/// `widgets::centered_stat_pills` 是同一个思路。
///
/// 这里的 `+ 2.0 * item_spacing`：`action_button` 内部是拿 `ui.add_space` 手动摆
/// 图标/文字间距的，但图标和文字本身还是走 `allocate_painter`/`allocate_exact_size`
/// 正常的部件分配路径，egui 会在每个部件放完之后**额外**把 `item_spacing` 计入
/// 光标（这是 `advance_after_rects` 的行为，跟手动 `add_space` 是否存在无关）。
/// 于是 `action_button` 实际总宽比"内边距+图标+间距+文字"这几个手动常数加起来
/// 还要再宽两份 `item_spacing`（图标后一份、文字后一份）。量的时候漏了这个，
/// 按钮行整体会比算出来的居中位置偏右——这行代码就是补上这个差。
///
/// （这条注释和下面这一行代码之前已经加过一次、后来在别的改动里意外弄丢了——
/// 症状是"闪电扫描/全盘扫描"按钮行肉眼可见地偏右，跟上面圆环/文字对不齐。
/// 如果这个问题再出现，先检查这一行是不是又被误删了。）
fn action_button_width(ui: &egui::Ui, label: &str) -> f32 {
    let text_w = widgets::measure_text_width(ui, label, 14.0);
    ACTION_BTN_H_PAD * 2.0
        + ACTION_BTN_ICON_SIZE
        + ACTION_BTN_ICON_GAP
        + text_w
        + 2.0 * ui.spacing().item_spacing.x
}

/// 一个"图标 + 文字"的胶囊按钮。
///
/// 不能直接用 `Frame::show(...)`——`Frame`/`ui.horizontal` 默认会把自己的
/// "期望尺寸"报成父容器当前的全部可用宽度（因为它们要等内容画完才知道真实大小，
/// 只能先占住最大空间），这样一来外层 `vertical_centered` 之类的居中布局，
/// 拿到的就是一个"和容器一样宽"的东西，居中也就没有意义——表现出来就是按钮
/// 从容器最左边一路铺到很宽的位置，而不是一颗居中的小胶囊。
///
/// 解决办法是自己先量出图标+文字真正需要的尺寸，把这个"小尺寸"传给
/// `allocate_ui_with_layout`，父容器的 `Align::Center` 才有东西可对齐。
///
/// 大部分调用点只关心"点没点击"，用 `action_button`（返回 `bool`）就够；病毒库
/// 页的"查看完整路径"按钮还需要挂 `.on_hover_text(...)` 做 tooltip，那种场景要
/// 拿到完整的 `Response`，用 `action_button_response`。
fn action_button(
    ui: &mut egui::Ui,
    label: &str,
    draw: impl FnOnce(&egui::Painter, egui::Rect, Stroke),
) -> bool {
    action_button_response(ui, label, draw).clicked()
}

fn action_button_response(
    ui: &mut egui::Ui,
    label: &str,
    draw: impl FnOnce(&egui::Painter, egui::Rect, Stroke),
) -> egui::Response {
    const ICON_SIZE: f32 = ACTION_BTN_ICON_SIZE;
    const ICON_GAP: f32 = ACTION_BTN_ICON_GAP;
    const H_PAD: f32 = ACTION_BTN_H_PAD;
    const V_PAD: f32 = ACTION_BTN_V_PAD;

    let text_size = Vec2::new(
        widgets::measure_text_width(ui, label, 14.0),
        ui.text_style_height(&egui::TextStyle::Body),
    );

    let desired = Vec2::new(
        H_PAD * 2.0 + ICON_SIZE + ICON_GAP + text_size.x,
        V_PAD * 2.0 + ICON_SIZE.max(text_size.y),
    );

    let bg_shape_idx = ui.painter().add(egui::Shape::Noop);
    let response = ui
        .allocate_ui_with_layout(
            desired,
            egui::Layout::left_to_right(egui::Align::Center),
            |ui| {
                ui.add_space(H_PAD);
                let (icon_resp, painter) =
                    ui.allocate_painter(Vec2::splat(ICON_SIZE), egui::Sense::hover());
                draw(
                    &painter,
                    icon_resp.rect,
                    Stroke::new(1.6, colors::ACCENT_BLUE),
                );
                ui.add_space(ICON_GAP);
                ui.label(egui::RichText::new(label).color(colors::TEXT_PRIMARY));
                ui.add_space(H_PAD);
            },
        )
        .response;

    let bg_rect = response.rect;
    let interact = ui
        .interact(bg_rect, response.id.with("btn"), egui::Sense::click())
        .on_hover_cursor(egui::CursorIcon::PointingHand);
    let fill = if interact.hovered() {
        colors::ACCENT_BLUE_BG
    } else {
        colors::BG_CARD
    };
    let shape = egui::epaint::RectShape::new(
        bg_rect,
        egui::CornerRadius::same(12),
        fill,
        Stroke::new(1.0, colors::BORDER),
        egui::epaint::StrokeKind::Inside,
    );
    ui.painter().set(bg_shape_idx, egui::Shape::Rect(shape));

    interact
}

pub(super) fn quick_scan_page(ui: &mut egui::Ui, app: &mut App) {
    // `is_running()` 只读 `&self`，跟紧接着对 `app.core` 的可变解构不冲突。
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

pub(super) fn full_scan_page(ui: &mut egui::Ui, app: &mut App) {
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

// 参数是有点多，但拆成一个配置 struct 目前收益不大（调用点就 2 个，字段名本身
// 已经很直白），先用 allow 压掉这条 lint。`other_running`：另一类扫描（闪电/
// 全盘）是否正在跑——两者共享临时扫描列表文件与文件基因缓存，绝不能并发，见
// `AppCore::any_scan_running` 的注释。
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
    // 跟 dashboard_page 一样：内容少（Idle/Done 态、没有威胁列表）时用上一帧测量到
    // 的高度把这一帧的空白平分到上下，不让内容整体贴着顶部。内容比可用高度还高时
    // （威胁列表很长）`vertically_centered` 内部的 `.max(0.0)` 会让顶部空白归零，
    // 自然向下溢出，不会比原来的写法更差。
    let mut content_height = state.content_height;
    widgets::vertically_centered(ui, &mut content_height, |ui| {
        match &state.phase {
            ScanPhase::Idle => {
                // 仪表盘页那种"大圆环 + 图标"的视觉语言在这里也来一份，跟概览页
                // 呼应，不然闪电扫描/全盘扫描的待机画面只有两行字，太空。这里没有
                // 状态色（还没扫描，谈不上安全/危险），就用页面自己的强调色。
                const IDLE_RING_GLYPH: f32 = 52.0;
                // 全盘扫描的电脑图标在小圆环里偏小，单独放大 25%。
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
                let deco_glyph =
                    egui::Rect::from_center_size(deco_center, Vec2::splat(glyph_size));
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
                // 全盘扫描 walk 阶段：total 尚为 None、尚无 FileScanned。
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
                // 旋转环靠 time 推进、已用时长每秒变化——两者都需要持续重绘，否则一
                // 停下来界面就静止了。限制在 ~30fps：动画仍然流畅，但不会在老机器上
                // 按 vsync 满帧率白烧 CPU/GPU。
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
        for (i, threat) in state.threats.iter().enumerate() {
            ui.horizontal(|ui| {
                ui.add_space((ui.available_width() - 700.0).max(0.0) / 2.0);
                ui.vertical(|ui| {
                    ui.set_width(700.0_f32.min(ui.available_width()));
                    let path_str = threat.path.display().to_string();
                    let action = widgets::threat_card(ui, &threat.virus_name, &path_str);
                    match action {
                        ThreatAction::Ignore => ignore_target = Some(i),
                        ThreatAction::Quarantine => {
                            toasts.push(Toast::new(
                                "Quarantine will be available in a future release",
                            ));
                        }
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
    });
    state.content_height = content_height;
}

pub(super) fn virus_db_page(ui: &mut egui::Ui, app: &mut App) {
    ui.add_space(28.0);
    ui.columns(2, |columns| {
        virus_db_status_column(&mut columns[0], app);
        virus_db_about_column(&mut columns[1], app);
    });
}

/// 左栏：病毒库状态 + 手动更新交互。
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

        // 引擎可用性 + 病毒库目录只在首次探测（或更新成功后失效重探）时真的碰
        // 文件系统，其余帧直接读缓存——见 `VirusDbState::engine_probe`。
        let (available, detail_dir) = {
            let probe = core.virus_db.engine_probe();
            (probe.available, probe.detail_dir.clone())
        };
        // 第一次画这一栏时顺带查一次版本（只查这一次，之后靠"更新完成"事件刷新），
        // 避免每帧都拉起 clamscan 进程。更新成功后 `db_version` 会从旧值刷新成新值。
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
        // "打开所在文件夹"和"手动更新病毒库"放同一行——都是这一栏里的辅助操作，
        // 分两行意义不大，合一行更紧凑。宽度量出来再居中，见 `action_button_width`
        // 的注释。
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
                    // 文件夹不一定存在（比如引擎没找到、数据库还没更新过一次）——
                    // 先确保目录存在再打开，不然系统文件管理器会直接报错。
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

/// 右栏：关于（真实品牌图标 + 名称 + 版本 + 简介）。
fn virus_db_about_column(ui: &mut egui::Ui, app: &mut App) {
    let mut content_height = app.core.virus_db.about_col_height;
    widgets::vertically_centered(ui, &mut content_height, |ui| {
        const LOGO_DISPLAY_PT: f32 = 90.0;
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
