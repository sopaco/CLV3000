//! 主界面编排：模块声明、`Page` 路由枚举，以及 `App` 的对外导出。

mod app_shell;
mod chrome;
mod core;
mod freshclam;
mod lifecycle_view;
mod pages;
mod settings;
mod util;

pub use app_shell::App;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Page {
    Dashboard,
    QuickScan,
    VirusDb,
    FullScan,
    Settings,
}
