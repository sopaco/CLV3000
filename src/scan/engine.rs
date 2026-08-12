//! ClamAV 调用封装：闪电扫描和全盘扫描共用这一份逻辑。
//!
//! 设计要点（对应技术方案 3.3 节）：
//! - 只 spawn 一次 `clamscan`，用 `--file-list=<临时文件>` 传入待扫描路径。
//!   ClamAV 1.5.x 不支持 `--file-list=-`（stdin），必须用实际文件。
//! - 用 `-v` + `--stdout` 让 clamscan 对每个文件都输出一行结果到 stdout，从而拿到逐文件级进度。
//! - 取消扫描：一个"看门狗"线程检测到取消标记后直接 kill 子进程，stdout 管道关闭后读取循环自然退出。
//!
//! 平台差异只在"怎么叫起 clamscan"这一段，扫描编排逻辑（临时文件、stdout 解析、
//! 看门狗、基因缓存、签名预筛）是完全共享的：
//! - Windows 调 `clamscan.exe`，并加 `CREATE_NO_WINDOW` 避免弹出黑框，`--scan-pe=yes`。
//! - macOS 调 `clamscan`（系统安装 / 随包内置 / PATH 兜底），无 `creation_flags`，`--scan-pe=no`
//!   （macOS 没有 PE，Mach-O 由 ClamAV 单独解析，不受此开关影响）。
//! - 病毒库目录通过 `paths::resolved_clamav_database_dir()` 显式传给 `--database=`，
//!   优先用内置/系统安装/用户级 `~/.clamav`，不依赖 clamscan 编译期默认目录。
//! - 其它（Linux 等开发机预览）：`mock` 用同样的 `Vec<PathBuf> -> ScanEvent` 接口
//!   模拟扫描过程，方便在开发机上直接看 UI/交互效果。

use super::{CancelFlag, PathSource, ScanEvent};
use std::sync::mpsc::Sender;

/// 消费 `source` 里的路径，交给扫描引擎处理，结果通过 `tx` 发出去。
///
/// 这个函数是阻塞的，调用者需要自己在后台线程里跑它。
pub fn run(source: PathSource, tx: Sender<ScanEvent>, cancel: CancelFlag) {
    #[cfg(any(windows, target_os = "macos"))]
    real::run(source, tx, cancel);
    #[cfg(not(any(windows, target_os = "macos")))]
    mock::run(source, tx, cancel);
}

#[cfg(any(windows, target_os = "macos"))]
mod real {
    use super::super::authenticode;
    use super::super::cache;
    use super::super::{CancelFlag, PathSource, ScanEvent};
    use crate::paths;
    use std::collections::HashMap;
    use std::io::{BufRead, BufReader, Write};
    use std::path::{Path, PathBuf};
    use std::process::{Command, Stdio};
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::mpsc::Sender;
    use std::sync::{Arc, Mutex};
    use std::time::Instant;

    /// 预筛结束后、在当前（已由调用者放到后台的）线程上同步执行的 clamscan 批次。
    struct ClamscanWork {
        to_scan: Vec<PathBuf>,
        hash_by_path: HashMap<String, String>,
        cache: cache::ScanCache,
        scanned: usize,
    }

    /// `run_clamscan_batch` 的返回值：本批扫描数、更新后的缓存、可选的 stderr 输出。
    type ClamscanOutcome = (usize, cache::ScanCache, Option<String>);

    // `creation_flags` 只在 Windows 上有意义；非 Windows 构建不需要这个 trait，门控掉避免 unused import。
    #[cfg(windows)]
    use std::os::windows::process::CommandExt;

    /// 不弹出黑色命令行窗口（仅 Windows 有意义）。
    #[cfg(windows)]
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;

    /// 大于这个体积的文件不进缓存（避免无谓地哈希大文件）；它们仍会被 ClamAV 正常扫描。
    const CACHE_SKIP_SIZE: u64 = 64 * 1024 * 1024;

