//! 小工具函数：时长格式化。四个页面渲染都会用到，跟具体某一个页面无关，
//! 独立成文件避免散落在 `pages.rs` 里显得突兀。
//! （原有的字符数截断 `truncate` 已删：所有页面改用 `widgets::truncate_middle_to_px`
//! 像素截断，字符数版本宽窄全靠猜。）

use std::time::Duration;

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
