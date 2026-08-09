//! 程序图标：正式美术图标在 `assets/icons/icon_app.png`，编译时用 `include_bytes!`
//! 直接嵌进二进制（图标文件不大，嵌进去比运行时再去找文件路径可靠——不会有
//! "装到别的目录、图标文件没跟着走"这种问题）。
//! `shield_rgba` 是没有正式图标之前手绘的占位方案，现在只当解码失败时的兜底。

/// 正式图标原始 PNG 字节，编译期嵌入。
static APP_ICON_PNG: &[u8] = include_bytes!("../assets/icons/icon_app.png");

/// 解码内嵌的正式图标，返回 `(rgba, width, height)`，窗口图标和托盘图标共用同一份。
pub fn load_app_icon() -> (Vec<u8>, u32, u32) {
    match image::load_from_memory(APP_ICON_PNG) {
        Ok(img) => {
            let rgba = img.to_rgba8();
            let (w, h) = rgba.dimensions();
            (rgba.into_raw(), w, h)
        }
        Err(_) => {
            // 正常情况下编译时打包的图片不会解码失败，这里只是防御性兜底。
            const SIZE: u32 = 64;
            (shield_rgba(SIZE, [58, 160, 224, 255], [0, 0, 0, 0]), SIZE, SIZE)
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
