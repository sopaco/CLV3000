//! 可复用 UI 组件：圆环进度、状态胶囊、威胁卡片、Toast 提示。

use crate::theme::colors;
use egui::{
    Align2, Color32, FontId, Pos2, Response, RichText, Sense, Stroke, Ui, Vec2, epaint::PathStroke,
};

/// 页面内容统一左侧/顶部留白：页头、卡片行、设置页列表都用这个对齐，
/// 不然各页各自为政会显得错位。
pub const PAGE_INSET: f32 = 24.0;

/// 底部资源条高度。Toast 浮层要避开它（见 `show_toasts`），单独抽成常量
/// 免得两处各写一个 50 之后悄悄漂移。
pub const RESOURCE_BAR_HEIGHT: f32 = 50.0;

/// 页面统一页头：38px 图标砖（品牌蓝底纹）+ 20px 标题 + 12.5px 副标题，右侧
/// 留一个动作位（比如扫描中放 Cancel）。五个 tab 页共用这套页头——2026-10
/// 改版前每页都是一坨居中内容、五个页长得一样，页头给了各页统一的锚点，
/// 页内才能去做差异化布局。
///
/// `icon` 接 `fn(&Painter, Rect, Stroke)` 签名；`icons::bolt` 是 fill 签名，
/// 调用方包一层 `|p, r, s| icons::bolt(p, r, s.color)`（无捕获闭包可转 fn 指针）。
pub fn page_header(
    ui: &mut Ui,
    icon: fn(&egui::Painter, egui::Rect, Stroke),
    title: &str,
    subtitle: &str,
    right_side: impl FnOnce(&mut Ui),
) {
    ui.horizontal(|ui| {
        ui.add_space(PAGE_INSET);
        let (tile_resp, painter) = ui.allocate_painter(Vec2::splat(38.0), Sense::hover());
        painter.rect_filled(
            tile_resp.rect,
            crate::theme::radius::MD,
            colors::ACCENT_BLUE_BG,
        );
        (icon)(
            &painter,
            tile_resp.rect.shrink(10.0),
            Stroke::new(1.7, colors::ACCENT_BLUE),
        );
        ui.add_space(12.0);
        ui.vertical(|ui| {
            bold_label(ui, title, 20.0, colors::TEXT_PRIMARY);
            ui.label(
                egui::RichText::new(subtitle)
                    .color(colors::TEXT_SECONDARY)
                    .small(),
            );
        });
        // 右侧动作位：right_to_left 布局里先 add_space 占掉右缘留白，内容再往左排。
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.add_space(PAGE_INSET);
            right_side(ui);
        });
    });
}

/// 主操作按钮：品牌蓝实底 + 白字（"Start Quick Scan"/"Update Database" 这类
/// 一页唯一的推进动作）。与 `action_button`（卡片底+描边）构成主次层级。
/// 与 `pill_button`/`ghost_button` 同一套"量宽 + 占位 Shape 回填"实现，
/// widget_info + 手型光标齐全（AGENTS.md 规则 3）。
pub fn primary_button(
    ui: &mut Ui,
    label: &str,
    draw: impl FnOnce(&egui::Painter, egui::Rect, Stroke),
) -> bool {
    primary_button_named(ui, label, label, draw)
}

/// 同 `primary_button`，但可以给一个与可见文字不同的无障碍名。
///
/// 仪表盘的 "Quick Scan" 按钮跟侧栏的 "Quick Scan" 导航项同名——accesskit 树里
/// 两个节点同名，kittest `get_by_label` 撞多匹配会 panic。可���名用
/// "Start Quick Scan"（可见文字仍包含在内，满足 WCAG 2.5.3 Label in Name）。
pub fn primary_button_named(
    ui: &mut Ui,
    label: &str,
    accessible_label: &str,
    draw: impl FnOnce(&egui::Painter, egui::Rect, Stroke),
) -> bool {
    const ICON_SIZE: f32 = ACTION_BTN_ICON_SIZE;
    const ICON_GAP: f32 = ACTION_BTN_ICON_GAP;
    // 比次操作按钮稍宽一档：实底按钮的分量要配得上它是主动作。
    const H_PAD: f32 = ACTION_BTN_H_PAD + 4.0;
    const V_PAD: f32 = ACTION_BTN_V_PAD;

    let text_color = Color32::WHITE;
    let font_id = egui::FontId::proportional(14.0);
    let galley = ui
        .ctx()
        .fonts_mut(|f| f.layout_no_wrap(label.to_owned(), font_id, text_color));
    let text_size = galley.size();
    let desired = Vec2::new(
        H_PAD * 2.0 + ICON_SIZE + ICON_GAP + text_size.x,
        V_PAD * 2.0 + ICON_SIZE.max(text_size.y),
    );

    // 与 ghost_button 同构：一次性分配 + 占位背景，图标/文字手动居中直画。
    // 文字不能用 ui.label（会另生成一个同 label 的 accesskit 文本节点，
    // kittest get_by_label 多匹配 panic）。
    let bg_idx = ui.painter().add(egui::Shape::Noop);
    let (bg_rect, response) = ui.allocate_exact_size(desired, Sense::click());
    let icon_rect = egui::Rect::from_min_size(
        egui::pos2(bg_rect.left() + H_PAD, bg_rect.center().y - ICON_SIZE / 2.0),
        Vec2::splat(ICON_SIZE),
    );
    draw(ui.painter(), icon_rect, Stroke::new(1.6, Color32::WHITE));
    let text_pos = egui::pos2(
        icon_rect.right() + ICON_GAP,
        bg_rect.center().y - text_size.y / 2.0,
    );
    ui.painter()
        .add(egui::epaint::TextShape::new(text_pos, galley, text_color));
    let interact = response.on_hover_cursor(egui::CursorIcon::PointingHand);
    interact.widget_info(|| {
        egui::WidgetInfo::labeled(
            egui::WidgetType::Button,
            true,
            accessible_label.to_owned(),
        )
    });
    let fill = if interact.hovered() {
        colors::ACCENT_ACTION_HOVER
    } else {
        colors::ACCENT_ACTION
    };
    let shape = egui::epaint::RectShape::new(
        bg_rect,
        crate::theme::radius::MD,
        fill,
        Stroke::NONE,
        egui::epaint::StrokeKind::Inside,
    );
    ui.painter().set(bg_idx, egui::Shape::Rect(shape));

    interact.clicked()
}

/// 量出 `primary_button` **本体**占多宽（不含外侧 item_spacing——那个由调用
/// 方的布局决定，按钮本体是 `allocate_exact_size` 出来的，不带隐含间距）。
///
/// 别在"居中摆放单颗按钮"时额外加 item_spacing：按钮本体就是 `desired` 宽，
/// 多算 20px 会让居中偏移量偏大 10px（实测扫描页的 Start 按钮左偏 10px）。
pub fn primary_button_body_width(ui: &Ui, label: &str) -> f32 {
    (ACTION_BTN_H_PAD + 4.0) * 2.0
        + ACTION_BTN_ICON_SIZE
        + ACTION_BTN_ICON_GAP
        + measure_text_width(ui, label, 14.0)
}

