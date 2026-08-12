//! 全盘扫描：枚举本地固定磁盘，按可执行文件类型过滤后交给 clamscan。
//!
//! 流程：先遍历磁盘收集所有待扫文件路径（walk），再把完整列表交给 engine.rs
//! 一次性扫描。不再使用 stdin 流式喂路径——ClamAV 1.5.x 的 `--file-list=-` 不支持
//! stdin，必须写入临时文件，所以只能先收集完整列表。
//!
//! 真实 walk（Windows/macOS）不在内存里攢这份列表：磁盘遍历可能耗时数分钟、
//! 匹配到几万甚至更多可执行文件，`WalkListWriter` 边发现边追加写本地临时
//! 文件，walk 期间内存只留一个计数器。`engine::run` 拿到的是
//! `PathSource::File{path, count}`，在 walk 结束、遍历本身的耗时已经付出之后
//! 才一次性读回内存（见 `engine.rs` 的 `load_path_source`），供预筛阶段按块
//! 分给多个 worker 线程。
//!
//! 平台实现：
//! - Windows：用 `GetLogicalDrives`/`GetDriveTypeW` 枚举盘符，按 `.exe/.dll/...`
//!   扩展名白名单过滤（`real_windows`）。
//! - macOS：启动盘固定为 `/`，可选包含 `/Volumes/*` 挂载的可移动盘；按 **Mach-O 魔数**
//!   识别可执行文件（macOS 没有 `.exe` 扩展名概念），WalkDir 遍历时实时过滤
//!   （`real_macos`）。权限不足/不可读的目录由 WalkDir 静默跳过，不阻塞整体流程。
//! - 其它（Linux 等开发机预览）：`mock` 直接生成一批假路径喂给（同样是 mock 的）引擎，
//!   规模小，用不上流式落盘，仍走内存里的 `Vec<PathBuf>`。

use super::engine;
use super::{CancelFlag, PathSource, ScanEvent};
use std::sync::atomic::Ordering;
use std::sync::mpsc::Sender;
use std::time::Instant;

/// walk 过程中向 UI 汇报进度的最小间隔（文件数），避免 channel 被刷爆。
const WALK_PROGRESS_STEP: usize = 100;

fn report_walk_progress(tx: &Sender<ScanEvent>, files_found: usize, last_reported: &mut usize) {
    if files_found == 0 {
        return;
    }
    if *last_reported == 0 || files_found >= *last_reported + WALK_PROGRESS_STEP {
        let _ = tx.send(ScanEvent::WalkProgress { files_found });
        *last_reported = files_found;
    }
}

/// 边发现边把匹配的文件路径追加写入本地临时文件，只在内存里留一个计数——
/// 真实 walk（Windows/macOS）可能匹配到几万甚至更多可执行文件，遍历本身又
/// 可能耗时数分钟，不该在这整段时间里攢一个越走越大的 `Vec<PathBuf>`。
/// 文件格式跟 `engine.rs` 的 `write_path_list` 一致：每行一个路径，LF 换行，
/// UTF-8 无 BOM。
#[cfg(any(windows, target_os = "macos"))]
struct WalkListWriter {
    /// 创建失败（临时目录不可写等极端情况）时为 `None`：`push` 静默变成
    /// no-op，最终 `count` 仍会保持 0——不阻断整个扫描流程，效果上等同于
    /// "这次遍历没找到任何可执行文件"，engine 那边本就有空列表的正常路径。
    file: Option<std::io::BufWriter<std::fs::File>>,
    path: std::path::PathBuf,
    count: usize,
}

#[cfg(any(windows, target_os = "macos"))]
impl WalkListWriter {
    fn create() -> Self {
        let path = std::env::temp_dir()
            .join(format!("clv3000_walklist_{}.txt", std::process::id()));
        let file = std::fs::File::create(&path).ok().map(std::io::BufWriter::new);
        WalkListWriter { file, path, count: 0 }
    }

