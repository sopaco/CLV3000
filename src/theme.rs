//! 深色主题配色 + 全局样式设置 + 圆点网格背景。

use egui::{Color32, Context, CornerRadius, Rect, Stroke, Visuals};

pub mod colors {
    use egui::Color32;

    /// 背景三层（chrome 外壳 → 画布 → 卡片）。2026-10 视觉评审实测：旧值三层
    /// 亮度差都 <3/255，肉眼完全分不出区域，整个窗口读作一整块黑。现在保证相邻
    /// 层有可辨的台阶；区域交界仍然只靠深浅区分、不加分割线（沿用原设计稿
    /// 语言，见 design skill 坑 4）。
    pub const BG_SIDEBAR: Color32 = Color32::from_rgb(5, 7, 10);
    pub const BG_TITLEBAR: Color32 = Color32::from_rgb(7, 9, 13);
    pub const BG_APP: Color32 = Color32::from_rgb(13, 17, 23);
    pub const BG_CARD: Color32 = Color32::from_rgb(24, 29, 38);
    pub const BORDER: Color32 = Color32::from_rgb(42, 49, 61);
    // 注意：这是"premultiplied" alpha，RGB 分量要按 alpha 比例先乘过一遍——
    // 之前误写成 (255,255,255,10)，等于告诉渲染器"这几乎是一块不透明的白"，
    // 圆点自然亮得很显眼。16/255 的透明白，premultiplied 后 RGB 也要是 16，
    // 不是 255。（评审实测 8/255 时点阵几乎不可见，提到 16 ≈ 6% 白。）
    pub const DOT_GRID: Color32 = Color32::from_rgba_premultiplied(16, 16, 16, 16);

    pub const ACCENT_BLUE: Color32 = Color32::from_rgb(58, 160, 224);
    pub const ACCENT_BLUE_BG: Color32 = Color32::from_rgb(17, 50, 71);
    /// 主操作按钮实底（"Start Scan"/"Update Database" 这类一页唯一的推进动作）。
    /// 故意比 ACCENT_BLUE 深一档：亮蓝 #3AA0E0 上放白字只有 ~2.9:1（连图形标准
    /// 3:1 都不到），这个色号白字 ≥4.5:1（AA）。区分主次：主操作=实底白字，
    /// 次操作=`action_button` 的卡片底描边——2026-10 改版前所有按钮长得一样，
    /// 页面没有视觉重心。
    pub const ACCENT_ACTION: Color32 = Color32::from_rgb(30, 110, 173);
    /// hover 提亮一档，白字仍 ≥4.5:1（比 ACCENT_ACTION_HOVER 更亮的蓝会跌破 AA）。
    pub const ACCENT_ACTION_HOVER: Color32 = Color32::from_rgb(39, 120, 186);
    /// 卡片 hover 描边：比 BORDER 亮一档，暗示"这张卡可以点"。配 BG_CARD 底色
    /// 不闪烁、只提描边——深色主题里大面积高亮很扎眼。
    pub const BORDER_STRONG: Color32 = Color32::from_rgb(62, 73, 92);

    pub const GREEN: Color32 = Color32::from_rgb(34, 197, 94);
    /// 资源条偏高时的警告色（`accent_for`），从硬编码字面量收进 token。
    pub const AMBER: Color32 = Color32::from_rgb(230, 180, 60);

    /// 状态红：大图形/描边用（状态环、图标、进度弧），对比度按 3:1 图形标准够。
    pub const RED: Color32 = Color32::from_rgb(239, 68, 68);
    /// 危险动作红：白字实底按钮/徽标用。比 RED 深一档，白字对比度 ≥4.5:1
    /// （WCAG AA 小字标准；旧值直接拿 RED 当底，白字只有 3.8:1）。
    pub const RED_ACTION: Color32 = Color32::from_rgb(217, 45, 32);
    pub const RED_ACTION_HOVER: Color32 = Color32::from_rgb(222, 70, 59);
    pub const RED_BG: Color32 = Color32::from_rgb(42, 20, 22);
    pub const RED_BORDER: Color32 = Color32::from_rgb(80, 32, 36);

