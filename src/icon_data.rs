//! 程序图标：两份正式美术图标都编译时用 `include_bytes!` 直接嵌进二进制（不大，
//! 嵌进去比运行时再去找文件路径可靠——不会有"装到别的目录、图标文件没跟着走"
//! 这种问题）。
//!
//! - `icon_app.png`：带"CLV3000"字样、圆角方块背景的完整品牌图标，细节多，
//!   用在"关于"页这种可以画大一点的地方（源图约 512×504）。
//! - `icon_tray.png`：去掉文字/背景的简化版（纯盾牌+闪电），线条粗一点，缩到
//!   16~32px 的托盘/任务栏尺寸也认得清楚，用在窗口图标和系统托盘。
//!
//! `shield_rgba` 是没有正式图标之前手绘的占位方案，现在只当解码失败时的兜底。

use image::imageops::FilterType;

static APP_ICON_PNG: &[u8] = include_bytes!("../assets/icons/icon_app.png");
static TRAY_ICON_PNG: &[u8] = include_bytes!("../assets/icons/icon_tray.png");

/// 内嵌 `icon_app.png` 的最长边（像素），缩放目标不超过此值以免无意义放大。
const APP_ICON_SOURCE_MAX: u32 = 512;

/// 逻辑点 → 物理像素（向上取整），供 HiDPI 屏按真实像素密度加载纹理。
pub fn physical_pixels(logical: f32, pixels_per_point: f32) -> u32 {
    ((logical * pixels_per_point).ceil() as u32).max(1)
}

/// 按 UI 上的逻辑尺寸和当前 `pixels_per_point` 解码品牌图标。
///
/// 在 Retina 等高分屏上会加载比逻辑尺寸更大的纹理，避免 GPU 二次放大导致发糊；
/// 缩放使用 Lanczos3，比托盘小图用的 Triangle 更锐利。
pub fn load_app_icon_for_display(
    logical_size: f32,
    pixels_per_point: f32,
) -> (Vec<u8>, u32, u32) {
    let target = physical_pixels(logical_size, pixels_per_point).min(APP_ICON_SOURCE_MAX);
    decode_and_resize(APP_ICON_PNG, target, FilterType::Lanczos3)
}

/// 解码内嵌的简化版图标（适合小尺寸），返回 `(rgba, width, height)`。
/// 窗口图标建议大一点（比如 128），托盘图标按系统惯例给小图（比如 32）。
pub fn load_tray_icon(size: u32) -> (Vec<u8>, u32, u32) {
    decode_and_resize(TRAY_ICON_PNG, size, FilterType::Triangle)
}

fn decode_and_resize(png_bytes: &[u8], size: u32, filter: FilterType) -> (Vec<u8>, u32, u32) {
    match image::load_from_memory(png_bytes) {
        Ok(img) => {
            let resized = img.resize_exact(size, size, filter);
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
