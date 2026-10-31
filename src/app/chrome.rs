//! 窗口"外壳"：自绘标题栏、左侧导航栏、底部资源条。跟四个页面
//! 的业务内容无关，是每个页面都共用的固定框架。

use super::{App, Page};
use crate::icons;
use crate::sysmon::ResourceSample;
use crate::theme::{self, colors};
use crate::widgets;
use eframe::egui;
use egui::{Stroke, Vec2, ViewportCommand};

pub(super) const TITLE_BAR_HEIGHT: f32 = 44.0;

pub(super) fn title_bar(
    ui: &mut egui::Ui,
    ctx: &egui::Context,
    icon_texture: &egui::TextureHandle,
    app: &mut App,
) {
    egui::Panel::top("title_bar")
        .exact_size(TITLE_BAR_HEIGHT)
        .resizable(false)
        .show_separator_line(false)
        .frame(egui::Frame::default().fill(colors::BG_TITLEBAR))
        .show(ui, |ui| {
            // 按钮位置直接按 `full_rect` 算好精确坐标，不走 `ui.horizontal` 的光标累加——
            // 光标累加容易因为间距估算偏差导致最后一个按钮离边缘忽远忽近，看起来"没对齐"。
            let full_rect = ui.max_rect();
            let btn_size = 32.0;
            let btn_gap = 4.0;
            let edge_margin = 8.0;

            let close_rect = egui::Rect::from_center_size(
                egui::pos2(
                    full_rect.right() - edge_margin - btn_size / 2.0,
                    full_rect.center().y,
                ),
                Vec2::splat(btn_size),
            );
            let min_rect = egui::Rect::from_center_size(
                egui::pos2(
                    close_rect.left() - btn_gap - btn_size / 2.0,
                    full_rect.center().y,
                ),
                Vec2::splat(btn_size),
            );

            if title_bar_button(
                ui,
                close_rect,
                "close",
                "Close window",
                true,
                |painter, rect, hovered| {
                    // hover 红底时换白图标，保持图形对比（行业惯例：关闭悬停变红）。
                    let stroke = if hovered {
                        Stroke::new(1.4, egui::Color32::WHITE)
                    } else {
                        Stroke::new(1.4, colors::TEXT_SECONDARY)
                    };
                    icons::close(painter, rect, stroke);
                },
            ) {
                app.hide_to_tray(ctx);
            }
            if title_bar_button(
                ui,
                min_rect,
                "minimize",
                "Minimize window",
                false,
                |painter, rect, _hovered| {
                    icons::minimize(painter, rect, Stroke::new(1.4, colors::TEXT_SECONDARY));
                },
            ) {
                ctx.send_viewport_cmd(ViewportCommand::Minimized(true));
            }

            // 左边图标 + 标题文字，走正常的光标布局就行，反正不需要跟右边对齐。
            // 图标用真实的简化版美术图标（icon_tray.png），跟窗口图标/托盘图标
            // 保持一致，不再用矢量画的盾牌。
            ui.horizontal_centered(|ui| {
                ui.add_space(14.0);
                ui.add(
                    egui::Image::new((icon_texture.id(), icon_texture.size_vec2()))
                        .fit_to_exact_size(Vec2::splat(22.0)),
                );
                ui.add_space(8.0);
                crate::widgets::bold_label(ui, "CLV3000", 15.0, colors::TEXT_PRIMARY);
            });

            // 整条标题栏（除了右上角两个按钮）都能拖动窗口——包括图标和标题文字
            // 那块区域。图标/文字本身只声明了 `Sense::hover()`，不感知拖拽，
            // 所以叠在它们上面这个更大的拖拽区域不会跟点击/悬浮冲突。
            let drag_rect = egui::Rect::from_min_max(
                egui::pos2(full_rect.left(), full_rect.top()),
                egui::pos2(min_rect.left() - btn_gap, full_rect.bottom()),
            );
            if drag_rect.width() > 0.0 {
                let drag_resp = ui.interact(
                    drag_rect,
                    ui.id().with("titlebar_drag"),
                    egui::Sense::drag(),
                );
                // 用 `is_pointer_button_down_on`（按下当帧就 StartDrag），不要用
                // `drag_started`——后者要等越过拖拽阈值，系统 mouseDown 已过去，
                // macOS 上会拖不动窗口（见 about_dialog.rs 同款说明）。
                if drag_resp.is_pointer_button_down_on() {
                    ctx.send_viewport_cmd(ViewportCommand::StartDrag);
                }
            }
        });
}