    pub fn run(source: PathSource, tx: Sender<ScanEvent>, cancel: CancelFlag) {
        let start = Instant::now();
        // 已扫文件计数：缓存命中的文件也会在这里累加，UI 进度才对得上总数。
        let mut scanned: usize = 0;

        if !paths::clamscan_available() {
            let _ = tx.send(ScanEvent::Error(format!(
                "Scan engine not found: {}\nMake sure ClamAV is installed (or bundled in the clamav/ directory).",
                paths::clamscan_path().display()
            )));
            let _ = tx.send(ScanEvent::Finished {
                scanned: 0,
                elapsed: start.elapsed(),
                cancelled: cancel.load(Ordering::SeqCst),
            });
            return;
        }

        if cancel.load(Ordering::SeqCst) {
            let _ = tx.send(ScanEvent::Finished {
                scanned: 0,
                elapsed: start.elapsed(),
                cancelled: true,
            });
            return;
        }

        let paths = load_path_source(source);

        // 枚举阶段未发现任何可扫文件：直接结束，不打开缓存、不碰 clamscan。
        if paths.is_empty() {
            let _ = tx.send(ScanEvent::Finished {
                scanned: 0,
                elapsed: start.elapsed(),
                cancelled: false,
            });
            return;
        }

        // ── 文件基因缓存：按内容哈希复用上次结果，重复扫描近乎免费 ──
        let cache_path = paths::app_data_dir().join("scan_cache.tsv");
        paths::ensure_dir(&paths::app_data_dir());
        // 解析真实可用的病毒库目录：缓存以"所用库目录 + 版本"为身份，库变了自动失效。
        // 没有可用库目录时退回内置路径（此时扫描多半会因 clamscan 找不到库而报错，属正常拦截）。
        let resolved_db = paths::resolved_clamav_database_dir();
        let effective_db = resolved_db
            .clone()
            .unwrap_or_else(|| paths::clamav_database_dir());
        // 缓存不可用不会阻断扫描：open 内部失败会退化为空缓存，insert 失败也会静默忽略。
        let mut cache = cache::ScanCache::open(&cache_path, &effective_db);

        // 预筛：缓存/签名可即时判定的立刻发 `FileScanned`，需 clamscan 的记入
        // 待扫列表——对用户呈现为统一的「逐文件检测」进度。多核机器上并行跑
        // （见 `prescan`），单核或列表很小时自动退化为顺序执行。`prescan` 内部
        // 不共享任何可变缓存状态（教训见 `cache::CacheSnapshot` 的文档），这里
        // 传 `&mut cache` 只是给它"跑完之后把新学到的记录写回来"，不会被多线程
        // 并发访问。
        let (to_scan, hash_by_path, prescanned) = prescan(&paths, &tx, &cancel, &mut cache);
        scanned += prescanned;

        let pending = to_scan.len();

        // 预筛后无需 clamscan（全部命中基因缓存 / 可信签名）：跳过 spawn 与病毒库加载。
        if pending == 0 {
            finish_scan(start, scanned, &cancel, &tx, cache, None);
            return;
        }

        if cancel.load(Ordering::SeqCst) {
            finish_scan(start, scanned, &cancel, &tx, cache, None);
            return;
        }

        let (final_scanned, final_cache, stderr) = run_clamscan_batch(
            ClamscanWork {
                to_scan,
                hash_by_path,
                cache,
                scanned,
            },
            &tx,
            &cancel,
            &resolved_db,
        );
        finish_scan(
            start,
            final_scanned,
            &cancel,
            &tx,
            final_cache,
            stderr.as_deref(),
        );
    }

    /// 把 `PathSource` 归一化成内存里的 `Vec<PathBuf>`。
    ///
    /// - `InMemory`：闪电扫描的枚举结果，本来就不大，直接用。
    /// - `File`：全盘扫描 walk 阶段边发现边流式写盘的临时列表（见
    ///   `full_scan.rs` 的 `WalkListWriter`）——这里做**一次性**读回，发生在
    ///   磁盘遍历已经结束之后：读的是本机刚写完、还热在系统页缓存里的本地
    ///   文件，跟"walk 期间越走越大的 Vec"是两件不同的事——walk 那段可能耗时
    ///   很久（遍历整个磁盘），期间内存只有一个计数器；这里的读回是遍历结束后
    ///   的一次性操作，千级到十万级路径通常在毫秒到几百毫秒内读完。之所以还要
    ///   落回一个 `Vec`：预筛阶段要把路径分片给多个 worker 线程并行处理（见
    ///   `prescan`），最简单可靠的分片方式就是一段连续的 `&[PathBuf]`。
    ///   读完立即删除这份临时文件——它已经被完整消费进内存，没有必要留到整个
    ///   扫描结束才清理。
    fn load_path_source(source: PathSource) -> Vec<PathBuf> {
        match source {
            PathSource::InMemory(paths) => paths,
            PathSource::File { path, count } => {
                if count == 0 {
                    let _ = std::fs::remove_file(&path);
                    return Vec::new();
                }
                let mut paths = Vec::with_capacity(count);
                if let Ok(f) = std::fs::File::open(&path) {
                    for line in BufReader::new(f).lines().map_while(Result::ok) {
                        if !line.is_empty() {
                            paths.push(PathBuf::from(line));
                        }
                    }
                }
                let _ = std::fs::remove_file(&path);
                paths
            }
        }
    }

    /// 预筛单个文件：先查文件基因缓存（含 `quick_hash` 快速路径，见
    /// `cache::quick_hash` 的安全权衡说明），再查可信签名；两者都没命中就交给
    /// clamscan（`NeedsScan`）。
    enum PrescanOutcome {
        /// 缓存/签名已判定，已经发出 `FileScanned`，不需要 clamscan 再扫一遍。
        Resolved,
        /// 需要交给 clamscan；`hash` 是"算出来了、体积也没超限"的内容哈希——
        /// clamscan 给出结果后要用它把新结果回写缓存（见 `run_clamscan_batch`）。
        /// 大文件跳过哈希、或者读文件失败时为 `None`，不参与后续缓存回写。
        NeedsScan { hash: Option<String> },
    }

