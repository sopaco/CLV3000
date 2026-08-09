//! 可复用 UI 组件：圆环进度、状态徽标、威胁卡片、Toast 提示。

use crate::theme::colors;
use egui::{Align2, Color32, FontId, Pos2, Response, Sense, Stroke, Ui, Vec2, epaint::PathStroke};

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
    let (response, painter) =
        ui.allocate_painter(Vec2::splat(diameter), Sense::hover());
    let rect = response.rect;
    let center = rect.center();
    let radius = diameter / 2.0 - 6.0;
    let ring_width = (diameter * 0.045).max(4.0);

    painter.circle_stroke(
        center,
        radius,
        Stroke::new(ring_width, Color32::from_rgba_premultiplied(255, 255, 255, 18)),
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
            ui.ctx().request_repaint();
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
                ui.label(egui::RichText::new(value).color(colors::TEXT_PRIMARY).strong());
                ui.label(egui::RichText::new(label).color(colors::TEXT_SECONDARY).small());
            });
        });
}

pub enum ThreatAction {
    None,
    Quarantine,
    Ignore,
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
                let icon_size = 34.0;
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
                        ui.label(
                            egui::RichText::new(virus_name)
                                .color(colors::TEXT_PRIMARY)
                                .strong()
                                .size(15.0),
                        );
                        egui::Frame::default()
                            .fill(colors::RED)
                            .corner_radius(999.0)
                            .inner_margin(egui::Margin::symmetric(8, 2))
                            .show(ui, |ui| {
                                ui.label(egui::RichText::new("高危").color(Color32::WHITE).small());
                            });
                    });
                    ui.label(
                        egui::RichText::new(truncate_middle(path, 46))
                            .color(colors::TEXT_SECONDARY)
                            .small(),
                    );
                });

                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui
                        .add(egui::Button::new(
                            egui::RichText::new("忽略").color(colors::TEXT_SECONDARY),
                        ))
                        .clicked()
                    {
                        action = ThreatAction::Ignore;
                    }
                    let quarantine_btn = egui::Button::new(
                        egui::RichText::new("隔离").color(Color32::WHITE).strong(),
                    )
                    .fill(colors::RED);
                    if ui.add(quarantine_btn).clicked() {
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