    pub const TEXT_PRIMARY: Color32 = Color32::from_rgb(245, 247, 250);
    pub const TEXT_SECONDARY: Color32 = Color32::from_rgb(138, 147, 163);
    /// 弱化文字：提亮到对 BG_CARD ≥4.5:1（旧值 3.0:1，用于版本号/路径/说明文字
    /// 时达不到 AA）。
    pub const TEXT_MUTED: Color32 = Color32::from_rgb(127, 136, 148);
}

/// 圆角刻度：SM 小块（悬停底/图标底/toast）、MD 卡片/按钮/侧栏格、LG 窗口/弹窗。
///
/// 2026-10 评审两次调整：先把散落的 4/6/7/8/9/10/12/14/15/16 十一种值收敛成
/// 三档（8/12/16），当天又整体收紧一档（6/8/12），随后再减半到 **3/4/6**。
/// 理由：大色块上 12~16px 的圆角显得"圆钝"，8px 已经是常规偏方，收到 4~6px
/// 才真正是"干练"——圆角只留一点暗示，不再是造型元素（企业安全工具的常见
/// 观感：近乎直角，靠层次和留白而不是圆润度表达结构）。
///
/// 不进刻度的两类：
/// - 胶囊（255 / 999）：视觉由高度决定，是"药丸"语义而非"圆角"语义；
/// - 细进度条（≈高/2）：同胶囊语义。
pub mod radius {
    use egui::CornerRadius;

    /// 三档的**像素值**。`CornerRadius` 常量取不出 f32，而
    /// `painter.rect_filled(rect, 3.0, …)` 要的是 f32——所以像素值单列一份，
    /// `CornerRadius` 常量由它派生，避免两处各写一个数字后走偏。
    pub const SM_PX: f32 = 3.0;
    pub const MD_PX: f32 = 4.0;
    pub const LG_PX: f32 = 6.0;

    pub const SM: CornerRadius = CornerRadius::same(SM_PX as u8);
    pub const MD: CornerRadius = CornerRadius::same(MD_PX as u8);
    pub const LG: CornerRadius = CornerRadius::same(LG_PX as u8);

    /// 弹窗头部：上圆角比外框小 1px（留给外框描边），下圆角为 0。
    pub const HEADER: CornerRadius = CornerRadius {
        nw: LG_PX as u8 - 1,
        ne: LG_PX as u8 - 1,
        sw: 0,
        se: 0,
    };
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
    visuals.window_corner_radius = CornerRadius::same(6);
    visuals.menu_corner_radius = CornerRadius::same(4);
    // 原生 egui 控件（ComboBox / selectable_label 等）的圆角：egui 默认 2~3px，
    // 跟我们自绘的 4px 卡片/按钮放一起仍然偏圆——统一收到 2px，比自绘卡片更方
    // （控件本来就小），但不再是尖角。
    for w in [
        &mut visuals.widgets.noninteractive,
        &mut visuals.widgets.inactive,
        &mut visuals.widgets.hovered,
        &mut visuals.widgets.active,
        &mut visuals.widgets.open,
    ] {
        w.corner_radius = CornerRadius::same(2);
    }
    ctx.set_visuals(visuals);

    // `all_styles_mut` 同时设置 light_style（防御性）；本项目固定深色主题。
    // 字号层级：Small(12.5) 是正文 14 的下一档，专给说明文字/路径/徽标——
    // 2026-10 评审前 Small 被改成与 Body 同为 14，`.small()` 完全无效，
    // 页面所有文字一样大、层级全靠颜色硬撑。
    ctx.all_styles_mut(|style| {
        style.spacing.item_spacing = egui::vec2(10.0, 10.0);
        style.spacing.button_padding = egui::vec2(14.0, 8.0);

        style.text_styles = [
            (egui::TextStyle::Small, egui::FontId::proportional(12.5)),
            (egui::TextStyle::Body, egui::FontId::proportional(14.0)),
            (egui::TextStyle::Button, egui::FontId::proportional(14.0)),
            (egui::TextStyle::Heading, egui::FontId::proportional(20.0)),
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

/// 按百分比取强调色：正常蓝、偏高黄、超阈值红。
pub fn accent_for(cpu_or_mem_percent: f32) -> Color32 {
    if cpu_or_mem_percent > 85.0 {
        colors::RED
    } else if cpu_or_mem_percent > 60.0 {
        colors::AMBER
    } else {
        colors::ACCENT_BLUE
    }
}