    /// 预筛期间新学到的一条缓存记录——不直接写进任何共享的 `ScanCache`，而是
    /// 由调用方本地攢起来，等所有 worker 都跑完之后单线程一次性应用（见
    /// `cache::CacheSnapshot` 顶部关于"为什么不能共享可变缓存"的说明）。
    enum CacheWrite {
        /// 路径级快速哈希：`quick_hash` miss、真的读了文件算出哈希之后要记住。
        PathHash {
            path: String,
            size: u64,
            mtime_ns: i128,
            hash: String,
        },
        /// 内容哈希级判定：可信签名直接判为 clean。
        Verdict { hash: String, result: String },
    }

    fn prescan_one(
        p: &Path,
        snapshot: &cache::CacheSnapshot,
        tx: &Sender<ScanEvent>,
        writes: &mut Vec<CacheWrite>,
    ) -> PrescanOutcome {
        let path_str = p.display().to_string();
        let _ = tx.send(ScanEvent::ScanningFile {
            path: path_str.clone(),
        });

        let meta = match std::fs::metadata(p) {
            Ok(m) => m,
            Err(_) => return PrescanOutcome::NeedsScan { hash: None },
        };
        if meta.len() > CACHE_SKIP_SIZE {
            return PrescanOutcome::NeedsScan { hash: None };
        }
        let size = meta.len();
        let mtime_ns = cache::mtime_ns(&meta);

        // 快速路径：路径 + (size, mtime) 跟快照里记录的一致就直接拿旧哈希，
        // 省掉重新读整个文件算 blake3；miss 了才真的读文件，并把新哈希记到
        // 本地 `writes`（不直接写共享状态）。
        let hash = match snapshot.quick_hash(&path_str, size, mtime_ns) {
            Some(h) => Some(h),
            None => {
                let h = cache::file_hash(p);
                if let Some(h) = &h {
                    writes.push(CacheWrite::PathHash {
                        path: path_str.clone(),
                        size,
                        mtime_ns,
                        hash: h.clone(),
                    });
                }
                h
            }
        };

        let Some(hash) = hash else {
            return PrescanOutcome::NeedsScan { hash: None };
        };

        if let Some(res) = snapshot.lookup(&hash) {
            let infected = if res == "clean" { None } else { Some(res) };
            let _ = tx.send(ScanEvent::FileScanned {
                path: path_str,
                infected,
            });
            return PrescanOutcome::Resolved;
        }

        // WinVerifyTrust / codesign 都是"每次调用独立状态，不依赖全局可变数据"的
        // 只读校验 API，多个 worker 线程各自并发调用是安全的（跟 clamscan 那段
        // 单进程串行完全不同，预筛这里天然是可并行的 I/O + 校验工作）。
        if authenticode::is_trusted_signed(p) {
            let _ = tx.send(ScanEvent::FileScanned {
                path: path_str,
                infected: None,
            });
            writes.push(CacheWrite::Verdict {
                hash: hash.clone(),
                result: "clean".to_string(),
            });
            return PrescanOutcome::Resolved;
        }

        PrescanOutcome::NeedsScan { hash: Some(hash) }
    }

    /// 一个分片（顺序执行时是整份列表，并行时是一个 worker 的那一块）跑完后的
    /// 原始结果：待 clamscan 扫的路径 + 路径→哈希映射（结果回写缓存用）+ 已经
    /// 就地判完（缓存命中/签名可信）的文件数 + 这个分片本地攢下的新缓存记录。
    type PrescanChunkRaw = (Vec<PathBuf>, HashMap<String, String>, usize, Vec<CacheWrite>);
    /// `prescan()` 对外的返回类型：`CacheWrite` 已经在 `prescan()` 内部合并写回
    /// 真正的 `ScanCache`，调用方不需要再看到它。
    type PrescanChunkResult = (Vec<PathBuf>, HashMap<String, String>, usize);

    /// 只读预筛一个分片，只依赖不可变的 `&CacheSnapshot`——可以被多个线程各自
    /// 借用同一份快照并发调用，不需要任何锁（单核/小列表时也直接复用这同一份
    /// 实现顺序跑一遍）。
    fn prescan_chunk(
        paths: &[PathBuf],
        snapshot: &cache::CacheSnapshot,
        tx: &Sender<ScanEvent>,
        cancel: &CancelFlag,
    ) -> PrescanChunkRaw {
        let mut to_scan = Vec::new();
        let mut hash_by_path = HashMap::new();
        let mut scanned = 0usize;
        let mut writes = Vec::new();
        for p in paths {
            if cancel.load(Ordering::SeqCst) {
                break;
            }
            match prescan_one(p, snapshot, tx, &mut writes) {
                PrescanOutcome::Resolved => scanned += 1,
                PrescanOutcome::NeedsScan { hash } => {
                    if let Some(hash) = hash {
                        hash_by_path.insert(p.display().to_string(), hash);
                    }
                    to_scan.push(p.clone());
                }
            }
        }
        (to_scan, hash_by_path, scanned, writes)
    }

