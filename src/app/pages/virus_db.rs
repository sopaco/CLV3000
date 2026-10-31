//! 病毒库页面：页头 + 单张居中英雄卡（版本 + 引擎状态 + 更新/打开按钮）。
//!
//! 2026-10 结构改版：原来是两栏（左病毒库状态、右 About 介绍块）——About 信息
//! 跟病毒库无关，且侧栏底部已有版本号、托盘菜单有独立 About 弹窗，右栏整个
//! 删掉；病毒库信息收进一张卡，一眼看完。
//!
//! 布局坑（2026-10 预览探针实测）：不要在 Frame 闭包里 `set_min_width` 给卡定
//! 宽——egui 0.36 的 `set_min_*` 是"从 cursor 当前位置再申请一块"的语义，放在
//! 内容之前会把后续内容（如 Definitions 路径行）的可用宽挤到只剩边角，label
//! 反向溢出把 Frame 撑到 755px 宽（右缘 1025，超出窗口 125px）。定宽一律由
//! 外层 cell `allocate_ui_with_layout` + 闭包开头 `set_min_size` 完成（同
//! `widgets::stat_card` 的修复模式），Frame 只负责描边/底色/圆角。

use super::super::App;
use crate::icons;
use crate::localtime::Timestamp;
use crate::paths;
use crate::theme::colors;
use crate::widgets::{self, action_button, primary_button};
use eframe::egui;
use egui::{RichText, Stroke, Vec2};

/// 英雄卡的目标宽度（窄窗口时退化为全宽减页边距）。
const CARD_W: f32 = 560.0;

pub(in crate::app) fn virus_db_page(ui: &mut egui::Ui, app: &mut App) {
    ui.add_space(20.0);
    widgets::page_header(
        ui,
        icons::database,
        "Virus Database",
        "Signature definitions and scan engine",
        |_| {},
    );

    let mut content_height = app.core.virus_db.content_height;
    widgets::vertically_centered(ui, &mut content_height, |ui| {
        let avail = ui.available_width();
        let card_w = CARD_W.min(avail);
        let left = ((avail - card_w) / 2.0).max(0.0);
        ui.horizontal_top(|ui| {
            ui.add_space(left);
            // 外层 cell 定宽（期望高度给个宽松值，真实高度由内容决定——
            // allocate_ui_with_layout 只保证"至少这么大"，不裁内容）。
            ui.allocate_ui_with_layout(
                Vec2::new(card_w, 100.0),
                egui::Layout::top_down(egui::Align::Min),
                |ui| {
                    ui.set_min_width(card_w);
                    ui.set_max_width(card_w);
                    db_card(ui, app);
                },
            );
        });
    });
    app.core.virus_db.content_height = content_height;
}

