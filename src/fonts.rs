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
        // 中文字体的字面在 em 格子里通常比拉丁字体撑得更满/坐得更高，同一行混排时
        // 中文和数字/英文的视觉中心容易看起来没对齐。这个 y_offset_factor 把中文
        // 字形整体往下挪一点，尽量跟拉丁字体的基线对上——没法在这台机器上直接肉眼
        // 调，是按经验给的保守值，如果实际看还有偏差可以再调这个数字（正数往下移）。
        let tweak = FontTweak {
            y_offset_factor: 0.08,
            ..Default::default()
        };
        ctx.add_font(FontInsert {
            name: "cjk-fallback".to_owned(),
            data: FontData::from_owned(bytes).tweak(tweak),
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