    /// 预筛最多开这么多个 worker 线程——多了在给 clamscan 阶段留 CPU 之外也没什么
    /// 意义，反而增加调度抖动。
    const PRESCAN_MAX_WORKERS: usize = 8;
    /// 每个 worker 至少要分到这么多文件才值得开线程；闪电扫描枚举到的文件常常
    /// 只有几十到几百个，开线程的调度开销比省下来的时间还多，这时退化为顺序执行。
    const PRESCAN_MIN_FILES_PER_WORKER: usize = 32;

    fn prescan_worker_count(total: usize) -> usize {
        if total < PRESCAN_MIN_FILES_PER_WORKER * 2 {
            return 1;
        }
        let cpus = std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(1);
        let by_workload = total / PRESCAN_MIN_FILES_PER_WORKER;
        cpus.min(by_workload).clamp(1, PRESCAN_MAX_WORKERS)
    }

    /// 预筛这一步的活儿——读盘算哈希（CPU 密集）+ 查缓存（几乎零成本）+ 校验
    /// 可信签名（I/O/子进程等待为主）——天然可并行，单线程串行只用得上一个核。
    ///
    /// 并发模型刻意选择"不共享可变缓存"：`cache.snapshot()` 拷贝一份不可变
    /// 只读快照，多个 worker 用 `thread::scope` 按连续区间分片、各自借用同一份
    /// `&CacheSnapshot`（作用域内借用栈上的 `paths`/`snapshot` 是安全的，
    /// `scope` 保证所有子线程在返回前 join 完，不需要 `Arc`）；每个 worker
    /// 新学到的记录本地攢成 `Vec<CacheWrite>` 随结果一起返回，全部 worker join
    /// 完之后单线程把它们应用到真正的 `cache`。
    ///
    /// 这个设计是为了绕开一次真实死锁（早期版本细粒度锁共享 `ScanCache`，6
    /// worker 实测卡死）——完整事故记录见 `CacheSnapshot` 的文档注释和
    /// `clv3000-scan-engine-pitfalls` skill。**不要**再引入跨 worker 共享的锁
    /// 保护缓存，宁可多付一次快照拷贝的成本。
    fn prescan(
        paths: &[PathBuf],
        tx: &Sender<ScanEvent>,
        cancel: &CancelFlag,
        cache: &mut cache::ScanCache,
    ) -> PrescanChunkResult {
        let snapshot = cache.snapshot();
        let worker_count = prescan_worker_count(paths.len());

        let (to_scan, hash_by_path, scanned, writes) = if worker_count <= 1 {
            prescan_chunk(paths, &snapshot, tx, cancel)
        } else {
            let chunk_size = paths.len().div_ceil(worker_count).max(1);
            let results: Mutex<Vec<PrescanChunkRaw>> =
                Mutex::new(Vec::with_capacity(worker_count));

            std::thread::scope(|scope| {
                for chunk in paths.chunks(chunk_size) {
                    let tx = tx.clone();
                    // 用引用重新绑定同名变量再 move：见下方 `results` 的说明。
                    let results = &results;
                    let snapshot = &snapshot;
                    scope.spawn(move || {
                        let local = prescan_chunk(chunk, snapshot, &tx, cancel);
                        // 这里是并行阶段唯一的一次上锁——每个 worker 只在整块
                        // 分片处理完之后才锁一次 `results` 把结果推进去，跟
                        // "每个文件锁几次"完全是两个数量级的竞争强度，不会重现
                        // 上面文档说的那种卡死。
                        results.lock().unwrap().push(local);
                    });
                }
            });

            let mut to_scan = Vec::new();
            let mut hash_by_path = HashMap::new();
            let mut scanned = 0usize;
            let mut writes = Vec::new();
            for (ts, hp, s, w) in results.into_inner().unwrap() {
                to_scan.extend(ts);
                hash_by_path.extend(hp);
                scanned += s;
                writes.extend(w);
            }
            (to_scan, hash_by_path, scanned, writes)
        };

        // 单线程把所有 worker（或顺序路径自己）攢下的新记录写回真正的 `cache`。
        for w in writes {
            match w {
                CacheWrite::PathHash { path, size, mtime_ns, hash } => {
                    cache.remember_path_hash(&path, size, mtime_ns, &hash);
                }
                CacheWrite::Verdict { hash, result } => {
                    cache.insert(&hash, &result);
                }
            }
        }

        (to_scan, hash_by_path, scanned)
    }