fn db_card(ui: &mut egui::Ui, app: &mut App) {
    let mut pending_toast: Option<String> = None;
    let core = &mut app.core;
    // 版本懒加载：预览/测试预置 Some 跳过真实 clamscan -V 子进程。
    if core.virus_db.db_version.is_none() {
        core.virus_db.refresh_db_version(ui.ctx().clone());
    }
    let (available, detail_dir) = {
        let probe = core.virus_db.engine_probe();
        (probe.available, probe.detail_dir.clone())
    };

    egui::Frame::default()
        .fill(colors::BG_CARD)
        .stroke(Stroke::new(1.0, colors::BORDER))
        .corner_radius(crate::theme::radius::LG)
        .inner_margin(egui::Margin::same(24))
        .show(ui, |ui| {
            // 撑满 cell 宽（外层 allocate_ui_with_layout 已限宽 card_w）：
            // Frame 默认只包内容那么大，不限的话卡会缩成内容宽，居中就失效了。
            // （同 dashboard 英雄卡的模式；注意必须在内容之前调，且这里的语义是
            // "cursor 起可用宽度"，开头调就是整卡宽。）
            ui.set_min_width(ui.available_width());
            ui.horizontal(|ui| {
                // 图标砖：品牌蓝底纹 + database 图形，跟页头的图标砖同一语言，
                // 只是这一张是页面的主视觉，尺寸放大到 64。
                let (tile_resp, painter) =
                    ui.allocate_painter(Vec2::splat(64.0), egui::Sense::hover());
                painter.rect_filled(
                    tile_resp.rect,
                    crate::theme::radius::MD,
                    colors::ACCENT_BLUE_BG,
                );
                icons::database(
                    &painter,
                    tile_resp.rect.shrink(15.0),
                    Stroke::new(2.2, colors::ACCENT_BLUE),
                );
                ui.add_space(20.0);
                ui.vertical(|ui| {
                    // 卡里不再重复一遍 "Virus Database"——页头已经写着页面名，
                    // 卡片的主标题让给"版本号"本身（这才是这一页要回答的问题）。
                    let text_w = (ui.available_width() - 24.0).max(80.0);
                    match &core.virus_db.db_version {
                        Some(ver) => {
                            // 版本串生产形态是 `28167 (Sat Oct 28 09:32:57 2026)`
                            // ——整串一行太占地方，编号与日期拆开：编号当主标题
                            // （加千分位，28,167 更好读），日期挪到状态行。
                            let (build, date) = crate::clamav_info::split_version(ver);
                            let build = crate::clamav_info::format_build_number(&build);
                            let ver_w = (text_w
                                - widgets::measure_text_width(ui, "Version ", 20.0))
                            .max(40.0);
                            widgets::bold_label(
                                ui,
                                &format!(
                                    "Version {}",
                                    widgets::truncate_middle_to_px(ui, &build, 20.0, ver_w)
                                ),
                                20.0,
                                colors::TEXT_PRIMARY,
                            );
                            ui.add_space(6.0);
                            let status = match (available, date) {
                                (false, _) => "Scan engine not found".to_string(),
                                (true, Some(d)) => format!("Updated {d} · Built-in database ready"),
                                (true, None) => "Built-in database ready".to_string(),
                            };
                            ui.label(
                                egui::RichText::new(status)
                                    .color(colors::TEXT_MUTED)
                                    .small(),
                            );
                        }
                        None => {
                            widgets::bold_label(ui, "Reading version…", 20.0, colors::TEXT_SECONDARY);
                            ui.add_space(6.0);
                            let status = if available {
                                "Built-in database ready"
                            } else {
                                "Scan engine not found"
                            };
                            ui.label(
                                egui::RichText::new(status)
                                    .color(colors::TEXT_MUTED)
                                    .small(),
                            );
                        }
                    };
                    ui.add_space(14.0);

                    // 主/次操作并排一行：原来上下堆叠，两颗按钮宽度不一、右缘参差。
                    // 并排后主操作在左、次操作在右，垂直居中对齐；文本列不够宽时
                    // （窄窗口）退回上下堆叠。
                    let update_label = if core.virus_db.updating {
                        "Updating…"
                    } else {
                        "Update Database"
                    };
                    let mut clicked_update = false;
                    let mut clicked_open = false;
                    if ui.available_width() >= 380.0 {
                        ui.horizontal(|ui| {
                            if primary_button(ui, update_label, icons::database) {
                                clicked_update = true;
                            }
                            ui.add_space(10.0);
                            if action_button(ui, "Open Folder", icons::folder) {
                                clicked_open = true;
                            }
                        });
                    } else {
                        if primary_button(ui, update_label, icons::database) {
                            clicked_update = true;
                        }
                        ui.add_space(10.0);
                        if action_button(ui, "Open Folder", icons::folder) {
                            clicked_open = true;
                        }
                    }
                    // 副作用延迟到渲染后统一落地，渲染闭包里不再穿插可变借用。
                    if clicked_update && !core.virus_db.updating {
                        core.virus_db.start_update(ui.ctx().clone());
                        pending_toast = Some("Updating database…".to_string());
                    }
                    if clicked_open {
                        paths::ensure_dir(&detail_dir);
                        if let Err(e) = paths::open_in_file_explorer(&detail_dir) {
                            pending_toast = Some(e);
                        }
                    }
                });
            });
            ui.add_space(14.0);
            // 路径行：按卡内像素宽中间省略（换行会把路径拆得更难读，硬裁切又没
            // 有提示“后面还有内容”）。“Definitions stored in ” 前缀也计入宽度。
            let path_w = (ui.available_width() - 4.0).max(40.0);
            let prefix = "Definitions stored in ";
            let path = detail_dir.display().to_string();
            let budget = (path_w - widgets::measure_text_width(ui, prefix, 12.5)).max(40.0);
            ui.label(
                RichText::new(format!(
                    "{prefix}{}",
                    widgets::truncate_middle_to_px(ui, &path, 12.5, budget)
                ))
                .color(colors::TEXT_MUTED)
                .small(),
            );

            // 自动更新开关 + 上次检查时间。放在路径行下面、用一条细分割线隔开：
            // 上面是"库的状态"，下面是"更新策略"，两件事不该混在一段文字里。
            ui.add_space(12.0);
            widgets::row_divider(ui);
            ui.add_space(10.0);
            let mut auto = core.config.auto_update_definitions;
            if widgets::toggle_setting_row(
                ui,
                &mut auto,
                "Automatic updates",
                "Check for new definitions on launch and refresh the database silently",
                true,
            ) {
                core.config.auto_update_definitions = auto;
                core.config.save();
            }
            ui.add_space(6.0);
            // "上次检查"这一行要说清下一步，而不是只报一个空白状态。
            let last_check = match core.config.last_update_check {
                Some(t) => format!("Last checked {}", t.display_relative_to(&Timestamp::now())),
                None if auto => "The first check runs when CLV3000 starts".to_string(),
                None => "Automatic checks are off — update manually".to_string(),
            };
            ui.label(
                egui::RichText::new(last_check)
                    .color(colors::TEXT_MUTED)
                    .small(),
            );
        });

    if let Some(msg) = pending_toast {
        app.toast(msg);
    }
}