/// 量出 `action_button` **本体**占多宽（同 `primary_button_body_width`：
/// 不含外侧 item_spacing）。
pub fn action_button_width(ui: &Ui, label: &str) -> f32 {
    ACTION_BTN_H_PAD * 2.0 + ACTION_BTN_ICON_SIZE + ACTION_BTN_ICON_GAP
        + measure_text_width(ui, label, 14.0)
}

/// `primary_button` 本体占多高（纵向居中预算用）。
pub fn primary_button_body_height(ui: &Ui, label: &str) -> f32 {
    ACTION_BTN_V_PAD * 2.0 + ACTION_BTN_ICON_SIZE.max(measure_text_height(ui, label, 14.0))
}

/// `action_button` 本体占多高（纵向居中预算用）。
pub fn action_button_height(ui: &Ui, label: &str) -> f32 {
    ACTION_BTN_V_PAD * 2.0 + ACTION_BTN_ICON_SIZE.max(measure_text_height(ui, label, 14.0))
}

/// 小圆角标签（可点选的 chip）：用于"周内天"多选、"扫描类型"单选这类紧凑选择器。
///
/// 自绘而不用 egui 原生控件：原生 `selectable_label` 的选中态是整块高亮色块，
/// 在设置页里显得太重；这里选中只是品牌蓝底 + 蓝描边，未选中是透明底 + 灰描边。
/// `accessible` 与可见文字分开——"M/T/W/T/F/S/S" 里两个 T、两个 S 重名会撞
/// accesskit 树（kittest `get_by_label` 多匹配 panic）。
pub fn toggle_chip(ui: &mut Ui, label: &str, accessible: &str, selected: bool) -> bool {
    const HEIGHT: f32 = 28.0;
    const H_PAD: f32 = 10.0;
    const RADIUS: f32 = crate::theme::radius::SM_PX;

    let font_id = FontId::proportional(13.0);
    let galley = ui
        .ctx()
        .fonts_mut(|f| f.layout_no_wrap(label.to_owned(), font_id, Color32::WHITE));
    let text_size = galley.size();
    let desired = Vec2::new(text_size.x + H_PAD * 2.0, HEIGHT);

    let bg_idx = ui.painter().add(egui::Shape::Noop);
    let (rect, response) = ui.allocate_exact_size(desired, Sense::click());
    let interact = response.on_hover_cursor(egui::CursorIcon::PointingHand);
    interact.widget_info(|| {
        egui::WidgetInfo::labeled(egui::WidgetType::Button, selected, accessible.to_owned())
    });
    let (fill, stroke, text_color) = if selected {
        (
            colors::ACCENT_BLUE_BG,
            Stroke::new(1.0, colors::ACCENT_BLUE),
            colors::TEXT_PRIMARY,
        )
    } else {
        (
            Color32::TRANSPARENT,
            Stroke::new(1.0, colors::BORDER),
            colors::TEXT_SECONDARY,
        )
    };
    let shape = egui::epaint::RectShape::new(
        rect,
        RADIUS,
        fill,
        stroke,
        egui::epaint::StrokeKind::Inside,
    );
    ui.painter().set(bg_idx, egui::Shape::Rect(shape));
    let text_pos = rect.center() - text_size / 2.0;
    ui.painter()
        .add(egui::epaint::TextShape::new(text_pos, galley, text_color));

    interact.clicked()
}

/// 仪表盘信息卡：小图标 + 标签 + 主值 + 说明行，整卡可点（跳对应页面）。
/// hover 提示靠描边提亮（BORDER → BORDER_STRONG）+ 手型光标，不改底色。
/// 颜色由调用方给：空值用 TEXT_MUTED，扫出威胁的说明行用 RED。
///
/// **整张卡自绘**（不用 egui 的布局流）：卡里三行文字要严格等距、上下留白
/// 必须对称，而嵌套布局算不准——横向行在竖向容器里会被 cross-align 拉满可用
/// 高度（design skill 坑 19），第一行被推到中间、后面的行挤出卡片被裁掉
/// （实测：卡里只剩底色，文字全没）。自己算行高 + 自己定位，就不会再有
/// "上 39px / 下 15px"这种不对称留白。
///
/// 内边距 20（2026-10 用户反馈：16 太贴边，"QUICK SCAN" 那行几乎贴着左边框）。
#[allow(clippy::too_many_arguments)]
pub fn stat_card(
    ui: &mut Ui,
    width: f32,
    icon: fn(&egui::Painter, egui::Rect, Stroke),
    label: &str,
    value: &str,
    detail: &str,
    value_color: Color32,
    detail_color: Color32,
) -> bool {
    const PAD: f32 = 20.0;
    const ICON: f32 = 16.0;
    const ICON_GAP: f32 = 8.0;
    const ROW_GAP: f32 = 10.0; // 标签行 → 主值
    const VALUE_GAP: f32 = 6.0; // 主值 → 说明行

    // 文字预算 = 卡宽 − 左右内边距。
    let text_w = (width - PAD * 2.0).max(40.0);
    let value_text = truncate_middle_to_px(ui, value, 16.0, text_w);
    let detail_text = truncate_middle_to_px(ui, detail, 12.5, text_w);

    // 行高用与渲染一致的字号量（`measure_text_height` 与 galley 同源）。
    let label_h = measure_text_height(ui, label, 12.5);
    let value_h = measure_text_height(ui, &value_text, 16.0);
    let detail_h = measure_text_height(ui, &detail_text, 12.5);
    let row_h = ICON.max(label_h);
    let card_h = PAD + row_h + ROW_GAP + value_h + VALUE_GAP + detail_h + PAD;

    // 卡皮先占位后回填，保证它排在文字**下面**（直接 add 形状会盖住文字）。
    let bg_idx = ui.painter().add(egui::Shape::Noop);
    let (card_rect, response) =
        ui.allocate_exact_size(Vec2::new(width, card_h), Sense::click());
    let painter = ui.painter();

    let x = card_rect.left() + PAD;
    let mut y = card_rect.top() + PAD;

    // 标签行：图标 + 标签文字（同一行，垂直居中对齐）
    let icon_rect = egui::Rect::from_min_size(
        egui::pos2(x, y + (row_h - ICON) / 2.0),
        Vec2::splat(ICON),
    );
    (icon)(painter, icon_rect, Stroke::new(1.4, colors::TEXT_SECONDARY));
    painter.text(
        egui::pos2(x + ICON + ICON_GAP, y + row_h / 2.0),
        egui::Align2::LEFT_CENTER,
        label,
        egui::FontId::proportional(12.5),
        colors::TEXT_SECONDARY,
    );
    y += row_h + ROW_GAP;

    // 主值：伪粗体（同 `bold_label` 的做法——同一段文字横向错开 0.6px 描两遍）
    let galley = ui.ctx().fonts_mut(|f| {
        f.layout_no_wrap(
            value_text,
            egui::FontId::proportional(16.0),
            value_color,
        )
    });
    painter.galley(egui::pos2(x, y), galley.clone(), value_color);
    painter.galley(egui::pos2(x + 0.6, y), galley, value_color);
    y += value_h + VALUE_GAP;

    // 说明行
    painter.text(
        egui::pos2(x, y),
        egui::Align2::LEFT_TOP,
        detail_text,
        egui::FontId::proportional(12.5),
        detail_color,
    );

    // 无障碍名不能直接用 label（"Quick Scan" 会跟侧栏同名项撞名，kittest 查询
    // 遇多匹配会 panic）——统一用 "Open …" 句式，也和点击行为对得上。
    let accessible = format!("Open {label}");
    let interact = response.on_hover_cursor(egui::CursorIcon::PointingHand);
    interact.widget_info(|| {
        egui::WidgetInfo::labeled(egui::WidgetType::Button, true, accessible.clone())
    });
    let stroke = if interact.hovered() {
        Stroke::new(1.0, colors::BORDER_STRONG)
    } else {
        Stroke::new(1.0, colors::BORDER)
    };
    let shape = egui::epaint::RectShape::new(
        card_rect,
        crate::theme::radius::MD,
        colors::BG_CARD,
        stroke,
        egui::epaint::StrokeKind::Inside,
    );
    ui.painter().set(bg_idx, egui::Shape::Rect(shape));

    interact.clicked()
}