/// `danger=true` 的按钮（关闭）hover 时转红底白图标，行业惯例；其余用品牌蓝底。
/// `accessible_label` 进 accesskit 树（屏幕阅读器可读，kittest 可查——AGENTS.md
/// 规则 3：自绘可交互组件必须补 widget_info）。
#[allow(clippy::too_many_arguments)]
fn title_bar_button(
    ui: &mut egui::Ui,
    rect: egui::Rect,
    name: &str,
    accessible_label: &str,
    danger: bool,
    draw: impl FnOnce(&egui::Painter, egui::Rect, bool),
) -> bool {
    let response = ui
        .interact(
            rect,
            ui.id().with(("titlebar_btn", name)),
            egui::Sense::click(),
        )
        .on_hover_cursor(egui::CursorIcon::PointingHand);
    response.widget_info(|| {
        egui::WidgetInfo::labeled(egui::WidgetType::Button, true, accessible_label.to_owned())
    });
    let painter = ui.painter();
    let hovered = response.hovered();
    if hovered {
        let fill = if danger {
            colors::RED_ACTION
        } else {
            colors::ACCENT_BLUE_BG
        };
        painter.rect_filled(rect, theme::radius::SM, fill);
    }
    draw(painter, rect.shrink(9.0), hovered);
    response.clicked()
}

struct SidebarItem {
    page: Page,
    /// 名字同时用于 tooltip 与 accesskit 标签——纯图标导航必须两者都有，
    /// 否则用户靠猜、屏幕阅读器读不出来。
    label: &'static str,
    draw: fn(&egui::Painter, egui::Rect, Stroke),
}

/// 宽侧栏（图标 + 文字）与纯图标窄栏两种形态。窗口宽于阈值时用宽栏——
/// 纯图标导航要靠 tooltip 猜页面，是原型感的主来源之一；窗口很窄时
/// （MIN_INNER_SIZE 440）内容区只剩 ~240px，200px 宽栏会把页面挤爆，
/// 退回纯图标保内容。
pub(super) const SIDEBAR_WIDE: f32 = 200.0;
pub(super) const SIDEBAR_COMPACT: f32 = 64.0;
const SIDEBAR_WIDE_MIN_WINDOW: f32 = 620.0;

/// 侧栏内容区左右各留的边距。导航行不贴着窗口边缘——否则 active 行的左缘
/// 指示条会变成"焊在窗框上的一小条"，看着像被裁掉一半；同时给导航项一个
/// 与页面 PAGE_INSET 同源的呼吸位。
pub(super) const SIDEBAR_INSET: i8 = 8;

/// 导航行之间的行距（行高 40 之外的额外间距）。
const NAV_ROW_GAP: f32 = 8.0;

/// 按窗口宽度选侧栏形态（`app_shell` 建 Panel 时问一次）。
pub(super) fn sidebar_width(window_width: f32) -> f32 {
    if window_width >= SIDEBAR_WIDE_MIN_WINDOW {
        SIDEBAR_WIDE
    } else {
        SIDEBAR_COMPACT
    }
}

pub(super) fn sidebar(ui: &mut egui::Ui, _ctx: &egui::Context, app: &mut App) {
    // Panel 宽度已经按窗口宽度定好了，这里从自己的可用宽度反推形态。
    let labeled = ui.available_width() > SIDEBAR_COMPACT + 40.0;
    ui.add_space(12.0);
    // 导航行宽在循环外算一次并传下去。循环里每次现算的话，上一行会把自己的
    // 可用宽撑大、下一行就跟着变宽（历史 bug：194 → 310 → 427 → 531 → 672，
    // 交互矩形一路伸进主内容区，页面空白处点击都会误触导航）。
    let row_w = ui.available_width().max(64.0);
    let items = [
        SidebarItem {
            page: Page::Dashboard,
            label: "Dashboard",
            draw: |p, r, s| icons::shield(p, r, s, None),
        },
        SidebarItem {
            page: Page::QuickScan,
            label: "Quick Scan",
            draw: |p, r, s| icons::bolt(p, r, s.color),
        },
        SidebarItem {
            page: Page::FullScan,
            label: "Full Scan",
            draw: |p, r, s| icons::computer(p, r, s),
        },
        SidebarItem {
            page: Page::VirusDb,
            label: "Virus Database",
            draw: |p, r, s| icons::database(p, r, s),
        },
        SidebarItem {
            page: Page::Settings,
            label: "Settings",
            draw: |p, r, s| icons::gear(p, r, s),
        },
    ];

    for item in items {
        let active = app.core.page == item.page;
        let response = if labeled {
            labeled_nav_item(ui, &item, active, row_w)
        } else {
            compact_nav_item(ui, &item, active)
        };
        if response.clicked() {
            app.navigate(item.page);
        }
        ui.add_space(NAV_ROW_GAP);
    }

    // 底部版本号：填充图标列下方的大片留白，也让"关于"信息不必再挤在病毒库页。
    ui.with_layout(egui::Layout::bottom_up(egui::Align::Min), |ui| {
        ui.add_space(12.0);
        let version = format!("v{}", env!("CARGO_PKG_VERSION"));
        if labeled {
            ui.horizontal(|ui| {
                // 与导航图标列左缘对齐（图标列起点 = SIDEBAR_INSET + 12）。
                ui.add_space(12.0);
                ui.label(
                    egui::RichText::new(version)
                        .color(colors::TEXT_MUTED)
                        .small(),
                );
            });
        } else {
            ui.vertical_centered(|ui| {
                ui.label(
                    egui::RichText::new(version)
                        .color(colors::TEXT_MUTED)
                        .small(),
                );
            });
        }
    });
}

