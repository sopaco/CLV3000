//! 设置页：两个 tab——隔离区/忽略列表管理，和一个统一的「General」（开机自启动 +
//! 右键菜单[仅 Windows]，未来同类应用级开关也归这里，不再按功能各开一个 tab）。
//! 跟仪表盘/扫描页那种"内容居中"的视觉语言不同：这里是列表/开关为主的工具型
//! 页面，走常规的顶部对齐布局（标题 → tab 栏 → 内容），不套 `vertically_centered`。

use super::App;
use super::core::SettingsTab;
use crate::config::{IgnoredEntry, QuarantineEntry};
use crate::icons;
use crate::localtime::Timestamp;
use crate::theme::colors;
use crate::widgets;
use eframe::egui;
use egui::{Stroke, Vec2};

pub(super) fn settings_page(ui: &mut egui::Ui, app: &mut App) {
    // 页头跟其他四个页面同一套（图标砖 + 标题 + 副标题），取代原来孤零零一行
    // 20px 标题——五个页面的头部锚点统一，页面各自的差异化在页内做。
    ui.add_space(20.0);
    widgets::page_header(
        ui,
        icons::gear,
        "Settings",
        "Preferences, quarantine, and ignore lists",
        |_| {},
    );
    ui.add_space(14.0);

    let tab = app.core.settings.tab;
    ui.horizontal(|ui| {
        ui.add_space(widgets::PAGE_INSET);
        if tab_button(
            ui,
            "Quarantine & Ignore",
            tab == SettingsTab::QuarantineIgnore,
        ) {
            app.core.settings.tab = SettingsTab::QuarantineIgnore;
        }
        ui.add_space(4.0);
        if tab_button(ui, "General", tab == SettingsTab::General) {
            app.core.settings.tab = SettingsTab::General;
        }
    });
    ui.add_space(20.0);

    egui::ScrollArea::vertical().show(ui, |ui| {
        ui.horizontal(|ui| {
            ui.add_space(widgets::PAGE_INSET);
            ui.vertical(|ui| {
                ui.set_width((ui.available_width() - widgets::PAGE_INSET).max(200.0));
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
        .allocate_ui_with_layout(
            desired,
            egui::Layout::left_to_right(egui::Align::Center),
            |ui| {
                ui.add_space(H_PAD);
                ui.label(egui::RichText::new(label).color(text_color));
                ui.add_space(H_PAD);
            },
        )
        .response;

    let bg_rect = response.rect;
    let interact = ui
        .interact(
            bg_rect,
            response.id.with("settings_tab"),
            egui::Sense::click(),
        )
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
        crate::theme::radius::MD,
        fill,
        Stroke::new(1.0, colors::BORDER),
        egui::epaint::StrokeKind::Inside,
    );
    ui.painter().set(bg_idx, egui::Shape::Rect(shape));

    interact.clicked()
}

fn quarantine_ignore_tab(ui: &mut egui::Ui, app: &mut App) {
    let today = Timestamp::now();
    // 两列等宽：不用 ui.columns（它在首列前也吞一个 item_spacing，两列卡片
    // 左右边距不对称；且列宽会互相拉扯，一边内容多就把另一边折得很难看）。
    // 跟 dashboard::stat_cards 同款：horizontal_top + 逐列 allocate_ui_with_layout。
    // 宽度预算同样要把 egui 自动插入的 item_spacing.x 算进去，否则两列合起来
    // 比内容列宽出 10px，右缘被窗口裁掉（实测卡右缘 x=877，内容列右缘 x=867）。
    const GAP: f32 = 14.0;
    let avail = ui.available_width();
    let spacing_x = ui.spacing().item_spacing.x;
    let col_w = ((avail - GAP) / 2.0).max(160.0);
    let mut pending_action: Option<(usize, RowAction)> = None;
    let mut pending_remove: Option<usize> = None;
    ui.horizontal_top(|ui| {
        ui.allocate_ui_with_layout(
            Vec2::new(col_w, 0.0),
            egui::Layout::top_down(egui::Align::Min),
            |ui| {
                ui.set_min_width(col_w);
                quarantine_column(ui, app, &today, &mut pending_action);
            },
        );
        // 隐含的 item_spacing 已经在两列之间，这里只补足到 GAP。
        ui.add_space((GAP - spacing_x).max(0.0));
        ui.allocate_ui_with_layout(
            Vec2::new(col_w, 0.0),
            egui::Layout::top_down(egui::Align::Min),
            |ui| {
                ui.set_min_width(col_w);
                ignored_column(ui, app, &mut pending_remove);
            },
        );
    });

    // 动作统一在渲染结束后落地（渲染闭包里同时借用 app 的两个字段会打架）。
    let Some((i, action)) = pending_action else {
        return;
    };
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
    if let Some(i) = pending_remove {
        let entry = app.core.config.ignored[i].clone();
        app.core
            .config
            .remove_ignored(&entry.path, &entry.virus_name);
        app.toast("Removed from ignore list");
    }
}

#[derive(Clone, Copy)]
enum RowAction {
    None,
    Restore,
    Delete,
}

fn quarantine_column(
    ui: &mut egui::Ui,
    app: &mut App,
    today: &Timestamp,
    pending: &mut Option<(usize, RowAction)>,
) {
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

    for (i, entry) in app.core.config.quarantined.iter().enumerate() {
        let action = quarantine_row(ui, entry, today);
        if !matches!(action, RowAction::None) {
            *pending = Some((i, action));
        }
        ui.add_space(8.0);
    }
}

fn quarantine_row(ui: &mut egui::Ui, entry: &QuarantineEntry, today: &Timestamp) -> RowAction {
    let mut action = RowAction::None;
    egui::Frame::default()
        .fill(colors::BG_CARD)
        .stroke(Stroke::new(1.0, colors::BORDER))
        .corner_radius(crate::theme::radius::MD)
        .inner_margin(egui::Margin::symmetric(12, 10))
        .show(ui, |ui| {
            // 吃满列宽：不设的话卡会缩成内容宽，右列同样内容少的卡就窄一截，
            // 两列并排一看就不整齐（set_min_* 必须在内容之前调，见 design skill 坑 10）。
            ui.set_min_width(ui.available_width());
            // 路径按卡内像素宽截断（字符数版在窄列会溢出）。
            let text_w = (ui.available_width() - 8.0).max(40.0);
            ui.vertical(|ui| {
                ui.horizontal(|ui| {
                    widgets::bold_label(
                        ui,
                        &widgets::truncate_middle_to_px(ui, &entry.virus_name, 13.0, text_w),
                        13.0,
                        colors::TEXT_PRIMARY,
                    );
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.label(
                            egui::RichText::new(entry.quarantined_at.display_relative_to(today))
                                .color(colors::TEXT_MUTED)
                                .small(),
                        );
                    });
                });
                ui.label(
                    egui::RichText::new(widgets::truncate_middle_to_px(
                        ui,
                        &entry.original_path,
                        12.5,
                        text_w,
                    ))
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

fn ignored_column(ui: &mut egui::Ui, app: &mut App, pending: &mut Option<usize>) {
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

    for (i, entry) in app.core.config.ignored.iter().enumerate() {
        if ignored_row(ui, entry) {
            *pending = Some(i);
        }
        ui.add_space(8.0);
    }
}

fn ignored_row(ui: &mut egui::Ui, entry: &IgnoredEntry) -> bool {
    let mut clicked = false;
    egui::Frame::default()
        .fill(colors::BG_CARD)
        .stroke(Stroke::new(1.0, colors::BORDER))
        .corner_radius(crate::theme::radius::MD)
        .inner_margin(egui::Margin::symmetric(12, 10))
        .show(ui, |ui| {
            // 吃满列宽（同 quarantine_row 的说明）。
            ui.set_min_width(ui.available_width());
            // 路径按卡内像素宽截断（字符数版在窄列会溢出）。
            let text_w = (ui.available_width() - 8.0).max(40.0);
            // 结构跟 quarantine_row 一致：病毒名一行、路径一行、动作按钮一行。
            // 两列的行卡长得不一样（这边把按钮塞在右侧、那边在下面）会让整个
            // 设置页看着像两个页面拼的。
            ui.vertical(|ui| {
                widgets::bold_label(
                    ui,
                    &widgets::truncate_middle_to_px(ui, &entry.virus_name, 13.0, text_w),
                    13.0,
                    colors::TEXT_PRIMARY,
                );
                ui.label(
                    egui::RichText::new(widgets::truncate_middle_to_px(
                        ui,
                        &entry.path,
                        12.5,
                        text_w,
                    ))
                    .color(colors::TEXT_SECONDARY)
                    .small(),
                );
                ui.add_space(6.0);
                if widgets::pill_button(ui, "Remove", false) {
                    clicked = true;
                }
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
        .corner_radius(crate::theme::radius::MD)
        .inner_margin(egui::Margin::same(16))
        .show(ui, |ui| {
            // 吃满列宽（同 quarantine/ignored 行卡；原来的 CARD_WIDTH=560 固定宽
            // 在宽窗口下右侧留一大条空白，跟其它页满宽的卡片不搭）。
            ui.set_min_width(ui.available_width());
            auto_scan_rows(ui, app);
            ui.add_space(14.0);
            row_divider(ui);
            ui.add_space(14.0);
            autostart_row(ui, app);
            ui.add_space(14.0);
            row_divider(ui);
            ui.add_space(14.0);
            context_menu_row(ui, app);
        });
}

/// 自动扫描三件套：开机扫描 / 定时扫描（周内天 + 固定时刻）/ 病毒库更新后扫描。
fn auto_scan_rows(ui: &mut egui::Ui, app: &mut App) {
    let mut boot = app.core.config.auto_scan_on_boot;
    if widgets::toggle_setting_row(
        ui,
        &mut boot,
        "Scan at startup",
        "Runs a quick scan when CLV3000 starts",
        true,
    ) {
        app.core.config.auto_scan_on_boot = boot;
        app.core.config.save();
    }

    ui.add_space(14.0);
    row_divider(ui);
    ui.add_space(14.0);

    let mut schedule_on = app.core.config.scheduled_scan.is_some();
    if widgets::toggle_setting_row(
        ui,
        &mut schedule_on,
        "Scheduled scan",
        "Runs automatically on the days and time you pick",
        true,
    ) {
        app.core.config.scheduled_scan = if schedule_on {
            Some(app.core.config.scheduled_scan.unwrap_or_default())
        } else {
            None
        };
        app.core.config.save();
    }
    // 开关打开时展开计划编辑器（星期几 / 时刻 / 扫哪种）——设置项的细节就地展开，
    // 不用再开一个子页面。
    if let Some(mut sched) = app.core.config.scheduled_scan
        && schedule_editor(ui, &mut sched)
    {
        app.core.config.scheduled_scan = Some(sched);
        app.core.config.save();
    }

    ui.add_space(14.0);
    row_divider(ui);
    ui.add_space(14.0);

    let mut after_update = app.core.config.auto_scan_after_update;
    if widgets::toggle_setting_row(
        ui,
        &mut after_update,
        "Scan after database update",
        "Runs a quick scan when new virus definitions are installed",
        true,
    ) {
        app.core.config.auto_scan_after_update = after_update;
        app.core.config.save();
    }
}

/// 星期缩写 + 全称（无障碍名）。可见文字用单字母是紧凑排布的老做法，但两个
/// "T"、两个 "S" 会撞名，所以无障碍名用全称。
const WEEKDAYS: [(&str, &str); 7] = [
    ("M", "Monday"),
    ("T", "Tuesday"),
    ("W", "Wednesday"),
    ("T", "Thursday"),
    ("F", "Friday"),
    ("S", "Saturday"),
    ("S", "Sunday"),
];

/// 定时扫描的计划编辑器：星期几多选 + 时刻 + 扫描类型。返回是否有改动。
fn schedule_editor(ui: &mut egui::Ui, sched: &mut crate::config::ScheduleConfig) -> bool {
    let mut changed = false;
    egui::Frame::default()
        .fill(colors::BG_APP)
        .stroke(Stroke::new(1.0, colors::BORDER))
        .corner_radius(crate::theme::radius::SM)
        .inner_margin(egui::Margin::symmetric(14, 12))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(
                    egui::RichText::new("Runs on")
                        .color(colors::TEXT_SECONDARY)
                        .small(),
                );
                for (i, (short, name)) in WEEKDAYS.iter().enumerate() {
                    if widgets::toggle_chip(ui, short, name, sched.includes_day(i as u8)) {
                        sched.days_mask ^= 1 << i;
                        changed = true;
                    }
                }
            });
            ui.add_space(10.0);
            ui.horizontal(|ui| {
                ui.label(
                    egui::RichText::new("At")
                        .color(colors::TEXT_SECONDARY)
                        .small(),
                );
                // ── 让两个时间下拉框垂直对齐 ─────────────────────────────
                //
                // 现象：小时框比分钟框高 7px（肉眼就是"没对齐"）。
                //
                // 根因：egui 的 ComboBox 不是"按内容定高"的控件——它取
                // 「所在行当前可用高度」与 `interact_size.y` 的**较大者**作为
                // 自身高度（egui-0.36 `containers/combo_box.rs` 的 `button_frame`：
                // `outer_rect.set_height(outer_rect.height().at_least(interact_size.y))`）。
                // 本主题把 `button_padding` 调成了 (14, 8)，但没动 `interact_size`，
                // 它还是 egui 默认的 18px；而下拉框的实际内容高
                // （按钮文字行高 + 2×8px 内边距）约 32px。于是同一行里两个
                // 下拉框按不同的基准参与行的交叉轴对齐，越靠后的偏得越多
                // （实测两框中心相差 7px）。
                //
                // 修法：把这一行的基准高度 `interact_size.y` 提到下拉框的实际
                // 内容高度，"行基准"与"控件高"一致，两个框的垂直中心就对齐了
                // （实测偏差 0）。这个 `Ui` 里只有两个下拉框是交互控件，
                // 标签/chip 不受影响。
                let combo_h = ui
                    .text_style_height(&egui::TextStyle::Button)
                    .max(ui.spacing().icon_width)
                    + 2.0 * ui.spacing().button_padding.y;
                if ui.spacing().interact_size.y < combo_h {
                    ui.spacing_mut().interact_size.y = combo_h;
                }
                egui::ComboBox::from_id_salt("schedule_hour")
                    .selected_text(format!("{:02}", sched.hour))
                    .width(52.0)
                    .show_ui(ui, |ui| {
                        for h in 0..24u8 {
                            ui.selectable_value(&mut sched.hour, h, format!("{h:02}"));
                        }
                    });
                ui.label(egui::RichText::new(":").color(colors::TEXT_SECONDARY));
                egui::ComboBox::from_id_salt("schedule_minute")
                    .selected_text(format!("{:02}", sched.minute))
                    .width(52.0)
                    .show_ui(ui, |ui| {
                        for m in (0..60u8).step_by(5) {
                            ui.selectable_value(&mut sched.minute, m, format!("{m:02}"));
                        }
                    });
                ui.add_space(20.0);
                ui.label(
                    egui::RichText::new("Scan")
                        .color(colors::TEXT_SECONDARY)
                        .small(),
                );
                ui.add_space(4.0);
                if widgets::toggle_chip(ui, "Quick", "Quick scan", !sched.scan_full) {
                    sched.scan_full = false;
                    changed = true;
                }
                ui.add_space(6.0);
                if widgets::toggle_chip(ui, "Full", "Full scan", sched.scan_full) {
                    sched.scan_full = true;
                    changed = true;
                }
            });
        });
    changed
}

/// 行间细分割线：跟卡片描边同色（`BORDER`），比 egui 默认 `ui.separator()` 更淡，
/// 只在这一张卡片内部区分"两条设置"，不是 `Panel` 级的硬分割线（design skill
/// 坑 4 说的是面板边缘那种，这里是同一张卡片里主动想要的行间分隔，两者不矛盾）。
fn row_divider(ui: &mut egui::Ui) {
    widgets::row_divider(ui);
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
    widgets::toggle_setting_row(ui, checked, title, description, enabled)
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
        "Launches minimized to the tray when system log in (starting with --tray-only).",
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

#[cfg(test)]
mod tests {
    //! egui_kittest 驱动**真实**组件的 UI 测试（见 AGENTS.md「UI 代码验证流程」）。

    use super::schedule_editor;
    use crate::config::ScheduleConfig;
    use egui_kittest::Harness;
    use egui_kittest::kittest::Queryable;

    /// 计划编辑器里"小时 / 分钟"两个下拉框必须上下对齐。
    ///
    /// 2026-10 用户反馈"时间选择区域的小时和分钟没有对齐"（实测两个下拉框的
    /// 垂直中心相差 7px）。
    ///
    /// 根因不是"两个文案宽度不同"（那只影响宽度，不影响高度），而是 egui 的
    /// ComboBox 不是"按内容定高"的控件：它取「所在行当前可用高度」与
    /// `interact_size.y` 的**较大者**作为自身高度（egui-0.36
    /// `containers/combo_box.rs` 的 `button_frame`）。本主题把 `button_padding`
    /// 调成 (14, 8) 但没动 `interact_size`，它还是 egui 默认的 18px，而下拉框
    /// 的实际内容高约 32px——同一行里两个下拉框按不同基准参与行的交叉轴
    /// 对齐，越靠后偏得越多。
    ///
    /// 修法见 `schedule_editor`：把该行的 `interact_size.y` 提到下拉框的实际
    /// 内容高度。
    ///
    /// 判据：两个下拉框的**垂直中心**必须一致（允许 1px 的取整误差）。
    #[test]
    fn schedule_hour_minute_dropdowns_are_aligned() {
        let mut harness = Harness::builder()
            .with_size(egui::Vec2::new(900.0, 600.0))
            .with_max_steps(10)
            .build_ui_state(
                |ui, sched: &mut ScheduleConfig| {
                    schedule_editor(ui, sched);
                },
                ScheduleConfig {
                    days_mask: 0b0100_1010,
                    hour: 9,
                    minute: 30,
                    scan_full: false,
                },
            );
        crate::theme::apply(&harness.ctx);
        harness.run();

        // ComboBox 在 accesskit 树里是 `Role::ComboBox`，值就是选中的文本。
        let hour = harness.get_by_value("09").rect();
        let minute = harness.get_by_value("30").rect();

        let hour_center = hour.center().y;
        let minute_center = minute.center().y;
        assert!(
            (hour_center - minute_center).abs() <= 1.0,
            "小时/分钟下拉框未上下对齐：小时中心 y={hour_center:.1}，分钟中心 y={minute_center:.1}，\
             相差 {:.1}px（小时 {:?}，分钟 {:?}）",
            (hour_center - minute_center).abs(),
            hour,
            minute
        );
    }
}
