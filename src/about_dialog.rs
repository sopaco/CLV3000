//! 轻量「关于」对话框，有两种呈现方式：
//!
//! 1. 覆盖层模态（`paint_about_modal`）：画在主界面之上，用于"主窗内打开关于"的场景
//!    （当前没有入口，预留）。
//! 2. **独占整个窗口**（`paint_about_fullscreen`）：来自托盘时调用——整个窗口只画关于页、
//!    不画主界面，背后是深色主题底，看起来就是一张独立的关于窗口；关闭后由
//!    `App::reconcile_lifecycle` 自动把视口重新藏起来（回到托盘），不会残留主窗口。
//!
//! 为什么不用独立的原生子视口（`show_viewport_deferred`）？eframe 在「根视口被
//! `Visible(false)` 隐藏」时不会为 deferred 子视口创建原生窗口，从托盘弹独立窗口不可靠。
//! 而"独占主视口"同样能给出"只显示关于页、不要主窗口"的独立观感，且稳定。

use crate::clamav_info::ClamAvInfo;
use crate::icon_data;
use crate::theme::colors;
use crate::widgets;
use egui::{Align2, CornerRadius, CursorIcon, Frame, Key, Rect, Sense, Stroke, TextureHandle, Vec2, pos2};
use std::sync::atomic::{AtomicBool, Ordering};

const LOGO_TEX_ID: &str = "clv3000_about_logo";
/// 关于弹窗 logo 的 UI 逻辑尺寸（pt）。
const LOGO_DISPLAY_PT: f32 = 88.0;
/// logo 的最小显示尺寸：窗口不够高时按缺口等比缩小，缩到这个下限就不再缩
/// （再缩就认不出是 app 图标了）；还差的那点由收紧卡片内边距补上。
const LOGO_MIN_PT: f32 = 56.0;

const SIZE: [f32; 2] = [410.0, 340.0];

/// ── 关于卡片的排版常量 ────────────────────────────────────────────
/// 主体（logo + 标题 + 版本 + 两条信息 + 标语）里每一段之间的间距。
///
/// `paint_about_body` 负责按这些间距排版，`about_body_height` 负责按同一份
/// 间距算出主体有多高——两者共用同一份常量，避免"算出来装得下、实际渲染
/// 超界"（超界的直接后果就是卡片底部被裁、OK 按钮贴到窗口边）。
const BODY_GAP_TOP: f32 = 5.0;
const BODY_GAP_LOGO: f32 = 20.0;
const BODY_GAP_TITLE: f32 = 10.0;
const BODY_GAP_VERSION: f32 = 24.0;
const BODY_GAP_INFO: f32 = 16.0;
const BODY_GAP_TAGLINE: f32 = 16.0;
/// 信息行里"标签"与"取值"两行之间的距离。
const INFO_ROW_GAP: f32 = 10.0;

/// 关于卡片自身的几何量。
const CARD_MARGIN: f32 = 28.0;
/// 主体与 OK 按钮之间的间隙。
const OK_TOP_GAP: f32 = 18.0;
/// OK 按钮的最小高度（`ok_button` 里 min_size 给的值；实际渲染高度更大，
/// 见 `ok_button_height`）。
const OK_BUTTON_MIN_H: f32 = 32.0;
/// OK 按钮下方、卡片内边距之外的额外间隙。
const OK_BOTTOM_GAP: f32 = 12.0;
/// 卡片内容区的最小高度：窗口被压得极矮时的下限（再小内容就没法看了）。
const BODY_MIN_H: f32 = 220.0;
/// 卡片与窗口边缘的间距（左右 + 底部）。
const CARD_EDGE_MARGIN: f32 = 8.0;
/// 主体高度预算的安全余量：高度公式（按字体行高算）实测比实际渲染低几个
/// 像素，留一段余量让 logo 该缩的时候缩，别让卡片撑破可用高度。
const BODY_SAFETY: f32 = 16.0;

/// 关于窗关闭信号：OK / Esc / 窗口关闭按钮都会置位，`App::logic` 据此关闭覆盖层。
/// 用全局原子量是因为「关闭」发生在 UI 绘制阶段，而关闭处理在 logic 阶段，二者跨方法，
/// 全局量最简单。
static ABOUT_CLOSED: AtomicBool = AtomicBool::new(false);