/// 自绘复选框：18px 圆角方框，勾选时品牌蓝实底 + 白色对勾。
///
/// 不用 egui 内置的 `ui.checkbox`：它的方框底色取 `visuals.bg_fill`、描边取
/// `visuals.bg_stroke`，而本主题把 `widgets.noninteractive.bg_fill` 与
/// `inactive.bg_fill` 都设成了 BG_CARD——放在同样 BG_CARD 底色的设置卡片上，
/// 方框与卡片同色同描边，画面上只剩一个悬空的"✓"（2026-10 预览实测：
/// 3× 放大截图里只有一个小小的对勾，框线完全看不见）。
///
/// `enabled=false` 时画灰态且不可点（当前平台不支持的功能用这个形态）。
/// 返回是否被用户切换（切换后 `checked` 已经是新值）。
pub fn toggle_checkbox(ui: &mut Ui, checked: &mut bool, enabled: bool, label: &str) -> bool {
    const BOX: f32 = 18.0;
    // 圆角走 SM 刻度（3px）：18px 的方框用 4px 已经有 22% 的圆，比"干练"
    // 的整体基调差一档。
    const RADIUS: f32 = crate::theme::radius::SM_PX;

    let (rect, response) = ui.allocate_exact_size(
        Vec2::splat(BOX),
        if enabled { Sense::click() } else { Sense::hover() },
    );
    let interact = if enabled {
        response.on_hover_cursor(egui::CursorIcon::PointingHand)
    } else {
        response
    };
    // 自绘控件不会自动进 accesskit 树（egui 只为标准控件调 widget_info）——补上
    // 无障碍信息：屏幕阅读器读得出"已勾选/未勾选"，egui_kittest 也能按 label 查到
    // （见 AGENTS.md「UI 代码验证流程」）。
    interact.widget_info(|| {
        egui::WidgetInfo::labeled(egui::WidgetType::Checkbox, *checked, label.to_owned())
    });

    let painter = ui.painter();
    if *checked {
        let fill = if enabled {
            colors::ACCENT_ACTION
        } else {
            colors::BORDER
        };
        painter.rect_filled(rect, RADIUS, fill);
        // 对勾：右下短线 + 左上长线的折线。
        let o = rect.min;
        let stroke = Stroke::new(
            1.8,
            if enabled {
                Color32::WHITE
            } else {
                colors::TEXT_MUTED
            },
        );
        painter.add(egui::Shape::line(
            vec![
                o + Vec2::new(4.2, 9.0),
                o + Vec2::new(7.4, 12.2),
                o + Vec2::new(13.4, 5.6),
            ],
            PathStroke::from(stroke),
        ));
    } else {
        let stroke = if enabled {
            if interact.hovered() {
                colors::ACCENT_BLUE
            } else {
                colors::BORDER_STRONG
            }
        } else {
            colors::BORDER
        };
        painter.rect_stroke(
            rect,
            RADIUS,
            Stroke::new(1.0, stroke),
            egui::epaint::StrokeKind::Inside,
        );
    }

    if enabled && interact.clicked() {
        *checked = !*checked;
        return true;
    }
    false
}

/// 一行设置项：checkbox + 标题 + 说明文字。`enabled=false` 时整行变灰且 checkbox
/// 不可交互，但布局跟正常行完全一致，不会因为"不可用"而错位。
///
/// 共享给设置页与病毒库页（自动更新开关）——两处用同一个组件，避免"设置页
/// 的开关长这样、别的页面的开关长那样"。
pub fn toggle_setting_row(
    ui: &mut Ui,
    checked: &mut bool,
    title: &str,
    description: &str,
    enabled: bool,
) -> bool {
    let mut changed = false;
    let title_color = if enabled {
        colors::TEXT_PRIMARY
    } else {
        colors::TEXT_MUTED
    };
    let desc_color = if enabled {
        colors::TEXT_SECONDARY
    } else {
        colors::TEXT_MUTED
    };
    // 复选框与文字块之间的间距。横向布局会在两个子控件之间自动插入
    // item_spacing.x（主题里 10），这里只补足到 GAP。
    const GAP: f32 = 12.0;
    let spacing_x = ui.spacing().item_spacing.x;
    // horizontal_top（cross-align = Min）而不是 horizontal：后者把复选框按整个
    // 两行文字块垂直居中，勾选框会掉到标题下面一行的位置，看着像错行。
    ui.horizontal_top(|ui| {
        if toggle_checkbox(ui, checked, enabled, title) {
            changed = true;
        }
        ui.add_space((GAP - spacing_x).max(0.0));
        ui.vertical(|ui| {
            bold_label(ui, title, 14.0, title_color);
            ui.add_space(2.0);
            ui.label(egui::RichText::new(description).color(desc_color).small());
        });
    });
    changed
}

/// 卡片内部的行间细分割线：跟卡片描边同色（`BORDER`），比 egui 默认
/// `ui.separator()` 更淡，只在卡片内部区分"两组内容"。
pub fn row_divider(ui: &mut Ui) {
    let (rect, _) =
        ui.allocate_exact_size(Vec2::new(ui.available_width(), 1.0), Sense::hover());
    ui.painter().hline(
        rect.x_range(),
        rect.center().y,
        Stroke::new(1.0, colors::BORDER),
    );
}

