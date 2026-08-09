//! 闪电扫描：枚举所有运行中进程，取出每个进程加载的模块（含主 exe 本体），
//! 去重后交给 clamscan 扫描。
//!
//! 权限说明：部分系统/受保护进程在普通用户权限下打不开或枚举不到模块，
//! 这里直接跳过并计入统计，不会阻塞整体流程——不是 bug。
//!
//! 非 Windows（macOS/Linux 开发机预览）：`Toolhelp32` 这套 API 是 Windows 独有的，
//! `mock` 子模块用程序生成的假进程/模块列表替代，数量级贴近实际 Windows 机器
//! （几百个进程、上千个去重后的 DLL），方便直接看 UI 效果。

use super::engine;
use super::{CancelFlag, ScanEvent};
use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::atomic::Ordering;
use std::sync::mpsc::Sender;

#[cfg(windows)]
use real::{modules_of_process, snapshot_pids};

#[cfg(not(windows))]
use mock::{modules_of_process, snapshot_pids};

/// 阻塞执行，调用者需要自己 spawn 一个线程来跑它。
///
/// 分两步走，和全盘扫描的"边发现边扫"不一样：
/// 1. 先把进程和模块枚举完（这一步很快，通常一两秒内），拿到去重后的完整文件列表，
///    这样才能给出准确的"总数"，UI 上才能画出确定的百分比进度环。
/// 2. 枚举完成后再启动 clamscan，把文件列表喂给它。
pub fn run(tx: Sender<ScanEvent>, cancel: CancelFlag) {
    let pids = snapshot_pids();
    let processes_total = pids.len();
    let mut seen_paths: HashSet<PathBuf> = HashSet::new();
    let mut ordered_paths: Vec<PathBuf> = Vec::new();

    let _ = tx.send(ScanEvent::Enumerating {
        processes_done: 0,
        processes_total,
        files_found: 0,
    });

    for (i, pid) in pids.into_iter().enumerate() {
        if cancel.load(Ordering::SeqCst) {
            let _ = tx.send(ScanEvent::Finished {
                scanned: 0,
                elapsed: std::time::Duration::ZERO,
                cancelled: true,
            });
            return;
        }

        for module_path in modules_of_process(pid) {
            if seen_paths.insert(module_path.clone()) {
                ordered_paths.push(module_path);
            }
        }

        let _ = tx.send(ScanEvent::Enumerating {
            processes_done: i + 1,
            processes_total,
            files_found: seen_paths.len(),
        });
    }

    // 枚举已经完成，文件总数已知。先发 ScanStarted 让 UI 立刻从 "Enumerating" 切到
    // "Scanning"，否则 clamscan 加载病毒库的十几秒里 UI 会一直显示 "Enumerating N/N"。
    let total_files = ordered_paths.len();
    let _ = tx.send(ScanEvent::ScanStarted {
        total: Some(total_files),
    });

    let (path_tx, path_rx) = std::sync::mpsc::channel::<PathBuf>();
    let engine_cancel = cancel.clone();
    let engine_thread = std::thread::spawn(move || {
        engine::run(path_rx, tx, engine_cancel);
    });

    for path in ordered_paths {
        if cancel.load(Ordering::SeqCst) {
            break;
        }
        if path_tx.send(path).is_err() {
            break;
        }
    }
    drop(path_tx);
    let _ = engine_thread.join();
}

#[cfg(windows)]
mod real {
    use std::path::PathBuf;
    use windows::Win32::Foundation::CloseHandle;
    use windows::Win32::System::Diagnostics::ToolHelp::{
        CreateToolhelp32Snapshot, Module32FirstW, Module32NextW, Process32FirstW, Process32NextW,
        MODULEENTRY32W, PROCESSENTRY32W, TH32CS_SNAPMODULE, TH32CS_SNAPMODULE32,
        TH32CS_SNAPPROCESS,
    };

