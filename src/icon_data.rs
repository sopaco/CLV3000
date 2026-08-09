//! 程序图标：两份正式美术图标都编译时用 `include_bytes!` 直接嵌进二进制（不大，
//! 嵌进去比运行时再去找文件路径可靠——不会有"装到别的目录、图标文件没跟着走"
//! 这种问题）。
//!
//! - `icon_app.png`：带"CLV3000"字样、圆角方块背景的完整品牌图标，细节多，
//!   用在"关于"页这种可以画大一点的地方。
//! - `icon_tray.png`：去掉文字/背景的简化版（纯盾牌+闪电），线条粗一点，缩到
//!   16~32px 的托盘/任务栏尺寸也认得清楚，用在窗口图标和系统托盘。
//!
//! `shield_rgba` 是没有正式图标之前手绘的占位方案，现在只当解码失败时的兜底。

static APP_ICON_PNG: &[u8] = include_bytes!("../assets/icons/icon_app.png");
static TRAY_ICON_PNG: &[u8] = include_bytes!("../assets/icons/icon_tray.png");

/// 解码内嵌的完整品牌图标并缩放到 `size x size`，返回 `(rgba, width, height)`。
/// 源图是 1254x1254 的，比实际需要的尺寸大得多，解码后统一缩小，不然平白占内存
/// （老机器友好）。
pub fn load_app_icon(size: u32) -> (Vec<u8>, u32, u32) {
    decode_and_resize(APP_ICON_PNG, size)
}

/// 解码内嵌的简化版图标（适合小尺寸），返回 `(rgba, width, height)`。
/// 窗口图标建议大一点（比如 128），托盘图标按系统惯例给小图（比如 32）。
pub fn load_tray_icon(size: u32) -> (Vec<u8>, u32, u32) {
    decode_and_resize(TRAY_ICON_PNG, size)
}

fn decode_and_resize(png_bytes: &[u8], size: u32) -> (Vec<u8>, u32, u32) {
    match image::load_from_memory(png_bytes) {
        Ok(img) => {
            // Triangle 比 Lanczos3 便宜很多，缩小到图标这种尺寸肉眼看不出差别，
            // 老机器上启动时这一下缩放能省不少时间。
            let resized = img.resize_exact(size, size, image::imageops::FilterType::Triangle);
            let rgba = resized.to_rgba8();
            let (w, h) = rgba.dimensions();
            (rgba.into_raw(), w, h)
        }
        Err(_) => {
            // 正常情况下编译时打包的图片不会解码失败，这里只是防御性兜底。
            (
                shield_rgba(size, [58, 160, 224, 255], [0, 0, 0, 0]),
                size,
                size,
            )
        }
    }
}

/// 盾牌形状顶点（和 icons::shield 用的是同一组归一化坐标）。
const SHIELD_POINTS: [(f32, f32); 6] = [
    (0.50, 0.03),
    (0.90, 0.16),
    (0.85, 0.55),
    (0.50, 0.97),
    (0.15, 0.55),
    (0.10, 0.16),
];

/// 生成一张 `size x size` 的 RGBA8 盾牌图标。
pub fn shield_rgba(size: u32, fg: [u8; 4], bg: [u8; 4]) -> Vec<u8> {
    let mut buf = vec![0u8; (size * size * 4) as usize];
    for y in 0..size {
        for x in 0..size {
            let u = (x as f32 + 0.5) / size as f32;
            let v = (y as f32 + 0.5) / size as f32;
            let inside = point_in_polygon(u, v, &SHIELD_POINTS);
            let color = if inside { fg } else { bg };
            let idx = ((y * size + x) * 4) as usize;
            buf[idx..idx + 4].copy_from_slice(&color);
        }
    }
    buf
}

fn point_in_polygon(x: f32, y: f32, poly: &[(f32, f32)]) -> bool {
    let mut inside = false;
    let n = poly.len();
    let mut j = n - 1;
    for i in 0..n {
        let (xi, yi) = poly[i];
        let (xj, yj) = poly[j];
        if (yi > y) != (yj > y) {
            let x_intersect = xi + (y - yi) / (yj - yi) * (xj - xi);
            if x < x_intersect {
                inside = !inside;
            }
        }
        j = i;
    }
    inside
}