    /// 追加一个路径。写失败（极少见：磁盘满、临时文件被外部删掉）静默丢弃这一条，
    /// 不中断遍历——`count` 只统计"确实写成功"的条数，跟最终读回来的行数对得上。
    fn push(&mut self, p: &std::path::Path) {
        use std::io::Write;
        let Some(file) = self.file.as_mut() else { return };
        if writeln!(file, "{}", p.display()).is_ok() {
            self.count += 1;
        }
    }

    /// 收尾：flush 落盘，交出临时文件路径 + 条数给 `engine::run` 读回。
    fn finish(mut self) -> (std::path::PathBuf, usize) {
        use std::io::Write;
        if let Some(file) = self.file.as_mut() {
            let _ = file.flush();
        }
        (self.path, self.count)
    }
}

/// 阻塞执行，调用者需要自己 spawn 一个线程来跑它。
pub fn run(tx: Sender<ScanEvent>, cancel: CancelFlag, include_removable: bool) {
    let start = Instant::now();
    // mock::walk 不需要这个参数——只有真实 walk（真的枚举磁盘）才关心是否包含可移动盘。
    #[cfg(not(any(windows, target_os = "macos")))]
    let _ = include_removable;

    // 1. 遍历磁盘，收集所有待扫文件路径（期间发 WalkProgress 驱动 UI）。真实
    //    walk 边发现边流式落盘，只拿回 (临时文件路径, 条数)；mock 规模小，仍
    //    直接产出内存里的 Vec。
    #[cfg(windows)]
    let (source, total_files) = {
        let (path, count) = real_windows::walk(&tx, &cancel, include_removable);
        (PathSource::File { path, count }, count)
    };
    #[cfg(target_os = "macos")]
    let (source, total_files) = {
        let (path, count) = real_macos::walk(&tx, &cancel, include_removable);
        (PathSource::File { path, count }, count)
    };
    #[cfg(not(any(windows, target_os = "macos")))]
    let (source, total_files) = {
        let paths = mock::walk(&tx, &cancel);
        let count = paths.len();
        (PathSource::InMemory(paths), count)
    };

    if cancel.load(Ordering::SeqCst) {
        // 已经写入的临时文件（若有）在这里直接删掉，不留给 engine 侧处理——
        // 反正不会再进 `engine::run`，没必要为这一次性清理多传一层状态。
        if let PathSource::File { path, .. } = &source {
            let _ = std::fs::remove_file(path);
        }
        let _ = tx.send(ScanEvent::Finished {
            scanned: 0,
            elapsed: start.elapsed(),
            cancelled: true,
        });
        return;
    }

    if total_files == 0 {
        if let PathSource::File { path, .. } = &source {
            let _ = std::fs::remove_file(path);
        }
        let _ = tx.send(ScanEvent::Finished {
            scanned: 0,
            elapsed: start.elapsed(),
            cancelled: false,
        });
        return;
    }

    // 与闪电扫描一致：walk 完成、总数已知后再切到带百分比的扫描 UI。
    let _ = tx.send(ScanEvent::ScanStarted {
        total: Some(total_files),
    });

    // 2. 交给 engine 扫描。
    engine::run(source, tx, cancel);
}

#[cfg(windows)]
mod real_windows {
    use super::CancelFlag;
    use super::{report_walk_progress, ScanEvent, WalkListWriter};
    use std::path::PathBuf;
    use std::sync::atomic::Ordering;
    use std::sync::mpsc::Sender;
    use walkdir::WalkDir;

    /// 参与扫描的可执行文件扩展名白名单（大小写不敏感）。
    const EXECUTABLE_EXTENSIONS: &[&str] =
        &["exe", "dll", "sys", "scr", "com", "cpl", "ocx", "drv"];

    /// Win32 `GetDriveTypeW` 返回值，对应 `DRIVE_FIXED` / `DRIVE_REMOVABLE`。
    const DRIVE_FIXED: u32 = 3;
    const DRIVE_REMOVABLE: u32 = 2;