/// 覆盖清单的一行：说明文字 + 图标绘制函数（`icons::*` 的签名）。
pub type CoverageRow = (&'static str, fn(&egui::Painter, egui::Rect, Stroke));

/// 扫描覆盖清单卡：每行一个图标 + 一行说明，回答"这次扫描到底扫什么"。
///
/// 这块内容原来被压成卡片里的灰色小字（12.5px），是整页里信息量最大的部分
/// 却最不显眼；现在它是扫描页空闲态的视觉重心——图标 20px 品牌蓝描边 +
/// 14px 正文色文字，三行等距。
///
/// 卡片宽度由调用方在**外层 cell** 上定（限宽居中卡的标准做法，见 design
/// skill 坑 12），本函数只负责卡片皮与行内容。
pub fn coverage_card(ui: &mut Ui, rows: &[CoverageRow]) {
    egui::Frame::default()
        .fill(colors::BG_CARD)
        .stroke(Stroke::new(1.0, colors::BORDER))
        .corner_radius(crate::theme::radius::LG)
        .inner_margin(egui::Margin::symmetric(20, 16))
        .show(ui, |ui| {
            // 吃满外层 cell 宽：Frame 默认只包内容那么大，不撞满的话卡片会
            // 收缩成内容宽（外层是按固定宽居中的，卡片宽度必须跟着走）。
            ui.set_min_width(ui.available_width());
            for (i, (text, icon)) in rows.iter().enumerate() {
                if i > 0 {
                    ui.add_space(12.0);
                }
                // 单行内容用 horizontal（cross-align Center）：图标 20px 与
                // 文字行高不等高时要垂直居中，而不是像 horizontal_top 那样
                // 顶对齐（design skill 坑 11 的反向场景）。
                ui.horizontal(|ui| {
                    let (icon_resp, painter) =
                        ui.allocate_painter(Vec2::splat(20.0), Sense::hover());
                    (icon)(&painter, icon_resp.rect, Stroke::new(1.6, colors::ACCENT_BLUE));
                    ui.add_space(14.0);
                    ui.label(RichText::new(*text).color(colors::TEXT_PRIMARY));
                });
            }
        });
}

/// 把一块内容在当前可用高度里整体垂直居中（水平居中沿用 `vertical_centered`）。
///
/// immediate-mode GUI 没法在画之前就知道内容有多高——`egui::Layout::top_down` 的
/// `main_align` 字段名字很像"能让整块内容居中"，但读了 epaint/egui 的
/// `next_frame_ignore_wrap` 源码就知道：TopDown 布局下主轴（纵向）永远是
/// `Align::TOP`，`main_align` 实际只影响横向布局里"一行"内部的纵向对齐，对
/// "纵向堆一串控件、把整串居中"这件事完全不起作用——这里最早想当然地用
/// `allocate_ui_with_layout(avail, Layout::top_down(Align::Center), ...)` 想让它
/// 居中，实测发现内容其实还是贴着顶部画，视觉上看起来"变居中了"只是因为顺手删掉
/// 了原来那个硬编码的顶部 `add_space`，纯属偶然对上、没有真的居中。
///
/// 真正的做法：用上一帧实际测量到的内容高度，算出这一帧该留多少顶部空白
/// （`(可用高度 - 上一帧高度) / 2`），画完之后再把这一帧的真实高度存回去。
/// 内容高度不变时（多数时候）就是精确居中；内容一变（比如扫描状态切换、威胁
/// 列表变长变短）会有一帧的滞后再收敛，本来就每 250ms 重绘一次，肉眼看不出来。
pub fn vertically_centered<R>(
    ui: &mut Ui,
    last_height: &mut f32,
    add_contents: impl FnOnce(&mut Ui) -> R,
) -> R {
    let top_pad = ((ui.available_height() - *last_height) * 0.5).max(0.0);
    ui.add_space(top_pad);
    let inner = ui.vertical_centered(add_contents);
    *last_height = inner.response.rect.height();
    inner.inner
}

/// "伪粗体"文字：`.strong()` 在 egui 里其实只是换个颜色，并不会真的变粗（字体本身
/// 只有一个字重，没有 bold 变体可选）。这里手动把同一段文字描两遍、横向错开不到
/// 1px，模拟笔画加粗的效果——不依赖额外的粗体字体文件，跨 Windows/macOS 都一样。
pub fn bold_label(ui: &mut Ui, text: &str, size: f32, color: Color32) -> Response {
    bold_label_nudged(ui, text, size, color, Vec2::ZERO)
}

/// 跟 `bold_label` 一样，但多一个手动微调的偏移量——用在"图标 + 文字"并排、
/// 需要让文字的墨迹跟图标视觉对齐的地方。egui 用文字的行高包围盒去做自动居中，
/// 中文字体的行高内部本身不是墨迹对称的，交给自动布局会看起来比图标偏高/偏右，
/// 这里直接手动纠正最终落墨位置，不去跟布局系统的包围盒计算较劲。
pub fn bold_label_nudged(
    ui: &mut Ui,
    text: &str,
    size: f32,
    color: Color32,
    nudge: Vec2,
) -> Response {
    const OFFSET: f32 = 0.6;
    let font_id = FontId::proportional(size);
    let galley = ui
        .ctx()
        .fonts_mut(|f| f.layout_no_wrap(text.to_owned(), font_id, color));
    let desired = galley.size() + Vec2::new(OFFSET, 0.0);
    let (rect, response) = ui.allocate_exact_size(desired, Sense::hover());
    let painter = ui.painter();
    let pos = rect.min + nudge;
    painter.galley(pos, galley.clone(), color);
    painter.galley(pos + Vec2::new(OFFSET, 0.0), galley, color);
    response
}

/// 在圆心周围画一层柔和的"光晕"：从外到内几层半透明同色圆叠加，模拟发光/辉光效果。
/// egui 没有现成的高斯模糊，这是最简单的近似——层数越多越顺滑，但也越费一点绘制开销，
/// 这里几层就够用，调用者要记得留够画布空间（半径 + 光晕扩散量），别被裁掉。
pub fn paint_glow(painter: &egui::Painter, center: Pos2, radius: f32, color: Color32) {
    const LAYERS: i32 = 6;
    const SPREAD: f32 = 0.6; // 光晕最外层比本体半径多出的比例
    // 最内层（最浓）的透明度：故意压低避免糊成一片，但要肉眼可辨——
    // 旧值 16 实测几乎看不见，等于白画（2026-10 评审）。
    const MAX_ALPHA: u8 = 24;

    for i in (0..LAYERS).rev() {
        let t = i as f32 / LAYERS as f32; // 0 = 贴着本体，1 = 光晕最外圈
        let r = radius + t * radius * SPREAD;
        let alpha = (MAX_ALPHA as f32 * (1.0 - t)).round() as u8;
        if alpha == 0 {
            continue;
        }
        let glow_color = Color32::from_rgba_unmultiplied(color.r(), color.g(), color.b(), alpha);
        painter.circle_filled(center, r, glow_color);
    }
}

/// 圆环进度指示器。`percent` 为 `None` 时画一段跟随时间旋转的不定长弧（表示"未知总量，
/// 正在进行中"），否则画一段从正上方顺时针延伸的确定进度弧。
pub fn progress_ring(
    ui: &mut Ui,
    diameter: f32,
    percent: Option<f32>,
    ring_color: Color32,
    center_title: &str,
    center_sub: &str,
) -> Response {
    let ring_width = (diameter * 0.045).max(4.0);
    // Middle 描边会向内外各扩 half width；allocate_painter 的 clip 正好卡在
    // diameter 边界上，所以半径要按线宽留边，否则轨道左右会被裁掉。
    let radius = diameter / 2.0 - ring_width / 2.0 - 1.0;
    let (response, painter) = ui.allocate_painter(Vec2::splat(diameter), Sense::hover());
    let rect = response.rect;
    let center = rect.center();

    // 背景轨道与前景弧必须用同一套 draw_arc 绘制：egui 的 circle_stroke 在
    // 内部走 StrokeKind::Outside，而 PathStroke 默认是 Middle，两者半径语义
    // 不同，会出现"灰圈和蓝圈对不上"以及灰圈外缘被 clip 的问题。
    let track_color = Color32::from_rgba_unmultiplied(255, 255, 255, 18);
    draw_arc(
        &painter,
        center,
        radius,
        ring_width,
        -std::f32::consts::FRAC_PI_2,
        std::f32::consts::TAU,
        track_color,
    );

    let start_angle = -std::f32::consts::FRAC_PI_2;
    match percent {
        Some(p) => {
            let sweep = p.clamp(0.0, 1.0) * std::f32::consts::TAU;
            draw_arc(
                &painter,
                center,
                radius,
                ring_width,
                start_angle,
                sweep,
                ring_color,
            );
        }
        None => {
            let t = ui.input(|i| i.time) as f32;
            let offset = t * 2.4;
            draw_arc(
                &painter,
                center,
                radius,
                ring_width,
                start_angle + offset,
                std::f32::consts::PI * 0.6,
                ring_color,
            );
            // 旋转动画需要持续重绘；限制在 ~30fps，兼顾流畅与老机器的 CPU 开销。
            ui.ctx()
                .request_repaint_after(std::time::Duration::from_millis(33));
        }
    }

    // 有副标题时标题上移，给副标题留空间；无副标题时标题居中。
    let title_y = if center_sub.is_empty() {
        0.0
    } else {
        -diameter * 0.04
    };
    painter.text(
        center + Vec2::new(0.0, title_y),
        Align2::CENTER_CENTER,
        center_title,
        FontId::proportional(diameter * 0.16),
        colors::TEXT_PRIMARY,
    );
    if !center_sub.is_empty() {
        painter.text(
            center + Vec2::new(0.0, diameter * 0.09),
            Align2::CENTER_CENTER,
            center_sub,
            FontId::proportional(diameter * 0.055),
            colors::TEXT_SECONDARY,
        );
    }

    response
}

fn draw_arc(
    painter: &egui::Painter,
    center: Pos2,
    radius: f32,
    width: f32,
    start_angle: f32,
    sweep_angle: f32,
    color: Color32,
) {
    let segments = ((sweep_angle.abs() / std::f32::consts::TAU) * 128.0).clamp(2.0, 128.0) as usize;
    let step = sweep_angle / segments as f32;
    let points: Vec<Pos2> = (0..=segments)
        .map(|i| {
            let a = start_angle + step * i as f32;
            center + Vec2::new(a.cos(), a.sin()) * radius
        })
        .collect();
    painter.add(egui::Shape::line(
        points,
        PathStroke::from(Stroke::new(width, color)),
    ));
}

/// 一个数据胶囊：数值 + 说明文字，比如 "128 / 342  进程"。
///
/// **唯一保留药丸形态的元素**：它是纯读数（不可点），药丸形在这里是"这是
/// 一个数据读数"的可视语言（行业惯例），不是控件造型。所有**可点**的东西
/// ——按钮、chip、徽标——都已统一到 `radius::MD/SM` 刻度，页面里不再混用
/// 两套形态。
pub fn stat_pill(ui: &mut Ui, value: &str, label: &str) {
    egui::Frame::default()
        .fill(colors::BG_CARD)
        .stroke(Stroke::new(1.0, colors::BORDER))
        .corner_radius(999.0)
        .inner_margin(egui::Margin::symmetric(16, 8))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                bold_label(ui, value, 14.0, colors::TEXT_PRIMARY);
                ui.label(
                    egui::RichText::new(label)
                        .color(colors::TEXT_SECONDARY)
                        .small(),
                );
            });
        });
}

