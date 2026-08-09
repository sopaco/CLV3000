//! 全盘扫描：枚举本地固定磁盘，按可执行文件扩展名过滤后交给 clamscan。
//!
//! 流程：先遍历磁盘收集所有可执行文件路径（walk），再把完整列表交给 engine.rs
//! 一次性扫描。不再使用 stdin 流式喂路径——ClamAV 1.5.x 的 `--file-list=-` 不支持
//! stdin，必须写入临时文件，所以只能先收集完整列表。
//!
//! 非 Windows（macOS/Linux 开发机预览）：磁盘枚举（`GetLogicalDrives`/`GetDriveTypeW`）
//! 是 Win32 API，这里不去扫真实的 mac 文件系统（也扫不出什么"可执行文件"，还白白
//! 耗 I/O），改成 `mock` 子模块直接生成一批假路径喂给（同样是 mock 的）引擎。

use super::engine;
use super::{CancelFlag, ScanEvent};
use std::sync::atomic::Ordering;
use std::sync::mpsc::Sender;

/// 阻塞执行，调用者需要自己 spawn 一个线程来跑它。
pub fn run(tx: Sender<ScanEvent>, cancel: CancelFlag, include_removable: bool) {
    // mock::walk 不需要这个参数——只有 real::walk（真的枚举磁盘）才关心是否包含可移动盘。
    #[cfg(not(windows))]
    let _ = include_removable;

    // 1. 遍历磁盘，收集所有可执行文件路径。
    //    全盘扫描不知道总数（遍历完才知道），所以 total 传 None，UI 显示"已扫描 N 个"而非百分比。
    #[cfg(windows)]
    let paths = real::walk(&cancel, include_removable);
    #[cfg(not(windows))]
    let paths = mock::walk(&cancel);

    if cancel.load(Ordering::SeqCst) {
        let _ = tx.send(ScanEvent::Finished {
            scanned: 0,
            elapsed: std::time::Duration::ZERO,
            cancelled: true,
        });
        return;
    }

    // 2. 交给 engine 扫描。tx 的所有权在此转移给 engine。
    //    全盘扫描的 phase 已经在 start() 里设成 Scanning 了（和闪电扫描不同，不需要 ScanStarted）。
    engine::run(paths, tx, cancel);
}

#[cfg(windows)]
mod real {
    use super::CancelFlag;
    use std::path::PathBuf;
    use std::sync::atomic::Ordering;
    use walkdir::WalkDir;

    /// 参与扫描的可执行文件扩展名白名单（大小写不敏感）。
    const EXECUTABLE_EXTENSIONS: &[&str] =
        &["exe", "dll", "sys", "scr", "com", "cpl", "ocx", "drv"];

    /// Win32 `GetDriveTypeW` 返回值，对应 `DRIVE_FIXED` / `DRIVE_REMOVABLE`。
    const DRIVE_FIXED: u32 = 3;
    const DRIVE_REMOVABLE: u32 = 2;

    /// 遍历本地磁盘，收集所有匹配白名单扩展名的文件路径。
    pub fn walk(cancel: &CancelFlag, include_removable: bool) -> Vec<PathBuf> {
        let roots = local_drive_roots(include_removable);
        let mut paths = Vec::new();

        'roots: for root in roots {
            for entry in WalkDir::new(&root)
                .follow_links(false)
                .into_iter()
                .filter_map(|e| e.ok())
            {
                if cancel.load(Ordering::SeqCst) {
                    break 'roots;
                }
                if !entry.file_type().is_file() {
                    continue;
                }
                if !has_executable_extension(entry.path()) {
                    continue;
                }
                paths.push(entry.path().to_path_buf());
            }
        }
        paths
    }

    fn has_executable_extension(path: &std::path::Path) -> bool {
        path.extension()
            .and_then(|e| e.to_str())
            .map(|e| {
                EXECUTABLE_EXTENSIONS
                    .iter()
                    .any(|allowed| allowed.eq_ignore_ascii_case(e))
            })
            .unwrap_or(false)
    }

    /// 枚举本地磁盘根目录：默认只包含固定磁盘（本机硬盘），可移动盘/网络盘按配置决定。
    fn local_drive_roots(include_removable: bool) -> Vec<PathBuf> {
        use windows::core::HSTRING;
        use windows::Win32::Storage::FileSystem::GetDriveTypeW;

        let mut roots = Vec::new();
        // SAFETY: GetLogicalDrives 无参数、无前置条件。
        let mask = unsafe { windows::Win32::Storage::FileSystem::GetLogicalDrives() };
        for i in 0..26u32 {
            if mask & (1 << i) == 0 {
                continue;
            }
            let letter = (b'A' + i as u8) as char;
            let root = format!("{letter}:\\");
            let wide = HSTRING::from(root.as_str());
            // SAFETY: 只是查询驱动器类型，不做任何写操作。
            let drive_type = unsafe { GetDriveTypeW(&wide) };
            let ok =
                drive_type == DRIVE_FIXED || (include_removable && drive_type == DRIVE_REMOVABLE);
            if ok {
                roots.push(PathBuf::from(root));
            }
        }
        roots
    }
}

/// 开发预览用的假全盘扫描：不碰真实文件系统，直接生成一批"看起来像 Windows 路径"
/// 的假数据。生成本身很快，实际扫描节奏由（同样是 mock 的）engine 那边控制。
#[cfg(not(windows))]
mod mock {
    use super::CancelFlag;
    use std::path::PathBuf;
    use std::sync::atomic::Ordering;

    const APP_NAMES: &[&str] = &[
        "Chrome", "Office", "Adobe", "Steam", "Zoom", "Slack", "VSCode", "Docker", "Python",
        "NodeJS", "Git", "7Zip", "VLC", "Notion", "Discord",
    ];
    const EXTENSIONS: &[&str] = &["exe", "dll", "sys"];

    pub fn walk(cancel: &CancelFlag) -> Vec<PathBuf> {
        const TOTAL: usize = 3000;
        let mut paths = Vec::with_capacity(TOTAL + 1);

        // 保证至少有一个"看起来可疑"的路径，方便预览威胁卡片 UI（是否真的被
        // 标红取决于 engine mock 那边这一轮是不是"该翻到有威胁的一面"）。
        paths.push(PathBuf::from(
            r"C:\Users\Alice\Downloads\setup_crack_v2.exe",
        ));

        for i in 0..TOTAL {
            if cancel.load(Ordering::SeqCst) {
                break;
            }
            let app = APP_NAMES[i % APP_NAMES.len()];
            let ext = EXTENSIONS[i % EXTENSIONS.len()];
            paths.push(PathBuf::from(format!(
                r"C:\Program Files\{app}\bin\module_{i}.{ext}"
            )));
        }
        paths
    }
}
