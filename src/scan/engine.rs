//! ClamAV 调用封装：闪电扫描和全盘扫描共用这一份逻辑。
//!
//! 设计要点（对应技术方案 3.3 节）：
//! - 只 spawn 一次 `clamscan.exe`，用 `--file-list=<临时文件>` 传入待扫描路径。
//!   ClamAV 1.5.x 不支持 `--file-list=-`（stdin），必须用实际文件。
//! - 用 `-v` + `--stdout` 让 clamscan 对每个文件都输出一行结果到 stdout，从而拿到逐文件级进度。
//! - 取消扫描：一个"看门狗"线程检测到取消标记后直接 kill 子进程，stdout 管道关闭后读取循环自然退出。
//!
//! 非 Windows（macOS/Linux 开发机预览）：没有 clamscan.exe 可跑，`mock` 子模块用同样的
//! `Vec<PathBuf> -> ScanEvent` 接口模拟扫描过程，方便在开发机上直接看 UI/交互效果。

use super::{CancelFlag, ScanEvent};
use std::path::PathBuf;
use std::sync::mpsc::Sender;

/// 消费 `paths` 里的路径，交给扫描引擎处理，结果通过 `tx` 发出去。
///
/// 这个函数是阻塞的，调用者需要自己在后台线程里跑它。
pub fn run(paths: Vec<PathBuf>, tx: Sender<ScanEvent>, cancel: CancelFlag) {
    #[cfg(windows)]
    real::run(paths, tx, cancel);
    #[cfg(not(windows))]
    mock::run(paths, tx, cancel);
}

#[cfg(windows)]
mod real {
    use super::super::{CancelFlag, ScanEvent};
    use crate::paths;
    use std::io::{BufRead, BufReader, Write};
    use std::os::windows::process::CommandExt;
    use std::path::{Path, PathBuf};
    use std::process::{Command, Stdio};
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::mpsc::Sender;
    use std::sync::{Arc, Mutex};
    use std::time::Instant;

    /// 不弹出黑色命令行窗口。
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;

