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

/// 扫描一个用户指定的单个文件或文件夹——右键菜单"用 CLV3000 扫描"/
/// `--scan-path` 命令行参数触发（见 `lifecycle::parse_scan_path`），不是全盘。
/// 阻塞执行，调用者需要自己 spawn 一个线程来跑它（跟 `run` 一致）。
pub fn run_single_target(tx: Sender<ScanEvent>, cancel: CancelFlag, target: std::path::PathBuf) {
    let start = Instant::now();

    if !target.exists() {
        let _ = tx.send(ScanEvent::Error(format!(
            "Path not found: {}",
            target.display()
        )));
        let _ = tx.send(ScanEvent::Finished {
            scanned: 0,
            elapsed: start.elapsed(),
            cancelled: false,
        });
        return;
    }

    // 单个文件：不用 walk，直接扔给引擎；只有目录才需要遍历收集可执行文件。
    let (source, total_files) = if target.is_file() {
        (PathSource::InMemory(vec![target]), 1)
    } else {
        #[cfg(windows)]
        {
            let (path, count) = real_windows::walk_single(&tx, &cancel, &target);
            (PathSource::File { path, count }, count)
        }
        #[cfg(target_os = "macos")]
        {
            let (path, count) = real_macos::walk_single(&tx, &cancel, &target);
            (PathSource::File { path, count }, count)
        }
        #[cfg(not(any(windows, target_os = "macos")))]
        {
            let paths = mock::walk_single_dir(&target);
            let count = paths.len();
            (PathSource::InMemory(paths), count)
        }
    };

    if cancel.load(Ordering::SeqCst) {
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

    let _ = tx.send(ScanEvent::ScanStarted {
        total: Some(total_files),
    });
    engine::run(source, tx, cancel);
}

#[cfg(windows)]
mod real_windows {
    use super::CancelFlag;
    use super::{report_walk_progress, ScanEvent, WalkListWriter};
    use std::path::{Path, PathBuf};
    use std::sync::atomic::Ordering;
    use std::sync::mpsc::Sender;
    use walkdir::WalkDir;

    /// 参与扫描的可执行文件扩展名白名单（大小写不敏感）。
    const EXECUTABLE_EXTENSIONS: &[&str] =
        &["exe", "dll", "sys", "scr", "com", "cpl", "ocx", "drv"];

    /// Win32 `GetDriveTypeW` 返回值，对应 `DRIVE_FIXED` / `DRIVE_REMOVABLE`。
    const DRIVE_FIXED: u32 = 3;
    const DRIVE_REMOVABLE: u32 = 2;

    /// 遍历单个根目录，把匹配白名单扩展名的文件边发现边写进 `writer`。返回
    /// `false` 表示中途被取消（调用者应停止遍历更多 root）。被
    /// `walk`（多盘循环）和 `walk_single`（右键菜单扫指定文件夹）共用，避免
    /// 两处各写一份几乎一样的 `WalkDir` 遍历逻辑。
    fn walk_root(
        root: &Path,
        writer: &mut WalkListWriter,
        tx: &Sender<ScanEvent>,
        cancel: &CancelFlag,
        last_reported: &mut usize,
    ) -> bool {
        for entry in WalkDir::new(root)
            .follow_links(false)
            .into_iter()
            .filter_map(|e| e.ok())
        {
            if cancel.load(Ordering::SeqCst) {
                return false;
            }
            if !entry.file_type().is_file() {
                continue;
            }
            if !has_executable_extension(entry.path()) {
                continue;
            }
            writer.push(entry.path());
            report_walk_progress(tx, writer.count, last_reported);
        }
        true
    }

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

        for root in roots {
            if !walk_root(&root, &mut writer, tx, cancel, &mut last_reported) {
                break;
            }
        }
        report_walk_progress(tx, writer.count, &mut last_reported);
        writer.finish()
    }

    /// 右键菜单"用 CLV3000 扫描"/`--scan-path` 指定的单个文件夹：只遍历这一个
    /// 根目录，不枚举磁盘。返回 (临时文件路径, 条数)。
    pub fn walk_single(
        tx: &Sender<ScanEvent>,
        cancel: &CancelFlag,
        target: &Path,
    ) -> (PathBuf, usize) {
        let mut writer = WalkListWriter::create();
        let mut last_reported = 0usize;
        walk_root(target, &mut writer, tx, cancel, &mut last_reported);
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
    use std::path::{Path, PathBuf};
    use std::sync::atomic::Ordering;
    use std::sync::mpsc::Sender;
    use walkdir::WalkDir;

    /// 廉价正过滤：只有"无扩展名"或扩展名属于 Mach-O 常见载体"的文件才值得
    /// 付出一次 `open()`+`read()` 去嗅探魔数。其余（文本/源码/配置/数据/媒体等）
    /// 一律跳过，省下海量 `open` 系统调用。
    ///
    /// 注意：`.o`（MH_OBJECT 目标文件）已从白名单移除——它们数量巨大却无法独立
    /// 运行，是"扫描了大量非可执行程序"的主要噪声；配合 `is_collectable_macho` 的
    /// `filetype` 过滤，这类纯构建中间产物连 `open` 都不会触发（既不被白名单收下、
    /// 也不在保留的 filetype 之列）。其余扩展名都是会被实际加载执行的 Mach-O 载体
    /// （dylib/so 动态库、bundle/kext 包、macho 裸二进制、node 原生插件）。`.a` 静态
    /// 库不是 Mach-O 魔数（以 `!<arch>` 开头），本就不会被收下，跳过反而省一次
    /// `open`。带其它奇怪扩展名的 Mach-O 在 macOS 上极罕见，本过滤会跳过去；要扩面
    /// 往这个白名单里加扩展名即可。
    const MACHO_LIKELY_EXTENSIONS: &[&str] = &[
        "dylib", "so", "bundle", "kext", "macho", "node",
    ];

    /// 遍历单个根目录，把匹配的 Mach-O 可执行文件边发现边写进 `writer`。返回
    /// `false` 表示中途被取消。被 `walk`（多盘循环）和 `walk_single`（右键菜单/
    /// `--scan-path` 扫指定文件/文件夹）共用。
    ///
    /// 用 `filter_entry` 而不是"进了子树再逐个 entry 判断丢弃"：后者对
    /// `/System/Volumes` 这类整棵子树都要跳过的目录，WalkDir 仍会完整下降进去、
    /// 对每个文件都 stat 一遍，只是最后不收进结果——等于把这块子树白走了一遍。
    /// `filter_entry` 在返回 `false` 时直接不下降这个目录，整棵子树被剪掉。
    /// `is_system_root`：`true` 表示正在遍历启动盘根 `/`，此时 `is_excluded` 会剪掉
    /// `/Volumes`/`/Network`（可移动/网络挂载由 `local_drive_roots` 的显式根单独
    /// 覆盖，避免重复扫描）；`false` 表示遍历的是某个可移动盘或用户显式选中的
    /// 目录，不再二次剪枝 `/Volumes`、`/Network`。
    fn walk_root(
        root: &Path,
        is_system_root: bool,
        writer: &mut WalkListWriter,
        tx: &Sender<ScanEvent>,
        cancel: &CancelFlag,
        last_reported: &mut usize,
    ) -> bool {
        let walker = WalkDir::new(root)
            .follow_links(false)
            .into_iter()
            .filter_entry(|e| {
                // 构建/版本控制中间产物目录：里面几乎没有"会被实际运行"的成品可执行
                // 文件，却常含海量文件（Xcode `DerivedData` 满是 `.o`、`.git` 满是松散
                // 对象），整棵剪掉既缩小收集集、又省下大量 stat / open。注意不放
                // `build`/`target`/`node_modules` 等——它们可能含最终发布的成品可执行
                // 文件，剪掉会有覆盖损失。
                if e.file_type().is_dir() && is_build_artifact_dir(e.file_name()) {
                    return false;
                }
                !is_excluded(e.path(), is_system_root)
            });
        for entry in walker.filter_map(|e| e.ok()) {
            if cancel.load(Ordering::SeqCst) {
                return false;
            }
            if !entry.file_type().is_file() {
                continue;
            }
            let path = entry.path();
            // 廉价正过滤：扩展名不在候选范围的文件直接跳过，连 `open` 都不用做。
            if !is_macho_candidate(path) {
                continue;
            }
            // 空文件不可能是合法 Mach-O，跳过 `open`（省一次系统调用）。
            if entry.metadata().map(|m| m.len()).unwrap_or(1) == 0 {
                continue;
            }
            // 读文件头魔数 + `filetype`：只保留会被实际加载执行的 Mach-O（MH_EXECUTE
            // / MH_DYLIB / MH_BUNDLE），丢弃纯构建中间产物 `.o`（MH_OBJECT）。避免把
            // 数百万个非可执行文件塞进待扫列表、再喂给引擎。
            if !super::super::is_collectable_macho(path) {
                continue;
            }
            writer.push(path);
            report_walk_progress(tx, writer.count, last_reported);
        }
        true
    }

    /// 遍历挂载点，把匹配的 Mach-O 可执行文件路径边发现边写入临时列表文件。
    /// 返回 (临时文件路径, 条数)。
    pub fn walk(
        tx: &Sender<ScanEvent>,
        cancel: &CancelFlag,
        include_removable: bool,
    ) -> (PathBuf, usize) {
        let roots = local_drive_roots(include_removable);
        let mut writer = WalkListWriter::create();
        let mut last_reported = 0usize;

        for (root, is_system_root) in roots {
            if !walk_root(&root, is_system_root, &mut writer, tx, cancel, &mut last_reported) {
                break;
            }
        }
        report_walk_progress(tx, writer.count, &mut last_reported);
        writer.finish()
    }

    /// 右键菜单"用 CLV3000 扫描"/`--scan-path` 指定的单个文件夹：只遍历这一个
    /// 根目录，不枚举挂载点。返回 (临时文件路径, 条数)。
    pub fn walk_single(
        tx: &Sender<ScanEvent>,
        cancel: &CancelFlag,
        target: &Path,
    ) -> (PathBuf, usize) {
        let mut writer = WalkListWriter::create();
        let mut last_reported = 0usize;
        walk_root(target, false, &mut writer, tx, cancel, &mut last_reported);
        report_walk_progress(tx, writer.count, &mut last_reported);
        writer.finish()
    }

    /// 廉价正过滤：是否值得为这个路径付出一次 `open()` 去读魔数。
    /// - 无扩展名 → 可能是二进制（命令、app 内部二进制、框架内部二进制）→ 候选。
    /// - 扩展名在 `MACHO_LIKELY_EXTENSIONS` → 候选。
    /// - 其它扩展名（文本/源码/配置/数据/媒体…）→ 直接跳过，不 `open`。
    fn is_macho_candidate(path: &std::path::Path) -> bool {
        match path.extension().and_then(|e| e.to_str()) {
            None => true,
            Some(e) => MACHO_LIKELY_EXTENSIONS
                .iter()
                .any(|ok| ok.eq_ignore_ascii_case(e)),
        }
    }

    /// 构建/版本控制中间产物目录名（见 `walk_root` 的 `filter_entry`）：整棵剪掉，
    /// 不下降、不 stat 内部的海量文件。只放"绝对不可能是发布成品可执行文件所在"的
    /// 目录，避免覆盖损失。
    fn is_build_artifact_dir(name: &std::ffi::OsStr) -> bool {
        name == std::ffi::OsStr::new("DerivedData") || name == std::ffi::OsStr::new(".git")
    }

    /// 判断某个路径是否属于"不应纳入全盘扫描"的子树，配合 `filter_entry` 整棵剪掉。
    /// `is_system_root` 标记当前正在遍历的是否为启动盘根 `/`：
    /// - 启动盘根遍历时，剪掉 `/Volumes`、`/Network`（可移动/网络挂载由显式根单独
    ///   覆盖，否则会重复扫描）；
    /// - 可移动盘 / 单目标根遍历时（`is_system_root=false`），不再剪 `/Volumes`、
    ///   `/Network`，保证这些盘自身能被完整扫到。
    fn is_excluded(path: &std::path::Path, is_system_root: bool) -> bool {
        const ALWAYS_EXCLUDED: &[&str] = &[
            // 数据卷镜像：firmlink 已把 `/Users`、`/Applications` 等呈现到 `/` 下，
            // 再走一遍会重复扫描同一批文件。
            "/System/Volumes",
            // 休眠镜像 / swap，体积大且不含可执行文件。
            "/private/var/vm",
            // 每用户临时目录 / 缓存。
            "/private/var/folders",
            // Spotlight 索引数据，不含可执行文件。
            "/.Spotlight-V100",
            // autofs 自动挂载点，下降进去可能触发实际网络挂载并卡住整个 walk。
            "/net",
            "/Network",
            "/home",
            // 各类缓存 / 日志 / 派生数据：体积巨大且几乎不含可执行 Mach-O，
            // 全量 `open` 嗅探魔数纯属浪费，也拖慢收集阶段。
            "/System/Library/Caches",
            "/Library/Caches",
            "/private/var/db",
            "/private/var/log",
            "/private/var/spool",
        ];
        if ALWAYS_EXCLUDED.iter().any(|p| path.starts_with(p)) {
            return true;
        }
        // 启动盘根遍历时剪掉可移动 / 网络挂载；它们由 `local_drive_roots` 的显式
        // 根单独覆盖，避免与 `/` 遍历重复。
        if is_system_root && (path.starts_with("/Volumes") || path.starts_with("/Network")) {
            return true;
        }
        false
    }

    /// 枚举磁盘根目录：macOS 启动盘永远是 `/`；可移动盘挂在 `/Volumes/` 下。
    /// 返回 `(根路径, 是否系统根)`——`is_system_root=true` 的 `/` 在遍历时会剪掉
    /// `/Volumes`/`/Network`（这些由下面的显式可移动根覆盖，避免重复扫描），
    /// `false` 的可移动/单目标根则不再二次剪枝。
    fn local_drive_roots(include_removable: bool) -> Vec<(PathBuf, bool)> {
        let mut roots = vec![(PathBuf::from("/"), true)];
        if include_removable {
            if let Ok(entries) = std::fs::read_dir("/Volumes") {
                for e in entries.flatten() {
                    let p = e.path();
                    // 只收真实挂载目录，跳过符号链接：启动盘会以 `Macintosh HD -> /`
                    // 的 symlink 形式出现在 /Volumes 下，若跟随它会把整个系统盘再扫
                    // 一遍（整盘被扫两遍）。`DirEntry::file_type()` 不跟随符号链接，
                    // 所以 symlink→目录在这里 `is_symlink()` 为 true，自然被排除。
                    let ft = match e.file_type() {
                        Ok(ft) => ft,
                        Err(_) => continue,
                    };
                    if ft.is_symlink() || !ft.is_dir() {
                        continue;
                    }
                    // 防御：解析后若等于启动盘 `/`（某些非 symlink 形式的自引用
                    // 挂载），也跳过，避免重复扫描。
                    if std::fs::canonicalize(&p)
                        .map(|c| c.as_os_str() == std::ffi::OsStr::new("/"))
                        .unwrap_or(false)
                    {
                        continue;
                    }
                    roots.push((p, false));
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

    /// 开发预览用：右键菜单/`--scan-path` 扫一个目录时，假装在里面找到几个
    /// "可疑"文件——不真的碰文件系统，只是让这条路径在预览环境里也能点通。
    pub fn walk_single_dir(target: &std::path::Path) -> Vec<PathBuf> {
        vec![
            target.join("suspicious_tool.exe"),
            target.join("subdir").join("payload.dll"),
        ]
    }
}