/// 进程内只采集一次病毒库信息（避免每帧都起 clamscan 子进程）。
fn cached_info() -> &'static ClamAvInfo {
    static INFO: std::sync::OnceLock<ClamAvInfo> = std::sync::OnceLock::new();
    INFO.get_or_init(ClamAvInfo::gather)
}

/// 消费关闭信号：若为 true 则清零并返回 true。
pub fn take_closed() -> bool {
    ABOUT_CLOSED.swap(false, Ordering::Relaxed)
}

/// 由 `App::ui` 在 `about_open` 为真时调用，把关于内容画成叠在主界面之上的居中模态窗。
/// 用户点 OK / 按 Esc / 点窗口关闭按钮即视为关闭，置 `ABOUT_CLOSED`，由 `App::logic`
/// 据此关掉覆盖层。整窗底色（主界面）由调用方在下方已经画好。
pub fn paint_about_modal(ctx: &egui::Context) {
    // 主题（visuals/styles）由 `App::new` 在会话建立时调一次 `theme::apply` 即可，
    // egui 不会在帧间重置它们。这里曾每帧都 `theme::apply`，会失效 egui 的
    // visuals/styles 缓存、强制重新布局——关于窗打开期间鼠标移动以满帧率触发重绘，
    // 每帧重设 visuals 把开销放大成持续微卡顿 + CPU。详见 skill 第 2 节。

    let logo = load_logo_texture(ctx);
    let info = cached_info();

    let mut open = true;
    egui::Window::new("About CLV3000")
        .collapsible(false)
        .resizable(false)
        .fixed_size(SIZE)
        .anchor(Align2::CENTER_CENTER, [0.0, 0.0])
        .open(&mut open)
        .show(ctx, |ui| {
            paint_about_body(ui, &logo, info, LOGO_DISPLAY_PT);
            ui.add_space(12.0);
            ui.vertical_centered(|ui| {
                if ok_button(ui).clicked() || ui.input(|i| i.key_pressed(Key::Escape)) {
                    ABOUT_CLOSED.store(true, Ordering::Relaxed);
                }
            });
        });

    // 用户点了窗口自带的关闭按钮（open 被置 false）。
    if !open {
        ABOUT_CLOSED.store(true, Ordering::Relaxed);
    }
}

fn load_logo_texture(ctx: &egui::Context) -> TextureHandle {
    // 每个会话只解码/上传一次，句柄缓存在 egui 的临时数据区。
    // 之前每帧都重新解码内嵌 PNG 并重新上传纹理——只要鼠标在关于窗上移动/尝试拖动
    // （指针事件会以满帧率触发重绘），每帧都重复这份编解码 + GPU 上传的重活，
    // 表现为"关于窗一开就卡、CPU 飙升"。
    let key = egui::Id::new(LOGO_TEX_ID);
    if let Some(handle) = ctx.data_mut(|d| d.get_temp::<TextureHandle>(key)) {
        return handle;
    }
    let (rgba, w, h) =
        icon_data::load_app_icon_for_display(LOGO_DISPLAY_PT, ctx.pixels_per_point());
    let image = egui::ColorImage::from_rgba_unmultiplied([w as usize, h as usize], &rgba);
    let handle = ctx.load_texture(LOGO_TEX_ID, image, egui::TextureOptions::LINEAR);
    ctx.data_mut(|d| d.insert_temp(key, handle.clone()));
    handle
}

fn paint_about_body(ui: &mut egui::Ui, logo: &TextureHandle, info: &ClamAvInfo, logo_pt: f32) {
    ui.vertical_centered(|ui| {
        // 段间距全部由下面的显式 add_space 控制（把布局自带的 item_spacing.y
        // 归零）。这样"主体有多高"才能被 `about_body_height` 精确算出来——
        // 窗口不够高时按实际缺口收缩 logo，而不是让卡片溢出被裁。
        ui.spacing_mut().item_spacing.y = 0.0;
        ui.add_space(BODY_GAP_TOP);
        ui.add(
            egui::Image::new((logo.id(), logo.size_vec2()))
                .fit_to_exact_size(Vec2::splat(logo_pt))
                .corner_radius(crate::theme::radius::LG),
        );
        ui.add_space(BODY_GAP_LOGO);
        widgets::bold_label(ui, "CLV3000", 20.0, colors::TEXT_PRIMARY);
        ui.add_space(BODY_GAP_TITLE);
        ui.label(
            egui::RichText::new(format!("Version {}", env!("CARGO_PKG_VERSION")))
                .color(colors::TEXT_SECONDARY)
                .small(),
        );
        ui.add_space(BODY_GAP_VERSION);
        info_row(ui, "ClamAV Engine", &info.engine);
        ui.add_space(BODY_GAP_INFO);
        info_row(ui, "Virus Database", &info.database);
        ui.add_space(BODY_GAP_TAGLINE);
        ui.label(
            egui::RichText::new("Fast, reliable virus protection for even older PCs.")
                .color(colors::TEXT_MUTED)
                .small(),
        );
    });
}