/// 量出一段文字在给定字号下的真实渲染宽度。用来在摆放前就知道"这块东西到底
/// 多宽"，而不是猜一个固定值——扫描过程中数字会变（比如 "5 / 342" 变成
/// "128 / 342"），猜的固定宽度会跟着跑偏，量出来的不会。
pub fn measure_text_width(ui: &Ui, text: &str, size: f32) -> f32 {
    ui.ctx()
        .fonts_mut(|f| {
            f.layout_no_wrap(text.to_owned(), FontId::proportional(size), Color32::WHITE)
        })
        .size()
        .x
}

/// 量出一段**单行**文字的渲染高度（不换行）。跟 `bold_label` 画出来的行高
/// 一致（bold_label 用的是同一套 `layout_no_wrap`）。
pub fn measure_text_height(ui: &Ui, text: &str, size: f32) -> f32 {
    ui.ctx()
        .fonts_mut(|f| {
            f.layout_no_wrap(text.to_owned(), FontId::proportional(size), Color32::WHITE)
        })
        .size()
        .y
}

/// 量出一段**会换行**文字在给定宽度下的渲染高度（跟 `ui.label` 的换行规则
/// 一致）。用于"这块内容有多高"的预算——纵向居中（把内容对齐到某个中线）
/// 必须先知道内容高度，否则只能靠 `available_height` 猜。
pub fn measure_wrapped_text_height(ui: &Ui, text: &str, size: f32, wrap_width: f32) -> f32 {
    ui.ctx()
        .fonts_mut(|f| {
            f.layout(
                text.to_owned(),
                FontId::proportional(size),
                Color32::WHITE,
                wrap_width,
            )
        })
        .size()
        .y
}

/// `stat_pill` 实际渲染宽度的估算：内边距(16*2) + 边框(1*2) + value/label 间距(10，
/// 全局 item_spacing) + 两段文字宽度。value 走的是 `bold_label`（横向多描 0.6px）。
fn stat_pill_width(ui: &Ui, value: &str, label: &str) -> f32 {
    let value_w = measure_text_width(ui, value, 14.0) + 0.6;
    // label 走 `.small()`，在 theme.rs 里是 12.5px——量宽必须用同一个字号，
    // 否则数字一变（"5 / 342" → "128 / 342"）胶囊宽度就跟内容脱节。
    let label_w = measure_text_width(ui, label, 12.5);
    16.0 * 2.0 + 1.0 * 2.0 + 10.0 + value_w + label_w
}

