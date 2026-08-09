//! 扫描相关的共享类型：闪电扫描 (quick_scan) 和全盘扫描 (full_scan) 都会产出这些事件，
//! 由 engine.rs 负责真正调用 clamscan 子进程，quick_scan/full_scan 负责"喂路径"并附加各自的统计信息。

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
