//! 设置页：两个 tab——隔离区/忽略列表管理，和一个统一的「General」（开机自启动 +
//! 右键菜单[仅 Windows]，未来同类应用级开关也归这里，不再按功能各开一个 tab）。
//! 跟仪表盘/扫描页那种"内容居中"的视觉语言不同：这里是列表/开关为主的工具型
//! 页面，走常规的顶部对齐布局（标题 → tab 栏 → 内容），不套 `vertically_centered`。

use super::App;
use super::core::SettingsTab;
use crate::config::{IgnoredEntry, QuarantineEntry};
use crate::localtime::Timestamp;
use crate::theme::colors;
use crate::widgets;
use eframe::egui;
use egui::{Stroke, Vec2};

/// 内容区左侧起始留白，标题/tab 栏/卡片统一用这个对齐，不然贴着面板左边缘显得
/// 局促（对比 `virus_db_page` 靠 `columns` 自身居中不需要这个——这里是列表/卡片，
/// 天然贴左，需要显式留白）。
const CONTENT_INSET: f32 = 24.0;
/// General tab 那张分组卡片的宽度：内容（checkbox + 标题 + 说明文字）都是静态
/// 文案，不随运行时变化，拍一个校准过的固定值即可（design skill 坑 1 的"静态
/// 文字可以拍一个校准过的偏移量"那条），不用像扫描进度那种会变的数字一样量出来。
const CARD_WIDTH: f32 = 560.0;

pub(super) fn settings_page(ui: &mut egui::Ui, app: &mut App) {
    ui.add_space(24.0);
    ui.horizontal(|ui| {
        ui.add_space(CONTENT_INSET);
        widgets::bold_label(ui, "Settings", 20.0, colors::TEXT_PRIMARY);
    });
    ui.add_space(18.0);

    let tab = app.core.settings.tab;
    ui.horizontal(|ui| {
        ui.add_space(CONTENT_INSET);
        if tab_button(ui, "Quarantine & Ignore", tab == SettingsTab::QuarantineIgnore) {
            app.core.settings.tab = SettingsTab::QuarantineIgnore;
        }
        ui.add_space(8.0);
        if tab_button(ui, "General", tab == SettingsTab::General) {
            app.core.settings.tab = SettingsTab::General;
        }
    });
    ui.add_space(20.0);

    egui::ScrollArea::vertical().show(ui, |ui| {
        ui.horizontal(|ui| {
            ui.add_space(CONTENT_INSET);
            ui.vertical(|ui| {
                ui.set_width((ui.available_width() - CONTENT_INSET).max(200.0));
                match app.core.settings.tab {
                    SettingsTab::QuarantineIgnore => quarantine_ignore_tab(ui, app),
                    SettingsTab::General => general_tab(ui, app),
                }
            });
        });
    });
}

/// tab 栏的文字胶囊按钮：当前 tab 用 `ACCENT_BLUE_BG` 填充高亮，跟侧栏 active 态
/// 同一个视觉语言。用"量尺寸 + `allocate_ui_with_layout` + 占位 Shape 回填"这套
/// （design skill 坑 1），不直接用 `Frame`/`ui.horizontal`——虽然这里没有外层
/// `Align::Center` 要满足，但这套写法本身就是这个项目里"自定义可点击胶囊"的
/// 标准实现（跟 `pages.rs::action_button`、`widgets.rs::pill_button` 一致），
/// 沿用统一风格。
fn tab_button(ui: &mut egui::Ui, label: &str, active: bool) -> bool {
    const H_PAD: f32 = 16.0;
    const V_PAD: f32 = 8.0;

    let text_color = if active {
        colors::TEXT_PRIMARY
    } else {
        colors::TEXT_SECONDARY
    };
    let text_size = Vec2::new(
        widgets::measure_text_width(ui, label, 14.0),
        ui.text_style_height(&egui::TextStyle::Body),
    );
    let desired = Vec2::new(H_PAD * 2.0 + text_size.x, V_PAD * 2.0 + text_size.y);

    let bg_idx = ui.painter().add(egui::Shape::Noop);
    let response = ui
        .allocate_ui_with_layout(desired, egui::Layout::left_to_right(egui::Align::Center), |ui| {
            ui.add_space(H_PAD);
            ui.label(egui::RichText::new(label).color(text_color));
            ui.add_space(H_PAD);
        })
        .response;

    let bg_rect = response.rect;
    let interact = ui
        .interact(bg_rect, response.id.with("settings_tab"), egui::Sense::click())
        .on_hover_cursor(egui::CursorIcon::PointingHand);
    let fill = if active {
        colors::ACCENT_BLUE_BG
    } else if interact.hovered() {
        colors::BG_CARD
    } else {
        egui::Color32::TRANSPARENT
    };
    let shape = egui::epaint::RectShape::new(
        bg_rect,
        egui::CornerRadius::same(10),
        fill,
        Stroke::new(1.0, colors::BORDER),
        egui::epaint::StrokeKind::Inside,
    );
    ui.painter().set(bg_idx, egui::Shape::Rect(shape));

    interact.clicked()
}

