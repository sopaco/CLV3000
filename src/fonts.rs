//! egui 自带字体（`default_fonts`）只覆盖拉丁字母/emoji 子集，不含中文字形，
//! 中文文字会渲染成方框。这里在启动时找一个系统自带的中文字体文件，注册成
//! egui 的 fallback 字体——不随包携带字体文件（省体积），也不依赖 chrono 之类
//! 额外 crate，只是读一个本机已经有的 `.ttf`/`.ttc`。
//!
//! 找不到任何候选字体时静默跳过，不影响程序启动（只是中文会显示成方框）。

use egui::epaint::text::{FontInsert, FontPriority, FontTweak, InsertFontFamily};
use egui::{Context, FontData, FontFamily};
use std::path::PathBuf;

pub fn install_cjk_font(ctx: &Context) {
    for path in candidate_paths() {
        let Ok(bytes) = std::fs::read(&path) else {
            continue;
        };
        // 之前这里加过一个 y_offset_factor，想让中文字形跟拉丁字体的基线对得更好看，
        // 结果发现它"只挪墨迹、不挪布局包围盒"，会把图标/文字对齐搞坏（见 SKILL 坑8），
        // 于是删掉了。但删掉之后中文与数字混排时反而更明显地暴露了另一个真问题：
        // 用 fontTools 量过 Hiragino Sans GB（macOS 预览用的 CJK 后备字体）跟 egui
        // 内置拉丁字体 Ubuntu-Light 的字形边界，同一个 1000-unit 的 em 里，汉字的
        // 视觉高度（约 900 units）比拉丁数字的字面高度（约 720 units）大 25% 左右
        // ——这是中日韩方块字天生比拉丁字母"更填满"em 格子的字体设计差异，不是
        // 某个字号或某台机器的偶然现象。同时 Hiragino 的 line-gap 也比 Ubuntu-Light
        // 大得多（500 vs 28 units），而 epaint 混合字体排版时会按两个字体的
        // row_height 差值做居中补偿（`text_layout.rs` 里 `0.5 * (font_height -
        // font_face_height)`），这个差值一大，中文相对数字就会被推得更高，看起来
        // 像"数字忽大忽小、没和中文对齐"。
        //
        // 这里用 `tweak.scale` 把 CJK 字体整体缩小到跟拉丁字体视觉上匹配的比例——
        // 跟 y_offset 不同，`scale` 会真的改变这个字体的 ascent/descent（进而改变
        // row_height），所以它能同时压低"字形填得太满"和"row_height 差太多"这两个
        // 根因，不是"越俎代庖"式的事后位移。0.80 是拿字形边界框算出来的比例
        // （720/900 ≈ 0.80），在真机截图里逐个验证过仪表盘/病毒库/扫描结果页的
        // 中文数字混排都对得上；换一个 CJK 字体（比如 Windows 上的雅黑）具体比例
        // 可能不完全一样，但同类字体的字面设计大同小异，这个量级基本通用。
        const CJK_SCALE: f32 = 0.80;
        ctx.add_font(FontInsert {
            name: "cjk-fallback".to_owned(),
            data: FontData {
                font: std::borrow::Cow::Owned(bytes),
                index: 0,
                tweak: FontTweak {
                    scale: CJK_SCALE,
                    ..Default::default()
                },
            },
            families: vec![
                InsertFontFamily {
                    family: FontFamily::Proportional,
                    priority: FontPriority::Lowest,
                },
                InsertFontFamily {
                    family: FontFamily::Monospace,
                    priority: FontPriority::Lowest,
                },
            ],
        });
        return;
    }
}

#[cfg(windows)]
fn candidate_paths() -> Vec<PathBuf> {
    let win_dir = std::env::var("WINDIR").unwrap_or_else(|_| r"C:\Windows".to_string());
    let fonts_dir = PathBuf::from(win_dir).join("Fonts");
    [
        "msyh.ttc",    // 微软雅黑，Win10/11 默认中文 UI 字体
        "msyhbd.ttc",
        "simhei.ttf",  // 黑体，更老的系统上也基本都有
        "simsun.ttc",  // 宋体，最后兜底
    ]
    .iter()
    .map(|f| fonts_dir.join(f))
    .collect()
}

#[cfg(not(windows))]
fn candidate_paths() -> Vec<PathBuf> {
    // macOS 开发机预览用；不追求覆盖 Linux（本项目本身是 Windows-only 的产品）。
    [
        "/System/Library/Fonts/PingFang.ttc",
        "/System/Library/Fonts/Hiragino Sans GB.ttc",
        "/System/Library/Fonts/STHeiti Light.ttc",
        "/System/Library/Fonts/Supplemental/Songti.ttc",
    ]
    .iter()
    .map(PathBuf::from)
    .collect()
}
