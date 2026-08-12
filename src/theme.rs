//! 深色主题配色 + 全局样式设置 + 圆点网格背景。

use egui::{Color32, Context, CornerRadius, Rect, Stroke, Visuals};

pub mod colors {
    use egui::Color32;

    pub const BG_APP: Color32 = Color32::from_rgb(10, 13, 18);
    pub const BG_SIDEBAR: Color32 = Color32::from_rgb(6, 8, 11);
    #[cfg(not(windows))]
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

    // `all_styles_mut` 同时设置 light_style（防御性）；本项目固定深色主题。
    ctx.all_styles_mut(|style| {
        style.spacing.item_spacing = egui::vec2(10.0, 10.0);
        style.spacing.button_padding = egui::vec2(14.0, 8.0);

        style.text_styles = [
            (egui::TextStyle::Small, egui::FontId::proportional(14.0)),
            (egui::TextStyle::Body, egui::FontId::proportional(14.0)),
            (egui::TextStyle::Button, egui::FontId::proportional(14.0)),
            (egui::TextStyle::Heading, egui::FontId::proportional(19.0)),
            (
                egui::TextStyle::Monospace,
                egui::FontId::new(14.0, egui::FontFamily::Monospace),
            ),
        ]
        .into();
    });
}

/// 点阵背景一个瓦片对应的逻辑尺寸（点）。原来的 `circle_filled` 实现里点间距
/// 就是 28pt，纹理平铺方案沿用同一个数值，视觉效果不变。
pub const DOT_TILE_PT: f32 = 28.0;
/// 纹理超采样倍数：瓦片实际按 `DOT_TILE_PT * SUPERSAMPLE` 像素生成，只是为了让
/// 圆点边缘有个软过渡（近似原来 `circle_filled` 自带的抗锯齿），不是为了适配
/// 某个具体的 `pixels_per_point`——纹理经 GPU 双线性采样后再铺到屏幕上，缩放本身
/// 就会做插值，不需要跟真实设备像素一一对应。
const DOT_TILE_SUPERSAMPLE: usize = 4;

/// 生成一张 `DOT_TILE_PT` 见方的点阵瓦片纹理源数据：中心一个极淡的圆点，四周
/// 透明。配合 `TextureOptions::LINEAR_REPEAT` 平铺整块背景。
///
/// 为什么要从"每帧画几百个 `circle_filled`"换成"画一次纹理，每帧铺一个矩形"：
/// `circle_filled` 提交的是矢量图元，epaint 每帧都要重新 tessellate 成三角形网格
/// （每个带羽化的圆约 10~16 个顶点）——900×600 的面板铺满大约 600 个点，就是每帧
/// 上万顶点的分摊开销，而这层背景纹理感本身淡到几乎看不见，付出的顶点生成成本
/// 和它带来的视觉收益完全不成比例。烘成纹理后，无论背景铺多大，每帧只提交一个
/// 矩形（2 个三角形），GPU 采样负责重复图案，成本从"跟点数量成正比"变成常数。
pub fn dotted_tile_image() -> egui::ColorImage {
    let px = (DOT_TILE_PT as usize) * DOT_TILE_SUPERSAMPLE;
    let radius_px = DOT_TILE_SUPERSAMPLE as f32; // 对应逻辑半径 1.0pt
    let center = px as f32 / 2.0;
    let mut pixels = vec![Color32::TRANSPARENT; px * px];
    for y in 0..px {
        for x in 0..px {
            let dx = x as f32 + 0.5 - center;
            let dy = y as f32 + 0.5 - center;
            let dist = (dx * dx + dy * dy).sqrt();
            // 边缘留 1 像素的软过渡，近似原来矢量圆的抗锯齿羽化。
            let coverage = (radius_px + 1.0 - dist).clamp(0.0, 1.0);
            if coverage <= 0.0 {
                continue;
            }
            // DOT_GRID 本身是 premultiplied 颜色（r=g=b=a，纯白按 alpha 比例乘过）；
            // 按 coverage 等比缩小 rgb 和 a，premultiplied 语义仍然成立。
            let v = (colors::DOT_GRID.r() as f32 * coverage).round() as u8;
            let a = (colors::DOT_GRID.a() as f32 * coverage).round() as u8;
            pixels[y * px + x] = Color32::from_rgba_premultiplied(v, v, v, a);
        }
    }
    egui::ColorImage::new([px, px], pixels)
}

/// 在 `rect` 范围内铺一层很淡的圆点网格背景，营造"科技感底纹"。`tile` 是
/// `dotted_tile_image()` 生成、并以 `TextureOptions::LINEAR_REPEAT` 加载好的纹理
/// （由调用方缓存复用，见 `App::ensure_ui_resources`），这里只负责按 `rect` 尺寸
/// 算好重复次数的 UV 并画一次。
///
/// `tile` 是 `Option`：跟 `app_icon_texture`/`titlebar_icon_texture` 一样，纹理
/// 可能在本帧还没加载、或刚被 `release_ui_resources` 释放——自绘标题栏的关闭按钮
/// （`title_bar` 里 `app.hide_to_tray(ctx)`）会在 `ui()` 走到这里**之前**同步释放
/// 资源，同一帧稍后仍会画到这块背景，如果这里硬取值（`.expect()`/`.unwrap()`）
/// 就会直接 panic。`None` 时只铺纯色背景、跳过点阵，视觉上无害（下一帧资源已经
/// 因为窗口隐藏而不会再被用到）。
pub fn paint_dotted_background(
    painter: &egui::Painter,
    rect: Rect,
    tile: Option<&egui::TextureHandle>,
) {
    painter.rect_filled(rect, CornerRadius::ZERO, colors::BG_APP);
    let Some(tile) = tile else { return };
    let uv = Rect::from_min_max(
        egui::pos2(0.0, 0.0),
        egui::pos2(rect.width() / DOT_TILE_PT, rect.height() / DOT_TILE_PT),
    );
    painter.image(tile.id(), rect, uv, Color32::WHITE);
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