    /// 遍历本地磁盘，把匹配白名单扩展名的文件路径边发现边写入临时列表文件。
    /// 返回 (临时文件路径, 条数)。
    pub fn walk(
        tx: &Sender<ScanEvent>,
        cancel: &CancelFlag,
        include_removable: bool,
    ) -> (PathBuf, usize) {
        let roots = local_drive_roots(include_removable);
        let mut writer = WalkListWriter::create();
        let mut last_reported = 0usize;

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
                writer.push(entry.path());
                report_walk_progress(tx, writer.count, &mut last_reported);
            }
        }
        report_walk_progress(tx, writer.count, &mut last_reported);
        writer.finish()
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

/// macOS：启动盘固定为 `/`，可选包含 `/Volumes/*` 下的可移动盘。按 **Mach-O 魔数**
/// 识别可执行文件——macOS 没有 `.exe` 扩展名概念，靠文件头魔数才是正确做法。
#[cfg(target_os = "macos")]
mod real_macos {
    use super::CancelFlag;
    use super::{report_walk_progress, ScanEvent, WalkListWriter};
    use std::path::PathBuf;
    use std::sync::atomic::Ordering;
    use std::sync::mpsc::Sender;
    use walkdir::WalkDir;

    /// 明显不可能是可执行文件的扩展名，在 open+读魔数之前先按名字挡掉——一次
    /// 字符串比较换掉一次系统调用，全盘扫描里这类文件占绝大多数（图片/文档/
    /// 媒体/数据文件），是 walk 阶段最便宜的一刀。真正的可执行文件（包括无
    /// 扩展名的）仍然会走到魔数判断，不依赖这个列表保证不漏判。
    const NEVER_EXECUTABLE_EXTENSIONS: &[&str] = &[
        "png", "jpg", "jpeg", "gif", "webp", "heic", "bmp", "tiff", "svg", "ico",
        "mp3", "mp4", "mov", "avi", "m4a", "m4v", "wav", "flac",
        "pdf", "txt", "md", "json", "xml", "html", "htm", "css", "js",
        "plist", "strings", "nib", "storyboard", "xib", "car",
        "zip", "gz", "tar", "dmg", "pkg",
        "log", "db", "sqlite", "sqlite3",
    ];

    /// 遍历挂载点，把匹配的 Mach-O 可执行文件路径边发现边写入临时列表文件。
    /// 返回 (临时文件路径, 条数)。
    ///
    /// 用 `filter_entry` 而不是"进了子树再逐个 entry 判断丢弃"：后者对
    /// `/System/Volumes` 这类整棵子树都要跳过的目录，WalkDir 仍会完整下降进去、
    /// 对每个文件都 stat 一遍，只是最后不收进结果——等于把这块子树白走了一遍。
    /// `filter_entry` 在返回 `false` 时直接不下降这个目录，整棵子树被剪掉。
    pub fn walk(
        tx: &Sender<ScanEvent>,
        cancel: &CancelFlag,
        include_removable: bool,
    ) -> (PathBuf, usize) {
        let roots = local_drive_roots(include_removable);
        let mut writer = WalkListWriter::create();
        let mut last_reported = 0usize;

        'roots: for root in roots {
            let walker = WalkDir::new(&root)
                .follow_links(false)
                .into_iter()
                .filter_entry(|e| !is_excluded(e.path(), include_removable));
            for entry in walker.filter_map(|e| e.ok()) {
                if cancel.load(Ordering::SeqCst) {
                    break 'roots;
                }
                if !entry.file_type().is_file() {
                    continue;
                }
                let path = entry.path();
                if has_never_executable_extension(path) {
                    continue;
                }
                // 实时读文件头判断是不是 Mach-O，避免把数百万个普通文件都塞进待扫列表。
                if !super::super::is_macho_file(path) {
                    continue;
                }
                writer.push(path);
                report_walk_progress(tx, writer.count, &mut last_reported);
            }
        }
        report_walk_progress(tx, writer.count, &mut last_reported);
        writer.finish()
    }

    fn has_never_executable_extension(path: &std::path::Path) -> bool {
        path.extension()
            .and_then(|e| e.to_str())
            .map(|e| {
                NEVER_EXECUTABLE_EXTENSIONS
                    .iter()
                    .any(|skip| skip.eq_ignore_ascii_case(e))
            })
            .unwrap_or(false)
    }

    /// 判断某个路径是否属于"不应纳入全盘扫描"的子树，配合 `filter_entry` 整棵剪掉：
    /// - `/System/Volumes`：`/` 的数据卷镜像，firmlink 已经把 `/Users`、`/Applications`
    ///   等呈现到 `/` 下，再走一遍会重复扫描同一批文件。
    /// - `/private/var/vm`：休眠镜像/swap 文件所在目录，体积大且不含可执行文件。
    /// - `/private/var/folders`：每用户临时目录/缓存，同上。
    /// - `/.Spotlight-V100`：Spotlight 索引数据，不含可执行文件。
    /// - `/net`、`/home`：autofs 自动挂载点，一旦下降进去可能触发实际的网络挂载
    ///   并卡住整个 walk 线程。
    /// - 不包含可移动盘时：`/Volumes`、`/Network` 下的其它挂载点也不要扫。
    fn is_excluded(path: &std::path::Path, include_removable: bool) -> bool {
        const ALWAYS_EXCLUDED: &[&str] = &[
            "/System/Volumes",
            "/private/var/vm",
            "/private/var/folders",
            "/.Spotlight-V100",
            "/net",
            "/home",
        ];
        if ALWAYS_EXCLUDED.iter().any(|p| path.starts_with(p)) {
            return true;
        }
        if !include_removable && (path.starts_with("/Volumes") || path.starts_with("/Network")) {
            return true;
        }
        false
    }

    /// 枚举磁盘根目录：macOS 启动盘永远是 `/`；可移动盘挂在 `/Volumes/` 下。
    fn local_drive_roots(include_removable: bool) -> Vec<PathBuf> {
        let mut roots = vec![PathBuf::from("/")];
        if include_removable {
            if let Ok(entries) = std::fs::read_dir("/Volumes") {
                for e in entries.flatten() {
                    let p = e.path();
                    if p.is_dir() {
                        roots.push(p);
                    }
                }
            }
        }
        roots
    }
}

