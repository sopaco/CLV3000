//! 扫描相关的共享类型：闪电扫描 (quick_scan) 和全盘扫描 (full_scan) 都会产出这些事件，
//! 由 engine.rs 负责真正调用 clamscan 子进程，quick_scan/full_scan 负责"喂路径"并附加各自的统计信息。
//!
//! 约定：`engine::run`、`full_scan::run`、`quick_scan::run` 都是阻塞函数，调用者需要
//! 自己在后台线程里跑它们——三处的 `pub fn run` 各自重复了这句提示，是故意的：
//! 每个调用点独立提醒，比只在这里写一遍、要求读者记住并跨文件回溯更不容易被忽略。

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

/// 交给 `engine::run` 的待扫路径来源。
///
/// - `InMemory`：闪电扫描枚举出的模块/进程列表，规模通常是几百到几千条，
///   一次性放内存里完全无妨。
/// - `File`：全盘扫描 walk 阶段边发现边流式写盘的临时列表文件（见
///   `full_scan.rs` 的 `WalkListWriter`），配 `count` 让 `engine::run` 不用先
///   读一遍文件才知道要不要处理。全盘扫描可能匹配到几万甚至更多可执行文件，
///   walk 期间只在内存里留一个计数器，不再攢一个越走越大的 `Vec<PathBuf>`。
#[derive(Debug)]
pub enum PathSource {
    InMemory(Vec<PathBuf>),
    File { path: PathBuf, count: usize },
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
    /// 闪电扫描在进程枚举后发送；全盘扫描在磁盘 walk 结束后发送（带文件总数）。
    ScanStarted { total: Option<usize> },
    /// 全盘扫描磁盘遍历中：已发现的可扫文件数（walk 完成前总数未知，靠此驱动 UI）。
    WalkProgress { files_found: usize },
    /// clamscan 已启动、病毒库加载中；尚有 `remaining` 个文件待在引擎内扫描。
    /// 不切换 UI 阶段，进度仍随 `FileScanned` / `ScanningFile` 推进。
    EngineLoading { remaining: usize },
    /// clamscan `-v` 的 `Scanning <path>` 行：文件已开始扫但尚未产出 OK/FOUND 行。
    ScanningFile { path: String },
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
    /// mock 引擎（非 Windows 和 MacOS的 开发预览）不会产生这个事件，所以该 target 上会报 dead_code。
    #[allow(dead_code)]
    Error(String),
}

/// 触发取消时，扫描线程和 clamscan 子进程都会尽快退出。
pub type CancelFlag = std::sync::Arc<std::sync::atomic::AtomicBool>;

pub fn new_cancel_flag() -> CancelFlag {
    std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false))
}

/// 判断文件是否是 Mach-O（含 fat/universal 二进制）：读文件头 4 字节比魔数。
/// macOS 上 `full_scan::real_macos::walk`（磁盘遍历时筛出可执行文件）和
/// `authenticode::macos`（预筛时判断是否该跑 codesign）都要用这同一个判定，
/// 之前两处各自重复实现了一份完全相同的逐字节读取逻辑——共用一份，且能避免
/// walk 阶段判过一次之后、预筛阶段又对同一个文件重新 open+read 一次文件头。
#[cfg(target_os = "macos")]
pub fn is_macho_file(path: &std::path::Path) -> bool {
    use std::io::Read;
    let mut f = match std::fs::File::open(path) {
        Ok(f) => f,
        Err(_) => return false,
    };
    let mut magic = [0u8; 4];
    if f.read_exact(&mut magic).is_err() {
        return false;
    }
    matches!(
        magic,
        [0xFE, 0xED, 0xFA, 0xCE] | // MH_MAGIC（32 位）
        [0xCE, 0xFA, 0xED, 0xFE] | // MH_CIGAM
        [0xFE, 0xED, 0xFA, 0xCF] | // MH_MAGIC_64
        [0xCF, 0xFA, 0xED, 0xFE] | // MH_CIGAM_64
        [0xCA, 0xFE, 0xBA, 0xBE] | // FAT_MAGIC（universal）
        [0xBE, 0xBA, 0xFE, 0xCA]   // FAT_CIGAM
    )
}
