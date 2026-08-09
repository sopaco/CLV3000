//! 深色主题配色 + 全局样式设置 + 圆点网格背景。

use egui::{Color32, Context, CornerRadius, Rect, Stroke, Visuals};

pub mod colors {
    use egui::Color32;

    pub const BG_APP: Color32 = Color32::from_rgb(10, 13, 18);
    pub const BG_SIDEBAR: Color32 = Color32::from_rgb(6, 8, 11);
    pub const BG_TITLEBAR: Color32 = Color32::from_rgb(8, 10, 14);
    pub const BG_CARD: Color32 = Color32::from_rgb(18, 22, 29);
    pub const BORDER: Color32 = Color32::from_rgb(34, 40, 50);
    // 注意：这是"premultiplied" alpha，RGB 分量要按 alpha 比例先乘过一遍——
    // 之前误写成 (255,255,255,10)，等于告诉渲染器"这几乎是一块不透明的白"，
    // 圆点自然亮得很显眼。8/255 的透明白，premultiplied 后 RGB 也要是 8，不是 255。
    pub const DOT_GRID: Color32 = Color32::from_rgba_premultiplied(8, 8, 8, 8);

    pub const ACCENT_BLUE: Color32 = Color32::from_rgb(58, 160, 224);
    pub const ACCENT_BLUE_BG: Color32 = Color32::from_rgb(17, 50, 71);

    pub const GREEN: Color32 = Color32::from_rgb(34, 197, 94);
    #[allow(dead_code)]
    pub const GREEN_DIM_BG: Color32 = Color32::from_rgb(16, 40, 27);

    pub const RED: Color32 = Color32::from_rgb(239, 68, 68);
    pub const RED_BG: Color32 = Color32::from_rgb(42, 20, 22);
    pub const RED_BORDER: Color32 = Color32::from_rgb(80, 32, 36);

    pub const TEXT_PRIMARY: Color32 = Color32::from_rgb(245, 247, 250);
    pub const TEXT_SECONDARY: Color32 = Color32::from_rgb(138, 147, 163);
    pub const TEXT_MUTED: Color32 = Color32::from_rgb(92, 100, 114);
}

/// 全局一次性设置：字体大小、圆角、间距、配色。
pub fn apply(ctx: &Context) {
    let mut visuals = Visuals::dark();
    visuals.override_text_color = Some(colors::TEXT_PRIMARY);
    visuals.panel_fill = colors::BG_APP;
    visuals.window_fill = colors::BG_CARD;
    visuals.extreme_bg_color = colors::BG_SIDEBAR;
    visuals.faint_bg_color = colors::BG_CARD;
    visuals.widgets.noninteractive.bg_fill = colors::BG_CARD;
    visuals.widgets.noninteractive.bg_stroke = Stroke::new(1.0, colors::BORDER);
    visuals.widgets.inactive.bg_fill = colors::BG_CARD;
    visuals.widgets.inactive.weak_bg_fill = colors::BG_CARD;
    visuals.widgets.hovered.bg_fill = colors::ACCENT_BLUE_BG;
    visuals.widgets.active.bg_fill = colors::ACCENT_BLUE_BG;
    visuals.selection.bg_fill = colors::ACCENT_BLUE_BG;
    visuals.selection.stroke = Stroke::new(1.0, colors::ACCENT_BLUE);
    // "取消扫描"那个链接默认会用 egui 自己的蓝，跟我们的品牌蓝不是同一个色号，
    // 混在一起会有点违和，统一成 ACCENT_BLUE。
    visuals.hyperlink_color = colors::ACCENT_BLUE;
    // 鼠标悬停在任何可点击元素上都换成手型指针——这个设置只对标准 `egui::Button`
    // 生效，我们自己拿 `ui.interact()` 手搓的按钮（action_button/pill_button/
    // 侧边栏图标/标题栏按钮）还得各自显式 `.on_hover_cursor(...)`，见那几处调用点。
    visuals.interact_cursor = Some(egui::CursorIcon::PointingHand);
    visuals.window_corner_radius = CornerRadius::same(14);
    visuals.menu_corner_radius = CornerRadius::same(10);
    ctx.set_visuals(visuals);

    ctx.global_style_mut(|style| {
        style.spacing.item_spacing = egui::vec2(10.0, 10.0);
        style.spacing.button_padding = egui::vec2(14.0, 8.0);
    });
}

/// 在 `rect` 范围内画一层很淡的圆点网格背景，营造"科技感底纹"。
pub fn paint_dotted_background(painter: &egui::Painter, rect: Rect) {
    const SPACING: f32 = 28.0;
    const RADIUS: f32 = 1.0;

    painter.rect_filled(rect, CornerRadius::ZERO, colors::BG_APP);

    let start_x = (rect.left() / SPACING).floor() * SPACING;
    let start_y = (rect.top() / SPACING).floor() * SPACING;

    let mut y = start_y;
    while y < rect.bottom() {
        let mut x = start_x;
        while x < rect.right() {
            painter.circle_filled(egui::pos2(x, y), RADIUS, colors::DOT_GRID);
            x += SPACING;
        }
        y += SPACING;
    }
}

/// 圆角矩形卡片背景，带边框，供各页面复用。
/// 目前 action_button 已经改成自己量尺寸 + 手动画背景（不能直接用 Frame，见 app.rs
/// 里的说明），这个函数暂时没有调用点了，但仍是一个好用的通用卡片样式，先保留。
#[allow(dead_code)]
pub fn card_frame() -> egui::Frame {
    egui::Frame::default()
        .fill(colors::BG_CARD)
        .stroke(Stroke::new(1.0, colors::BORDER))
        .corner_radius(12.0)
        .inner_margin(egui::Margin::same(16))
}

#[allow(dead_code)]
pub fn accent_for(cpu_or_mem_percent: f32) -> Color32 {
    if cpu_or_mem_percent > 85.0 {
        colors::RED
    } else if cpu_or_mem_percent > 60.0 {
        Color32::from_rgb(230, 180, 60)
    } else {
        colors::ACCENT_BLUE
    }
}
