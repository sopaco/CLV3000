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

/// 采样一段二次贝塞尔曲线（`p0` 到 `p2`，`p1` 是控制点），返回 `segments+1` 个点。
fn quad_bezier(p0: Pos2, p1: Pos2, p2: Pos2, segments: usize) -> Vec<Pos2> {
    (0..=segments)
        .map(|i| {
            let t = i as f32 / segments as f32;
            let mt = 1.0 - t;
            Pos2::new(
                mt * mt * p0.x + 2.0 * mt * t * p1.x + t * t * p2.x,
                mt * mt * p0.y + 2.0 * mt * t * p1.y + t * t * p2.y,
            )
        })
        .collect()
}

/// 盾牌轮廓的控制点：经典"盾形徽章"比例——顶部中点微微隆起，两侧圆角肩部，
/// 直落到最宽处，再平滑收窄成一个圆润的底尖，是最常见、最容易辨认的盾牌形状
/// （对标 Feather/Heroicons 的 shield 图标），刻意不做腰部凹陷之类的花活，
/// 保持简洁。6 段二次贝塞尔首尾相接、从正上方开始顺时针绕一圈，天然闭合
/// （最后一段终点等于起点）。
fn shield_outline(rect: Rect, segments_per_curve: usize) -> Vec<Pos2> {
    const CURVES: [((f32, f32), (f32, f32)); 6] = [
        ((0.70, 0.06), (0.86, 0.17)),  // 顶部中点 -> 右肩角
        ((0.90, 0.30), (0.865, 0.50)), // 右肩角 -> 右侧最宽处
        ((0.80, 0.76), (0.50, 0.95)),  // 右侧最宽处 -> 底部尖
        ((0.20, 0.76), (0.135, 0.50)), // 底部尖 -> 左侧最宽处
        ((0.10, 0.30), (0.14, 0.17)),  // 左侧最宽处 -> 左肩角
        ((0.30, 0.06), (0.50, 0.05)),  // 左肩角 -> 顶部中点
    ];

    let mut cur = map(rect, 0.50, 0.05);
    let mut pts = vec![cur];
    for &((cu, cv), (eu, ev)) in &CURVES {
        let ctrl = map(rect, cu, cv);
        let end = map(rect, eu, ev);
        pts.extend(quad_bezier(cur, ctrl, end, segments_per_curve).into_iter().skip(1));
        cur = end;
    }
    pts
}

/// 盾牌轮廓。
pub fn shield(painter: &Painter, rect: Rect, stroke: Stroke, fill: Option<Color32>) {
    let pts = shield_outline(rect, 8);
    if let Some(fill) = fill {
        painter.add(Shape::convex_polygon(pts.clone(), fill, Stroke::NONE));
    }
    // 轮廓采样点首尾已经重合（闭合曲线），直接画折线就是闭合的，不用再补一段。
    painter.add(Shape::line(pts, PathStroke::from(stroke)));
}

fn path_stroke(stroke: Stroke) -> PathStroke {
    PathStroke::new(stroke.width, stroke.color)
}

/// 大圆环内的"安全"状态图形：半透明填充 + 高对比勾选，避免纯线框在大尺寸下显得空/单薄。
pub fn status_glyph_secure(painter: &Painter, rect: Rect, color: Color32) {
    const SHIELD_SCALE: f32 = 1.25;
    let shield_rect = Rect::from_center_size(rect.center(), rect.size() * SHIELD_SCALE);

    let body_fill = Color32::from_rgba_unmultiplied(color.r(), color.g(), color.b(), 38);
    let outline = Stroke::new(2.2, color);
    shield(painter, shield_rect, outline, Some(body_fill));
    let check: Vec<Pos2> = [(0.30, 0.50), (0.44, 0.66), (0.74, 0.32)]
        .iter()
        .map(|&(u, v)| map(rect, u, v))
        .collect();
    let mark = Stroke::new(3.0, Color32::from_rgb(245, 247, 250));
    painter.add(Shape::line(check, path_stroke(mark)));
}

/// 大圆环内的"风险"状态图形，风格与 [`status_glyph_secure`] 对称。
pub fn status_glyph_at_risk(painter: &Painter, rect: Rect, color: Color32) {
    let body_fill = Color32::from_rgba_unmultiplied(color.r(), color.g(), color.b(), 38);
    let outline = Stroke::new(2.2, color);
    warning_triangle(painter, rect, outline, Some(body_fill));
    let mark = Stroke::new(2.8, Color32::from_rgb(245, 247, 250));
    let top = map(rect, 0.50, 0.36);
    let bottom = map(rect, 0.50, 0.60);
    painter.line_segment([top, bottom], mark);
    painter.circle_filled(map(rect, 0.50, 0.74), mark.width * 0.55, mark.color);
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
    let bottom_arc: Vec<Pos2> = ellipse_points(bottom_center, rx, ry, 24)
        .into_iter()
        .filter(|p| p.y >= bottom_center.y)
        .collect();
    painter.add(polyline(&bottom_arc, stroke, false));
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

/// 电脑图标：显示器外框 + 内屏 + 支架，正方形画布内对称，表达"整机/全盘扫描"。
pub fn computer(painter: &Painter, rect: Rect, stroke: Stroke) {
    let bezel: Vec<Pos2> = [(0.18, 0.14), (0.82, 0.14), (0.82, 0.56), (0.18, 0.56)]
        .iter()
        .map(|&(u, v)| map(rect, u, v))
        .collect();
    painter.add(polyline(&bezel, stroke, true));

    let screen: Vec<Pos2> = [(0.26, 0.22), (0.74, 0.22), (0.74, 0.48), (0.26, 0.48)]
        .iter()
        .map(|&(u, v)| map(rect, u, v))
        .collect();
    painter.add(polyline(&screen, stroke, true));

    painter.line_segment([map(rect, 0.50, 0.56), map(rect, 0.50, 0.68)], stroke);
    painter.line_segment([map(rect, 0.34, 0.74), map(rect, 0.66, 0.74)], stroke);
}

/// "文件夹"图标：线框风格，前片比后片矮一点，暗示"翻盖"的立体感。"打开所在
/// 文件夹"按钮用。
pub fn folder(painter: &Painter, rect: Rect, stroke: Stroke) {
    let pts: Vec<Pos2> = [
        (0.10, 0.24),
        (0.10, 0.20),
        (0.36, 0.20),
        (0.46, 0.34),
        (0.90, 0.34),
        (0.90, 0.80),
        (0.10, 0.80),
    ]
    .iter()
    .map(|&(u, v)| map(rect, u, v))
    .collect();
    painter.add(polyline(&pts, stroke, true));
}

/// 标题栏"最小化"按钮图标：一条横线（仅 macOS 自绘标题栏使用）。
#[cfg(not(windows))]
pub fn minimize(painter: &Painter, rect: Rect, stroke: Stroke) {
    painter.line_segment([map(rect, 0.2, 0.5), map(rect, 0.8, 0.5)], stroke);
}

/// 标题栏"关闭"按钮图标：一个 X（仅 macOS 自绘标题栏使用）。
#[cfg(not(windows))]
pub fn close(painter: &Painter, rect: Rect, stroke: Stroke) {
    painter.line_segment([map(rect, 0.2, 0.2), map(rect, 0.8, 0.8)], stroke);
    painter.line_segment([map(rect, 0.8, 0.2), map(rect, 0.2, 0.8)], stroke);
}
