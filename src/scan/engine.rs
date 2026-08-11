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

use super::{CancelFlag, ScanEvent};
use std::path::PathBuf;
use std::sync::mpsc::Sender;

/// 消费 `paths` 里的路径，交给扫描引擎处理，结果通过 `tx` 发出去。
///
/// 这个函数是阻塞的，调用者需要自己在后台线程里跑它。
pub fn run(paths: Vec<PathBuf>, tx: Sender<ScanEvent>, cancel: CancelFlag) {
    #[cfg(any(windows, target_os = "macos"))]
    real::run(paths, tx, cancel);
    #[cfg(not(any(windows, target_os = "macos")))]
    mock::run(paths, tx, cancel);
}

#[cfg(any(windows, target_os = "macos"))]
mod real {
    use super::super::authenticode;
    use super::super::cache;
    use super::super::{CancelFlag, ScanEvent};
    use crate::paths;
    use std::collections::HashMap;
    use std::io::{BufRead, BufReader, Write};
    use std::path::{Path, PathBuf};
    use std::process::{Command, Stdio};
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::mpsc::Sender;
    use std::sync::{Arc, Mutex};
    use std::time::Instant;

    /// 预筛结束后交给后台线程执行的 clamscan 批次。
    struct ClamscanWork {
        to_scan: Vec<PathBuf>,
        hash_by_path: HashMap<String, String>,
        cache: cache::ScanCache,
        scanned: usize,
    }

    /// 后台线程返回值（`run_clamscan_batch` 始终返回 `Some`）。
    type ClamscanOutcome = (usize, cache::ScanCache, Option<String>);

    // `creation_flags` 只在 Windows 上有意义；非 Windows 构建不需要这个 trait，门控掉避免 unused import。
    #[cfg(windows)]
    use std::os::windows::process::CommandExt;

    /// 不弹出黑色命令行窗口（仅 Windows 有意义）。
    #[cfg(windows)]
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;

    pub fn run(paths: Vec<PathBuf>, tx: Sender<ScanEvent>, cancel: CancelFlag) {
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
        // 大于这个体积的文件不进缓存（避免无谓地哈希大文件）；它们仍会被 ClamAV 正常扫描。
        const CACHE_SKIP_SIZE: u64 = 64 * 1024 * 1024;
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

        // 按路径列表顺序逐个处理：缓存/签名可即时判定的立刻发 `FileScanned`，
        // 需 clamscan 的记入待扫列表——对用户呈现为统一的「逐文件检测」进度。
        let mut to_scan: Vec<PathBuf> = Vec::new();
        let mut hash_by_path: HashMap<String, String> = HashMap::new();
        for p in &paths {
            if cancel.load(Ordering::SeqCst) {
                finish_scan(start, scanned, &cancel, &tx, cache, None);
                return;
            }
            let path_str = p.display().to_string();
            let _ = tx.send(ScanEvent::ScanningFile {
                path: path_str.clone(),
            });
            let too_big = std::fs::metadata(p)
                .map(|m| m.len() > CACHE_SKIP_SIZE)
                .unwrap_or(false);
            if too_big {
                to_scan.push(p.clone());
                continue;
            }
            match cache::file_hash(p) {
                Some(hash) => {
                    if let Some(res) = cache.lookup(&hash) {
                        let infected = if res == "clean" { None } else { Some(res) };
                        scanned += 1;
                        let _ = tx.send(ScanEvent::FileScanned {
                            path: p.display().to_string(),
                            infected,
                        });
                        continue;
                    }
                    if authenticode::is_trusted_signed(p) {
                        scanned += 1;
                        let _ = tx.send(ScanEvent::FileScanned {
                            path: p.display().to_string(),
                            infected: None,
                        });
                        cache.insert(&hash, "clean");
                        continue;
                    }
                    hash_by_path.insert(p.display().to_string(), hash);
                    to_scan.push(p.clone());
                }
                None => {
                    to_scan.push(p.clone());
                }
            }
        }

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
                let infected = parse_infected(status);
                if let Some(hash) = hash_by_path.get(path) {
                    let result = infected.clone().unwrap_or_else(|| "clean".to_string());
                    cache.insert(hash, &result);
                }
                scanned += 1;
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

    fn parse_infected(status: &str) -> Option<String> {
        let status = status.trim();
        if status == "OK" {
            None
        } else {
            status.strip_suffix(" FOUND").map(|name| name.to_string())
        }
    }

    /// 速度优化相关的扫描开关。跳过与可执行文件无关的格式，并对 PE 解析按平台微调。
    fn apply_scan_flags(cmd: &mut Command) {
        // 关闭 bytecode（单文件耗时杀手：JIT 编译+模拟执行，最多 5s）与 PUA 签名匹配。
        cmd.arg("--scan-elf=no") // 不扫 ELF（macOS/Linux 可执行文件由各自解析器处理）
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
}

/// 开发预览用的假扫描引擎：不碰真实文件，只是按小延迟消费路径列表，
/// 模拟"逐个文件扫完"的节奏，方便在 Linux 等开发机上看 UI 动起来的效果。
#[cfg(not(any(windows, target_os = "macos")))]
mod mock {
    use super::super::{CancelFlag, ScanEvent};
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::mpsc::Sender;
    use std::time::{Duration, Instant};

    /// 每跑一次就翻一次面：让"重新扫描"能交替看到"未发现威胁"和"发现威胁"两种结果页样式。
    static RUN_COUNTER: AtomicUsize = AtomicUsize::new(0);

    pub fn run(paths: Vec<PathBuf>, tx: Sender<ScanEvent>, cancel: CancelFlag) {
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