    /// 执行 clamscan 批次。每次 spawn 都是新进程，须重新加载病毒库（约数秒）；
    /// 与预筛是否并行无关——子进程退出后内存中的库即释放。
    fn run_clamscan_batch(
        work: ClamscanWork,
        tx: &Sender<ScanEvent>,
        cancel: &CancelFlag,
        resolved_db: &Option<PathBuf>,
    ) -> ClamscanOutcome {
        let mut scanned = work.scanned;
        let mut cache = work.cache;
        let to_scan = work.to_scan;
        let hash_by_path = work.hash_by_path;
        let pending = to_scan.len();

        let _ = tx.send(ScanEvent::EngineLoading { remaining: pending });

        let temp_path = std::env::temp_dir()
            .join(format!("clv3000_scanlist_{}.txt", std::process::id()));
        if let Err(e) = write_path_list(&temp_path, &to_scan) {
            let _ = tx.send(ScanEvent::Error(format!("Failed to write scan list: {e}")));
            return (scanned, cache, None);
        }

        let mut cmd = Command::new(paths::clamscan_path());
        cmd.arg("--no-summary")
            .arg("-v")
            .arg("--stdout")
            .arg(format!("--file-list={}", temp_path.display()));
        if let Some(db) = resolved_db {
            cmd.arg(format!("--database={}", db.display()));
        }
        apply_scan_flags(&mut cmd);
        #[cfg(windows)]
        cmd.creation_flags(CREATE_NO_WINDOW);
        cmd.stdout(Stdio::piped()).stderr(Stdio::piped());

        let mut child = match cmd.spawn() {
            Ok(c) => c,
            Err(e) => {
                let _ = std::fs::remove_file(&temp_path);
                let _ = tx.send(ScanEvent::Error(format!("Failed to start scan engine: {e}")));
                return (scanned, cache, None);
            }
        };

        let stdout = match child.stdout.take() {
            Some(s) => s,
            None => {
                let _ = child.kill();
                let _ = std::fs::remove_file(&temp_path);
                let _ = tx.send(ScanEvent::Error("Failed to read scan engine stdout".to_string()));
                return (scanned, cache, None);
            }
        };
        let stderr = child.stderr.take();
        let child = Arc::new(Mutex::new(child));

        let watchdog_cancel = cancel.clone();
        let watchdog_child = Arc::clone(&child);
        let watchdog_done = Arc::new(AtomicBool::new(false));
        let watchdog_done_flag = Arc::clone(&watchdog_done);
        let watchdog = std::thread::spawn(move || {
            while !watchdog_done_flag.load(Ordering::SeqCst) {
                if watchdog_cancel.load(Ordering::SeqCst) {
                    if let Ok(mut c) = watchdog_child.lock() {
                        let _ = c.kill();
                    }
                    break;
                }
                std::thread::sleep(std::time::Duration::from_millis(100));
            }
        });

        let stderr_thread = std::thread::spawn(move || -> String {
            let Some(stderr) = stderr else { return String::new() };
            BufReader::new(stderr)
                .lines()
                .filter_map(|l| l.ok())
                .collect::<Vec<_>>()
                .join("\n")
        });

        let reader = BufReader::new(stdout);
        for line in reader.lines() {
            let Ok(line) = line else { break };
            if let Some(path) = line.strip_prefix("Scanning ") {
                let path = path.trim();
                if !path.is_empty() {
                    let _ = tx.send(ScanEvent::ScanningFile {
                        path: path.to_string(),
                    });
                }
                continue;
            }
            if let Some((path, status)) = rsplit_result_line(&line) {
                let verdict = parse_verdict(status);
                // 只有真正扫过的文件（干净或检出）才写缓存；`Unscannable`（clamscan
                // 报 "Empty file"/"Access denied" 之类，没能真正打开/扫描这个文件）
                // 绝不能被记成"clean"——否则下次遇到同一份内容会直接跳过 ClamAV，
                // 把"没扫成"永久当成"扫过且安全"，缓存 TTL 到期前都揪不出真的威胁。
                if !matches!(verdict, ScanVerdict::Unscannable)
                    && let Some(hash) = hash_by_path.get(path)
                {
                    let result = match &verdict {
                        ScanVerdict::Infected(name) => name.clone(),
                        _ => "clean".to_string(),
                    };
                    cache.insert(hash, &result);
                }
                scanned += 1;
                let infected = match verdict {
                    ScanVerdict::Infected(name) => Some(name),
                    ScanVerdict::Clean | ScanVerdict::Unscannable => None,
                };
                let _ = tx.send(ScanEvent::FileScanned {
                    path: path.to_string(),
                    infected,
                });
            }
        }

        watchdog_done.store(true, Ordering::SeqCst);
        let _ = watchdog.join();
        let stderr_output = stderr_thread.join().unwrap_or_default();

        if let Ok(mut c) = child.lock() {
            let _ = c.wait();
        }

        let _ = std::fs::remove_file(&temp_path);
        let stderr = if stderr_output.is_empty() {
            None
        } else {
            Some(stderr_output)
        };
        (scanned, cache, stderr)
    }