/// 居中摆放一行 `stat_pill`，宽度是量出来的，不是猜的——配合 SKILL 里"坑 1"的
/// 说明：外层 `vertical_centered` 只有拿到准确的 `desired_size` 才能正确居中。
pub fn centered_stat_pills(ui: &mut Ui, pills: &[(String, &str)]) {
    const GAP: f32 = 8.0;
    let mut total_width = 0.0;
    for (i, (value, label)) in pills.iter().enumerate() {
        if i > 0 {
            total_width += GAP;
        }
        total_width += stat_pill_width(ui, value, label);
    }
    let desired = Vec2::new(total_width, 40.0);
    ui.allocate_ui_with_layout(
        desired,
        egui::Layout::left_to_right(egui::Align::Center),
        |ui| {
            for (i, (value, label)) in pills.iter().enumerate() {
                if i > 0 {
                    ui.add_space(GAP);
                }
                stat_pill(ui, value, label);
            }
        },
    );
}

pub enum ThreatAction {
    None,
    Quarantine,
    Ignore,
}

/// `pill_button` 的水平内边距。量宽辅助 `pill_button_width` 与它共用同一个
/// 常量——两边各写一份 14.0 迟早会漂移。
const PILL_H_PAD: f32 = 14.0;

/// 威胁卡片上"隔离"/"忽略"用的小胶囊按钮。不用 `egui::Button` 是因为想让两个按钮
/// 的尺寸/圆角/居中方式完全统一可控（`egui::Button` 默认样式在深色红卡片背景上
/// 对比度不太对，两个按钮风格也不一致）。`filled=true` 是红底白字的"隔离"，
/// `filled=false` 是描边的"忽略"。`pub(crate)`：设置页（`app/settings.rs`）的
/// "还原/删除/移除"按钮复用同一套样式，不重新造一套。
pub(crate) fn pill_button(ui: &mut Ui, label: &str, filled: bool) -> bool {
    const V_PAD: f32 = 7.0;

    let text_color = if filled {
        Color32::WHITE
    } else {
        colors::TEXT_SECONDARY
    };
    let font_id = FontId::proportional(13.0);
    let galley = ui
        .ctx()
        .fonts_mut(|f| f.layout_no_wrap(label.to_owned(), font_id, text_color));
    let text_size = galley.size();
    let desired = Vec2::new(PILL_H_PAD * 2.0 + text_size.x, V_PAD * 2.0 + text_size.y);

    // 精确分配 desired 尺寸，文字用同一个 galley 居中绘制。之前用
    // `add_space + ui.label` 摆内容：label 按默认 Body 字号渲染，比量宽
    // 用的 13px 宽，再加 item_spacing，实际内容比 desired 宽 ~13px 并向
    // 右溢出分配区。在 right_to_left 布局里溢出会把父级 max_rect 一路撑
    // 大——威胁列表里每张卡都比上一张宽一点，按钮也无法对齐。
    let bg_idx = ui.painter().add(egui::Shape::Noop);
    let (bg_rect, response) = ui.allocate_exact_size(desired, Sense::click());
    let interact = response.on_hover_cursor(egui::CursorIcon::PointingHand);
    // 自绘按钮不会自动进 accesskit 树（egui 只为标准控件调 widget_info）——补上
    // 无障碍信息：屏幕阅读器能读出按钮名，egui_kittest 也能 get_by_label 查到
    // （威胁弹窗/设置页的 UI 测试都靠这个，见 AGENTS.md「UI 代码验证流程」）。
    interact.widget_info(|| {
        egui::WidgetInfo::labeled(egui::WidgetType::Button, true, label.to_owned())
    });
    let (fill, stroke) = if filled {
        // 危险实底按钮用 RED_ACTION（深一档，白字 ≥4.5:1），不用状态红 RED。
        let fill = if interact.hovered() {
            colors::RED_ACTION_HOVER
        } else {
            colors::RED_ACTION
        };
        (fill, Stroke::NONE)
    } else {
        let fill = if interact.hovered() {
            colors::BG_CARD
        } else {
            Color32::TRANSPARENT
        };
        (fill, Stroke::new(1.0, colors::RED_BORDER))
    };
    let shape = egui::epaint::RectShape::new(
        bg_rect,
        // 圆角走 MD 刻度，不再是胶囊：同一弹窗里 primary_button/action_button
        // 都是 4px 圆角矩形，威胁卡上这两个按钮要是还保持药丸形，同一个窗口里
        // 会出现两套按钮形态（2026-10 圆角减半后统一到"近乎直角"的基调）。
        crate::theme::radius::MD,
        fill,
        stroke,
        egui::epaint::StrokeKind::Inside,
    );
    ui.painter().set(bg_idx, egui::Shape::Rect(shape));

    let text_pos = bg_rect.center() - text_size / 2.0;
    ui.painter()
        .add(egui::epaint::TextShape::new(text_pos, galley, text_color));

    interact.clicked()
}

/// 量出 `pill_button` 最终会占的宽（`threat_card` 预算文本列宽度时用；
/// 模式同 `primary_button_width`）。
pub(crate) fn pill_button_width(ui: &Ui, label: &str) -> f32 {
    PILL_H_PAD * 2.0 + measure_text_width(ui, label, 13.0)
}

/// 中性描边胶囊按钮（"Cancel Scan" 这类次要/退出动作）：透明底 + BORDER 描边 +
/// 次要文字色，hover 时转红提示后果。与 `pill_button` 同一套量宽/占位回填实现，
/// 补 widget_info（AGENTS.md 规则 3）。取代扫描页原来的 `ui.link("Cancel Scan")`
/// ——取消扫描是重操作，超链接样式（蓝字下划线）撑不起这个分量。
pub fn ghost_button(ui: &mut Ui, label: &str, danger_hover: bool) -> bool {
    const H_PAD: f32 = 16.0;
    const V_PAD: f32 = 7.0;

    let font_id = FontId::proportional(13.0);
    let galley = ui
        .ctx()
        .fonts_mut(|f| f.layout_no_wrap(label.to_owned(), font_id, colors::TEXT_SECONDARY));
    let text_size = galley.size();
    let desired = Vec2::new(H_PAD * 2.0 + text_size.x, V_PAD * 2.0 + text_size.y);

    let bg_idx = ui.painter().add(egui::Shape::Noop);
    let (bg_rect, response) = ui.allocate_exact_size(desired, Sense::click());
    let interact = response.on_hover_cursor(egui::CursorIcon::PointingHand);
    interact.widget_info(|| {
        egui::WidgetInfo::labeled(egui::WidgetType::Button, true, label.to_owned())
    });

    let (fill, stroke) = if interact.hovered() && danger_hover {
        (colors::RED_BG, Stroke::new(1.0, colors::RED))
    } else if interact.hovered() {
        (colors::BG_CARD, Stroke::new(1.0, colors::BORDER))
    } else {
        (Color32::TRANSPARENT, Stroke::new(1.0, colors::BORDER))
    };
    let shape = egui::epaint::RectShape::new(
        bg_rect,
        crate::theme::radius::MD,
        fill,
        stroke,
        egui::epaint::StrokeKind::Inside,
    );
    ui.painter().set(bg_idx, egui::Shape::Rect(shape));

    let text_pos = bg_rect.center() - text_size / 2.0;
    ui.painter().add(egui::epaint::TextShape::new(
        text_pos,
        galley,
        colors::TEXT_SECONDARY,
    ));

    interact.clicked()
}