fn quarantine_ignore_tab(ui: &mut egui::Ui, app: &mut App) {
    let today = Timestamp::now();
    ui.columns(2, |columns| {
        quarantine_column(&mut columns[0], app, &today);
        ignored_column(&mut columns[1], app);
    });
}

#[derive(Clone, Copy)]
enum RowAction {
    None,
    Restore,
    Delete,
}

fn quarantine_column(ui: &mut egui::Ui, app: &mut App, today: &Timestamp) {
    widgets::bold_label(ui, "Quarantine", 15.0, colors::TEXT_PRIMARY);
    ui.add_space(10.0);
    if app.core.config.quarantined.is_empty() {
        ui.label(
            egui::RichText::new("No quarantined files yet.")
                .color(colors::TEXT_MUTED)
                .small(),
        );
        return;
    }

    let mut pending: Option<(usize, RowAction)> = None;
    for (i, entry) in app.core.config.quarantined.iter().enumerate() {
        let action = quarantine_row(ui, entry, today);
        if !matches!(action, RowAction::None) {
            pending = Some((i, action));
        }
        ui.add_space(8.0);
    }

    let Some((i, action)) = pending else { return };
    // 索引在应用动作之前重新按需读取一次（列表在动作发生前不会被并发修改），
    // clone 出这条记录再操作，避免一边借用 `app.core.config.quarantined` 一边
    // 又要 `&mut app.core.config` 调 `remove_quarantined`。
    let entry = app.core.config.quarantined[i].clone();
    match action {
        RowAction::Restore => match crate::quarantine::restore_file(&entry) {
            Ok(()) => {
                app.core.config.remove_quarantined(&entry.stored_name);
                app.toast("File restored to its original location");
            }
            Err(e) => app.toast(e),
        },
        RowAction::Delete => match crate::quarantine::delete_permanently(&entry) {
            Ok(()) => {
                app.core.config.remove_quarantined(&entry.stored_name);
                app.toast("Quarantined file deleted");
            }
            Err(e) => app.toast(e),
        },
        RowAction::None => {}
    }
}

fn quarantine_row(ui: &mut egui::Ui, entry: &QuarantineEntry, today: &Timestamp) -> RowAction {
    let mut action = RowAction::None;
    egui::Frame::default()
        .fill(colors::BG_CARD)
        .stroke(Stroke::new(1.0, colors::BORDER))
        .corner_radius(10.0)
        .inner_margin(egui::Margin::symmetric(12, 10))
        .show(ui, |ui| {
            ui.vertical(|ui| {
                ui.horizontal(|ui| {
                    widgets::bold_label(ui, &entry.virus_name, 13.0, colors::TEXT_PRIMARY);
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.label(
                            egui::RichText::new(entry.quarantined_at.display_relative_to(today))
                                .color(colors::TEXT_MUTED)
                                .small(),
                        );
                    });
                });
                ui.label(
                    egui::RichText::new(widgets::truncate_middle(&entry.original_path, 44))
                        .color(colors::TEXT_SECONDARY)
                        .small(),
                );
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    if widgets::pill_button(ui, "Restore", false) {
                        action = RowAction::Restore;
                    }
                    ui.add_space(8.0);
                    if widgets::pill_button(ui, "Delete", true) {
                        action = RowAction::Delete;
                    }
                });
            });
        });
    action
}

fn ignored_column(ui: &mut egui::Ui, app: &mut App) {
    widgets::bold_label(ui, "Ignored", 15.0, colors::TEXT_PRIMARY);
    ui.add_space(10.0);
    if app.core.config.ignored.is_empty() {
        ui.label(
            egui::RichText::new("No ignored threats yet.")
                .color(colors::TEXT_MUTED)
                .small(),
        );
        return;
    }

    let mut remove_target: Option<usize> = None;
    for (i, entry) in app.core.config.ignored.iter().enumerate() {
        if ignored_row(ui, entry) {
            remove_target = Some(i);
        }
        ui.add_space(8.0);
    }

    if let Some(i) = remove_target {
        let entry = app.core.config.ignored[i].clone();
        app.core.config.remove_ignored(&entry.path, &entry.virus_name);
        app.toast("Removed from ignore list");
    }
}

fn ignored_row(ui: &mut egui::Ui, entry: &IgnoredEntry) -> bool {
    let mut clicked = false;
    egui::Frame::default()
        .fill(colors::BG_CARD)
        .stroke(Stroke::new(1.0, colors::BORDER))
        .corner_radius(10.0)
        .inner_margin(egui::Margin::symmetric(12, 10))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.vertical(|ui| {
                    widgets::bold_label(ui, &entry.virus_name, 13.0, colors::TEXT_PRIMARY);
                    ui.label(
                        egui::RichText::new(widgets::truncate_middle(&entry.path, 40))
                            .color(colors::TEXT_SECONDARY)
                            .small(),
                    );
                });
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if widgets::pill_button(ui, "Remove", false) {
                        clicked = true;
                    }
                });
            });
        });
    clicked
}