    pub fn run(paths: Vec<PathBuf>, tx: Sender<ScanEvent>, cancel: CancelFlag) {
        let start = Instant::now();

        if !paths::clamscan_available() {
            let _ = tx.send(ScanEvent::Error(format!(
                "Scan engine not found: {}\nMake sure the clamav directory is bundled with the app.",
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

        // ClamAV 1.5.x 不支持 `--file-list=-`（stdin），必须写入实际临时文件。
        // 单实例应用 + is_running() 互斥保证同一时刻只有一个扫描在跑，用 PID 命名足够。
        let temp_path = std::env::temp_dir()
            .join(format!("clv3000_scanlist_{}.txt", std::process::id()));
        if let Err(e) = write_path_list(&temp_path, &paths) {
            let _ = tx.send(ScanEvent::Error(format!("Failed to write scan list: {e}")));
            let _ = tx.send(ScanEvent::Finished {
                scanned: 0,
                elapsed: start.elapsed(),
                cancelled: cancel.load(Ordering::SeqCst),
            });
            return;
        }

        let mut cmd = Command::new(paths::clamscan_path());
        cmd.arg("--no-summary")
            .arg("-v")
            .arg("--stdout")
            .arg(format!("--file-list={}", temp_path.display()))
            .arg(format!(
                "--database={}",
                paths::clamav_database_dir().display()
            ))
            // ── 扫描速度优化（关键）──
            // 目标文件都是 PE 可执行文件（exe/dll/sys 等）。ClamAV 默认开启的 bytecode
            // 签名会对每个可疑 PE 做 JIT 编译 + 模拟执行解包，单文件能跑到 2~5 秒，
            // 是慢扫的主因。关闭后单文件通常回落到 100~300ms，代价是丢失 bytecode
            // 签名检测能力（对常见 PE 检出影响有限，可接受）。
            // DB 加载的 10~30 秒固定开销无法在此消除，但单文件扫描时间可显著降低。
            // 保留：--scan-pe（核心需求）、--scan-ole2（部分 PE 内嵌 OLE 容器）。
            .arg("--scan-elf=no")        // 不扫 ELF（Linux 可执行文件）
            .arg("--scan-archive=no")    // 不扫压缩包（目标是独立 exe/dll，非自解压包）
            .arg("--scan-mail=no")       // 不扫邮件
            .arg("--scan-pdf=no")        // 不扫 PDF
            .arg("--scan-html=no")       // 不扫 HTML
            .arg("--scan-xmldocs=no")    // 不扫 XML 文档
            .arg("--scan-swf=no")        // 不扫 Flash
            .arg("--scan-hwp3=no")       // 不扫韩文办公文档
            .arg("--scan-onenote=no")    // 不扫 OneNote
            .arg("--scan-image=no")      // 不扫图片
            .arg("--scan-image-fuzzy-hash=no") // 不做图片模糊哈希
            .arg("--phishing-sigs=no")   // 不做钓鱼签名检测（针对邮件）
            .arg("--phishing-scan-urls=no")    // 不做 URL 钓鱼检测
            // ── 单文件耗时杀手：bytecode + PUA ──
            .arg("--bytecode=no")        // 关闭字节码签名（JIT 编译+模拟执行，单文件最多 5s）
            .arg("--detect-pua=no")      // 不检测"潜在不需要的应用"，省一遍 PUA 签名匹配
            // 大小与超时限制：跳过过大的文件、限制单文件扫描时间。
            .arg("--max-filesize=100M")       // 超过 100MB 的文件跳过（视为干净）
            .arg("--max-scansize=200M")       // 容器文件最大扫描数据量
            .arg("--max-scantime=5000")       // 单文件超 2 秒视为干净（原 10s 太宽松）
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .creation_flags(CREATE_NO_WINDOW);

        let mut child = match cmd.spawn() {
            Ok(c) => c,
            Err(e) => {
                let _ = std::fs::remove_file(&temp_path);
                let _ = tx.send(ScanEvent::Error(format!("Failed to start scan engine: {e}")));
                let _ = tx.send(ScanEvent::Finished {
                    scanned: 0,
                    elapsed: start.elapsed(),
                    cancelled: false,
                });
                return;
            }
        };

        let stdout = match child.stdout.take() {
            Some(s) => s,
            None => {
                let _ = child.kill();
                let _ = std::fs::remove_file(&temp_path);
                let _ = tx.send(ScanEvent::Error("Failed to read scan engine stdout".to_string()));
                let _ = tx.send(ScanEvent::Finished {
                    scanned: 0,
                    elapsed: start.elapsed(),
                    cancelled: false,
                });
                return;
            }
        };
        let stderr = child.stderr.take();
        let child = Arc::new(Mutex::new(child));

        // 看门狗：一旦收到取消信号，直接把子进程杀掉，stdout 管道随之关闭，读取循环退出。
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

        // stderr 读取线程：收集 clamscan 的错误/警告输出，供出错时展示。
        // 正常扫描时 stderr 为空；只有 spawn 失败、路径无法访问等才会写 stderr。
        let stderr_thread = std::thread::spawn(move || -> String {
            let Some(stderr) = stderr else { return String::new() };
            BufReader::new(stderr)
                .lines()
                .filter_map(|l| l.ok())
                .collect::<Vec<_>>()
                .join("\n")
        });

        // 读取线程（就用当前线程）：逐行解析 clamscan 的 stdout。
        // 输出形如：
        //   Scanning C:\path\to\file.exe        ← -v 的进度提示，不含 ": "，会被 rsplit 过滤掉
        //   C:\path\to\file.exe: OK             ← 正常
        //   C:\path\to\file.exe: Win.Test.EICAR_HDB-1 FOUND  ← 感染
        let mut scanned: usize = 0;
        let reader = BufReader::new(stdout);
        for line in reader.lines() {
            let Ok(line) = line else { break };
            if let Some((path, status)) = rsplit_result_line(&line) {
                let infected = parse_infected(status);
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

        let cancelled = cancel.load(Ordering::SeqCst);

        // 如果一个文件都没扫到且有 stderr 输出，把错误信息发给 UI。
        if scanned == 0 && !stderr_output.is_empty() {
            let _ = tx.send(ScanEvent::Error(stderr_output));
        }

        let _ = tx.send(ScanEvent::Finished {
            scanned,
            elapsed: start.elapsed(),
            cancelled,
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

    /// clamscan 一行输出形如 `C:\path\to\file.exe: OK` 或
    /// `C:\path\to\file.exe: Win.Test.EICAR_HDB-1 FOUND`。
    /// Windows 路径本身含有一个 `X:` 但后面紧跟 `\`，不会产生 `: `（冒号+空格），
    /// 所以从右边找 `: ` 分隔符是安全的。
    /// `Scanning <path>` 这类 `-v` 提示行不含 `: `，会被 `None` 过滤掉。
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
}

/// 开发预览用的假扫描引擎：不碰真实文件，只是按小延迟消费路径列表，
/// 模拟"逐个文件扫完"的节奏，方便在 macOS/Linux 开发机上看 UI 动起来的效果。
#[cfg(not(windows))]
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