/// 威胁详情卡片：红色警示样式 + 隔离/忽略按钮。
pub fn threat_card(ui: &mut Ui, virus_name: &str, path: &str) -> ThreatAction {
    let mut action = ThreatAction::None;
    egui::Frame::default()
        .fill(colors::RED_BG)
        .stroke(Stroke::new(1.0, colors::RED_BORDER))
        .corner_radius(crate::theme::radius::MD)
        .inner_margin(egui::Margin::same(14))
        .show(ui, |ui| {
            // 卡先吃满弹窗内容宽（与设置页行卡同一模式）；病毒名/路径的像素预算
            // 都以这个卡内宽为基准，长文本在这里截断，不把卡撑爆、不挤压按钮。
            let card_w = ui.available_width();
            ui.set_min_width(card_w);
            // 先量出右侧按钮区与「High Risk」徽标的占宽，再预算文本列。
            let buttons_w =
                pill_button_width(ui, "Ignore") + 8.0 + pill_button_width(ui, "Quarantine");
            let badge_w = measure_text_width(ui, "High Risk", 12.5) + 16.0;
            // 文本列预算 = 卡内宽 − 图标 39 − 图标后间距 6 − 两个列间 item_spacing
            // − 按钮区，再留 12px 安全余量吸收伪粗体/徽标内边距的估算误差。
            let col_w = (card_w - 39.0 - 6.0 - 16.0 - buttons_w - 12.0).max(60.0);
            let name_w = (col_w - badge_w - ui.spacing().item_spacing.x).max(40.0);

            ui.horizontal(|ui| {
                let icon_size = 39.0;
                let (icon_response, painter) =
                    ui.allocate_painter(Vec2::splat(icon_size), Sense::hover());
                let icon_rect = icon_response.rect;
                painter.rect_filled(icon_rect, crate::theme::radius::SM_PX, colors::RED);
                let glyph_rect = icon_rect.shrink(icon_size * 0.24);
                crate::icons::warning_triangle(
                    &painter,
                    glyph_rect,
                    Stroke::new(2.0, colors::RED_BG),
                    None,
                );

                ui.add_space(6.0);
                ui.vertical(|ui| {
                    ui.horizontal(|ui| {
                        bold_label(
                            ui,
                            &truncate_middle_to_px(ui, virus_name, 15.0, name_w),
                            15.0,
                            colors::TEXT_PRIMARY,
                        );
                        egui::Frame::default()
                            .fill(colors::RED_ACTION)
                            .corner_radius(crate::theme::radius::SM)
                            .inner_margin(egui::Margin::symmetric(8, 2))
                            .show(ui, |ui| {
                                ui.label(
                                    egui::RichText::new("High Risk")
                                        .color(Color32::WHITE)
                                        .small(),
                                );
                            });
                    });
                    ui.label(
                        egui::RichText::new(truncate_middle_to_px(ui, path, 12.5, col_w))
                            .color(colors::TEXT_SECONDARY)
                            .small(),
                    );
                });

                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if pill_button(ui, "Ignore", false) {
                        action = ThreatAction::Ignore;
                    }
                    ui.add_space(8.0);
                    if pill_button(ui, "Quarantine", true) {
                        action = ThreatAction::Quarantine;
                    }
                });
            });
        });
    action
}

/// 按字符数截断（中间省略）：仅供 `truncate_middle_to_px` 收尾拼串用，
/// 外部一律用像素版——字符数版本宽窄全靠猜。
fn truncate_middle(s: &str, max_chars: usize) -> String {
    let chars: Vec<char> = s.chars().collect();
    if chars.len() <= max_chars {
        return s.to_string();
    }
    let head = max_chars * 3 / 5;
    let tail = max_chars - head - 1;
    let head_s: String = chars[..head].iter().collect();
    let tail_s: String = chars[chars.len() - tail..].iter().collect();
    format!("{head_s}…{tail_s}")
}

/// 按像素宽度截断（中间省略）：卡内定宽字段（信息卡版本号/说明行等）用
/// 这个，不要用字符数拍脑袋——16px 粗体的 26 字符 ≈234px，而卡内可用只有
/// 170px，字符数版会把卡撑爆/文字叠到卡边上（2026-10 实测）。
/// 返回的串保证在 `font_size` 下不超过 `max_px`（省略号计入）。
pub fn truncate_middle_to_px(ui: &Ui, text: &str, font_size: f32, max_px: f32) -> String {
    if max_px <= 8.0 || text.is_empty() {
        return text.to_string();
    }
    if measure_text_width(ui, text, font_size) <= max_px {
        return text.to_string();
    }
    // 二分收敛到“刚好塞下”的字符数（含省略号占位）。
    let chars: Vec<char> = text.chars().collect();
    let fits = |n: usize| {
        if n == 0 {
            return true;
        }
        let head = n * 3 / 5;
        let tail = n - head - 1;
        let mut probe = String::new();
        probe.extend(chars[..head].iter());
        probe.push('…');
        probe.extend(chars[chars.len() - tail..].iter());
        measure_text_width(ui, &probe, font_size) <= max_px
    };
    let mut lo = 0usize;
    let mut hi = chars.len();
    while lo < hi {
        let mid = (lo + hi + 1).div_ceil(2);
        if fits(mid) {
            lo = mid;
        } else {
            hi = mid - 1;
        }
    }
    truncate_middle(text, lo.max(4))
}

/// Toast 通知：右下角浮层，几秒后自动消失。
#[derive(Clone)]
pub struct Toast {
    pub text: String,
    pub created_at: std::time::Instant,
}

impl Toast {
    pub fn new(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            created_at: std::time::Instant::now(),
        }
    }

    pub fn expired(&self) -> bool {
        self.created_at.elapsed() > std::time::Duration::from_millis(2600)
    }
}

const ACTION_BTN_ICON_SIZE: f32 = 21.0;
const ACTION_BTN_ICON_GAP: f32 = 10.0;
const ACTION_BTN_H_PAD: f32 = 16.0;
const ACTION_BTN_V_PAD: f32 = 10.0;

/// 图标 + 文字的胶囊按钮；返回是否被点击。
pub fn action_button(
    ui: &mut Ui,
    label: &str,
    draw: impl FnOnce(&egui::Painter, egui::Rect, Stroke),
) -> bool {
    action_button_response(ui, label, label, draw).clicked()
}

/// 同 `action_button`，但可以给一个与可见文字不同的无障碍名（见
/// `primary_button_named` 的说明：跟侧栏同名项撞 accesskit 名会让 kittest
/// `get_by_label` 多匹配 panic）。
pub fn action_button_named(
    ui: &mut Ui,
    label: &str,
    accessible_label: &str,
    draw: impl FnOnce(&egui::Painter, egui::Rect, Stroke),
) -> bool {
    action_button_response(ui, label, accessible_label, draw).clicked()
}

