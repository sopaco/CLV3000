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
use egui::{Align2, Frame, Key, TextureHandle, Vec2};
// 以下导入仅 macOS 自绘标题栏使用（Windows 用系统标题栏，不编译相关代码）。
#[cfg(not(windows))]
use egui::{pos2, CursorIcon, Rect, Sense, Stroke};
use std::sync::atomic::{AtomicBool, Ordering};

const LOGO_TEX_ID: &str = "clv3000_about_logo";
/// 关于弹窗 logo 的 UI 逻辑尺寸（pt）。
const LOGO_DISPLAY_PT: f32 = 88.0;

const SIZE: [f32; 2] = [410.0, 340.0];

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
            paint_about_body(ui, &logo, info);
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

fn paint_about_body(ui: &mut egui::Ui, logo: &TextureHandle, info: &ClamAvInfo) {
    ui.vertical_centered(|ui| {
        ui.add_space(5.0);
        ui.add(
            egui::Image::new((logo.id(), logo.size_vec2()))
                .fit_to_exact_size(Vec2::splat(LOGO_DISPLAY_PT))
                .corner_radius(14.0),
        );
        ui.add_space(10.0);
        widgets::bold_label(ui, "CLV3000", 20.0, colors::TEXT_PRIMARY);
        ui.label(
            egui::RichText::new(format!("Version {}", env!("CARGO_PKG_VERSION")))
                .color(colors::TEXT_SECONDARY)
                .small(),
        );
        ui.add_space(14.0);
        info_row(ui, "ClamAV Engine", &info.engine);
        ui.add_space(6.0);
        info_row(ui, "Virus Database", &info.database);
        ui.add_space(6.0);
        ui.label(
            egui::RichText::new("Fast, reliable virus protection for even older PCs.")
                .color(colors::TEXT_MUTED)
                .small(),
        );
    });
}

fn info_row(ui: &mut egui::Ui, label: &str, value: &str) {
    ui.horizontal(|ui| {
        ui.add_space((ui.available_width() - 320.0).max(0.0) / 2.0);
        ui.vertical(|ui| {
            ui.set_width(320.0);
            ui.label(egui::RichText::new(label).color(colors::TEXT_MUTED).small());
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

/// 关于独占窗口的标题栏高度，与 `app.rs` 的 `TITLE_BAR_HEIGHT` 保持一致，
/// 保证主窗口和关于窗的标题栏观感统一。仅在 macOS 自绘标题栏时使用。
#[cfg(not(windows))]
const ABOUT_TITLE_BAR_HEIGHT: f32 = 44.0;

/// 由 `App::ui` 在「来自托盘的关于」（`about_open && about_standalone`）时调用：
/// 把关于内容画成**独占整个窗口**的页面。macOS 下窗口是无边框的（见 `main.rs` 的
/// `with_decorations(false)`），没有原生标题栏，所以这里自己画一条与主页风格一致的
/// 标题栏（标题 + 可拖动 + 关闭按钮），否则关于窗顶部会"没有标题栏"。Windows 下窗口
/// 用系统标题栏（`with_decorations(true)`），不画自绘标题栏，由系统提供拖动/关闭。
/// 整窗铺深色主题底（`BG_APP`），中间居中放一张**固定宽度**的关于卡片。用户点关闭 /
/// OK / 按 Esc 即视为关闭，置 `ABOUT_CLOSED`，由 `App::logic` 关掉关于层、再由
/// `reconcile_lifecycle` 把视口重新藏回托盘（或恢复原主窗口尺寸）。
pub fn paint_about_fullscreen(ui: &mut egui::Ui) {
    let ctx = ui.ctx().clone();
    // 主题由 `App::new` 调一次 `theme::apply` 即可，不在每帧重复设置——每帧重设
    // visuals/styles 会失效 egui 缓存、强制重新布局，鼠标在关于窗上移动时以满帧率
    // 触发重绘，把开销放大成持续卡顿。详见 skill 第 2 节。

    let logo = load_logo_texture(&ctx);
    let info = cached_info();

    // 顶部自绘标题栏：仅 macOS 使用（无边框窗口需要自绘）。Windows 用系统标题栏，
    // 不画这条 Panel，否则会与系统标题栏重复。标题文字在左，关闭按钮在右；除关闭
    // 按钮区域外整条可拖动窗口。
    #[cfg(not(windows))]
    {
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
    }

    egui::CentralPanel::default()
        .frame(Frame::default().fill(colors::BG_APP))
        .show(ui, |ui| {
            // 整窗居中：水平 + 垂直都居中一张固定宽度的卡片（不铺满整窗）。
            let avail = ui.available_size();
            let card_w = (avail.x - 48.0).clamp(360.0, 440.0);
            ui.with_layout(
                egui::Layout::centered_and_justified(egui::Direction::TopDown),
                |ui| {
                    Frame::default()
                        .fill(colors::BG_CARD)
                        .corner_radius(16.0)
                        .inner_margin(egui::Margin::same(28))
                        .stroke(egui::Stroke::new(1.0, colors::BORDER))
                        .show(ui, |ui| {
                            ui.set_width(card_w);
                            paint_about_body(ui, &logo, info);
                            ui.add_space(18.0);
                            ui.vertical_centered(|ui| {
                                if ok_button(ui).clicked()
                                    || ui.input(|i| i.key_pressed(Key::Escape))
                                {
                                    ABOUT_CLOSED.store(true, Ordering::Relaxed);
                                }
                            });
                        });
                },
            );
        });
}

/// 关于窗标题栏右侧的关闭按钮（红圈 ×），点击置 `ABOUT_CLOSED`。
/// 仅 macOS 自绘标题栏使用；Windows 由系统标题栏提供关闭按钮。
#[cfg(not(windows))]
fn about_title_close_button(ui: &mut egui::Ui, rect: Rect) -> bool {
    let response = ui
        .interact(rect, ui.id().with("about_close"), Sense::click())
        .on_hover_cursor(CursorIcon::PointingHand);
    if response.hovered() {
        ui.painter().rect_filled(rect, 6.0, colors::ACCENT_BLUE_BG);
    }
    crate::icons::close(
        ui.painter(),
        rect.shrink(9.0),
        Stroke::new(1.4, colors::TEXT_SECONDARY),
    );
    response.clicked()
}