/// 关于主体（logo + 标题 + 版本 + 两条信息 + 标语）在给定 logo 尺寸下的自然高度。
///
/// 跟 `paint_about_body` 一一对应：同一份间距常量、同样的文本样式、同样的
/// 行高来源。用途是在**渲染之前**就知道这块内容有多高，从而在窗口不够高时
/// 按实际缺口精确收缩 logo。
///
/// 主体高度 = 各段高度 + 各段间距（`paint_about_body` 已把布局自带的
/// item_spacing.y 归零，段间距全部来自上面的常量，所以这里是精确的）。
fn about_body_height(ui: &egui::Ui, logo_pt: f32) -> f32 {
    // 标题：加粗 20pt；`bold_label` 的高度就是这行文字的行高。
    let title_h = widgets::measure_text_height(ui, "CLV3000", 20.0);
    // 小号文字（版本号 / 标语 / 信息行标签）统一用主题的 Small 样式。
    let small_h = ui.text_style_height(&egui::TextStyle::Small);
    // 信息行的"取值"用正文样式。
    let body_h = ui.text_style_height(&egui::TextStyle::Body);
    let info_row_h = small_h + INFO_ROW_GAP + body_h;
    BODY_GAP_TOP
        + logo_pt
        + BODY_GAP_LOGO
        + title_h
        + BODY_GAP_TITLE
        + small_h
        + BODY_GAP_VERSION
        + info_row_h
        + BODY_GAP_INFO
        + info_row_h
        + BODY_GAP_TAGLINE
        + small_h
}

/// OK 按钮的实际高度。
///
/// egui 按钮的高度是 `max(min_size.y, 文字行高 + 2×按钮内边距)`，不是
/// `min_size.y`（实测 34px，而不是 min_size 里的 32px）。高度预算里必须用
/// 实际值，否则卡片会算出比实际矮一截，底部被裁、按钮贴边。
fn ok_button_height(ui: &egui::Ui) -> f32 {
    (ui.text_style_height(&egui::TextStyle::Button) + 2.0 * ui.spacing().button_padding.y)
        .max(OK_BUTTON_MIN_H)
}

/// 按可用高度算出 logo 的显示尺寸：装得下就用原尺寸；装不下就等比缩小，
/// 缩到 `LOGO_MIN_PT` 为止。
fn fit_logo_pt(ui: &egui::Ui, body_budget: f32) -> f32 {
    let natural = about_body_height(ui, LOGO_DISPLAY_PT);
    if natural <= body_budget {
        return LOGO_DISPLAY_PT;
    }
    (LOGO_DISPLAY_PT - (natural - body_budget)).clamp(LOGO_MIN_PT, LOGO_DISPLAY_PT)
}

fn info_row(ui: &mut egui::Ui, label: &str, value: &str) {
    ui.horizontal(|ui| {
        ui.add_space((ui.available_width() - 320.0).max(0.0) / 2.0);
        ui.vertical(|ui| {
            // 与 `paint_about_body` 一样把布局自带的行间距归零，改用显式的
            // `INFO_ROW_GAP`，高度才好精确计算。
            ui.spacing_mut().item_spacing.y = 0.0;
            ui.set_width(320.0);
            ui.label(egui::RichText::new(label).color(colors::TEXT_MUTED).small());
            ui.add_space(INFO_ROW_GAP);
            ui.label(egui::RichText::new(value).color(colors::TEXT_PRIMARY));
        });
    });
}

fn ok_button(ui: &mut egui::Ui) -> egui::Response {
    ui.add(
        egui::Button::new(egui::RichText::new("OK").color(colors::TEXT_PRIMARY))
            .fill(colors::ACCENT_BLUE_BG)
            .stroke(egui::Stroke::new(1.0, colors::BORDER))
            .min_size(Vec2::new(120.0, 32.0)),
    )
}

