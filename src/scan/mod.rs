//! 扫描相关的共享类型：闪电扫描 (quick_scan) 和全盘扫描 (full_scan) 都会产出这些事件，
//! 由 engine.rs 负责真正调用 clamscan 子进程，quick_scan/full_scan 负责"喂路径"并附加各自的统计信息。

// authenticode 预筛在 Windows（WinVerifyTrust）与 macOS（codesign）上都有真实实现，
// 由文件级 `#![cfg(any(windows, target_os = "macos"))]` 控制编译；Linux 等其它目标
// 不编译（mock 引擎也不引用它）。这里无条件声明，保证 engine 的 `use` 始终成立。
pub mod authenticode;
// 文件基因缓存（blake3 纯 Rust，跨平台）。Windows 与 macOS 的真实引擎都复用它做
// "内容哈希 → 上次结果" 的加速；非 Windows 的 mock 引擎不引用，故按目标平台门控，
// 避免主机构建出 never-used 警告。
#[cfg(any(windows, target_os = "macos"))]
pub mod cache;
pub mod engine;
pub mod full_scan;
pub mod quick_scan;

use std::path::PathBuf;
use std::time::Duration;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScanKind {
    Quick,
    Full,
}

#[derive(Debug, Clone)]
pub struct Threat {
    pub path: PathBuf,
    pub virus_name: String,
}

/// 从后台扫描线程发往 UI 线程的事件。
#[derive(Debug, Clone)]
pub enum ScanEvent {
    /// 仅闪电扫描会发：进程枚举进度（还没开始/正在喂给 clamscan 之前的统计）。
    Enumerating {
        processes_done: usize,
        processes_total: usize,
        files_found: usize,
    },
    /// 枚举完成、clamscan 即将启动（或已启动但还在加载病毒库、尚未产出结果）。
    /// 闪电扫描枚举完就知道文件总数了；全盘扫描不发这个事件，phase 直接从 Idle 跳到 Scanning。
    /// 之所以单独发一个事件：clamscan 加载病毒库通常要十几秒，期间一个 FileScanned 都不会来，
    /// 没有 ScanStarted 的话 UI 会一直停在 "Enumerating N/N processes" 看起来卡住。
    ScanStarted {
        total: Option<usize>,
    },
    /// clamscan 已启动、病毒库加载中；尚有 `remaining` 个文件待在引擎内扫描。
    /// 不切换 UI 阶段，进度仍随 `FileScanned` / `ScanningFile` 推进。
    EngineLoading {
        remaining: usize,
    },
    /// clamscan `-v` 的 `Scanning <path>` 行：文件已开始扫但尚未产出 OK/FOUND 行。
    ScanningFile {
        path: String,
    },
    /// clamscan 对一个文件给出了结果。
    FileScanned {
        path: String,
        infected: Option<String>,
    },
    /// 扫描流程结束（正常完成或被取消）。
    Finished {
        scanned: usize,
        elapsed: Duration,
        cancelled: bool,
    },
    /// 引擎不可用 / 启动失败等错误，不会中断已经展示的部分结果。
    /// mock 引擎（非 Windows 开发预览）不会产生这个事件，所以该 target 上会报 dead_code。
    #[allow(dead_code)]
    Error(String),
}

/// 触发取消时，扫描线程和 clamscan 子进程都会尽快退出。
pub type CancelFlag = std::sync::Arc<std::sync::atomic::AtomicBool>;

pub fn new_cancel_flag() -> CancelFlag {
    std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false))
}
