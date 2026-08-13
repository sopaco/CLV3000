//! 窗口"外壳"：自绘标题栏（非 Windows）、左侧导航栏、底部资源条。跟四个页面
//! 的业务内容无关，是每个页面都共用的固定框架。

use super::{Page, App};
use crate::icons;
use crate::sysmon::ResourceSample;
use crate::theme::{self, colors};
use eframe::egui;
use egui::{Stroke, Vec2};
#[cfg(not(windows))]
use egui::ViewportCommand;

#[cfg(not(windows))]
pub(super) const TITLE_BAR_HEIGHT: f32 = 44.0;

#[cfg(not(windows))]
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

            if title_bar_button(ui, close_rect, "close", |painter, rect| {
                icons::close(painter, rect, Stroke::new(1.4, colors::TEXT_SECONDARY));
            }) {
                app.hide_to_tray(ctx);
            }
            if title_bar_button(ui, min_rect, "minimize", |painter, rect| {
                icons::minimize(painter, rect, Stroke::new(1.4, colors::TEXT_SECONDARY));
            }) {
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

#[cfg(not(windows))]
fn title_bar_button(
    ui: &mut egui::Ui,
    rect: egui::Rect,
    name: &str,
    draw: impl FnOnce(&egui::Painter, egui::Rect),
) -> bool {
    let response = ui
        .interact(
            rect,
            ui.id().with(("titlebar_btn", name)),
            egui::Sense::click(),
        )
        .on_hover_cursor(egui::CursorIcon::PointingHand);
    let painter = ui.painter();
    if response.hovered() {
        painter.rect_filled(rect, 6.0, colors::ACCENT_BLUE_BG);
    }
    let glyph_rect = rect.shrink(9.0);
    draw(painter, glyph_rect);
    response.clicked()
}

struct SidebarItem {
    page: Page,
    draw: fn(&egui::Painter, egui::Rect, Stroke),
}

pub(super) fn sidebar(ui: &mut egui::Ui, _ctx: &egui::Context, app: &mut App) {
    ui.add_space(18.0);
    let items = [
        SidebarItem {
            page: Page::Dashboard,
            draw: |p, r, s| icons::shield(p, r, s, None),
        },
        SidebarItem {
            page: Page::QuickScan,
            draw: |p, r, s| icons::bolt(p, r, s.color),
        },
        SidebarItem {
            page: Page::FullScan,
            draw: |p, r, s| icons::computer(p, r, s),
        },
        SidebarItem {
            page: Page::VirusDb,
            draw: |p, r, s| icons::database(p, r, s),
        },
        SidebarItem {
            page: Page::Settings,
            draw: |p, r, s| icons::gear(p, r, s),
        },
    ];

    for item in items {
        let active = app.core.page == item.page;
        ui.vertical_centered(|ui| {
            let size = Vec2::splat(40.0);
            let (response, painter) = ui.allocate_painter(size, egui::Sense::click());
            let response = response.on_hover_cursor(egui::CursorIcon::PointingHand);
            if active {
                painter.rect_filled(response.rect, 10.0, colors::ACCENT_BLUE_BG);
            } else if response.hovered() {
                painter.rect_filled(response.rect, 10.0, colors::BG_CARD);
            }
            let color = if active {
                colors::ACCENT_BLUE
            } else {
                colors::TEXT_SECONDARY
            };
            let glyph_rect = response.rect.shrink(9.5);
            (item.draw)(&painter, glyph_rect, Stroke::new(1.6, color));
            if response.clicked() {
                app.navigate(item.page);
            }
        });
        ui.add_space(14.0);
    }
}

pub(super) fn resource_bar(ui: &mut egui::Ui, sample: ResourceSample) {
    ui.horizontal(|ui| {
        ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
            ui.add_space(ui.available_width() / 2.0 - 160.0);
            resource_meter(ui, "CPU", sample.cpu_percent);
            ui.add_space(24.0);
            resource_meter(ui, "Memory", sample.mem_percent());
        });
    });
}

fn resource_meter(ui: &mut egui::Ui, label: &str, percent: f32) {
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
    ui.label(egui::RichText::new(format!("{percent:.0}%")).color(colors::TEXT_PRIMARY));
}
