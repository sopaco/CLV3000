//! ClamAV 调用封装：闪电扫描和全盘扫描共用这一份逻辑。
//!
//! 设计要点（对应技术方案 3.3 节）：
//! - 只 spawn 一次 `clamscan.exe`，用 `--file-list=-` 从 stdin **流式**读入待扫描路径，
//!   避免为了拿到完整文件列表而等待整个遍历/枚举结束，也避免多次加载病毒库的开销。
//! - 用 `-v`（verbose）让 clamscan 对每个文件都输出一行结果，从而拿到逐文件级进度。
//! - 取消扫描：写入线程检测到取消标记后停止喂路径并关闭 stdin；同时一个专门的
//!   "看门狗"线程会在检测到取消标记后直接 kill 子进程，双重保证响应及时。
//!
//! 非 Windows（macOS/Linux 开发机预览）：没有 clamscan.exe 可跑，`mock` 子模块用同样的
//! `path_rx -> ScanEvent` 接口模拟扫描过程，方便在开发机上直接看 UI/交互效果。

use super::{CancelFlag, ScanEvent};
use std::path::PathBuf;
use std::sync::mpsc::{Receiver, Sender};

/// 消费 `path_rx` 里的路径，交给扫描引擎处理，结果通过 `tx` 发出去。
///
/// 这个函数是阻塞的，调用者需要自己在后台线程里跑它。
pub fn run(path_rx: Receiver<PathBuf>, tx: Sender<ScanEvent>, cancel: CancelFlag) {
    #[cfg(windows)]
    real::run(path_rx, tx, cancel);
    #[cfg(not(windows))]
    mock::run(path_rx, tx, cancel);
}

#[cfg(windows)]
mod real {
    use super::super::{CancelFlag, ScanEvent};
    use crate::paths;
    use std::io::{BufRead, BufReader, Write};
    use std::os::windows::process::CommandExt;
    use std::path::PathBuf;
    use std::process::{Command, Stdio};
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::mpsc::{Receiver, Sender};
    use std::sync::{Arc, Mutex};
    use std::time::Instant;

    /// 不弹出黑色命令行窗口。
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;

    pub fn run(path_rx: Receiver<PathBuf>, tx: Sender<ScanEvent>, cancel: CancelFlag) {
        if !paths::clamscan_available() {
            let _ = tx.send(ScanEvent::Error(format!(
                "找不到扫描引擎：{}\n请确认 clamav 目录随程序一起分发。",
                paths::clamscan_path().display()
            )));
            let _ = tx.send(ScanEvent::Finished {
                scanned: 0,
                elapsed: std::time::Duration::ZERO,
                cancelled: cancel.load(Ordering::SeqCst),
            });
            return;
        }

        let start = Instant::now();

        let mut cmd = Command::new(paths::clamscan_path());
        cmd.arg("--no-summary")
            .arg("-v")
            .arg("--file-list=-")
            .arg(format!(
                "--database={}",
                paths::clamav_database_dir().display()
            ))
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .creation_flags(CREATE_NO_WINDOW);

        let mut child = match cmd.spawn() {
            Ok(c) => c,
            Err(e) => {
                let _ = tx.send(ScanEvent::Error(format!("启动扫描引擎失败：{e}")));
                let _ = tx.send(ScanEvent::Finished {
                    scanned: 0,
                    elapsed: start.elapsed(),
                    cancelled: false,
                });
                return;
            }
        };

        let mut stdin = match child.stdin.take() {
            Some(s) => s,
            None => {
                let _ = tx.send(ScanEvent::Error("无法写入扫描引擎输入流".to_string()));
                let _ = child.kill();
                return;
            }
        };
        let stdout = match child.stdout.take() {
            Some(s) => s,
            None => {
                let _ = tx.send(ScanEvent::Error("无法读取扫描引擎输出流".to_string()));
                let _ = child.kill();
                return;
            }
        };

        let child = Arc::new(Mutex::new(child));

        // 看门狗：一旦收到取消信号，直接把子进程杀掉，不等它自然处理完 stdin EOF。
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

        // 写入线程：把发现的路径逐行写进 clamscan 的 stdin，收到取消信号或管道断开就停止。
        let writer_cancel = cancel.clone();
        let writer = std::thread::spawn(move || {
            for path in path_rx.iter() {
                if writer_cancel.load(Ordering::SeqCst) {
                    break;
                }
                let line = format!("{}\n", path.display());
                if stdin.write_all(line.as_bytes()).is_err() {
                    break;
                }
            }
            // 显式 drop，关闭 stdin，让 clamscan 知道路径列表已经喂完了。
            drop(stdin);
        });

        // 读取线程（就用当前线程）：逐行解析 clamscan 的输出。
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

        let _ = writer.join();
        watchdog_done.store(true, Ordering::SeqCst);
        let _ = watchdog.join();

        let cancelled = cancel.load(Ordering::SeqCst);
        if let Ok(mut c) = child.lock() {
            let _ = c.wait();
        }

        let _ = tx.send(ScanEvent::Finished {
            scanned,
            elapsed: start.elapsed(),
            cancelled,
        });
    }

    /// clamscan 一行输出形如 `C:\path\to\file.exe: OK` 或
    /// `C:\path\to\file.exe: Win.Test.EICAR_HDB-1 FOUND`。
    /// Windows 路径本身含有一个 `X:` 但后面紧跟 `\`，不会产生 `: `（冒号+空格），
    /// 所以从右边找 `: ` 分隔符是安全的。
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

/// 开发预览用的假扫描引擎：不碰真实文件，只是按小延迟消费 `path_rx`，
/// 模拟"逐个文件扫完"的节奏，方便在 macOS/Linux 开发机上看 UI 动起来的效果。
#[cfg(not(windows))]
mod mock {
    use super::super::{CancelFlag, ScanEvent};
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::mpsc::{Receiver, Sender};
    use std::time::{Duration, Instant};

    /// 每跑一次就翻一次面：让"重新扫描"能交替看到"未发现威胁"和"发现威胁"两种结果页样式。
    static RUN_COUNTER: AtomicUsize = AtomicUsize::new(0);

    pub fn run(path_rx: Receiver<PathBuf>, tx: Sender<ScanEvent>, cancel: CancelFlag) {
        let start = Instant::now();
        let should_flag = RUN_COUNTER.fetch_add(1, Ordering::Relaxed).is_multiple_of(2);

        let mut scanned = 0usize;
        let mut flagged = false;
        let mut last_path: Option<String> = None;

        for path in path_rx.iter() {
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
            && let Some(path) = last_path {
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