/// 「General」tab：应用级开关的统一分组——一张卡片，里面是一行一行的设置项，
/// 行间用一条细分割线隔开（不是每个开关各套一张卡片）。这是常见设置面板
/// （macOS 系统设置/VS Code 设置分组）的标准长相：同类设置聚在一张卡片里，
/// 靠行内分割线区分，视觉上比"一堆并排小卡片"更整洁、更像一个统一的设置区域。
fn general_tab(ui: &mut egui::Ui, app: &mut App) {
    egui::Frame::default()
        .fill(colors::BG_CARD)
        .stroke(Stroke::new(1.0, colors::BORDER))
        .corner_radius(12.0)
        .inner_margin(egui::Margin::same(16))
        .show(ui, |ui| {
            ui.set_width(CARD_WIDTH);
            autostart_row(ui, app);
            ui.add_space(14.0);
            row_divider(ui);
            ui.add_space(14.0);
            context_menu_row(ui, app);
        });
}

/// 行间细分割线：跟卡片描边同色（`BORDER`），比 egui 默认 `ui.separator()` 更淡，
/// 只在这一张卡片内部区分"两条设置"，不是 `Panel` 级的硬分割线（design skill
/// 坑 4 说的是面板边缘那种，这里是同一张卡片里主动想要的行间分隔，两者不矛盾）。
fn row_divider(ui: &mut egui::Ui) {
    let (rect, _) =
        ui.allocate_exact_size(Vec2::new(ui.available_width(), 1.0), egui::Sense::hover());
    ui.painter()
        .hline(rect.x_range(), rect.center().y, Stroke::new(1.0, colors::BORDER));
}

/// 一行设置项：checkbox + 标题 + 说明文字。`enabled=false` 时整行变灰且 checkbox
/// 不可交互（当前平台不支持这个功能——目前只有右键菜单的非 Windows 分支），但
/// 布局跟正常行完全一致，不会因为"不可用"而错位。返回是否被用户切换（切换后
/// `checked` 已经是新值，调用者决定是否要落地这次变化）。
fn toggle_row(
    ui: &mut egui::Ui,
    checked: &mut bool,
    title: &str,
    description: &str,
    enabled: bool,
) -> bool {
    let mut changed = false;
    let title_color = if enabled { colors::TEXT_PRIMARY } else { colors::TEXT_MUTED };
    let desc_color = if enabled { colors::TEXT_SECONDARY } else { colors::TEXT_MUTED };
    ui.horizontal(|ui| {
        ui.add_enabled_ui(enabled, |ui| {
            if ui.checkbox(checked, "").changed() {
                changed = true;
            }
        });
        ui.add_space(4.0);
        ui.vertical(|ui| {
            widgets::bold_label(ui, title, 14.0, title_color);
            ui.add_space(2.0);
            ui.label(egui::RichText::new(description).color(desc_color).small());
        });
    });
    changed
}

fn autostart_row(ui: &mut egui::Ui, app: &mut App) {
    let cached = *app
        .core
        .settings
        .autostart_enabled
        .get_or_insert_with(crate::autostart::is_enabled);
    let mut checked = cached;
    let changed = toggle_row(
        ui,
        &mut checked,
        "Start CLV3000 automatically at login",
        "Launches minimized to the system tray when you log in (same as starting with --tray-only).",
        true,
    );
    if changed {
        match crate::autostart::set_enabled(checked) {
            Ok(()) => app.core.settings.autostart_enabled = Some(checked),
            Err(e) => app.toast(e),
        }
    }
}

#[cfg(windows)]
fn context_menu_row(ui: &mut egui::Ui, app: &mut App) {
    let cached = *app
        .core
        .settings
        .context_menu_enabled
        .get_or_insert_with(crate::context_menu::is_enabled);
    let mut checked = cached;
    let changed = toggle_row(
        ui,
        &mut checked,
        "Add \"Scan with CLV3000\" to the right-click menu",
        "Right-click a file or folder in File Explorer and choose \"Scan with CLV3000\" to scan it directly.",
        true,
    );
    if changed {
        match crate::context_menu::set_enabled(checked) {
            Ok(()) => app.core.settings.context_menu_enabled = Some(checked),
            Err(e) => app.toast(e),
        }
    }
}

/// 非 Windows：这一行永远关闭且不可交互，只是让"General"分组卡片里始终能看到
/// 这项功能存在（灰态说明"暂不支持"），而不是干脆从卡片里消失——跟隔离区/忽略
/// 列表的"空态文案"是同一个设计意图：告诉用户这里本来有什么，而不是留一片空白。
#[cfg(not(windows))]
fn context_menu_row(ui: &mut egui::Ui, _app: &mut App) {
    let mut dummy = false;
    toggle_row(
        ui,
        &mut dummy,
        "Add \"Scan with CLV3000\" to the right-click menu",
        "Not available on this platform yet.",
        false,
    );
}
