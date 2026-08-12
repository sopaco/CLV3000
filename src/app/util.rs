//! 小工具函数：字符串截断、时长格式化。四个页面渲染都会用到，跟具体某一个
//! 页面无关，独立成文件避免散落在 `pages.rs` 里显得突兀。

use std::time::Duration;

pub(crate) fn truncate(s: &str, max_chars: usize) -> String {
    if s.chars().count() <= max_chars {
        s.to_string()
    } else {
        let head: String = s.chars().take(max_chars).collect();
        format!("{head}…")
    }
}

pub(crate) fn format_duration(d: Duration) -> String {
    let secs = d.as_secs();
    let m = secs / 60;
    let s = secs % 60;
    if m > 0 {
        format!("{m}m {s}s")
    } else {
        format!("{s}s")
    }
}