/// 关于独占窗口的标题栏高度，与 `chrome.rs` 的 `TITLE_BAR_HEIGHT` 保持一致，
/// 保证主窗口和关于窗的标题栏观感统一。
const ABOUT_TITLE_BAR_HEIGHT: f32 = 44.0;

/// 由 `App::ui` 在「来自托盘的关于」（`about_open && about_standalone`）时调用：
/// 把关于内容画成**独占整个窗口**的页面。窗口是无边框的（见 `main.rs` 的
/// `with_decorations(false)`），没有原生标题栏，所以这里自己画一条与主页风格一致的
/// 标题栏（标题 + 可拖动 + 关闭按钮）。整窗铺深色主题底（`BG_APP`），中间居中放一张
/// **固定宽度**的关于卡片。用户点关闭 / OK / 按 Esc 即视为关闭，置 `ABOUT_CLOSED`，
/// 由 `App::logic` 关掉关于层、再由 `reconcile_lifecycle` 把视口重新藏回托盘（或恢复
/// 原主窗口尺寸）。
pub fn paint_about_fullscreen(ui: &mut egui::Ui) {
    let ctx = ui.ctx().clone();
    // 主题由 `App::new` 调一次 `theme::apply` 即可，不在每帧重复设置——每帧重设
    // visuals/styles 会失效 egui 缓存、强制重新布局，鼠标在关于窗上移动时以满帧率
    // 触发重绘，把开销放大成持续卡顿。详见 skill 第 2 节。

    let logo = load_logo_texture(&ctx);
    let info = cached_info();

    // 顶部自绘标题栏：标题文字在左，关闭按钮在右；除关闭按钮区域外整条可拖动窗口。
    egui::Panel::top("about_title_bar")
        .exact_size(ABOUT_TITLE_BAR_HEIGHT)
        .resizable(false)
        .show_separator_line(false)
        .frame(egui::Frame::default().fill(colors::BG_TITLEBAR))
        .show(ui, |ui| {
            let full_rect = ui.max_rect();
            let btn_size = 32.0;
            let edge_margin = 8.0;
            let close_rect = Rect::from_center_size(
                pos2(
                    full_rect.right() - edge_margin - btn_size / 2.0,
                    full_rect.center().y,
                ),
                Vec2::splat(btn_size),
            );
            if about_title_close_button(ui, close_rect) {
                ABOUT_CLOSED.store(true, Ordering::Relaxed);
            }
            ui.horizontal_centered(|ui| {
                ui.add_space(14.0);
                // 用英文标题：egui 默认字体（default_fonts）不含中文字形，
                // 中文标题会渲染成豆腐块乱码；界面其余文案也全是英文，保持一致。
                widgets::bold_label(ui, "About CLV3000", 15.0, colors::TEXT_PRIMARY);
            });
            // 标题栏（除右侧关闭按钮区域外）可拖动窗口：文字那块叠一个拖拽区。
            // 注意用 `is_pointer_button_down_on`（按下的那一帧就发 StartDrag），
            // 不要用 `drag_started`——后者要等指针移动越过拖拽阈值才触发，此时
            // 系统的 mouseDown 事件早已过去，winit 在 macOS 上做窗口拖动依赖
            // "当前事件还是那次按下"，晚了就拖不动（表现：标题栏拖不动）。
            let drag_rect = Rect::from_min_max(
                pos2(full_rect.left(), full_rect.top()),
                pos2(close_rect.left() - 4.0, full_rect.bottom()),
            );
            if drag_rect.width() > 0.0 {
                let drag_resp = ui.interact(
                    drag_rect,
                    ui.id().with("about_titlebar_drag"),
                    Sense::drag(),
                );
                if drag_resp.is_pointer_button_down_on() {
                    ctx.send_viewport_cmd(egui::ViewportCommand::StartDrag);
                }
            }
        });

    // ── 卡片直接画在根 Ui 上（不走中央面板）────────────────────────
    //
    // 两个原因：
    // 1. 中央面板四周会留 8px 的 window_margin，卡片跟着缩进去，标题栏与
    //    卡片之间就多出一条 8px 暗色缝隙——在 macOS 上看着就是"标题栏变高了"。
    //    这里让卡片紧贴标题栏下沿（跟主窗口侧栏紧贴标题栏的观感一致）。
    // 2. 卡片高度必须**正好等于**可用区域高度：卡片矮了，窗口下方多出来的
    //    空间就是一条"黑条"（macOS 上报的问题）；卡片高了，中央面板把底部
    //    裁掉，被裁的正好是 OK 按钮下方的留白，按钮贴到窗口边（Windows 上
    //    最初报的问题）。改成自己算矩形，两个毛病一起消失。
    let view = ctx.input(|i| i.viewport_rect());
    let card_rect = Rect::from_min_max(
        pos2(CARD_EDGE_MARGIN, ABOUT_TITLE_BAR_HEIGHT),
        pos2(
            view.width() - CARD_EDGE_MARGIN,
            view.height() - CARD_EDGE_MARGIN,
        ),
    );
    // 标题栏以下的底色（卡片之外的窄边）。
    ui.painter().rect_filled(
        Rect::from_min_max(
            pos2(0.0, ABOUT_TITLE_BAR_HEIGHT),
            pos2(view.width(), view.height()),
        ),
        CornerRadius::ZERO,
        colors::BG_APP,
    );

    let margin = CARD_MARGIN;
    let inner_h = card_rect.height() - margin * 2.0;
    let content_w = card_rect.width() - margin * 2.0;
    let button_h = ok_button_height(ui);
    // 主体预算 = 卡片内高 - 按钮区（主体与按钮的间距 + 按钮 + 底部留白）。
    // 留 BODY_SAFETY 余量兜住"高度公式与实际渲染的偏差"（公式按字体行高算，
    // 实测比公式低几个像素——行高与墨迹包围盒不是一回事）：宁可 logo 略小
    // 一点，也不能让卡片超出可用高度被裁。
    let body_budget =
        (inner_h - OK_TOP_GAP - button_h - OK_BOTTOM_GAP - BODY_SAFETY).max(BODY_MIN_H);
    let logo_pt = fit_logo_pt(ui, body_budget);

    ui.scope_builder(egui::UiBuilder::new().max_rect(card_rect), |ui| {
        // 卡片在矩形里水平居中（宽度就是矩形宽，不需要额外对齐）。
        ui.with_layout(egui::Layout::top_down(egui::Align::Center), |ui| {
            Frame::default()
                .fill(colors::BG_CARD)
                .corner_radius(crate::theme::radius::LG)
                .inner_margin(egui::Margin::same(margin as i8))
                .stroke(egui::Stroke::new(1.0, colors::BORDER))
                .show(ui, |ui| {
                    ui.set_width(content_w);
                    // 卡片高度锁定：既不更矮（否则下方露出底色条），也不更高
                    // （否则底部被裁、按钮贴边）。
                    ui.set_min_height(inner_h);
                    ui.set_max_height(inner_h);
                    paint_about_body(ui, &logo, info, logo_pt);
                    // 弹性间隙：把 OK 按钮钉到卡片底部，下方留白固定。
                    // 主体高度用公式估算（`vertical_centered` 的响应矩形是
                    // 布局分配到的整块区域，不是内容实际高度，不能拿来量），
                    // 再留 BODY_SAFETY 余量兜住公式与实际渲染的偏差。
                    let body_h = about_body_height(ui, logo_pt) + BODY_SAFETY;
                    ui.add_space(
                        (inner_h - body_h - OK_TOP_GAP - button_h - OK_BOTTOM_GAP).max(0.0),
                    );
                    ui.vertical_centered(|ui| {
                        if ok_button(ui).clicked() || ui.input(|i| i.key_pressed(Key::Escape)) {
                            ABOUT_CLOSED.store(true, Ordering::Relaxed);
                        }
                    });
                    // 按钮下方补一段间隙，避免贴着卡片底部内边距显得拥挤。
                    ui.add_space(OK_BOTTOM_GAP);
                });
        });
    });
}

