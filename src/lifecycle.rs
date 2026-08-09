//! 应用运行模式：控制主窗口与纯托盘循环之间的切换。

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunMode {
    /// 显示 eframe 主窗口（含 OpenGL / egui）。
    ShowWindow,
    /// 无窗口，仅托盘 + 后台扫描轮询。
    TrayOnly,
    /// 显示独立「关于」小窗（结束后回到 `resume_after_about`）。
    AboutOnly,
    /// 退出整个进程。
    Quit,
}

pub struct Lifecycle {
    pub mode: RunMode,
    /// `AboutOnly` 关闭后恢复到的模式。
    pub resume_after_about: Option<RunMode>,
}

impl Lifecycle {
    pub fn new(start_tray_only: bool) -> Self {
        Self {
            mode: if start_tray_only {
                RunMode::TrayOnly
            } else {
                RunMode::ShowWindow
            },
            resume_after_about: None,
        }
    }
}

/// 解析命令行：支持 `--tray-only` / `--tray` 启动后只显示托盘。
pub fn parse_start_tray_only() -> bool {
    std::env::args().any(|a| a == "--tray-only" || a == "--tray")
}