/// 宽形态导航项：整行可点（图标 20 + 文字），active 行铺 ACCENT_BLUE_BG + 左缘
/// 指示条。行宽 = 侧栏内容宽（由调用方算好传入），高度 40 与窄形态的格子一致。
///
/// 这里不用 `allocate_ui_with_layout` + `set_min_size`：`set_min_*` 是"从 cursor
/// 再申请一块"的语义（design skill 坑 10），放在闭包末尾会把行矩形撑成
/// "内容宽 + 行宽"——第一行就把侧栏可用宽撑爆，后续行越走越宽（实测
/// 194 → 310 → 427 → 531 → 672，命中区一路伸进主内容区，页面空白处点击会
/// 误触导航；画出来的底色又被面板裁到只剩一条，看着倒"像没事"，最难查）。
/// 改成自己算精确矩形 + `allocate_exact_size`，行宽与命中区都锁死。
fn labeled_nav_item(
    ui: &mut egui::Ui,
    item: &SidebarItem,
    active: bool,
    row_w: f32,
) -> egui::Response {
    const ROW_H: f32 = 40.0;
    const ICON: f32 = 20.0;
    const LEFT_PAD: f32 = 12.0;
    const ICON_GAP: f32 = 12.0;

    let (rect, response) = ui.allocate_exact_size(Vec2::new(row_w, ROW_H), egui::Sense::click());
    let interact = response.on_hover_cursor(egui::CursorIcon::PointingHand);
    // 纯图标窄栏必须带 tooltip + widget_info，宽栏的文字本身就是标签（挂了
    // tooltip 反而遮挡内容），但 widget_info 一样要补——屏幕阅读器读得出。
    interact.widget_info(|| {
        egui::WidgetInfo::labeled(egui::WidgetType::Button, true, item.label.to_owned())
    });

    let painter = ui.painter();
    let fill = if active {
        colors::ACCENT_BLUE_BG
    } else if interact.hovered() {
        colors::BG_CARD
    } else {
        egui::Color32::TRANSPARENT
    };
    painter.rect_filled(rect, theme::radius::MD, fill);
    if active {
        // 左缘指示条：active 行的第二个锚点——ACCENT_BLUE_BG 与 hover 时的
        // BG_CARD 都是深色填充，光靠底色区分度不够。
        //
        // 位置要卡在行底色的**直边**里：行是 12px 圆角的矩形，指示条若按整行
        // 高度画（top+8…bottom-8），上下两截正好落在圆角上，会从圆角外侧
        // 支出来，看着像一片贴歪了的蓝标签（2026-10 预览 5× 放大实测）。收进
        // 直边里、并右移 0.5px 贴住边线，就是一条干净的短竖条。
        let bar = egui::Rect::from_min_max(
            egui::pos2(rect.left() + 0.5, rect.top() + 11.0),
            egui::pos2(rect.left() + 3.0, rect.bottom() - 11.0),
        );
        painter.rect_filled(bar, 1.25, colors::ACCENT_BLUE);
    }

    let color = if active {
        colors::ACCENT_BLUE
    } else {
        colors::TEXT_SECONDARY
    };
    let icon_rect = egui::Rect::from_min_size(
        egui::pos2(rect.left() + LEFT_PAD, rect.center().y - ICON / 2.0),
        Vec2::splat(ICON),
    );
    (item.draw)(painter, icon_rect, Stroke::new(1.6, color));

    // 文字直接画（galley 直画），不用 ui.label——否则 accesskit 树里会同时出现
    // "文本节点 + 按钮节点"两个同名节点，kittest get_by_label 多匹配 panic
    // （同 primary_button 的说明，design skill「坑 1」一节）。
    painter.text(
        egui::pos2(icon_rect.right() + ICON_GAP, rect.center().y),
        egui::Align2::LEFT_CENTER,
        item.label,
        egui::FontId::proportional(14.0),
        if active {
            colors::TEXT_PRIMARY
        } else {
            colors::TEXT_SECONDARY
        },
    );

    interact
}