/// 关于窗标题栏右侧的关闭按钮（红圈 ×），点击置 `ABOUT_CLOSED`。
fn about_title_close_button(ui: &mut egui::Ui, rect: Rect) -> bool {
    let response = ui
        .interact(rect, ui.id().with("about_close"), Sense::click())
        .on_hover_cursor(CursorIcon::PointingHand);
    if response.hovered() {
        ui.painter()
            .rect_filled(rect, crate::theme::radius::SM, colors::ACCENT_BLUE_BG);
    }
    crate::icons::close(
        ui.painter(),
        rect.shrink(9.0),
        Stroke::new(1.4, colors::TEXT_SECONDARY),
    );
    response.clicked()
}

#[cfg(test)]
mod tests {
    //! egui_kittest 驱动**真实**关于视图的 UI 测试（见 AGENTS.md「UI 代码验证流程」）。

    use egui_kittest::Harness;
    use egui_kittest::kittest::Queryable;

    /// 关于独占视图：卡片必须**铺满**窗口——紧贴标题栏下沿，且一直铺到
    /// 窗口底部（只留几像素的边距）。
    ///
    /// 2026-10 用户反馈"macOS 上 About 界面下方多了一条黑条，上方标题栏高度
    /// 偏高"。根因：卡片按内容自然高度排版 + 走中央面板（面板四周留 8px
    /// window_margin），于是窗口比内容高出来的部分全落在卡片下方（看着像
    /// "黑条"），标题栏与卡片之间又多出一条 8px 缝隙（看着像"标题栏变高"）。
    ///
    /// 判据：卡片上沿紧贴标题栏下沿（≤46px），下沿铺到窗口底部（≥窗口高-10px）。
    #[test]
    fn about_card_fills_window() {
        let size = crate::app::ABOUT_WINDOW_SIZE;
        let mut harness = Harness::builder()
            .with_size(egui::Vec2::new(size[0], size[1]))
            .with_max_steps(10)
            .build_ui_state(
                |ui, _: &mut ()| {
                    super::paint_about_fullscreen(ui);
                },
                (),
            );
        crate::theme::apply(&harness.ctx);
        harness.run();

        // 卡片矩形 = 背景色为 BG_CARD 的那个填充矩形（取面积最大的一个）。
        let mut card: Option<egui::Rect> = None;
        for shape in &harness.output().shapes {
            if let egui::epaint::Shape::Rect(r) = &shape.shape {
                let f = r.fill;
                if (f.r(), f.g(), f.b()) == (24, 29, 38) {
                    let rect = r.rect;
                    if card.is_none_or(|c| rect.area() > c.area()) {
                        card = Some(rect);
                    }
                }
            }
        }
        let card = card.expect("应能找到关于卡片的填充矩形");
        assert!(
            card.min.y <= 46.0,
            "卡片应紧贴标题栏下沿（44px），实际 top={:.1}",
            card.min.y
        );
        assert!(
            card.max.y >= size[1] - 10.0,
            "卡片应铺到窗口底部，实际 bottom={:.1}，窗口高 {:.1}（下方多了一条底色条）",
            card.max.y,
            size[1]
        );
    }

