//! 闪电扫描 / 全盘扫描页面与共用的 `scan_page` 渲染。

use super::super::App;
use super::super::core::{AppCore, ScanPageState, ScanPhase};
use super::super::util::format_duration;
use crate::config::AppConfig;
use crate::icons;
use crate::localtime::Timestamp;
use crate::scan::ScanKind;
use crate::theme::colors;
use crate::widgets::{self, ThreatAction, Toast};
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
        // 副标题讲"能解决什么问题"，不列扫描项——具体扫哪三样由下面的覆盖
        // 清单展开，两处说同一件事就是冗余。原文案里的 "startup items, and
        // memory" 与实现不符（闪电扫描枚举的是运行中进程及其加载的模块，
        // 见 scan/quick_scan.rs），一并改准。
        "Finds threats hiding in what you are running right now",
        |p, r, s| icons::bolt(p, r, s.color),
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
        // 同上：副标题说收益，覆盖清单说范围，两边不重复。
        "Finds threats hiding anywhere on your local disks",
        icons::computer,
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
    subtitle: &str,
    icon: fn(&egui::Painter, egui::Rect, Stroke),
    other_running: bool,
) {
    // 页头：标题/副标题常驻，扫描中右侧挂 Cancel——取消是页级动作，
    // 放页头比放在居中内容底部更有"随时可退"的把握感（原来是个 ui.link）。
    ui.add_space(20.0);
    widgets::page_header(ui, icon, title, subtitle, |ui| {
        if state.is_running() && widgets::ghost_button(ui, "Cancel Scan", true) {
            state.request_cancel();
        }
    });
    ui.add_space(4.0);

    // 页面主体套 ScrollArea 兜底：原来用 widgets::vertically_centered 纯居中，
    // 内容一旦超过可视高度（idle 加了简报卡后、或窗口变矮）就直接裁掉底部。
    // ScrollArea 内 available_height 是无穷大，不能直接喂 vertically_centered，
    // 改用「外层可视高度 − 上帧内容高」手算 top_pad：短内容时同样居中，
    // 内容变高时 top_pad 归零、页面自然滚动（和仪表盘同为可滚页面）。
    let avail_h = ui.available_height();
    let top_pad = ((avail_h - state.content_height) * 0.5).max(0.0);
    let mut measured = 0.0;
    egui::ScrollArea::vertical().show(ui, |ui| {
        ui.add_space(top_pad);
        ui.vertical_centered(|ui| {
            match &state.phase {
                ScanPhase::Idle => {
                    // 空闲态版式（2026-10 重排）：原来是"120px 大圆环 + 一行标题
                    // + 一个按钮"，内容全挤在中间、四周大片空白，像 landing page
                    // 而不是功能页；标题还跟页头重复了一遍。
                    // 现在：64px 图标锚点 → 覆盖清单卡（视觉重心）→ 上次扫描/
                    // 耗时一行 → 主操作按钮。页头已经有标题与副标题，这里不再重复。
                    const ICON_SIZE: f32 = 64.0;
                    let (icon_resp, painter) =
                        ui.allocate_painter(Vec2::splat(ICON_SIZE), egui::Sense::hover());
                    accent_icon(&painter, state.kind, icon_resp.rect);
                    ui.add_space(18.0);

                    // 覆盖清单卡：限宽 560 居中（与病毒库页英雄卡同一套 scaffold：
                    // 外层 cell 定宽 + 卡片吃满 cell）。
                    const CARD_W: f32 = 560.0;
                    let avail = ui.available_width();
                    let card_w = CARD_W.min(avail);
                    let left = ((avail - card_w) / 2.0).max(0.0);
                    ui.horizontal_top(|ui| {
                        ui.add_space(left);
                        ui.allocate_ui_with_layout(
                            Vec2::new(card_w, 10.0),
                            egui::Layout::top_down(egui::Align::Min),
                            |ui| {
                                ui.set_min_width(card_w);
                                ui.set_max_width(card_w);
                                widgets::coverage_card(ui, &coverage_rows(state.kind));
                            },
                        );
                    });
                    ui.add_space(14.0);

                    // 上次扫描结果 / 首次扫描的耗时预期。一行说完，不重复"没扫过"。
                    let today = Timestamp::now();
                    let (last_label, record) = match state.kind {
                        ScanKind::Quick => ("Last quick scan", config.last_quick_scan.as_ref()),
                        ScanKind::Full => ("Last full scan", config.last_full_scan.as_ref()),
                    };
                    let summary = match record {
                        Some(r) => {
                            let threats = if r.threats_found == 0 {
                                "no threats found".to_string()
                            } else {
                                format!(
                                    "{} threat{} found",
                                    r.threats_found,
                                    if r.threats_found == 1 { "" } else { "s" }
                                )
                            };
                            format!(
                                "{} · {} · {} files · {}",
                                last_label,
                                r.time.display_relative_to(&today),
                                crate::config::format_count(r.scanned_count),
                                threats
                            )
                        }
                        // 没扫过的时候给"大概要多久"这个有用信息，而不是"没扫过"。
                        None => first_run_hint(state.kind).to_string(),
                    };
                    ui.label(
                        egui::RichText::new(summary)
                            .color(colors::TEXT_SECONDARY)
                            .small(),
                    );
                    ui.add_space(16.0);

                    // Start 是这一页唯一的推进动作，用主操作实底按钮（原来跟其他
                    // 按钮一样是卡片底描边，没有主次）。
                    let label = format!("Start {title}");
                    // 量**本体**宽度：`primary_button_width` 多含两道 item_spacing，
                    // 用它居中会让按钮偏左 10px（实测按钮中心比内容区中心左 10）。
                    let btn_w = widgets::primary_button_body_width(ui, &label);
                    let left = (ui.available_width() - btn_w) / 2.0;
                    ui.horizontal(|ui| {
                        ui.add_space(left.max(0.0));
                        if widgets::primary_button(ui, &label, icon) {
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
                    // 与全盘扫描的磁盘遍历阶段同构：进度环 + 数据胶囊（标题在页头）。
                    widgets::progress_ring(
                        ui,
                        220.0,
                        None,
                        colors::ACCENT_BLUE,
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

                    widgets::progress_ring(
                        ui,
                        220.0,
                        percent,
                        colors::ACCENT_BLUE,
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
                    // 状态行按内容区全宽做像素截断：路径里没有空格，label 的 wrap
                    // 帮不上忙；按字符数拍脑袋又会宽屏浪费/窄屏溢出。
                    let line_w = ui.available_width() - 8.0;
                    let status_line = if let Some(p) = &state.engine_scanning_path {
                        widgets::truncate_middle_to_px(ui, p, 12.5, line_w)
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
                        // 路径 + 后缀一起预算：后缀先量宽，剩下的额度才留给路径。
                        let suffix = " — loading engine…";
                        let path_w = line_w - widgets::measure_text_width(ui, suffix, 12.5);
                        format!(
                            "{}{suffix}",
                            widgets::truncate_middle_to_px(ui, current_path, 12.5, path_w)
                        )
                    } else if state.engine_loading {
                        "Loading scan engine…".to_string()
                    } else if current_path.is_empty() {
                        "Scanning…".to_string()
                    } else {
                        widgets::truncate_middle_to_px(ui, current_path, 12.5, line_w)
                    };
                    ui.label(
                        egui::RichText::new(status_line)
                            .color(colors::TEXT_SECONDARY)
                            .small(),
                    );
                    ui.add_space(12.0);
                    // 已扫文件 / 耗时 / 威胁数收进同一行胶囊（原来 Elapsed 是
                    // 单独一行小字，和 Done 阶段的三胶囊节奏不一致）。
                    let first_pill = if disk_walking {
                        (state.walk_files_found.to_string(), "to scan")
                    } else {
                        match total {
                            Some(t) => (format!("{scanned} / {t}"), "files"),
                            None => (scanned.to_string(), "scanned"),
                        }
                    };
                    let mut pills = vec![first_pill];
                    if let Some(started) = state.started_at.as_ref() {
                        pills.push((format_duration(started.elapsed()), "elapsed"));
                    }
                    pills.push((state.threats.len().to_string(), "threats"));
                    widgets::centered_stat_pills(ui, &pills);
                    if !state.threats.is_empty() {
                        ui.add_space(12.0);
                        if threats_entry_button(ui, state.threats.len()) {
                            state.threats_modal_open = true;
                        }
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
                    // 结果环比 hero/idle 环大一档（140 vs 120）：完成时刻是全页的视觉
                    // 主体；但发光呼吸位收窄到 26——600px 高的窗口里下面还有标题/胶囊/
                    // 入口按钮/再扫按钮四层要排，实测 50 会把最后一级按钮顶出可视区。
                    const DIAMETER: f32 = 140.0;
                    const GLOW_MARGIN: f32 = 26.0;
                    let (response, painter) = ui.allocate_painter(
                        Vec2::splat(DIAMETER + GLOW_MARGIN * 2.0),
                        egui::Sense::hover(),
                    );
                    let center = response.rect.center();
                    let radius = DIAMETER / 2.0 - 4.0;
                    widgets::paint_glow(&painter, center, radius, color);
                    painter.circle_filled(center, radius, colors::BG_CARD);
                    painter.circle_stroke(center, radius, Stroke::new(3.0, color));
                    let glyph_rect =
                        egui::Rect::from_center_size(center, Vec2::splat(DIAMETER * 0.46));
                    if has_threats {
                        icons::status_glyph_at_risk(&painter, glyph_rect, color);
                    } else {
                        icons::status_glyph_secure(&painter, glyph_rect, color);
                    }
                    ui.add_space(10.0);
                    let heading = if *cancelled {
                        "Scan cancelled".to_string()
                    } else if has_threats {
                        format!(
                            "{} threat{} found",
                            state.threats.len(),
                            if state.threats.len() == 1 { "" } else { "s" }
                        )
                    } else {
                        "No threats found".to_string()
                    };
                    widgets::bold_label(ui, &heading, 20.0, colors::TEXT_PRIMARY);
                    ui.add_space(10.0);
                    // 结果摘要用胶囊行（原来是一行小字，扫了多少文件/花了多久读不出
                    // 层级）。
                    widgets::centered_stat_pills(
                        ui,
                        &[
                            (scanned.to_string(), "files scanned"),
                            (format_duration(*elapsed), "duration"),
                            (state.threats.len().to_string(), "threats"),
                        ],
                    );
                    if has_threats {
                        ui.add_space(10.0);
                        if threats_entry_button(ui, state.threats.len()) {
                            state.threats_modal_open = true;
                        }
                        ui.add_space(10.0);
                    } else {
                        ui.add_space(12.0);
                    }
                    if widgets::primary_button(ui, &format!("Run {title} Again"), icon) {
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
            // 净内容高（不含 top_pad / 底部呼吸位），下一帧的居中偏移用它。
            measured = ui.min_rect().height();
        });
        // 底部呼吸位：内容滚到底时不贴边；不计入 content_height。
        ui.add_space(24.0);
    });
    state.content_height = measured;

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
            // 固定最小宽度 + 窗内可用宽像素截断：窗口不再被 56 字符的
            // 长路径撑宽（原来窄屏溢出、不同威胁弹出的窗口宽度参差）。
            egui::Window::new("Force Quarantine")
                .collapsible(false)
                .resizable(false)
                .min_width(440.0)
                .order(egui::Order::Foreground)
                .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
                .show(ui.ctx(), |ui| {
                    ui.add_space(4.0);
                    ui.label("The file is currently in use.");
                    ui.add_space(2.0);
                    let path_line = widgets::truncate_middle_to_px(
                        ui,
                        &path_display,
                        12.5,
                        ui.available_width(),
                    );
                    ui.label(
                        egui::RichText::new(path_line)
                            .color(colors::TEXT_SECONDARY)
                            .small(),
                    );
                    ui.add_space(8.0);
                    ui.label("Do you want to force quarantine?");
                    ui.add_space(10.0);
                    // 按钮行居中：原来用 add_space(80) 硬偏移，窗口 440 宽时
                    // 按钮偏左、不在对话框中轴上（用户反馈“未居中对齐”）。
                    ui.vertical_centered(|ui| {
                        ui.horizontal(|ui| {
                            if ui.button("Force Quarantine").clicked() {
                                confirmed = true;
                            }
                            ui.add_space(8.0);
                            if ui.button("Cancel").clicked() {
                                cancelled = true;
                            }
                        });
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

/// 扫描覆盖清单：三行"图标 + 说明"，回答"这次扫描到底扫什么"。
///
/// 文案必须跟引擎实际做的事一致——闪电扫描枚举的是**运行中进程及其加载的
/// 模块**（见 `scan/quick_scan.rs`），并不扫描启动项、也不做内存扫描，所以
/// 这里不能写"startup items""memory"这类没实现的承诺。
fn coverage_rows(kind: ScanKind) -> Vec<widgets::CoverageRow> {
    match kind {
        ScanKind::Quick => vec![
            ("Every running process on this PC", icons::process_window),
            ("The libraries and drivers each one loaded", icons::library_chip),
            (
                "Each executable file, checked against the virus database",
                |p, r, s| icons::shield(p, r, s, None),
            ),
        ],
        ScanKind::Full => vec![
            (
                "Every executable file on all local fixed disks",
                icons::computer,
            ),
            ("Programs, libraries and drivers in every folder", icons::folder),
            (
                "Removable drives too, when you enable them in Settings",
                icons::disk,
            ),
        ],
    }
}

/// 还没扫过时显示的一行：给"大概要多久"的预期，而不是"你没扫过"的事实陈述。
fn first_run_hint(kind: ScanKind) -> &'static str {
    match kind {
        ScanKind::Quick => "Your first quick scan usually takes under a minute",
        ScanKind::Full => "Your first full scan takes about 5–15 minutes",
    }
}

/// 空闲态页面中央的图标锚点（64px 描边图标，不画大圆环）。
fn accent_icon(painter: &egui::Painter, kind: ScanKind, rect: egui::Rect) {
    let stroke = Stroke::new(2.2, colors::ACCENT_BLUE);
    match kind {
        ScanKind::Quick => icons::bolt_outlined(painter, rect, stroke),
        ScanKind::Full => icons::computer(painter, rect, stroke),
    }
}

/// 主页面上的威胁入口按钮：红色警示「⚠ N threats found · View list」，
/// 点击打开威胁列表弹窗（`threats_modal`）。沿用项目里"量尺寸 + 占位 Shape
/// 回填"的自定义按钮标准实现（同 `widgets::pill_button`、`tab_button`）。
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
        // 跟其它按钮同一档圆角（MD）：同页上方的 "Cancel Scan"、"Run Quick
        // Scan Again" 都是 4px 圆角矩形，这个入口要是还保持药丸形，一页里
        // 就出现两套按钮形态。
        crate::theme::radius::MD,
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
                .corner_radius(crate::theme::radius::LG),
        )
        .show(ctx, |ui| {
            // ── 头部：红调底纹 + 警示图标 + 标题/副标题 + 右侧关闭按钮。
            //    上圆角比外框 LG 小 1px（留给外框描边），下圆角为 0。
            egui::Frame::default()
                .fill(colors::RED_BG)
                .corner_radius(crate::theme::radius::HEADER)
                .inner_margin(egui::Margin::symmetric(20, 14))
                .show(ui, |ui| {
                    // 显式撑满弹窗内容宽度：红色底纹必须覆盖弹窗完整横向区域，
                    // 不依赖内部行的自然宽度（否则内容窄时红色区域会缩成一条）。
                    ui.set_min_width(ui.available_width());
                    ui.horizontal(|ui| {
                        const ICON_BG: f32 = 36.0;
                        let (icon_resp, painter) =
                            ui.allocate_painter(Vec2::splat(ICON_BG), egui::Sense::hover());
                        painter.rect_filled(icon_resp.rect, crate::theme::radius::SM, colors::RED);
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
                                painter.rect_filled(
                                    resp.rect,
                                    crate::theme::radius::SM,
                                    colors::RED_BORDER,
                                );
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
                        // 用应用自己的 ghost_button，而不是 egui 标准 Button：
                        // 标准 Button 的圆角是 egui 默认的 2~3px，跟全应用
                        // 12px 圆角的按钮放在一起像个"外来件"（2026-10 视觉走查）。
                        // ghost_button = 透明底 + BORDER 描边 + 次要文字色、
                        // hover 转红提示后果，跟扫描页 "Cancel Scan" 同一套语言。
                        if widgets::ghost_button(ui, "Close", true) {
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

    /// 驱动真实 quick_scan_page（Idle 阶段）的 harness：App 由 harness 持有，
    /// config 覆写成默认值保证确定性（同 dashboard 测试的隔离手法；不点
    /// Start，不触发任何真实扫描/落盘副作用）。
    fn quick_idle_harness() -> Harness<'static, Option<crate::app::App>> {
        let mut harness = Harness::builder()
            .with_size(egui::Vec2::new(900.0, 600.0))
            .with_max_steps(10)
            .build_ui_state(
                |ui, app| {
                    let app = app.get_or_insert_with(|| {
                        let mut app = crate::app::App::new(
                            ui.ctx(),
                            None,
                            crate::lifecycle::InitialMode::ShowWindow,
                        );
                        app.core.config = crate::config::AppConfig::default();
                        app
                    });
                    super::super::quick_scan_page(ui, app);
                },
                None,
            );
        crate::theme::apply(&harness.ctx);
        harness.run();
        harness
    }

    /// 空闲页（2026-10 重排后）：覆盖清单卡 + 上次扫描行 + 明确的耗时预期。
    /// 三条覆盖项必须都在 accesskit 树里可查（这是页面信息的主体）。
    #[test]
    fn quick_scan_idle_shows_coverage_list_and_time_expectation() {
        // 无记录：显示"第一次扫大概多久"，不显示"Not run yet"。
        let mut harness = quick_idle_harness();
        harness.get_by_label("Every running process on this PC");
        harness.get_by_label("The libraries and drivers each one loaded");
        harness.get_by_label("Each executable file, checked against the virus database");
        assert!(
            harness.query_by_label("Not run yet").is_none(),
            "空状态不应再显示 \"Not run yet\""
        );
        harness.get_by_label("Your first quick scan usually takes under a minute");

        // 有记录（威胁 0）：摘要行走"无威胁"；覆盖清单仍在且限宽居中。
        {
            let app = harness.state_mut().as_mut().unwrap();
            app.core.config.last_quick_scan = Some(crate::config::ScanRecord {
                time: crate::localtime::Timestamp::now(),
                threats_found: 0,
                scanned_count: 342,
            });
        }
        harness.run();
        assert!(
            harness.query_by_label("Your first quick scan usually takes under a minute").is_none(),
            "有扫描记录后不应再显示首次扫描提示"
        );
        let bullet = harness.get_by_label("Every running process on this PC");
        assert!(
            bullet.rect().min.x > 100.0,
            "覆盖清单应限宽居中，实际左缘 {}",
            bullet.rect().min.x
        );
    }
}
