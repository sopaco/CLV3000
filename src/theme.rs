//! 深色主题配色 + 全局样式设置。

use egui::{Color32, Context, CornerRadius, Stroke, Visuals};

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
