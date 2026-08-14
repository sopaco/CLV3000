//! 应用运行模式：控制主窗口与纯托盘循环之间的切换。

/// eframe 会话启动时的初始模式（由 `main` 根据 `--tray-only` 与托盘事件决定）。
///
/// Windows 上 `--tray-only` 启动时，`main` 会在 eframe 之外空等托盘事件
/// （`wait_in_tray`），完全不创建窗口——既不闪窗、也不占用 OpenGL 上下文内存。
/// 用户从托盘请求显示窗口/关于/闪电扫描时才启动 eframe，并按此模式初始化。
///
/// `ScanPath` 带一个路径数据，所以这个枚举去掉了 `Copy`（其它几个变体不受影响，
/// 按值 `match` 时只要模式里不绑定字段就不会移动，见 `app/mod.rs::App::new`）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InitialMode {
    /// 直接显示主窗口。
    ShowWindow,
    /// 启动即隐藏到托盘（macOS tray-only 路径：eframe 仍启动但初始隐藏，因为
    /// 托盘事件投递依赖 NSApplication 事件循环，eframe 之外空等收不到托盘点击）。
    #[allow(dead_code)] // 仅 macOS 路径构造，Windows 编译时无构造点
    TrayOnly,
    /// 启动并直接进入闪电扫描。
    #[allow(dead_code)] // 仅 Windows 托盘菜单构造（wait_in_tray），非 Windows 编译时无构造点
    QuickScan,
    /// 启动并直接显示「关于」独占窗口（来自托盘）。
    #[allow(dead_code)] // 同上
    About,
    /// 启动并直接扫描给定的单个文件/文件夹——来自 `--scan-path` 命令行参数
    /// （右键菜单"用 CLV3000 扫描"触发的冷启动），或已有实例在跑时通过
    /// `single_instance` 转发过来的扫描请求（见 `wakeup::scan_requests`）。
    ScanPath(std::path::PathBuf),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunMode {
    /// 显示 eframe 主窗口（含 OpenGL / egui）。
    ShowWindow,
    /// 无窗口，仅托盘 + 后台扫描轮询。
    TrayOnly,
    /// 退出整个进程。
    Quit,
}

pub struct Lifecycle {
    pub mode: RunMode,
    /// 是否显示「关于」。它只是个覆盖标记，不单独改变窗口可见性——窗口是否可见
    /// 由 `mode == ShowWindow || about_open` 决定。
    pub about_open: bool,
    /// 关于是否以「独占整个窗口」的形式呈现。
    /// - `true`（来自托盘）：整个窗口只画关于页、不画主界面；关闭后由 reconcile 自动
    ///   缩回托盘，不会残留主窗口。这正是用户要的"只显示关于页、不要主窗口"。
    /// - `false`（来自主窗，当前没有入口，预留）：表现为叠在主界面之上的模态。
    pub about_standalone: bool,
}

impl Lifecycle {
    pub fn new(start_tray_only: bool) -> Self {
        Self {
            mode: if start_tray_only {
                RunMode::TrayOnly
            } else {
                RunMode::ShowWindow
            },
            about_open: false,
            about_standalone: false,
        }
    }
}

/// 解析命令行：支持 `--tray-only` / `--tray` 启动后只显示托盘。
pub fn parse_start_tray_only() -> bool {
    std::env::args().any(|a| a == "--tray-only" || a == "--tray")
}

/// 解析命令行：支持 `--scan-path=<path>` 和 `--scan-path <path>`（下一个参数）
/// 两种写法——Windows 右键菜单的 `command` 值用 `"<exe>" --scan-path "%1"`（后者），
/// 手动命令行调试用等号写法更方便，两种都认。只取第一次出现的一个，多个
/// `--scan-path` 没有意义。
pub fn parse_scan_path() -> Option<std::path::PathBuf> {
    let args: Vec<String> = std::env::args().collect();
    for (i, arg) in args.iter().enumerate() {
        if let Some(path) = arg.strip_prefix("--scan-path=") {
            if !path.is_empty() {
                return Some(std::path::PathBuf::from(path));
            }
        } else if arg == "--scan-path" {
            if let Some(next) = args.get(i + 1) {
                return Some(std::path::PathBuf::from(next));
            }
        }
    }
    None
}

/// 解析 `--force-quarantine <original> <dest>`：强制隔离提权子进程模式。
/// 仅 Windows 上构造（由 `main` 在单实例检查之前拦截）。返回 `(原文件路径, 目标路径)`。
#[cfg(windows)]
pub fn parse_force_quarantine() -> Option<(std::path::PathBuf, std::path::PathBuf)> {
    let args: Vec<String> = std::env::args().collect();
    let i = args.iter().position(|a| a == "--force-quarantine")?;
    let original = args.get(i + 1)?.clone();
    let dest = args.get(i + 2)?.clone();
    if original.is_empty() || dest.is_empty() {
        return None;
    }
    Some((std::path::PathBuf::from(original), std::path::PathBuf::from(dest)))
}