/// 窄形态（纯图标）导航项：居中的 40x40 格子，tooltip + 左缘指示条。
fn compact_nav_item(ui: &mut egui::Ui, item: &SidebarItem, active: bool) -> egui::Response {
    ui.vertical_centered(|ui| {
        let size = Vec2::splat(40.0);
        let (response, painter) = ui.allocate_painter(size, egui::Sense::click());
        let response = response
            .on_hover_cursor(egui::CursorIcon::PointingHand)
            .on_hover_text(item.label);
        response.widget_info(|| {
            egui::WidgetInfo::labeled(egui::WidgetType::Button, true, item.label.to_owned())
        });
        if active {
            painter.rect_filled(response.rect, theme::radius::MD, colors::ACCENT_BLUE_BG);
            // 左缘指示条：active 格子除了底色再多一个锚点——ACCENT_BLUE_BG 与
            // hover 时的 BG_CARD 都是深色填充，光靠底色区分度不够。位置同宽
            // 形态：收进 12px 圆角的直边里，别从圆角外侧支出来。
            let bar = egui::Rect::from_min_max(
                egui::pos2(response.rect.left() + 0.5, response.rect.top() + 11.0),
                egui::pos2(response.rect.left() + 3.0, response.rect.bottom() - 11.0),
            );
            painter.rect_filled(bar, 1.25, colors::ACCENT_BLUE);
        } else if response.hovered() {
            painter.rect_filled(response.rect, theme::radius::MD, colors::BG_CARD);
        }
        let color = if active {
            colors::ACCENT_BLUE
        } else {
            colors::TEXT_SECONDARY
        };
        let glyph_rect = response.rect.shrink(9.5);
        (item.draw)(&painter, glyph_rect, Stroke::new(1.6, color));
        response
    })
    .inner
}

pub(super) fn resource_bar(ui: &mut egui::Ui, sample: ResourceSample) {
    let cpu_pct = format!("{:.0}%", sample.cpu_percent);
    let mem_pct = format!("{:.0}%", sample.mem_percent());
    const GAP: f32 = 24.0;
    // 量宽居中：两个仪表的真实宽度（标签/条/百分比文字）量出来再加总，替代旧
    // 的 available_width()/2 - 160 拍脑袋补偿——那个常数没算右侧 "Memory/60%"
    // 比 "CPU/23%" 宽，评审实测整条内容右偏 44px。
    let total = meter_width(ui, "CPU", &cpu_pct) + GAP + meter_width(ui, "Memory", &mem_pct);
    let left = (ui.available_width() - total) / 2.0;
    ui.horizontal(|ui| {
        ui.add_space(left.max(0.0));
        resource_meter(ui, "CPU", sample.cpu_percent, &cpu_pct);
        ui.add_space(GAP);
        resource_meter(ui, "Memory", sample.mem_percent(), &mem_pct);
    });
}

/// 一个仪表的实占宽度：标签(Body 14) + 条(100) + 百分比(Body 14) + 两道 item_spacing。
fn meter_width(ui: &egui::Ui, label: &str, pct: &str) -> f32 {
    widgets::measure_text_width(ui, label, 14.0)
        + 100.0
        + widgets::measure_text_width(ui, pct, 14.0)
        + 2.0 * ui.spacing().item_spacing.x
}

fn resource_meter(ui: &mut egui::Ui, label: &str, percent: f32, pct_text: &str) {
    ui.label(egui::RichText::new(label).color(colors::TEXT_SECONDARY));
    let (response, painter) = ui.allocate_painter(Vec2::new(100.0, 8.0), egui::Sense::hover());
    let rect = response.rect;
    painter.rect_filled(rect, 4.0, colors::BG_CARD);
    let fraction = (percent / 100.0).clamp(0.0, 1.0);
    if fraction > 0.0 {
        let filled =
            egui::Rect::from_min_size(rect.min, Vec2::new(rect.width() * fraction, rect.height()));
        painter.rect_filled(filled, 4.0, theme::accent_for(percent));
    }
    ui.label(egui::RichText::new(pct_text).color(colors::TEXT_PRIMARY));
}
