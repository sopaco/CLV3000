//! 主界面编排：模块声明、`Page` 路由枚举，以及 `App` 的对外导出。

mod app_shell;
mod chrome;
mod core;
mod freshclam;
mod lifecycle_view;
mod pages;
mod settings;
mod util;

#[cfg(test)]
mod ui_preview;

pub use app_shell::App;

/// 关于独占窗口的尺寸（关于窗的 UI 测试要用它还原真实窗口大小）。
#[cfg(test)]
pub(crate) use lifecycle_view::ABOUT_WINDOW_SIZE;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Page {
    Dashboard,
    QuickScan,
    VirusDb,
    FullScan,
    Settings,
}
