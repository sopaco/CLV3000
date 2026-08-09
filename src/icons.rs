//! 手绘矢量图标集：不依赖任何图标字体/emoji 渲染，用 egui::Painter 的基础图元
//! （折线、多边形、圆）直接画，保证在任何机器上样式都一致。
//!
//! 所有函数都在 `rect` 范围内按统一比例作画，`rect` 建议传正方形。

use egui::{Color32, Painter, Pos2, Rect, Shape, Stroke, epaint::PathStroke};

fn map(rect: Rect, u: f32, v: f32) -> Pos2 {
    Pos2::new(
        rect.left() + u * rect.width(),
        rect.top() + v * rect.height(),
    )
}

fn polyline(points: &[Pos2], stroke: Stroke, closed: bool) -> Shape {
    let mut pts = points.to_vec();
    if closed {
        pts.push(points[0]);
    }
    Shape::line(pts, PathStroke::from(stroke))
}

/// 盾牌轮廓（六边形近似）。
pub fn shield(painter: &Painter, rect: Rect, stroke: Stroke, fill: Option<Color32>) {
    let pts: Vec<Pos2> = [
        (0.50, 0.03),
        (0.90, 0.16),
        (0.85, 0.55),
        (0.50, 0.97),
        (0.15, 0.55),
        (0.10, 0.16),
    ]
    .iter()
    .map(|&(u, v)| map(rect, u, v))
    .collect();

    if let Some(fill) = fill {
        painter.add(Shape::convex_polygon(pts.clone(), fill, Stroke::NONE));
    }
    painter.add(polyline(&pts, stroke, true));
}

/// 盾牌 + 中间一个勾选标记，仪表盘"安全"状态用。
pub fn shield_check(painter: &Painter, rect: Rect, stroke: Stroke) {
    shield(painter, rect, stroke, None);
    let check: Vec<Pos2> = [(0.32, 0.52), (0.46, 0.68), (0.72, 0.34)]
        .iter()
        .map(|&(u, v)| map(rect, u, v))
        .collect();
    painter.add(polyline(&check, stroke, false));
}

/// 警告三角形 + 感叹号，发现威胁状态用。
pub fn warning_triangle(painter: &Painter, rect: Rect, stroke: Stroke, fill: Option<Color32>) {
    let pts: Vec<Pos2> = [(0.5, 0.06), (0.94, 0.90), (0.06, 0.90)]
        .iter()
        .map(|&(u, v)| map(rect, u, v))
        .collect();
    if let Some(fill) = fill {
        painter.add(Shape::convex_polygon(pts.clone(), fill, Stroke::NONE));
    }
    painter.add(polyline(&pts, stroke, true));

    // 感叹号：一条竖线 + 一个点。
    let top = map(rect, 0.5, 0.38);
    let bottom = map(rect, 0.5, 0.62);
    painter.line_segment([top, bottom], stroke);
    painter.circle_filled(map(rect, 0.5, 0.76), stroke.width * 1.1, stroke.color);
}

/// 闪电图标。
pub fn bolt(painter: &Painter, rect: Rect, fill: Color32) {
    let pts: Vec<Pos2> = [
        (0.58, 0.02),
        (0.18, 0.56),
        (0.44, 0.56),
        (0.36, 0.98),
        (0.82, 0.40),
        (0.54, 0.40),
    ]
    .iter()
    .map(|&(u, v)| map(rect, u, v))
    .collect();
    painter.add(Shape::convex_polygon(pts, fill, Stroke::NONE));
}

fn ellipse_points(center: Pos2, rx: f32, ry: f32, segments: usize) -> Vec<Pos2> {
    (0..=segments)
        .map(|i| {
            let t = i as f32 / segments as f32 * std::f32::consts::TAU;
            Pos2::new(center.x + rx * t.cos(), center.y + ry * t.sin())
        })
        .collect()
}

/// "数据库"图标：叠起来的圆柱体，代表病毒库。
pub fn database(painter: &Painter, rect: Rect, stroke: Stroke) {
    let cx = rect.center().x;
    let rx = rect.width() * 0.40;
    let ry = rect.height() * 0.12;
    // 上下留白对称，不然整个图标看起来会偏下，跟旁边的文字/图标对不齐。
    let top_y = rect.top() + rect.height() * 0.14;
    let bottom_y = rect.bottom() - rect.height() * 0.14;

    let top_center = Pos2::new(cx, top_y);
    let bottom_center = Pos2::new(cx, bottom_y);

    // 顶部整圈椭圆。
    painter.add(Shape::closed_line(
        ellipse_points(top_center, rx, ry, 24),
        stroke,
    ));
    // 中间一条"腰线"，暗示是叠起来的圆柱体。
    let mid_center = Pos2::new(cx, (top_y + bottom_y) / 2.0);
    let mid_arc: Vec<Pos2> = ellipse_points(mid_center, rx, ry, 24)
        .into_iter()
        .filter(|p| p.y >= mid_center.y)
        .collect();
    painter.add(polyline(&mid_arc, stroke, false));
    // 底部半圈弧线（只画看得到的下半部分）。
    let bottom_arc: Vec<Pos2> = ellipse_points(bottom_center, rx, ry, 24)
        .into_iter()
        .filter(|p| p.y >= bottom_center.y)
        .collect();
    painter.add(polyline(&bottom_arc, stroke, false));
    // 两侧竖线，连接顶部椭圆和底部弧线。
    painter.line_segment(
        [
            Pos2::new(cx - rx, top_y),
            Pos2::new(cx - rx, bottom_center.y),
        ],
        stroke,
    );
    painter.line_segment(
        [
            Pos2::new(cx + rx, top_y),
            Pos2::new(cx + rx, bottom_center.y),
        ],
        stroke,
    );
}

/// 汉堡菜单（三条横线），对应"全盘扫描"侧边栏入口。
pub fn hamburger(painter: &Painter, rect: Rect, stroke: Stroke) {
    for v in [0.28, 0.5, 0.72] {
        painter.line_segment([map(rect, 0.12, v), map(rect, 0.88, v)], stroke);
    }
}

/// 标题栏"最小化"按钮图标：一条横线。
pub fn minimize(painter: &Painter, rect: Rect, stroke: Stroke) {
    painter.line_segment([map(rect, 0.2, 0.5), map(rect, 0.8, 0.5)], stroke);
}

/// 标题栏"关闭"按钮图标：一个 X。
pub fn close(painter: &Painter, rect: Rect, stroke: Stroke) {
    painter.line_segment([map(rect, 0.2, 0.2), map(rect, 0.8, 0.8)], stroke);
    painter.line_segment([map(rect, 0.8, 0.2), map(rect, 0.2, 0.8)], stroke);
}