    /// 拍一次全进程快照，只取 PID 列表（后面每个进程单独开模块快照）。
    pub fn snapshot_pids() -> Vec<u32> {
        let mut pids = Vec::new();
        let Ok(snapshot) = (unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) }) else {
            return pids;
        };

        let mut entry = PROCESSENTRY32W {
            dwSize: std::mem::size_of::<PROCESSENTRY32W>() as u32,
            ..Default::default()
        };
        unsafe {
            if Process32FirstW(snapshot, &mut entry).is_ok() {
                loop {
                    pids.push(entry.th32ProcessID);
                    if Process32NextW(snapshot, &mut entry).is_err() {
                        break;
                    }
                }
            }
            let _ = CloseHandle(snapshot);
        }
        pids
    }

    /// 枚举某个进程加载的所有模块（含它自己的 exe），拿不到就返回空列表。
    pub fn modules_of_process(pid: u32) -> Vec<PathBuf> {
        let mut paths = Vec::new();
        let flags = TH32CS_SNAPMODULE | TH32CS_SNAPMODULE32;
        let Ok(snapshot) = (unsafe { CreateToolhelp32Snapshot(flags, pid) }) else {
            return paths; // 权限不足或进程已退出，跳过
        };

        let mut entry = MODULEENTRY32W {
            dwSize: std::mem::size_of::<MODULEENTRY32W>() as u32,
            ..Default::default()
        };
        unsafe {
            if Module32FirstW(snapshot, &mut entry).is_ok() {
                loop {
                    if let Some(path) = wide_to_path(&entry.szExePath) {
                        paths.push(path);
                    }
                    if Module32NextW(snapshot, &mut entry).is_err() {
                        break;
                    }
                }
            }
            let _ = CloseHandle(snapshot);
        }
        paths
    }

    fn wide_to_path(buf: &[u16]) -> Option<PathBuf> {
        let end = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
        if end == 0 {
            return None;
        }
        Some(PathBuf::from(String::from_utf16_lossy(&buf[..end])))
    }
}

/// 开发预览用的假进程/模块数据源，数量级贴近真实 Windows 机器上闪电扫描能看到的规模。
#[cfg(not(windows))]
mod mock {
    use std::path::PathBuf;

    /// 系统里几乎每个进程都会加载的一批"公共 DLL"，去重后基本只剩这些——
    /// 用来让"文件数"明显小于"进程数 x 每进程模块数"，效果上和真机一致。
    const COMMON_MODULES: &[&str] = &[
        r"C:\Windows\System32\ntdll.dll",
        r"C:\Windows\System32\kernel32.dll",
        r"C:\Windows\System32\kernelbase.dll",
        r"C:\Windows\System32\ucrtbase.dll",
        r"C:\Windows\System32\combase.dll",
        r"C:\Windows\System32\rpcrt4.dll",
        r"C:\Windows\System32\advapi32.dll",
        r"C:\Windows\System32\sechost.dll",
        r"C:\Windows\System32\msvcrt.dll",
        r"C:\Windows\System32\gdi32.dll",
        r"C:\Windows\System32\user32.dll",
        r"C:\Windows\System32\win32u.dll",
        r"C:\Windows\System32\shell32.dll",
        r"C:\Windows\System32\shlwapi.dll",
        r"C:\Windows\System32\ws2_32.dll",
    ];

    /// 假装系统里跑着 342 个进程——和设计稿上的数字对上。
    pub fn snapshot_pids() -> Vec<u32> {
        (1..=342u32).collect()
    }

    /// 每个进程"加载"公共 DLL + 几个只属于它自己的假模块，去重后总数会落在
    /// 大几百到一千出头的量级（342 个进程 x 平均 3~4 个专属路径）。
    /// 顺带 sleep 一小会，让"枚举中"的进度条肉眼可见地跑起来，不是一帧闪过去。
    pub fn modules_of_process(pid: u32) -> Vec<PathBuf> {
        std::thread::sleep(std::time::Duration::from_millis(1));

        let mut paths: Vec<PathBuf> = COMMON_MODULES.iter().map(PathBuf::from).collect();
        paths.push(PathBuf::from(r"C:\Windows\System32\svchost.exe"));

        // pid 7 特意"加载"一个看起来不太正经的模块，方便预览威胁卡片 UI。
        if pid == 7 {
            paths.push(PathBuf::from(
                r"C:\Users\Alice\Downloads\RemoteHelper\injector.dll",
            ));
        }

        let unique_count = 2 + (pid % 3);
        for i in 0..unique_count {
            paths.push(PathBuf::from(format!(
                r"C:\Program Files\App{}\module{}.dll",
                pid % 50,
                i
            )));
        }
        paths
    }
}
