//! 可复用 UI 组件：圆环进度、状态胶囊、威胁卡片、Toast 提示。

use crate::theme::colors;
use egui::{Align2, Color32, FontId, Pos2, Response, Sense, Stroke, Ui, Vec2, epaint::PathStroke};

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
    const MAX_ALPHA: u8 = 16; // 最内层（最浓）的透明度，故意压得很低，避免糊成一片

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
    let (response, painter) =
        ui.allocate_painter(Vec2::splat(diameter), Sense::hover());
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
            draw_arc(&painter, center, radius, ring_width, start_angle, sweep, ring_color);
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

    painter.text(
        center + Vec2::new(0.0, -diameter * 0.04),
        Align2::CENTER_CENTER,
        center_title,
        FontId::proportional(diameter * 0.16),
        colors::TEXT_PRIMARY,
    );
    if !center_sub.is_empty() {
        painter.text(
            center + Vec2::new(0.0, diameter * 0.16),
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
    painter.add(egui::Shape::line(points, PathStroke::from(Stroke::new(width, color))));
}

/// 一个小圆角胶囊：数值 + 说明文字，比如 "128 / 342  进程"。
pub fn stat_pill(ui: &mut Ui, value: &str, label: &str) {
    egui::Frame::default()
        .fill(colors::BG_CARD)
        .stroke(Stroke::new(1.0, colors::BORDER))
        .corner_radius(999.0)
        .inner_margin(egui::Margin::symmetric(16, 8))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                bold_label(ui, value, 14.0, colors::TEXT_PRIMARY);
                ui.label(egui::RichText::new(label).color(colors::TEXT_SECONDARY).small());
            });
        });
}

/// 量出一段文字在给定字号下的真实渲染宽度。用来在摆放前就知道"这块东西到底
/// 多宽"，而不是猜一个固定值——扫描过程中数字会变（比如 "5 / 342" 变成
/// "128 / 342"），猜的固定宽度会跟着跑偏，量出来的不会。
pub fn measure_text_width(ui: &Ui, text: &str, size: f32) -> f32 {
    ui.ctx().fonts_mut(|f| {
        f.layout_no_wrap(text.to_owned(), FontId::proportional(size), Color32::WHITE)
    })
    .size()
    .x
}

/// `stat_pill` 实际渲染宽度的估算：内边距(16*2) + 边框(1*2) + value/label 间距(10，
/// 全局 item_spacing) + 两段文字宽度。value 走的是 `bold_label`（横向多描 0.6px）。
fn stat_pill_width(ui: &Ui, value: &str, label: &str) -> f32 {
    let value_w = measure_text_width(ui, value, 14.0) + 0.6;
    // `.small()` 在 theme.rs 里被覆盖成跟 Body 一样大（14px），不是 egui 默认的缩小字号，
    // 量的时候要用同一个 14.0，不能按"small 应该更小"的直觉去猜一个缩放系数。
    let label_w = measure_text_width(ui, label, 14.0);
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
    ui.allocate_ui_with_layout(desired, egui::Layout::left_to_right(egui::Align::Center), |ui| {
        for (i, (value, label)) in pills.iter().enumerate() {
            if i > 0 {
                ui.add_space(GAP);
            }
            stat_pill(ui, value, label);
        }
    });
}

pub enum ThreatAction {
    None,
    Quarantine,
    Ignore,
}

/// 威胁卡片上"隔离"/"忽略"用的小胶囊按钮。不用 `egui::Button` 是因为想让两个按钮
/// 的尺寸/圆角/居中方式完全统一可控（`egui::Button` 默认样式在深色红卡片背景上
/// 对比度不太对，两个按钮风格也不一致）。`filled=true` 是红底白字的"隔离"，
/// `filled=false` 是描边的"忽略"。
fn pill_button(ui: &mut Ui, label: &str, filled: bool) -> bool {
    const H_PAD: f32 = 14.0;
    const V_PAD: f32 = 7.0;

    let text_color = if filled { Color32::WHITE } else { colors::TEXT_SECONDARY };
    let font_id = FontId::proportional(13.0);
    let galley = ui
        .ctx()
        .fonts_mut(|f| f.layout_no_wrap(label.to_owned(), font_id, text_color));
    let text_size = galley.size();
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
        .interact(bg_rect, response.id.with("pill"), Sense::click())
        .on_hover_cursor(egui::CursorIcon::PointingHand);
    let (fill, stroke) = if filled {
        let fill = if interact.hovered() {
            Color32::from_rgb(224, 74, 74)
        } else {
            colors::RED
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
        egui::CornerRadius::same(255),
        fill,
        stroke,
        egui::epaint::StrokeKind::Inside,
    );
    ui.painter().set(bg_idx, egui::Shape::Rect(shape));

    interact.clicked()
}

/// 威胁详情卡片：红色警示样式 + 隔离/忽略按钮。
pub fn threat_card(ui: &mut Ui, virus_name: &str, path: &str) -> ThreatAction {
    let mut action = ThreatAction::None;
    egui::Frame::default()
        .fill(colors::RED_BG)
        .stroke(Stroke::new(1.0, colors::RED_BORDER))
        .corner_radius(12.0)
        .inner_margin(egui::Margin::same(14))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                let icon_size = 39.0;
                let (icon_response, painter) =
                    ui.allocate_painter(Vec2::splat(icon_size), Sense::hover());
                let icon_rect = icon_response.rect;
                painter.rect_filled(icon_rect, 8.0, colors::RED);
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
                        bold_label(ui, virus_name, 15.0, colors::TEXT_PRIMARY);
                        egui::Frame::default()
                            .fill(colors::RED)
                            .corner_radius(999.0)
                            .inner_margin(egui::Margin::symmetric(8, 2))
                            .show(ui, |ui| {
                                ui.label(egui::RichText::new("High Risk").color(Color32::WHITE).small());
                            });
                    });
                    ui.label(
                        egui::RichText::new(truncate_middle(path, 46))
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

pub fn show_toasts(ctx: &egui::Context, toasts: &[Toast]) {
    if toasts.is_empty() {
        return;
    }
    egui::Area::new(egui::Id::new("clv3000_toasts"))
        .anchor(egui::Align2::RIGHT_BOTTOM, egui::vec2(-20.0, -20.0))
        .order(egui::Order::Tooltip)
        .show(ctx, |ui| {
            ui.vertical(|ui| {
                for toast in toasts {
                    egui::Frame::default()
                        .fill(colors::BG_CARD)
                        .stroke(Stroke::new(1.0, colors::BORDER))
                        .corner_radius(8.0)
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