    /// 先发 `Finished` 让 UI 立刻进入 Done，再在后台线程落盘基因缓存（避免 compact
    /// 重写 .tsv 阻塞扫描线程、拖慢 Done 页出现）。
    fn finish_scan(
        start: Instant,
        scanned: usize,
        cancel: &CancelFlag,
        tx: &Sender<ScanEvent>,
        cache: cache::ScanCache,
        stderr_output: Option<&str>,
    ) {
        let cancelled = cancel.load(Ordering::SeqCst);
        if scanned == 0 {
            if let Some(stderr) = stderr_output {
                if !stderr.is_empty() {
                    let _ = tx.send(ScanEvent::Error(stderr.to_string()));
                }
            }
        }
        let _ = tx.send(ScanEvent::Finished {
            scanned,
            elapsed: start.elapsed(),
            cancelled,
        });
        std::thread::spawn(move || {
            let mut cache = cache;
            cache.save();
        });
    }

    /// 把路径列表写入临时文件，每行一个路径（LF 换行，UTF-8 无 BOM）。
    fn write_path_list(path: &Path, paths: &[PathBuf]) -> std::io::Result<()> {
        let mut file = std::fs::File::create(path)?;
        for p in paths {
            writeln!(file, "{}", p.display())?;
        }
        file.flush()?;
        Ok(())
    }

    /// clamscan 一行输出形如 `/path/to/file: OK` 或
    /// `/path/to/file: Win.Test.EICAR_HDB-1 FOUND`。
    /// 用 `rsplit_once(": ")` 找最后一个 "冒号+空格" 作为状态分隔符是安全的：
    /// 路径本身（macOS 用 `/`、Windows 用 `X:\`）不含状态分隔所需的 "colon+space"，
    /// 而 `-v` 的 `Scanning <path>` 提示行没有 ": " 会被过滤掉。
    fn rsplit_result_line(line: &str) -> Option<(&str, &str)> {
        line.rsplit_once(": ")
    }

    /// clamscan 对一个文件给出的三种可能结果。区分 `Unscannable` 是关键——它覆盖
    /// `Empty file`/`Access denied`/`Can't open file` 之类"clamscan 没能真正扫这个
    /// 文件"的状态行，既不是 `OK` 也没有 ` FOUND` 后缀。旧实现把这类状态一律并进
    /// "没检出" 分支，调用点又把"没检出"直接当"clean"写缓存——等于把"根本没扫"
    /// 缓存成"扫过且干净"。
    enum ScanVerdict {
        Clean,
        Infected(String),
        Unscannable,
    }

    fn parse_verdict(status: &str) -> ScanVerdict {
        let status = status.trim();
        if status == "OK" {
            ScanVerdict::Clean
        } else if let Some(name) = status.strip_suffix(" FOUND") {
            ScanVerdict::Infected(name.to_string())
        } else {
            ScanVerdict::Unscannable
        }
    }