/// 同 `action_button`，但返回完整 `Response`（用于挂 tooltip 等）。
pub fn action_button_response(
    ui: &mut Ui,
    label: &str,
    accessible_label: &str,
    draw: impl FnOnce(&egui::Painter, egui::Rect, Stroke),
) -> Response {
    const ICON_SIZE: f32 = ACTION_BTN_ICON_SIZE;
    const ICON_GAP: f32 = ACTION_BTN_ICON_GAP;
    const H_PAD: f32 = ACTION_BTN_H_PAD;
    const V_PAD: f32 = ACTION_BTN_V_PAD;

    let text_color = colors::TEXT_PRIMARY;
    let font_id = egui::FontId::proportional(14.0);
    let galley = ui
        .ctx()
        .fonts_mut(|f| f.layout_no_wrap(label.to_owned(), font_id, text_color));
    let text_size = galley.size();
    let desired = Vec2::new(
        H_PAD * 2.0 + ICON_SIZE + ICON_GAP + text_size.x,
        V_PAD * 2.0 + ICON_SIZE.max(text_size.y),
    );

    // 文字用 galley 直画（跟 primary_button 同款）。原来这里放的是
    // `ui.label`：横向布局会在图标与文字之间自动插一道 item_spacing，实际
    // 内容比 desired 宽 10px——右侧内边距被挤掉一半，按钮实际比量出来的宽，
    // 右边缘对不齐；而且 accesskit 树里会多出一个同名的"文本节点"（跟按钮
    // 节点撞名，kittest get_by_label 多匹配 panic）。
    let bg_idx = ui.painter().add(egui::Shape::Noop);
    let (bg_rect, response) = ui.allocate_exact_size(desired, Sense::click());
    let icon_rect = egui::Rect::from_min_size(
        egui::pos2(bg_rect.left() + H_PAD, bg_rect.center().y - ICON_SIZE / 2.0),
        Vec2::splat(ICON_SIZE),
    );
    draw(
        ui.painter(),
        icon_rect,
        Stroke::new(1.6, colors::ACCENT_BLUE),
    );
    let text_pos = egui::pos2(
        icon_rect.right() + ICON_GAP,
        bg_rect.center().y - text_size.y / 2.0,
    );
    ui.painter()
        .add(egui::epaint::TextShape::new(text_pos, galley, text_color));
    let interact = response.on_hover_cursor(egui::CursorIcon::PointingHand);
    interact.widget_info(|| {
        egui::WidgetInfo::labeled(egui::WidgetType::Button, true, accessible_label.to_owned())
    });
    let fill = if interact.hovered() {
        colors::ACCENT_BLUE_BG
    } else {
        colors::BG_CARD
    };
    let shape = egui::epaint::RectShape::new(
        bg_rect,
        crate::theme::radius::MD,
        fill,
        Stroke::new(1.0, colors::BORDER),
        egui::epaint::StrokeKind::Inside,
    );
    ui.painter().set(bg_idx, egui::Shape::Rect(shape));

    interact
}

pub fn show_toasts(ctx: &egui::Context, toasts: &[Toast]) {
    if toasts.is_empty() {
        return;
    }
    // 锚在窗口右下角，但要让开底部资源条（高 RESOURCE_BAR_HEIGHT）——直接锚
    // 窗口底缘的话 toast 会整个压在资源条的 CPU/Memory 仪表上（实测 toast 占
    // y 527..560，资源条从 y=542 开始，"Memory 60%" 被盖掉）。
    egui::Area::new(egui::Id::new("clv3000_toasts"))
        .anchor(
            egui::Align2::RIGHT_BOTTOM,
            egui::vec2(-20.0, -(RESOURCE_BAR_HEIGHT + 20.0)),
        )
        .order(egui::Order::Tooltip)
        .show(ctx, |ui| {
            ui.vertical(|ui| {
                for toast in toasts {
                    egui::Frame::default()
                        .fill(colors::BG_CARD)
                        .stroke(Stroke::new(1.0, colors::BORDER))
                        .corner_radius(crate::theme::radius::SM)
                        .inner_margin(egui::Margin::symmetric(14, 10))
                        .show(ui, |ui| {
                            ui.label(egui::RichText::new(&toast.text).color(colors::TEXT_PRIMARY));
                        });
                    ui.add_space(6.0);
                }
            });
        });
    ctx.request_repaint_after(std::time::Duration::from_millis(300));
}

#[cfg(test)]
mod tests {
    //! egui_kittest 驱动**真实**组件的 UI 测试（见 AGENTS.md「UI 代码验证流程」）。

    use super::{ghost_button, primary_button};
    use egui_kittest::Harness;
    use egui_kittest::kittest::Queryable;

    /// `ghost_button`（替代扫描页原来的 `ui.link("Cancel Scan")`）的三个约定：
    /// 进 accesskit 树（widget_info，屏幕阅读器/kittest 可查）、hover 手型光标、
    /// 点击真的触发动作。自定义按钮这三样都靠手工维护，容易悄悄失效。
    #[test]
    fn ghost_button_queryable_click_and_cursor() {
        let mut harness = Harness::builder()
            .with_size(egui::Vec2::new(300.0, 100.0))
            .build_ui_state(
                |ui, clicked| {
                    ui.vertical_centered(|ui| {
                        if ghost_button(ui, "Cancel Scan", true) {
                            *clicked = true;
                        }
                    });
                },
                false,
            );
        crate::theme::apply(&harness.ctx);
        harness.run();

        // hover 显示手型光标（与全应用其他按钮一致的交互惯例）。
        harness.get_by_label("Cancel Scan").hover();
        harness.run();
        assert_eq!(
            harness.output().platform_output.cursor_icon,
            egui::CursorIcon::PointingHand,
            "ghost_button hover 应显示手型光标"
        );

        // 点击触发动作。
        harness.get_by_label("Cancel Scan").click();
        harness.run();
        assert!(*harness.state(), "ghost_button 点击应返回 true");
    }

    /// `primary_button`（主操作实底按钮）与 `ghost_button` 同一套约定：进
    /// accesskit 树、hover 手型光标、点击触发动作。
    #[test]
    fn primary_button_queryable_click_and_cursor() {
        let mut harness = Harness::builder()
            .with_size(egui::Vec2::new(300.0, 100.0))
            .build_ui_state(
                |ui, clicked| {
                    ui.vertical_centered(|ui| {
                        if primary_button(ui, "Start Scan", |_, _, _| {}) {
                            *clicked = true;
                        }
                    });
                },
                false,
            );
        crate::theme::apply(&harness.ctx);
        harness.run();

        harness.get_by_label("Start Scan").hover();
        harness.run();
        assert_eq!(
            harness.output().platform_output.cursor_icon,
            egui::CursorIcon::PointingHand,
            "primary_button hover 应显示手型光标"
        );

        harness.get_by_label("Start Scan").click();
        harness.run();
        assert!(*harness.state(), "primary_button 点击应返回 true");
    }
}