    /// 关于独占视图：OK 按钮下方必须留出完整间隙，不能贴到窗口底边。
    ///
    /// 2026-10 用户反馈"Windows 的 About 视图里 OK 按钮太贴下边了，macOS 上
    /// 没问题"。
    ///
    /// 根因：关于卡片按内容自然排版（自然高 ≈452px），而窗口扣掉自绘标题栏
    /// （44px）和中央面板边距后只剩 412px 可用——卡片超出可用高度，中央面板
    /// 把它**底部裁掉**，被裁掉的正好是卡片下内边距和"OK 按钮下方的额外留白"，
    /// 按钮就贴到了窗口底边（这 40px 恰好就是按钮下方该有的留白）。
    ///
    /// 判据：OK 按钮的下沿离窗口底沿必须 ≥ 卡片内边距 + 额外留白（28+12=40px）
    /// 再留一点余量。两种窗口高度都要测：正式的 528，以及更矮的 472
    /// （模拟各平台客户区高度不一致的情况）。
    #[test]
    fn about_ok_button_keeps_bottom_gap() {
        for size in [crate::app::ABOUT_WINDOW_SIZE, [480.0f32, 472.0f32]] {
            let (w, h) = (size[0], size[1]);
            let mut harness = Harness::builder()
                .with_size(egui::Vec2::new(w, h))
                .with_max_steps(10)
                .build_ui_state(
                    |ui, _: &mut ()| {
                        super::paint_about_fullscreen(ui);
                    },
                    (),
                );
            crate::theme::apply(&harness.ctx);
            harness.run();

            let ok = harness.get_by_label("OK").rect();
            let gap = h - ok.max.y;
            assert!(
                gap >= 40.0,
                "OK 按钮太贴窗口底边：窗口 {w}x{h}，按钮下沿 y={:.1}，只留了 {gap:.1}px",
                ok.max.y,
            );
        }
    }
}