    /// 速度优化相关的扫描开关。跳过与可执行文件无关的格式，并对 PE 解析按平台微调。
    fn apply_scan_flags(cmd: &mut Command) {
        // 关闭 bytecode（单文件耗时杀手：JIT 编译+模拟执行，最多 5s）与 PUA 签名匹配。
        cmd.arg("--scan-elf=no") // 不扫 ELF：这里只处理 Windows/macOS，PE/Mach-O 由各自解析器处理，与 ELF 无关
            .arg("--scan-archive=no") // 目标是独立可执行文件，非自解压包
            .arg("--scan-mail=no")
            .arg("--scan-pdf=no")
            .arg("--scan-html=no")
            .arg("--scan-xmldocs=no")
            .arg("--scan-swf=no")
            .arg("--scan-hwp3=no")
            .arg("--scan-onenote=no")
            .arg("--scan-image=no")
            .arg("--scan-image-fuzzy-hash=no")
            .arg("--phishing-sigs=no")
            .arg("--phishing-scan-urls=no")
            .arg("--bytecode=no")
            .arg("--detect-pua=no")
            // 大小与超时限制：跳过过大的文件、限制单文件扫描时间。
            .arg("--max-filesize=100M")
            .arg("--max-scansize=200M")
            .arg("--max-scantime=5000"); // 单文件扫描超时上限 5 秒，超时则判为干净（避免个别文件拖垮整体）

        // PE 解析：Windows 目标就是 PE，开启；macOS 没有 PE，关掉省一次解析开销
        // （Mach-O 由 ClamAV 单独的解析器处理，不受 --scan-pe 开关影响）。
        #[cfg(windows)]
        cmd.arg("--scan-pe=yes");
        #[cfg(target_os = "macos")]
        cmd.arg("--scan-pe=no");
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        /// 造一批临时文件（各自内容不同，保证内容哈希不同），返回它们的完整路径。
        fn make_temp_files(dir: &Path, count: usize) -> Vec<PathBuf> {
            let mut paths = Vec::with_capacity(count);
            for i in 0..count {
                let p = dir.join(format!("f{i}.bin"));
                let mut f = std::fs::File::create(&p).unwrap();
                // 内容跟着索引变化，保证每个文件的 blake3 哈希互不相同。
                write!(f, "clv3000-prescan-test-file-{i}").unwrap();
                paths.push(p);
            }
            paths
        }

        /// 并行预筛（`prescan`，走 `thread::scope` 分片）在足够多文件、多核机器
        /// 上会真的分片跑多个 worker——结果必须跟"只读快照 + 单线程顺序处理"
        /// （`prescan_chunk` 一次性喂完整列表）在同样输入下完全一致：冷缓存 +
        /// 未签名的临时文件，两条路径都该把全部文件判定为"需要交给 clamscan"，
        /// 一个不多一个不少。这是这次改动里风险最高的部分（多线程分片处理 +
        /// 结果聚合 + 写回真正的缓存）——早期实现让多个 worker 直接共享一个
        /// `Mutex<ScanCache>`，在这台机器上 6 个 worker 并发下直接卡死（细节
        /// 见 `prescan`/`cache::CacheSnapshot` 的文档），这个测试就是为了盯住
        /// 不要回归到那个设计：必须验证没有死锁、没有路径丢失/重复。
        #[test]
        fn parallel_prescan_matches_sequential_on_cold_cache() {
            let dir = std::env::temp_dir().join("clv3000_engine_test_parallel");
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).unwrap();
            let db_dir = dir.join("db"); // 空目录，dbrev 确定且稳定
            std::fs::create_dir_all(&db_dir).unwrap();

            // 造够多文件、确保触发并行分支（PRESCAN_MIN_FILES_PER_WORKER=32，
            // 总数需 ≥ 64 才会尝试并行；200 个在多核机器上必然分给 >1 个 worker）。
            const N: usize = 200;
            let paths = make_temp_files(&dir, N);

            let (tx, _rx) = std::sync::mpsc::channel();
            let cancel = crate::scan::new_cancel_flag();

            let mut cache_a = cache::ScanCache::open(&dir.join("cache_a.tsv"), &db_dir);
            let (to_scan_par, hash_par, scanned_par) = prescan(&paths, &tx, &cancel, &mut cache_a);

            let cache_b = cache::ScanCache::open(&dir.join("cache_b.tsv"), &db_dir);
            let snapshot_b = cache_b.snapshot();
            let (to_scan_seq, hash_seq, scanned_seq, _writes) =
                prescan_chunk(&paths, &snapshot_b, &tx, &cancel);

            // 冷缓存 + 未签名的临时文件：两条路径都应该把全部文件判定为
            // "需要 clamscan"，没有文件被就地判定。
            assert_eq!(scanned_par, 0, "冷缓存下不该有文件被就地判定");
            assert_eq!(scanned_seq, 0);
            assert_eq!(to_scan_par.len(), N);
            assert_eq!(to_scan_seq.len(), N);

            // 用集合比较而非顺序比较：并行分片之间完成的先后顺序不保证跟输入
            // 顺序一致（每个 chunk 内部顺序保留，chunk 之间谁先 push 进
            // `results` 不确定），但集合内容必须跟输入完全对上。
            let set_par: std::collections::HashSet<_> = to_scan_par.iter().collect();
            let set_seq: std::collections::HashSet<_> = to_scan_seq.iter().collect();
            let input_set: std::collections::HashSet<_> = paths.iter().collect();
            assert_eq!(set_par, input_set, "并行预筛必须覆盖全部输入路径，不丢不重");
            assert_eq!(set_seq, input_set);

            // hash_by_path 要覆盖每一个 to_scan 里的路径——这些文件都很小，
            // 能正常读出内容哈希，clamscan 结果回写缓存需要这份映射。
            for p in &to_scan_par {
                assert!(
                    hash_par.contains_key(&p.display().to_string()),
                    "to_scan 里的路径必须有对应的哈希记录"
                );
            }
            assert_eq!(hash_par.len(), N);
            assert_eq!(hash_seq.len(), N);

            let _ = std::fs::remove_dir_all(&dir);
        }