/// 开发预览用的假全盘扫描：不碰真实文件系统，直接生成一批"看起来像 Windows 路径"
/// 的假数据。生成本身很快，实际扫描节奏由（同样是 mock 的）engine 那边控制。
/// 规模固定 3000 条，用不上流式落盘，仍是内存里的 `Vec<PathBuf>`。
#[cfg(not(any(windows, target_os = "macos")))]
mod mock {
    use super::CancelFlag;
    use super::{report_walk_progress, ScanEvent};
    use std::path::PathBuf;
    use std::sync::atomic::Ordering;
    use std::sync::mpsc::Sender;

    const APP_NAMES: &[&str] = &[
        "Chrome", "Office", "Adobe", "Steam", "Zoom", "Slack", "VSCode", "Docker", "Python",
        "NodeJS", "Git", "7Zip", "VLC", "Notion", "Discord",
    ];
    const EXTENSIONS: &[&str] = &["exe", "dll", "sys"];

    pub fn walk(tx: &Sender<ScanEvent>, cancel: &CancelFlag) -> Vec<PathBuf> {
        const TOTAL: usize = 3000;
        let mut paths = Vec::with_capacity(TOTAL + 1);
        let mut last_reported = 0usize;

        // 保证至少有一个"看起来可疑"的路径，方便预览威胁卡片 UI（是否真的被
        // 标红取决于 engine mock 那边这一轮是不是"该翻到有威胁的一面"）。
        paths.push(PathBuf::from(
            r"C:\Users\Alice\Downloads\setup_crack_v2.exe",
        ));
        report_walk_progress(tx, paths.len(), &mut last_reported);

        for i in 0..TOTAL {
            if cancel.load(Ordering::SeqCst) {
                break;
            }
            let app = APP_NAMES[i % APP_NAMES.len()];
            let ext = EXTENSIONS[i % EXTENSIONS.len()];
            paths.push(PathBuf::from(format!(
                r"C:\Program Files\{app}\bin\module_{i}.{ext}"
            )));
            report_walk_progress(tx, paths.len(), &mut last_reported);
        }
        report_walk_progress(tx, paths.len(), &mut last_reported);
        paths
    }
}