        /// 端到端验证 `quick_hash`/`remember_path_hash` 接上 `prescan` 之后的
        /// 正确性：第一轮冷缓存全部进 `to_scan`，模拟 clamscan 扫完后把结果
        /// （clean）回写缓存并落盘；第二轮重新打开缓存对同一批文件再跑一次
        /// 预筛，应该全部被缓存就地判定为 clean，一个都不用再交给 clamscan。
        /// I/O 节省本身已经在 `cache.rs` 的单元测试里直接验证过，这里关心的
        /// 是接线正确性——如果 `quick_hash`/`remember_path_hash` 用的 key
        /// 不一致，或者 size/mtime 换算哪里错了，第二轮会退化成又全部落进
        /// `to_scan`，测试会失败。用 `prescan()`（而不是更底层的
        /// `prescan_chunk`）走完整路径，顺带验证 `CacheWrite` 的合并写回逻辑。
        #[test]
        fn prescan_reuses_cache_across_two_passes() {
            let dir = std::env::temp_dir().join("clv3000_engine_test_cache_reuse");
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).unwrap();
            let db_dir = dir.join("db");
            std::fs::create_dir_all(&db_dir).unwrap();

            const N: usize = 10;
            let paths = make_temp_files(&dir, N);
            let (tx, _rx) = std::sync::mpsc::channel();
            let cancel = crate::scan::new_cancel_flag();
            let cache_path = dir.join("cache.tsv");

            // 第一轮：冷缓存，全部落进 to_scan；模拟 clamscan 扫完把结果回写
            // 缓存（跟 `run_clamscan_batch` 里"OK → clean"的逻辑一致）。
            {
                let mut cache = cache::ScanCache::open(&cache_path, &db_dir);
                let (to_scan, hash_by_path, scanned) =
                    prescan(&paths, &tx, &cancel, &mut cache);
                assert_eq!(to_scan.len(), N);
                assert_eq!(scanned, 0);
                for p in &to_scan {
                    let hash = hash_by_path.get(&p.display().to_string()).unwrap();
                    cache.insert(hash, "clean");
                }
                cache.save();
            }

            // 第二轮：重新 open（模拟下一次扫描），缓存里已经有这批文件的记录——
            // 应该全部被就地判定为 clean，不再进 to_scan。
            {
                let mut cache = cache::ScanCache::open(&cache_path, &db_dir);
                let (to_scan, _hash_by_path, scanned) =
                    prescan(&paths, &tx, &cancel, &mut cache);
                assert_eq!(to_scan.len(), 0, "第二轮缓存应该全部命中，不需要再交给 clamscan");
                assert_eq!(scanned, N);
            }

            let _ = std::fs::remove_dir_all(&dir);
        }
    }
}

/// 开发预览用的假扫描引擎：不碰真实文件，只是按小延迟消费路径列表，
/// 模拟"逐个文件扫完"的节奏，方便在 Linux 等开发机上看 UI 动起来的效果。
#[cfg(not(any(windows, target_os = "macos")))]
mod mock {
    use super::super::{CancelFlag, PathSource, ScanEvent};
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::mpsc::Sender;
    use std::time::{Duration, Instant};

    /// 每跑一次就翻一次面：让"重新扫描"能交替看到"未发现威胁"和"发现威胁"两种结果页样式。
    static RUN_COUNTER: AtomicUsize = AtomicUsize::new(0);

    /// mock 引擎只在非 Windows/macOS 目标上编译——同一目标上 `full_scan.rs` 的
    /// 真实 walk 实现（会产出 `PathSource::File`）也不会编译，所以这里实际只
    /// 会收到 `InMemory`；`File` 分支给个保守的空列表兜底，不 panic。
    pub fn run(source: PathSource, tx: Sender<ScanEvent>, cancel: CancelFlag) {
        let paths = match source {
            PathSource::InMemory(paths) => paths,
            PathSource::File { .. } => Vec::new(),
        };
        let start = Instant::now();
        let should_flag = RUN_COUNTER.fetch_add(1, Ordering::Relaxed).is_multiple_of(2);

        let mut scanned = 0usize;
        let mut flagged = false;
        let mut last_path: Option<String> = None;

        for path in paths {
            if cancel.load(Ordering::SeqCst) {
                break;
            }
            std::thread::sleep(Duration::from_millis(2));

            let path_str = path.display().to_string();
            let looks_suspicious = path_str.to_lowercase().contains("downloads");
            let infected = if should_flag && !flagged && looks_suspicious {
                flagged = true;
                Some("Trojan.GenericKD.12345".to_string())
            } else {
                None
            };

            scanned += 1;
            last_path = Some(path_str.clone());
            let _ = tx.send(ScanEvent::FileScanned {
                path: path_str,
                infected,
            });
        }

        // 保险：如果这一轮该出威胁但生成的假路径里没有命中"downloads"关键字，
        // 就把最后一个文件补记成威胁，保证"发现威胁"这条 UI 分支总能被看到。
        if should_flag && !flagged
            && let Some(path) = last_path
        {
            let _ = tx.send(ScanEvent::FileScanned {
                path,
                infected: Some("Trojan.GenericKD.12345".to_string()),
            });
        }

        let cancelled = cancel.load(Ordering::SeqCst);
        let _ = tx.send(ScanEvent::Finished {
            scanned,
            elapsed: start.elapsed(),
            cancelled,
        });
    }
}
