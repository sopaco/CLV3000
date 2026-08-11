//! 主界面：仪表盘 / 闪电扫描 / 病毒库 / 全盘扫描 四个页面 + 自绘标题栏 + 全局底部资源条。

use crate::clamav_info::ClamAvInfo;
use crate::config::{AppConfig, ScanRecord};
use crate::lifecycle::{Lifecycle, RunMode};
use crate::localtime::Timestamp;
use crate::paths;
use crate::scan::{self, CancelFlag, ScanEvent, ScanKind, Threat};
use crate::sysmon::{self, ResourceSample, SysMonHandle};
use crate::theme::{self, colors};
use crate::tray::Tray;
use crate::widgets::{self, ThreatAction, Toast};
use eframe::egui;
use egui::{Color32, Stroke, Vec2, ViewportCommand};
use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::atomic::Ordering;
use std::sync::mpsc::Receiver;
use std::time::{Duration, Instant};
use tray_icon::TrayIconEvent;

/// 主窗口默认尺寸（与 `main.rs` 里 `with_inner_size` 一致）。
const MAIN_WINDOW_SIZE: [f32; 2] = [900.0, 600.0];
/// 「关于」独占窗口尺寸：比主窗口小一圈，避免关于页背后留一大片黑底（见
/// `about_dialog::paint_about_fullscreen`）。注意要 ≥ `main.rs` 里的 `min_inner_size`。
const ABOUT_WINDOW_SIZE: [f32; 2] = [480.0, 460.0];

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Page {
    Dashboard,
    QuickScan,
    VirusDb,
    FullScan,
}

enum ScanPhase {
    Idle,
    /// 仅闪电扫描会经历这个阶段：进程/模块枚举中。
    Enumerating {
        done: usize,
        total: usize,
        files_found: usize,
    },
    Scanning {
        /// 全盘扫描不知道总数，只能显示"已扫描 N 个"；闪电扫描枚举完就知道总数了。
        total: Option<usize>,
        scanned: usize,
        current_path: String,
    },
    Done {
        scanned: usize,
        elapsed: Duration,
        cancelled: bool,
    },
}

struct ScanPageState {
    kind: ScanKind,
    phase: ScanPhase,
    cancel: Option<CancelFlag>,
    rx: Option<Receiver<ScanEvent>>,
    threats: Vec<Threat>,
    last_error: Option<String>,
    /// 扫描启动时刻；用于在 Scanning 阶段实时显示已用时长（clamscan 加载病毒库的
    /// 十几秒里没有任何 FileScanned 事件，靠这个让用户知道没卡死）。
    started_at: Option<Instant>,
    /// 上一帧这个页面实际画出来的内容高度，给 `widgets::vertically_centered` 用来
    /// 算这一帧该留多少顶部空白。见该函数文档注释。
    content_height: f32,
}

impl ScanPageState {
    fn new(kind: ScanKind) -> Self {
        Self {
            kind,
            phase: ScanPhase::Idle,
            cancel: None,
            rx: None,
            threats: Vec::new(),
            last_error: None,
            started_at: None,
            content_height: 0.0,
        }
    }

    fn is_running(&self) -> bool {
        matches!(
            self.phase,
            ScanPhase::Enumerating { .. } | ScanPhase::Scanning { .. }
        )
    }

    fn start(&mut self, scan_removable: bool) {
        if self.is_running() {
            return;
        }
        self.threats.clear();
        self.last_error = None;
        self.started_at = Some(Instant::now());
        let cancel = scan::new_cancel_flag();
        let (tx, rx) = std::sync::mpsc::channel();
        self.cancel = Some(cancel.clone());
        self.rx = Some(rx);
        self.phase = match self.kind {
            ScanKind::Quick => ScanPhase::Enumerating {
                done: 0,
                total: 0,
                files_found: 0,
            },
            ScanKind::Full => ScanPhase::Scanning {
                total: None,
                scanned: 0,
                current_path: String::new(),
            },
        };
        match self.kind {
            ScanKind::Quick => {
                std::thread::spawn(move || scan::quick_scan::run(tx, cancel));
            }
            ScanKind::Full => {
                std::thread::spawn(move || scan::full_scan::run(tx, cancel, scan_removable));
            }
        }
    }

    fn request_cancel(&self) {
        if let Some(c) = &self.cancel {
            c.store(true, Ordering::SeqCst);
        }
    }

    /// 返回 `Some((scanned, elapsed, cancelled))` 当这一批事件里出现了 `Finished`。
    ///
    /// 每帧最多处理 `MAX_EVENTS_PER_FRAME` 个事件：clamscan 的 stdout 在管道上是块缓冲的，
    /// 整个扫描过程的结果常常在进程退出时一次性 flush 进 channel。如果一帧全排空，
    /// phase 会从 `Scanning(0)` 直接跳到 `Done`，UI 永远画不到中间的计数爬升过程。
    /// 限流后突发的事件被分摊到连续几帧渲染，用户能肉眼看到 "1/109 → 2/109 → …"。
    /// `Finished` 也会因此晚几帧到达（最多 `count / MAX_EVENTS_PER_FRAME` 帧），可忽略。
    fn poll(&mut self, config: &AppConfig) -> Option<(usize, Duration, bool)> {
        const MAX_EVENTS_PER_FRAME: usize = 4;
        let mut finished = None;
        let mut processed = 0usize;
        let Some(rx) = &self.rx else { return None };
        while processed < MAX_EVENTS_PER_FRAME
            && let Ok(event) = rx.try_recv()
        {
            processed += 1;
            match event {
                ScanEvent::Enumerating {
                    processes_done,
                    processes_total,
                    files_found,
                } => {
                    self.phase = ScanPhase::Enumerating {
                        done: processes_done,
                        total: processes_total,
                        files_found,
                    };
                }
                ScanEvent::ScanStarted { total } => {
                    self.phase = ScanPhase::Scanning {
                        total,
                        scanned: 0,
                        current_path: String::new(),
                    };
                }
                ScanEvent::FileScanned { path, infected } => {
                    let total_hint = match &self.phase {
                        ScanPhase::Enumerating { files_found, .. } => Some(*files_found),
                        ScanPhase::Scanning { total, .. } => *total,
                        _ => None,
                    };
                    let scanned_before = match &self.phase {
                        ScanPhase::Scanning { scanned, .. } => *scanned,
                        _ => 0,
                    };
                    if let Some(name) = infected
                        && !config.is_ignored(&path, &name)
                    {
                        self.threats.push(Threat {
                            path: PathBuf::from(&path),
                            virus_name: name,
                        });
                    }
                    self.phase = ScanPhase::Scanning {
                        total: total_hint,
                        scanned: scanned_before + 1,
                        current_path: path,
                    };
                }
                ScanEvent::Finished {
                    scanned,
                    elapsed,
                    cancelled,
                } => {
                    self.phase = ScanPhase::Done {
                        scanned,
                        elapsed,
                        cancelled,
                    };
                    self.started_at = None;
                    finished = Some((scanned, elapsed, cancelled));
                }
                ScanEvent::Error(e) => {
                    self.last_error = Some(e);
                }
            }
        }
        finished
    }
}

/// 病毒库更新结果：区分"真的更新了"和"已经是最新、无需更新"。
/// freshclam 在两者下都返回退出码 0，不能只靠退出码判断，否则会把
/// "已是最新"也误报成"更新完成"，让用户以为版本涨了。
/// 两个变体分别在 `run_freshclam`（Windows 真更新 / 未变）与开发 mock 中构造，
/// 并由 `poll_background` 的 `match` 消费——无 dead_code 警告。
#[derive(Debug, Clone, Copy)]
enum UpdateOutcome {
    Updated,
    AlreadyUpToDate,
}

struct VirusDbState {
    updating: bool,
    rx: Option<Receiver<Result<UpdateOutcome, String>>>,
    /// 后台版本查询（`refresh_db_version`）的回传通道。`db_version` 在 `poll` 里
    /// 从这条通道接收结果写回，保证版本刷新不阻塞 UI 线程。
    version_rx: Option<Receiver<String>>,
    /// 左右两栏各自的上一帧内容高度，给 `widgets::vertically_centered` 用。两栏
    /// 内容不一样高，各自居中，不能共用一个高度。
    status_col_height: f32,
    about_col_height: f32,
    /// 当前病毒库版本（来自 `clamscan -V`），更新完成后刷新，避免界面一直显示旧版本。
    db_version: Option<String>,
}

impl VirusDbState {
    fn new() -> Self {
        Self {
            updating: false,
            rx: None,
            version_rx: None,
            status_col_height: 0.0,
            about_col_height: 0.0,
            db_version: None,
        }
    }

    /// 异步重新查询病毒库版本：后台线程跑 `clamscan -V`，结果经 `version_rx` 回传，
    /// 不在 UI 线程阻塞（之前是同步调 `database_version()`，会卡住一帧）。
    /// 已有查询在飞（`version_rx` 挂着）就跳过，避免重复发起。
    /// `ctx` 用于查询完成时唤醒 UI（闲置时 UI 没有定时心跳，不唤醒结果就一直没人收）。
    fn refresh_db_version(&mut self, ctx: egui::Context) {
        if self.version_rx.is_some() {
            return; // 已有查询在飞，不重复发起
        }
        let (tx, rx) = std::sync::mpsc::channel();
        self.version_rx = Some(rx);
        std::thread::spawn(move || {
            let v = ClamAvInfo::database_version();
            let _ = tx.send(v);
            ctx.request_repaint();
        });
    }

    fn start_update(&mut self, ctx: egui::Context) {
        if self.updating || !paths::freshclam_available() {
            return;
        }
        let (tx, rx) = std::sync::mpsc::channel();
        self.rx = Some(rx);
        self.updating = true;
        std::thread::spawn(move || {
            // 后台线程里跑 freshclam；万一它 panic，用 catch_unwind 兜住，
            // 把 panic 信息作为错误带回主线程，而不是只报一个含糊的
            // "Update thread stopped unexpectedly"（通道断开）。
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(run_freshclam))
                .unwrap_or_else(|payload| {
                    let msg = if let Some(s) = payload.downcast_ref::<&str>() {
                        (*s).to_string()
                    } else if let Some(s) = payload.downcast_ref::<String>() {
                        s.clone()
                    } else {
                        "unknown panic payload".to_string()
                    };
                    Err(format!("Update thread panicked: {msg}"))
                });
            let _ = tx.send(result);
            // 唤醒 UI 立刻消费更新结果（弹 Toast / 刷新版本号），不等任何心跳。
            ctx.request_repaint();
        });
    }

    /// 返回本次轮询里出现的更新结果（如果有）；同时把后台版本查询结果写回
    /// `db_version`（不弹 toast）。两条通道都在主线程这里排空，UI 线程读取的
    /// `db_version` 永远由主线程写入，无数据竞争。
    fn poll(&mut self) -> Option<Result<UpdateOutcome, String>> {
        // 先处理更新结果（来自 start_update 的后台线程）。
        let update_result = match self.rx.as_ref().map(|rx| rx.try_recv()) {
            Some(Ok(r)) => {
                self.updating = false;
                // 必须同时清掉已完成的 Receiver：发送端（后台线程）已销毁，
                // 若留着它，下一帧 try_recv 会得到 Disconnected，把一次成功的
                // 更新误报成 "Update thread stopped unexpectedly"。
                self.rx = None;
                Some(r)
            }
            Some(Err(std::sync::mpsc::TryRecvError::Disconnected)) => {
                // 后台线程异常退出（panic / 被强杀）会触发通道断开，
                // 否则 `updating` 永远为 true，按钮永久卡在 "Updating…"。
                self.updating = false;
                self.rx = None;
                Some(Err("Update thread stopped unexpectedly".to_string()))
            }
            _ => None,
        };

        // 再处理版本查询结果（来自 refresh_db_version 的后台线程）。
        match self.version_rx.as_ref().map(|rx| rx.try_recv()) {
            Some(Ok(v)) => {
                self.db_version = Some(v);
                self.version_rx = None;
            }
            Some(Err(std::sync::mpsc::TryRecvError::Disconnected)) => {
                // 版本查询线程异常退出：清掉孤儿 Receiver，下次需要时会重新发起。
                self.version_rx = None;
            }
            _ => {}
        }

        update_result
    }
}

#[cfg(windows)]
fn run_freshclam() -> Result<UpdateOutcome, String> {
    use std::os::windows::process::CommandExt;
    use std::process::{Command, Stdio};

    let db_dir = paths::resolved_clamav_database_dir()
        .unwrap_or_else(|| paths::clamav_database_dir());
    // 跑之前先记一份数据库目录签名，跑完再比对——freshclam 在"已是最新"时
    // 也返回退出码 0，光看退出码会把"没变化"误判成"更新成功"。
    let before = database_signature(&db_dir);

    let mut cmd = Command::new(paths::freshclam_path());
    cmd.arg(format!("--datadir={}", db_dir.display()))
        .stdout(Stdio::null())
        // 保留 stderr 管道：freshclam 的报错（配置文件缺失、连不上镜像源等）都走
        // stderr，失败时把它塞进错误提示，比只报退出码有用得多。
        .stderr(Stdio::piped())
        .creation_flags(0x0800_0000);

    match cmd.output() {
        Ok(out) if out.status.success() => {
            let after = database_signature(&db_dir);
            if after != before {
                Ok(UpdateOutcome::Updated)
            } else {
                Ok(UpdateOutcome::AlreadyUpToDate)
            }
        }
        Ok(out) => {
            let stderr = String::from_utf8_lossy(&out.stderr).trim().to_string();
            if stderr.is_empty() {
                Err(format!("Database update failed with exit code {}", out.status))
            } else {
                Err(format!("Database update failed (exit {}): {}", out.status, stderr))
            }
        }
        Err(e) => Err(format!("Failed to start freshclam: {e}")),
    }
}

/// 数据库目录签名：把所有签名文件（.cvd/.cld/.cud）的「文件名:大小:修改时间」
/// 拼成一段稳定字符串，用来判断 freshclam 跑完之后文件到底有没有变。
#[cfg(any(windows, target_os = "macos"))]
fn database_signature(dir: &std::path::Path) -> String {
    use std::collections::BTreeMap;
    let mut map: BTreeMap<String, (u64, std::time::SystemTime)> = BTreeMap::new();
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            let p = entry.path();
            let ext = p.extension().and_then(|e| e.to_str()).unwrap_or("");
            if matches!(ext, "cvd" | "cld" | "cud") {
                if let Ok(meta) = std::fs::metadata(&p) {
                    let mtime = meta.modified().unwrap_or(std::time::SystemTime::UNIX_EPOCH);
                    if let Some(name) = p.file_name().and_then(|n| n.to_str()) {
                        map.insert(name.to_string(), (meta.len(), mtime));
                    }
                }
            }
        }
    }
    let mut sig = String::new();
    for (name, (len, mtime)) in map {
        sig.push_str(&format!("{name}:{len}:{mtime:?}|"));
    }
    sig
}

/// 开发预览用：不真的联网更新，睡一下模拟"正在更新"的等待感，然后报成功。
/// 用原子计数器在 `Updated` / `AlreadyUpToDate` 之间来回切，方便开发时预览两种
/// 提示文案；同时保证 `AlreadyUpToDate` 变体在非 Windows 构建里也被构造
/// （否则会触发 dead_code 警告——Windows 真路径会构造它，但 dev 桩原本只造 Updated）。
/// macOS：真实调用 `freshclam` 更新病毒库，逻辑与 Windows 版一致（跑前/跑后比对
/// 数据库目录签名，区分"已更新"与"已是最新"），只是不需要 `creation_flags`。
#[cfg(target_os = "macos")]
fn run_freshclam() -> Result<UpdateOutcome, String> {
    use std::process::{Command, Stdio};

    let db_dir = paths::resolved_clamav_database_dir()
        .unwrap_or_else(|| paths::clamav_database_dir());
    // 跑之前先记一份数据库目录签名，跑完再比对——freshclam 在"已是最新"时
    // 也返回退出码 0，光看退出码会把"没变化"误判成"更新成功"。
    let before = database_signature(&db_dir);

    let mut cmd = Command::new(paths::freshclam_path());
    // macOS 上没有系统默认 freshclam.conf，必须显式指定，否则 freshclam 直接报
    // "Can't open/parse the config file" 退出。config 里已含 DatabaseDirectory，
    // 这里再补一个 --datadir（与 config 一致）兜底，确保写到 resolved 目录。
    if let Some(cfg) = paths::freshclam_config_path() {
        cmd.arg("--config-file").arg(cfg);
    }
    cmd.arg(format!("--datadir={}", db_dir.display()))
        // 保留 stdout / stderr 管道：freshclam 的更新进度走 stdout，报错（配置文件缺失、
        // 连不上镜像源等）走 stderr。失败时把它们写进调试日志，比只报退出码有用得多。
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    let mut spawned_args: Vec<String> = vec![paths::freshclam_path().display().to_string()];
    if let Some(cfg) = paths::freshclam_config_path() {
        spawned_args.push("--config-file".to_string());
        spawned_args.push(cfg.display().to_string());
    }
    spawned_args.push(format!("--datadir={}", db_dir.display()));

    // 先算出结果，再统一写调试日志（含 stdout/stderr/退出码），最后返回。
    // stderr_log 在 match 的两个分支里都会被赋值，故用延迟初始化声明。
    let mut stdout_log = String::new();
    let stderr_log: String;
    let result = match cmd.output() {
        Ok(out) => {
            stdout_log = String::from_utf8_lossy(&out.stdout).trim().to_string();
            stderr_log = String::from_utf8_lossy(&out.stderr).trim().to_string();
            if out.status.success() {
                let after = database_signature(&db_dir);
                if after != before {
                    Ok(UpdateOutcome::Updated)
                } else {
                    Ok(UpdateOutcome::AlreadyUpToDate)
                }
            } else if stderr_log.is_empty() {
                Err(format!("Database update failed with exit code {}", out.status))
            } else {
                Err(format!("Database update failed (exit {}): {}", out.status, stderr_log))
            }
        }
        Err(e) => {
            let msg = format!("Failed to start freshclam: {e}");
            stderr_log = msg.clone();
            Err(msg)
        }
    };
    debug_log_freshclam(&spawned_args, &result, &stdout_log, &stderr_log);
    result
}

/// 开发预览用（Linux 等）：不真的联网更新，睡一下模拟"正在更新"的等待感，然后报成功。
/// 用原子计数器在 `Updated` / `AlreadyUpToDate` 之间来回切，方便开发时预览两种
/// 提示文案。
#[cfg(not(any(windows, target_os = "macos")))]
fn run_freshclam() -> Result<UpdateOutcome, String> {
    use std::sync::atomic::{AtomicUsize, Ordering};
    static TOGGLE: AtomicUsize = AtomicUsize::new(0);
    std::thread::sleep(Duration::from_millis(1200));
    if TOGGLE.fetch_add(1, Ordering::Relaxed) % 2 == 0 {
        Ok(UpdateOutcome::Updated)
    } else {
        Ok(UpdateOutcome::AlreadyUpToDate)
    }
}

/// macOS 调试用：把 freshclam 的命令行、退出码、stdout、stderr 追加写到
/// `/tmp/clv3000_freshclam.log`，方便在 GUI 子进程里复现失败时拿到完整现场
/// （GUI 跑的命令和终端里手敲的偶尔会因环境不同而出错，光看弹窗里的简短报错不够）。
#[cfg(target_os = "macos")]
fn debug_log_freshclam(args: &[String], result: &Result<UpdateOutcome, String>, stdout: &str, stderr: &str) {
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let line = format!(
        "[{stamp}] args={args:?}\n  result={result:?}\n  stdout={stdout}\n  stderr={stderr}\n"
    );
    let _ = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open("/tmp/clv3000_freshclam.log")
        .and_then(|mut f| std::io::Write::write_all(&mut f, line.as_bytes()));
}

/// 跨「窗口会话」保留的业务状态（配置、扫描、页面路由）。纯托盘模式下也存在。
pub struct AppCore {
    page: Page,
    config: AppConfig,
    quick: ScanPageState,
    full: ScanPageState,
    virus_db: VirusDbState,
    dashboard_content_height: f32,
}

impl AppCore {
    pub fn new() -> Self {
        Self {
            page: Page::Dashboard,
            config: AppConfig::load(),
            quick: ScanPageState::new(ScanKind::Quick),
            full: ScanPageState::new(ScanKind::Full),
            virus_db: VirusDbState::new(),
            dashboard_content_height: 0.0,
        }
    }

    /// 轮询扫描/病毒库后台线程；返回需要弹 Toast 的消息（仅窗口会话消费）。
    /// `ctx` 透传给更新完成后的版本号刷新（后台线程完成时要能唤醒 UI）。
    pub fn poll_background(&mut self, ctx: &egui::Context) -> Vec<String> {
        let mut toasts = Vec::new();
        if let Some((scanned, elapsed, cancelled)) = self.quick.poll(&self.config) {
            if !cancelled {
                self.config.last_quick_scan = Some(ScanRecord {
                    time: Timestamp::now(),
                    threats_found: self.quick.threats.len(),
                    scanned_count: scanned,
                });
                self.config.save();
            }
            let _ = elapsed;
        }
        if let Some((scanned, elapsed, cancelled)) = self.full.poll(&self.config) {
            if !cancelled {
                self.config.last_full_scan = Some(ScanRecord {
                    time: Timestamp::now(),
                    threats_found: self.full.threats.len(),
                    scanned_count: scanned,
                });
                self.config.save();
            }
            let _ = elapsed;
        }
        if let Some(result) = self.virus_db.poll() {
            match result {
                Ok(UpdateOutcome::Updated) => {
                    self.virus_db.refresh_db_version(ctx.clone());
                    toasts.push("Virus database updated".to_string());
                }
                Ok(UpdateOutcome::AlreadyUpToDate) => {
                    self.virus_db.refresh_db_version(ctx.clone());
                    toasts.push("Virus database already up to date".to_string());
                }
                Err(e) => toasts.push(format!("Database update failed: {e}")),
            }
        }
        toasts
    }
}

/// 托盘/菜单事件：窗口与纯托盘循环共用。
/// 事件来自 `wakeup` 模块的转发队列——转发线程阻塞在 tray-icon/muda 的全局
/// channel 上，事件到达时已经替我们 `request_repaint` 唤醒了 UI，这里只管排空。
pub fn poll_tray_events(tray: &Tray, core: &mut AppCore, lifecycle: &mut Lifecycle) {
    // 锁只在这一小段排空循环里持有，发送端（wakeup 转发线程）基本碰不到竞争。
    while let Ok(event) = crate::wakeup::tray_events().lock().unwrap().try_recv() {
        if let TrayIconEvent::DoubleClick { .. } = event {
            // 双击托盘：显示主窗口（同时清掉可能打开的关于层）。
            lifecycle.mode = RunMode::ShowWindow;
            lifecycle.about_open = false;
            lifecycle.about_standalone = false;
        }
    }

    while let Ok(event) = crate::wakeup::menu_events().lock().unwrap().try_recv() {
        let id = event.id();
        if id == &tray.ids.show {
            // 显示主窗口：清掉关于层，否则关于独占窗口会挡住主界面。
            lifecycle.mode = RunMode::ShowWindow;
            lifecycle.about_open = false;
            lifecycle.about_standalone = false;
        } else if id == &tray.ids.quick_scan {
            lifecycle.mode = RunMode::ShowWindow;
            lifecycle.about_open = false;
            lifecycle.about_standalone = false;
            core.page = Page::QuickScan;
            core.quick.start(core.config.scan_removable_drives);
        } else if id == &tray.ids.about {
            // 来自托盘的关于：只占整个窗口画关于页，不画主界面（about_standalone）。
            lifecycle.about_open = true;
            lifecycle.about_standalone = true;
        } else if id == &tray.ids.quit {
            lifecycle.mode = RunMode::Quit;
        }
    }
}

pub struct App {
    core: Rc<RefCell<AppCore>>,
    lifecycle: Rc<RefCell<Lifecycle>>,
    /// 从 `main` 借入，Drop 时归还，供下一次 eframe 会话复用。
    tray_slot: Rc<RefCell<Option<Tray>>>,
    tray: Option<Tray>,
    sysmon: Option<SysMonHandle>,
    last_sample: ResourceSample,
    toasts: Vec<Toast>,
    allow_exit: bool,
    /// 完整品牌图标的纹理（`icon_app.png`），"关于"区块用它显示 logo。
    app_icon_texture: Option<egui::TextureHandle>,
    /// 简化版图标的纹理（`icon_tray.png`），自绘标题栏左上角那个小图标用它
    /// （Windows 用系统标题栏，不加载）。
    #[cfg(not(windows))]
    titlebar_icon_texture: Option<egui::TextureHandle>,
    /// 主视口当前是否处于「隐藏到托盘」状态。eframe 会话全程存活（不再关闭重建），
    /// 关闭窗口改成 `Visible(false)` 隐藏视口，靠这个标志位在 `logic` 里把生命周期
    /// 模式（ShowWindow / TrayOnly）对齐到真实的视口可见性。
    window_hidden: bool,
    /// 从托盘唤回窗口后的「置顶倒计时」：macOS 14+ 下单次 `activate()` 抢不到焦点、
    /// 窗口不会自动浮到最前，需要在接下来若干帧里反复 `orderFrontRegardless()` 才能稳
    /// 定把窗口提到最前（见 `macos_reopen::bring_to_front`）。每帧递减，归零后停止。
    activate_countdown: u8,
    /// 当前窗口尺寸的「意图」：0 = 主窗口尺寸，1 = 关于独占窗口尺寸。只在意图变化
    /// 时才发 `InnerSize` 指令，避免每帧重置、干扰用户对主窗口的手动缩放。
    size_intent: u8,
}

/// 把 `icon_data::load_*_icon` 解出来的 `(rgba, w, h)` 传进 egui 的纹理系统，
/// 拿到一个能在 `egui::Image` 里直接用的 `TextureHandle`。
fn load_texture(
    ctx: &egui::Context,
    name: &str,
    (rgba, w, h): (Vec<u8>, u32, u32),
) -> egui::TextureHandle {
    let color_image = egui::ColorImage::from_rgba_unmultiplied([w as usize, h as usize], &rgba);
    ctx.load_texture(name, color_image, egui::TextureOptions::LINEAR)
}

impl App {
    pub fn new(
        cc: &eframe::CreationContext<'_>,
        core: Rc<RefCell<AppCore>>,
        lifecycle: Rc<RefCell<Lifecycle>>,
        tray_slot: Rc<RefCell<Option<Tray>>>,
    ) -> Self {
        theme::apply(&cc.egui_ctx);
        // 注册当前会话的 egui Context，让 wakeup 转发线程 / sysmon 采样线程能在
        // 有事件（托盘点击、菜单、资源采样）时主动唤醒 UI——替代旧的定时心跳。
        crate::wakeup::register_ctx(&cc.egui_ctx);

        let tray = tray_slot.borrow_mut().take();
        let window_hidden = lifecycle.borrow().mode == RunMode::TrayOnly;

        Self {
            core,
            lifecycle,
            tray_slot,
            tray,
            sysmon: None,
            last_sample: ResourceSample::default(),
            toasts: Vec::new(),
            allow_exit: false,
            app_icon_texture: None,
            #[cfg(not(windows))]
            titlebar_icon_texture: None,
            window_hidden,
            activate_countdown: 0,
            size_intent: 0,
        }
    }

    fn toast(&mut self, text: impl Into<String>) {
        self.toasts.push(Toast::new(text));
    }

    fn navigate(&mut self, page: Page) {
        self.core.borrow_mut().page = page;
    }

    fn tray(&self) -> Option<&Tray> {
        self.tray.as_ref()
    }

    fn ensure_ui_resources(&mut self, ctx: &egui::Context) {
        if self.sysmon.is_none() {
            self.sysmon = Some(sysmon::spawn(ctx.clone()));
        }
        if self.app_icon_texture.is_none() {
            const LOGO_DISPLAY_PT: f32 = 90.0;
            self.app_icon_texture = Some(load_texture(
                ctx,
                "app_icon",
                crate::icon_data::load_app_icon_for_display(LOGO_DISPLAY_PT, ctx.pixels_per_point()),
            ));
        }
        #[cfg(not(windows))]
        if self.titlebar_icon_texture.is_none() {
            self.titlebar_icon_texture = Some(load_texture(
                ctx,
                "titlebar_icon",
                crate::icon_data::load_tray_icon(64),
            ));
        }
    }

    /// 释放 GPU 纹理与资源监控，为纯托盘模式腾出内存。
    fn release_ui_resources(&mut self, ctx: &egui::Context) {
        self.sysmon.take();
        // 纹理句柄 drop 即可；随后 eframe 会话结束会释放 OpenGL 资源。
        let _ = ctx;
        self.app_icon_texture.take();
        #[cfg(not(windows))]
        self.titlebar_icon_texture.take();
        self.toasts.clear();
        self.last_sample = ResourceSample::default();
    }

    fn hide_to_tray(&mut self, ctx: &egui::Context) {
        // 释放 GPU 纹理与资源监控，但**不关闭** eframe 会话——eframe 事件循环（以及
        // 其背后的 AppKit / winit 消息泵）必须一直存活，托盘图标的菜单点击才能被
        // 系统正常投递。真正"关闭窗口"改成把视口藏起来（`Visible(false)`），由
        // `reconcile_lifecycle` 对账到真实的视口可见性（并在隐藏态把 App 的激活策略
        // 切到 Accessory，从而离开 Dock——见 src/macos_reopen.rs）。
        self.release_ui_resources(ctx);
        self.lifecycle.borrow_mut().mode = RunMode::TrayOnly;
    }

    fn poll_tray(&mut self, ctx: &egui::Context) {
        let Some(tray) = self.tray() else { return };

        // 只在局部作用域里轮询托盘事件并取出下一模式，再 drop 所有 RefCell 借用——
        // 真正的视口指令（显示/隐藏/退出）统一交给 `reconcile_lifecycle`，避免在此处
        // 同步重入 `logic()` 导致仍持有 `borrow_mut` 时 panic。
        let next_mode = {
            let mut core = self.core.borrow_mut();
            let mut lifecycle = self.lifecycle.borrow_mut();
            poll_tray_events(tray, &mut core, &mut lifecycle);
            lifecycle.mode
        };

        match next_mode {
            RunMode::Quit => {
                self.allow_exit = true;
            }
            _ => {
                let _ = ctx;
            }
        }
    }

    /// 把生命周期模式对齐到真实的视口可见性 + macOS 激活策略。eframe 会话全程存活，
    /// 所以这里只发 `Visible` / 激活策略指令，绝不 `Close`（除非用户真的点了退出）。
    ///
    /// 可见条件：`ShowWindow` 模式，或「关于」打开（无论是覆盖在主窗上、还是独占窗口）。
    /// 「关于」独占窗口时主视口必须可见——这正是来自托盘的关于会把窗口带出来的原因；
    /// 关闭关于后若来源是托盘、`mode` 仍是 `TrayOnly`，下一帧这里就会把视口重新藏起来，
    /// 不会残留主窗口。
    ///
    /// macOS 激活策略（见 src/macos_reopen.rs）：
    /// - 有窗口时 → `Regular`：正常 App，带 Dock 图标与前台菜单；
    /// - 隐藏到托盘时 → `Accessory`：菜单栏小工具模式，无 Dock 图标。这样托盘态下 App
    ///   根本不在 Dock 上，用户想要"关闭窗口后只留托盘、不必再占 Dock"的需求直接满足，
    ///   也彻底绕开了"winit 不处理 Dock 重新打开事件 → 点 Dock 唤不回窗口"的坑。
    fn reconcile_lifecycle(&mut self, ctx: &egui::Context) {
        let (mode, about_open, about_standalone) = {
            let lc = self.lifecycle.borrow();
            (lc.mode, lc.about_open, lc.about_standalone)
        };
        if mode == RunMode::Quit {
            self.allow_exit = true;
            ctx.send_viewport_cmd(ViewportCommand::Close);
            return;
        }
        let desired_visible = mode == RunMode::ShowWindow || about_open;
        if desired_visible {
            // 有窗口：必须是 Regular（Dock 图标 + 前台菜单），并确保窗口可见。
            #[cfg(target_os = "macos")]
            crate::macos_reopen::set_accessory(false);
            if self.window_hidden {
                ctx.send_viewport_cmd(ViewportCommand::Visible(true));
                self.window_hidden = false;
                // 从托盘（隐藏）唤回：接下来若干帧反复把窗口提到最前
                // （Accessory→Regular 切换 + macOS 14 激活策略变化，单次 activate 不够）。
                self.activate_countdown = 12;
            }
            // 关于独占窗口用较小的尺寸，主窗口用默认尺寸；只在「尺寸意图」变化时发
            // 指令，避免每帧都重置、干扰用户对主窗口的手动缩放。
            let intent = if about_open && about_standalone { 1 } else { 0 };
            if intent != self.size_intent {
                let size = if intent == 1 { ABOUT_WINDOW_SIZE } else { MAIN_WINDOW_SIZE };
                ctx.send_viewport_cmd(ViewportCommand::InnerSize(size.into()));
                self.size_intent = intent;
                if intent == 1 {
                    // 关于窗刚打开：挪到屏幕正中央（只在这个边沿挪一次，之后用户可以
                    // 自由拖动，不会被反复拽回中心）。无边框窗口 OuterPosition 即内容
                    // 区左上角；取所在显示器的尺寸算居中。
                    if let Some(monitor) = ctx.input(|i| i.viewport().monitor_size) {
                        let origin = ((monitor - Vec2::from(size)) / 2.0).max(Vec2::ZERO);
                        ctx.send_viewport_cmd(ViewportCommand::OuterPosition(origin.to_pos2()));
                    }
                }
            }
        } else if !self.window_hidden {
            // 进托盘态：先把视口藏起来，再切到 Accessory（离开 Dock）。
            ctx.send_viewport_cmd(ViewportCommand::Visible(false));
            self.window_hidden = true;
            #[cfg(target_os = "macos")]
            crate::macos_reopen::set_accessory(true);
        } else {
            // 已隐藏：确保每个隐藏周期都落到 Accessory（例如 `--tray-only` 启动即隐藏）。
            #[cfg(target_os = "macos")]
            crate::macos_reopen::set_accessory(true);
        }
    }

    fn poll_background(&mut self, ctx: &egui::Context) {
        let toasts = self.core.borrow_mut().poll_background(ctx);
        for msg in toasts {
            self.toast(msg);
        }
    }
}

impl Drop for App {
    fn drop(&mut self) {
        crate::wakeup::unregister_ctx();
        if let Some(tray) = self.tray.take() {
            *self.tray_slot.borrow_mut() = Some(tray);
        }
    }
}

impl eframe::App for App {
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.poll_tray(ctx);
        self.reconcile_lifecycle(ctx);

        // 从托盘唤回后的几帧内，反复把窗口提到最前（见 macos_reopen::bring_to_front）。
        // macOS 14+ 下单次 activate 抢不到焦点，必须连续几帧 orderFrontRegardless 才稳。
        if self.activate_countdown > 0 {
            #[cfg(target_os = "macos")]
            crate::macos_reopen::bring_to_front();
            self.activate_countdown -= 1;
        }

        self.poll_background(ctx);

        // 「关于」关闭（OK / Esc / 窗口关闭按钮都会置位 ABOUT_CLOSED）：只关掉关于层，
        // 主窗口的去留由生命周期模式决定——来自托盘（`TrayOnly`）时下一帧 reconcile
        // 会自动把视口重新藏起来，不会残留主窗口。
        if self.lifecycle.borrow().about_open && crate::about_dialog::take_closed() {
            let mut lc = self.lifecycle.borrow_mut();
            lc.about_open = false;
            lc.about_standalone = false;
        }

        if let Some(sysmon) = &self.sysmon
            && let Ok(sample) = sysmon.rx.try_recv()
        {
            self.last_sample = sample;
        }

        // 重绘策略：尽量事件驱动，绝不让事件循环空转——这是老机器上常驻 CPU 的关键。
        // - 正在「置顶唤回」：30ms 短间隔快速收敛（原有行为，仅十几帧）。
        // - 有扫描在跑：扫描页自己按 ~30fps 刷新（见 scan_page / progress_ring）；
        //   这里只留一个低频兜底（可见 250ms / 托盘 500ms），保证用户停在其它页面、
        //   或窗口隐藏时，扫描事件仍能被排空、结果被记录。
        // - 其余（闲置，无论窗口可见还是纯托盘）：**不安排任何定时重绘**。托盘/菜单
        //   点击由 wakeup 转发线程唤醒，底部资源条由 sysmon 采样线程按 1Hz 唤醒，
        //   Toast 有自己的短时定时器，键盘/鼠标输入本身就会触发重绘。
        let visible = ctx.input(|i| i.viewport().visible().unwrap_or(true));
        if self.activate_countdown > 0 {
            ctx.request_repaint_after(Duration::from_millis(30));
        } else {
            let scan_running = {
                let core = self.core.borrow();
                core.quick.is_running() || core.full.is_running()
            };
            if scan_running {
                ctx.request_repaint_after(Duration::from_millis(if visible {
                    250
                } else {
                    500
                }));
            }
        }
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();

        // 关闭按钮：关于打开时只关关于层（绝不连带关主窗口）；否则（且非真正退出）
        // 最小化到托盘。
        if ctx.input(|i| i.viewport().close_requested()) {
            ctx.send_viewport_cmd(ViewportCommand::CancelClose);
            if self.lifecycle.borrow().about_open {
                let mut lc = self.lifecycle.borrow_mut();
                lc.about_open = false;
                lc.about_standalone = false;
                return;
            } else if self.lifecycle.borrow().mode != RunMode::Quit && !self.allow_exit {
                self.hide_to_tray(&ctx);
                return;
            }
        }

        let (about_open, about_standalone) = {
            let lc = self.lifecycle.borrow();
            (lc.about_open, lc.about_standalone)
        };

        // 隐藏到托盘（且没开关于）时整窗不绘制——既无可见内容，也避免出现"关掉关于
        // 窗却留下主窗口"的错觉。
        if self.window_hidden && !about_open {
            return;
        }

        // 来自托盘的关于：独占整个窗口画关于页，不画主界面——背后是深色主题底，
        // 看起来就是一张独立的关于窗口。关闭后由 reconcile 自动缩回托盘，不会残留主窗口。
        if about_open && about_standalone {
            crate::about_dialog::paint_about_fullscreen(ui);
            return;
        }

        self.ensure_ui_resources(&ctx);
        self.toasts.retain(|t| !t.expired());

        #[cfg(not(windows))]
        if let Some(tex) = self.titlebar_icon_texture.clone() {
            title_bar(ui, &ctx, &tex, self);
        }

        egui::Panel::bottom("resource_bar")
            .exact_size(50.0)
            .resizable(false)
            // Panel 默认会在边缘画一条分割线，用的是主题里偏亮的 noninteractive
            // stroke，在深色底上显得很突兀（白边）——设计稿里几个区域之间基本靠
            // 背景色深浅本身区分，没有这种硬分割线，所以统一关掉。
            .show_separator_line(false)
            .frame(
                egui::Frame::default()
                    .fill(colors::BG_APP)
                    .inner_margin(egui::Margin::symmetric(20, 10)),
            )
            .show(ui, |ui| resource_bar(ui, self.last_sample));

        egui::Panel::left("sidebar")
            .exact_size(64.0)
            .resizable(false)
            .show_separator_line(false)
            .frame(egui::Frame::default().fill(colors::BG_SIDEBAR))
            .show(ui, |ui| sidebar(ui, &ctx, self));

        egui::CentralPanel::default()
            .frame(egui::Frame::default().fill(colors::BG_APP))
            .show(ui, |ui| {
                theme::paint_dotted_background(ui.painter(), ui.max_rect());
                let page = self.core.borrow().page;
                match page {
                    Page::Dashboard => dashboard_page(ui, &ctx, self),
                    Page::QuickScan => quick_scan_page(ui, self),
                    Page::VirusDb => virus_db_page(ui, self),
                    Page::FullScan => full_scan_page(ui, self),
                }
            });

        widgets::show_toasts(&ctx, &self.toasts);

        // 主窗内打开的关于（当前无入口，预留）：覆盖在主界面之上的居中模态。
        if about_open && !about_standalone {
            crate::about_dialog::paint_about_modal(&ctx);
        }
    }
}

#[cfg(not(windows))]
const TITLE_BAR_HEIGHT: f32 = 44.0;

#[cfg(not(windows))]
fn title_bar(
    ui: &mut egui::Ui,
    ctx: &egui::Context,
    icon_texture: &egui::TextureHandle,
    app: &mut App,
) {
    egui::Panel::top("title_bar")
        .exact_size(TITLE_BAR_HEIGHT)
        .resizable(false)
        .show_separator_line(false)
        .frame(egui::Frame::default().fill(colors::BG_TITLEBAR))
        .show(ui, |ui| {
            // 按钮位置直接按 `full_rect` 算好精确坐标，不走 `ui.horizontal` 的光标累加——
            // 光标累加容易因为间距估算偏差导致最后一个按钮离边缘忽远忽近，看起来"没对齐"。
            let full_rect = ui.max_rect();
            let btn_size = 32.0;
            let btn_gap = 4.0;
            let edge_margin = 8.0;

            let close_rect = egui::Rect::from_center_size(
                egui::pos2(
                    full_rect.right() - edge_margin - btn_size / 2.0,
                    full_rect.center().y,
                ),
                Vec2::splat(btn_size),
            );
            let min_rect = egui::Rect::from_center_size(
                egui::pos2(
                    close_rect.left() - btn_gap - btn_size / 2.0,
                    full_rect.center().y,
                ),
                Vec2::splat(btn_size),
            );

            if title_bar_button(ui, close_rect, "close", |painter, rect| {
                icons::close(painter, rect, Stroke::new(1.4, colors::TEXT_SECONDARY));
            }) {
                app.hide_to_tray(ctx);
            }
            if title_bar_button(ui, min_rect, "minimize", |painter, rect| {
                icons::minimize(painter, rect, Stroke::new(1.4, colors::TEXT_SECONDARY));
            }) {
                ctx.send_viewport_cmd(ViewportCommand::Minimized(true));
            }

            // 左边图标 + 标题文字，走正常的光标布局就行，反正不需要跟右边对齐。
            // 图标用真实的简化版美术图标（icon_tray.png），跟窗口图标/托盘图标
            // 保持一致，不再用矢量画的盾牌。
            ui.horizontal_centered(|ui| {
                ui.add_space(14.0);
                ui.add(
                    egui::Image::new((icon_texture.id(), icon_texture.size_vec2()))
                        .fit_to_exact_size(Vec2::splat(22.0)),
                );
                ui.add_space(8.0);
                widgets::bold_label(ui, "CLV3000", 15.0, colors::TEXT_PRIMARY);
            });

            // 整条标题栏（除了右上角两个按钮）都能拖动窗口——包括图标和标题文字
            // 那块区域。图标/文字本身只声明了 `Sense::hover()`，不感知拖拽，
            // 所以叠在它们上面这个更大的拖拽区域不会跟点击/悬浮冲突。
            let drag_rect = egui::Rect::from_min_max(
                egui::pos2(full_rect.left(), full_rect.top()),
                egui::pos2(min_rect.left() - btn_gap, full_rect.bottom()),
            );
            if drag_rect.width() > 0.0 {
                let drag_resp = ui.interact(
                    drag_rect,
                    ui.id().with("titlebar_drag"),
                    egui::Sense::drag(),
                );
                // 用 `is_pointer_button_down_on`（按下当帧就 StartDrag），不要用
                // `drag_started`——后者要等越过拖拽阈值，系统 mouseDown 已过去，
                // macOS 上会拖不动窗口（见 about_dialog.rs 同款说明）。
                if drag_resp.is_pointer_button_down_on() {
                    ctx.send_viewport_cmd(ViewportCommand::StartDrag);
                }
            }
        });
}

#[cfg(not(windows))]
fn title_bar_button(
    ui: &mut egui::Ui,
    rect: egui::Rect,
    name: &str,
    draw: impl FnOnce(&egui::Painter, egui::Rect),
) -> bool {
    let response = ui
        .interact(
            rect,
            ui.id().with(("titlebar_btn", name)),
            egui::Sense::click(),
        )
        .on_hover_cursor(egui::CursorIcon::PointingHand);
    let painter = ui.painter();
    if response.hovered() {
        painter.rect_filled(rect, 6.0, colors::ACCENT_BLUE_BG);
    }
    let glyph_rect = rect.shrink(9.0);
    draw(painter, glyph_rect);
    response.clicked()
}

use crate::icons;

struct SidebarItem {
    page: Page,
    draw: fn(&egui::Painter, egui::Rect, Stroke),
}

fn sidebar(ui: &mut egui::Ui, _ctx: &egui::Context, app: &mut App) {
    ui.add_space(18.0);
    let items = [
        SidebarItem {
            page: Page::Dashboard,
            draw: |p, r, s| icons::shield(p, r, s, None),
        },
        SidebarItem {
            page: Page::QuickScan,
            draw: |p, r, s| icons::bolt(p, r, s.color),
        },
        SidebarItem {
            page: Page::FullScan,
            draw: |p, r, s| icons::hamburger(p, r, s),
        },
        SidebarItem {
            page: Page::VirusDb,
            draw: |p, r, s| icons::database(p, r, s),
        },
    ];

    for item in items {
        let active = app.core.borrow().page == item.page;
        ui.vertical_centered(|ui| {
            let size = Vec2::splat(40.0);
            let (response, painter) = ui.allocate_painter(size, egui::Sense::click());
            let response = response.on_hover_cursor(egui::CursorIcon::PointingHand);
            if active {
                painter.rect_filled(response.rect, 10.0, colors::ACCENT_BLUE_BG);
            } else if response.hovered() {
                painter.rect_filled(response.rect, 10.0, colors::BG_CARD);
            }
            let color = if active {
                colors::ACCENT_BLUE
            } else {
                colors::TEXT_SECONDARY
            };
            let glyph_rect = response.rect.shrink(9.5);
            (item.draw)(&painter, glyph_rect, Stroke::new(1.6, color));
            if response.clicked() {
                app.navigate(item.page);
            }
        });
        ui.add_space(14.0);
    }
}

fn resource_bar(ui: &mut egui::Ui, sample: ResourceSample) {
    ui.horizontal(|ui| {
        ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
            ui.add_space(ui.available_width() / 2.0 - 160.0);
            resource_meter(ui, "CPU", sample.cpu_percent);
            ui.add_space(24.0);
            resource_meter(ui, "Memory", sample.mem_percent());
        });
    });
}

fn resource_meter(ui: &mut egui::Ui, label: &str, percent: f32) {
    ui.label(egui::RichText::new(label).color(colors::TEXT_SECONDARY));
    let (response, painter) = ui.allocate_painter(Vec2::new(100.0, 8.0), egui::Sense::hover());
    let rect = response.rect;
    painter.rect_filled(rect, 4.0, colors::BG_CARD);
    let fraction = (percent / 100.0).clamp(0.0, 1.0);
    if fraction > 0.0 {
        let filled =
            egui::Rect::from_min_size(rect.min, Vec2::new(rect.width() * fraction, rect.height()));
        painter.rect_filled(filled, 4.0, theme::accent_for(percent));
    }
    ui.label(egui::RichText::new(format!("{percent:.0}%")).color(colors::TEXT_PRIMARY));
}

fn dashboard_page(ui: &mut egui::Ui, _ctx: &egui::Context, app: &mut App) {
    let today = Timestamp::now();
    let core = app.core.borrow();
    let has_threats = core
        .config
        .last_full_scan
        .as_ref()
        .map(|r| r.threats_found > 0)
        .unwrap_or(false)
        || core
            .config
            .last_quick_scan
            .as_ref()
            .map(|r| r.threats_found > 0)
            .unwrap_or(false);
    drop(core);

    let mut content_height = app.core.borrow().dashboard_content_height;
    widgets::vertically_centered(ui, &mut content_height, |ui| {
        let (color, title) = if has_threats {
            (colors::RED, "System Status: At Risk")
        } else {
            (colors::GREEN, "System Status: Secure")
        };

        // 画布比圆环本身大一圈，专门留给外面那层光晕，不然会被裁掉。
        const DIAMETER: f32 = 180.0;
        const GLOW_MARGIN: f32 = 60.0;
        let (response, painter) = ui.allocate_painter(
            Vec2::splat(DIAMETER + GLOW_MARGIN * 2.0),
            egui::Sense::hover(),
        );
        let center = response.rect.center();
        let radius = DIAMETER / 2.0 - 4.0;
        widgets::paint_glow(&painter, center, radius, color);
        painter.circle_filled(center, radius, colors::BG_CARD);
        painter.circle_stroke(center, radius, Stroke::new(3.0, color));
        let glyph_rect = egui::Rect::from_center_size(center, Vec2::splat(DIAMETER * 0.50));
        if has_threats {
            icons::warning_triangle(&painter, glyph_rect, Stroke::new(2.4, color), None);
        } else {
            icons::shield_check(&painter, glyph_rect, Stroke::new(2.4, color));
        }

        ui.add_space(20.0);
        widgets::bold_label(ui, title, 20.0, colors::TEXT_PRIMARY);
        ui.add_space(6.0);
        let sub = {
            let core = app.core.borrow();
            match &core.config.last_full_scan {
                Some(r) if r.threats_found == 0 => {
                    format!(
                        "Last Full Scan · {} · No threats found",
                        r.time.display_relative_to(&today)
                    )
                }
                Some(r) => format!(
                    "Last Full Scan · {} · {} threat(s) found",
                    r.time.display_relative_to(&today),
                    r.threats_found
                ),
                None => "No full scan performed yet".to_string(),
            }
        };
        ui.label(egui::RichText::new(sub).color(colors::TEXT_SECONDARY));

        ui.add_space(28.0);
        const BTN_GAP: f32 = 12.0;
        let row_width =
            action_button_width(ui, "Quick Scan") + BTN_GAP + action_button_width(ui, "Full Scan");
        ui.allocate_ui_with_layout(
            Vec2::new(row_width, 42.0),
            egui::Layout::left_to_right(egui::Align::Center),
            |ui| {
                if action_button(ui, "Quick Scan", |p, r, s| icons::bolt(p, r, s.color)) {
                    let removable = app.core.borrow().config.scan_removable_drives;
                    app.navigate(Page::QuickScan);
                    app.core.borrow_mut().quick.start(removable);
                }
                ui.add_space(BTN_GAP);
                if action_button(ui, "Full Scan", icons::database) {
                    let removable = app.core.borrow().config.scan_removable_drives;
                    app.navigate(Page::FullScan);
                    app.core.borrow_mut().full.start(removable);
                }
            },
        );
    });
    app.core.borrow_mut().dashboard_content_height = content_height;
}

const ACTION_BTN_ICON_SIZE: f32 = 21.0; // 原本 18，整体图标 +15% 的一部分
const ACTION_BTN_ICON_GAP: f32 = 10.0;
const ACTION_BTN_H_PAD: f32 = 16.0;
const ACTION_BTN_V_PAD: f32 = 10.0;

/// 量出 `action_button` 最终会占多宽——用来在外层把多个按钮组成的一行整体居中。
/// 之前有个地方（dashboard_page 的两个按钮）拍了个固定的"半宽"常数去居中，
/// 字体/字号一变常数就不准，按钮行跟着偏移——量出来才不会有这个问题，跟
/// `widgets::centered_stat_pills` 是同一个思路。
///
/// 这里的 `+ 2.0 * item_spacing`：`action_button` 内部是拿 `ui.add_space` 手动摆
/// 图标/文字间距的，但图标和文字本身还是走 `allocate_painter`/`allocate_exact_size`
/// 正常的部件分配路径，egui 会在每个部件放完之后**额外**把 `item_spacing` 计入
/// 光标（这是 `advance_after_rects` 的行为，跟手动 `add_space` 是否存在无关）。
/// 于是 `action_button` 实际总宽比"内边距+图标+间距+文字"这几个手动常数加起来
/// 还要再宽两份 `item_spacing`（图标后一份、文字后一份）。量的时候漏了这个，
/// 按钮行整体会比算出来的居中位置偏右——这行代码就是补上这个差。
///
/// （这条注释和下面这一行代码之前已经加过一次、后来在别的改动里意外弄丢了——
/// 症状是"闪电扫描/全盘扫描"按钮行肉眼可见地偏右，跟上面圆环/文字对不齐。
/// 如果这个问题再出现，先检查这一行是不是又被误删了。）
fn action_button_width(ui: &egui::Ui, label: &str) -> f32 {
    let text_w = widgets::measure_text_width(ui, label, 14.0);
    ACTION_BTN_H_PAD * 2.0
        + ACTION_BTN_ICON_SIZE
        + ACTION_BTN_ICON_GAP
        + text_w
        + 2.0 * ui.spacing().item_spacing.x
}

/// 一个"图标 + 文字"的胶囊按钮。
///
/// 不能直接用 `Frame::show(...)`——`Frame`/`ui.horizontal` 默认会把自己的
/// "期望尺寸"报成父容器当前的全部可用宽度（因为它们要等内容画完才知道真实大小，
/// 只能先占住最大空间），这样一来外层 `vertical_centered` 之类的居中布局，
/// 拿到的就是一个"和容器一样宽"的东西，居中也就没有意义——表现出来就是按钮
/// 从容器最左边一路铺到很宽的位置，而不是一颗居中的小胶囊。
///
/// 解决办法是自己先量出图标+文字真正需要的尺寸，把这个"小尺寸"传给
/// `allocate_ui_with_layout`，父容器的 `Align::Center` 才有东西可对齐。
///
/// 大部分调用点只关心"点没点击"，用 `action_button`（返回 `bool`）就够；病毒库
/// 页的"查看完整路径"按钮还需要挂 `.on_hover_text(...)` 做 tooltip，那种场景要
/// 拿到完整的 `Response`，用 `action_button_response`。
fn action_button(
    ui: &mut egui::Ui,
    label: &str,
    draw: impl FnOnce(&egui::Painter, egui::Rect, Stroke),
) -> bool {
    action_button_response(ui, label, draw).clicked()
}

fn action_button_response(
    ui: &mut egui::Ui,
    label: &str,
    draw: impl FnOnce(&egui::Painter, egui::Rect, Stroke),
) -> egui::Response {
    const ICON_SIZE: f32 = ACTION_BTN_ICON_SIZE;
    const ICON_GAP: f32 = ACTION_BTN_ICON_GAP;
    const H_PAD: f32 = ACTION_BTN_H_PAD;
    const V_PAD: f32 = ACTION_BTN_V_PAD;

    let text_size = Vec2::new(
        widgets::measure_text_width(ui, label, 14.0),
        ui.text_style_height(&egui::TextStyle::Body),
    );

    let desired = Vec2::new(
        H_PAD * 2.0 + ICON_SIZE + ICON_GAP + text_size.x,
        V_PAD * 2.0 + ICON_SIZE.max(text_size.y),
    );

    let bg_shape_idx = ui.painter().add(egui::Shape::Noop);
    let response = ui
        .allocate_ui_with_layout(
            desired,
            egui::Layout::left_to_right(egui::Align::Center),
            |ui| {
                ui.add_space(H_PAD);
                let (icon_resp, painter) =
                    ui.allocate_painter(Vec2::splat(ICON_SIZE), egui::Sense::hover());
                draw(
                    &painter,
                    icon_resp.rect,
                    Stroke::new(1.6, colors::ACCENT_BLUE),
                );
                ui.add_space(ICON_GAP);
                ui.label(egui::RichText::new(label).color(colors::TEXT_PRIMARY));
                ui.add_space(H_PAD);
            },
        )
        .response;

    let bg_rect = response.rect;
    let interact = ui
        .interact(bg_rect, response.id.with("btn"), egui::Sense::click())
        .on_hover_cursor(egui::CursorIcon::PointingHand);
    let fill = if interact.hovered() {
        colors::ACCENT_BLUE_BG
    } else {
        colors::BG_CARD
    };
    let shape = egui::epaint::RectShape::new(
        bg_rect,
        egui::CornerRadius::same(12),
        fill,
        Stroke::new(1.0, colors::BORDER),
        egui::epaint::StrokeKind::Inside,
    );
    ui.painter().set(bg_shape_idx, egui::Shape::Rect(shape));

    interact
}

fn quick_scan_page(ui: &mut egui::Ui, app: &mut App) {
    let mut core = app.core.borrow_mut();
    let AppCore { quick, config, .. } = &mut *core;
    scan_page(
        ui,
        quick,
        config,
        &mut app.toasts,
        "Quick Scan",
        colors::ACCENT_BLUE,
        |p, r, s| icons::bolt(p, r, s.color),
        true,
    );
}

fn full_scan_page(ui: &mut egui::Ui, app: &mut App) {
    let mut core = app.core.borrow_mut();
    let AppCore { full, config, .. } = &mut *core;
    scan_page(
        ui,
        full,
        config,
        &mut app.toasts,
        "Full Scan",
        colors::ACCENT_BLUE,
        icons::hamburger,
        true,
    );
}

/// 一个内容"量多少占多少"的居中卡片：宽高固定，父容器的居中布局才有东西可对齐
/// （原理和 `action_button` 里说的一样，`Frame` 自己没法参与外层的 `Align::Center`）。
/// 病毒库状态那处原本用它包"内置病毒库已就绪"，反馈说看起来像个不能点的按钮、
/// 很丑，改成裸文字了——这个函数暂时没有调用点，但仍是个好用的通用样式，先保留。
#[allow(dead_code)]
fn centered_card(
    ui: &mut egui::Ui,
    width: f32,
    height: f32,
    add_contents: impl FnOnce(&mut egui::Ui),
) {
    let bg_idx = ui.painter().add(egui::Shape::Noop);
    let response = ui
        .allocate_ui_with_layout(
            Vec2::new(width, height),
            egui::Layout::left_to_right(egui::Align::Center),
            |ui| {
                ui.add_space(16.0);
                add_contents(ui);
            },
        )
        .response;
    let shape = egui::epaint::RectShape::new(
        response.rect,
        egui::CornerRadius::same(10),
        colors::BG_CARD,
        Stroke::new(1.0, colors::BORDER),
        egui::epaint::StrokeKind::Inside,
    );
    ui.painter().set(bg_idx, egui::Shape::Rect(shape));
}

// 参数是有点多，但拆成一个配置 struct 目前收益不大（调用点就 2 个，字段名本身
// 已经很直白），先用 allow 压掉这条 lint。
#[allow(clippy::too_many_arguments)]
fn scan_page(
    ui: &mut egui::Ui,
    state: &mut ScanPageState,
    config: &mut AppConfig,
    toasts: &mut Vec<Toast>,
    title: &str,
    ring_color: Color32,
    icon: fn(&egui::Painter, egui::Rect, Stroke),
    show_start_button_when_idle: bool,
) {
    // 跟 dashboard_page 一样：内容少（Idle/Done 态、没有威胁列表）时用上一帧测量到
    // 的高度把这一帧的空白平分到上下，不让内容整体贴着顶部。内容比可用高度还高时
    // （威胁列表很长）`vertically_centered` 内部的 `.max(0.0)` 会让顶部空白归零，
    // 自然向下溢出，不会比原来的写法更差。
    let mut content_height = state.content_height;
    widgets::vertically_centered(ui, &mut content_height, |ui| {
        match &state.phase {
            ScanPhase::Idle => {
                // 仪表盘页那种"大圆环 + 图标"的视觉语言在这里也来一份，跟概览页
                // 呼应，不然闪电扫描/全盘扫描的待机画面只有两行字，太空。这里没有
                // 状态色（还没扫描，谈不上安全/危险），就用页面自己的强调色。
                let (deco_resp, painter) =
                    ui.allocate_painter(Vec2::splat(120.0), egui::Sense::hover());
                let deco_center = deco_resp.rect.center();
                let deco_radius = 56.0;
                painter.circle_filled(deco_center, deco_radius, colors::BG_CARD);
                painter.circle_stroke(deco_center, deco_radius, Stroke::new(2.0, ring_color));
                let deco_glyph = egui::Rect::from_center_size(deco_center, Vec2::splat(52.0));
                icon(&painter, deco_glyph, Stroke::new(2.0, ring_color));

                ui.add_space(14.0);
                ui.label(
                    egui::RichText::new(format!("Ready for {title}")).color(colors::TEXT_SECONDARY),
                );
                ui.add_space(16.0);
                const START_BTN_SHIFT_LEFT: f32 = 2.0;
                ui.horizontal(|ui| {
                    let label = format!("Start {title}");
                    let btn_w = action_button_width(ui, &label);
                    let left = (ui.available_width() - btn_w) / 2.0 - START_BTN_SHIFT_LEFT;
                    ui.add_space(left.max(0.0));
                    if show_start_button_when_idle && action_button(ui, &label, icon) {
                        state.start(config.scan_removable_drives);
                    }
                });
            }
            ScanPhase::Enumerating {
                done,
                total,
                files_found,
            } => {
                widgets::progress_ring(
                    ui,
                    220.0,
                    None,
                    ring_color,
                    "Enumerating",
                    &format!("{done}/{total} processes"),
                );
                ui.add_space(16.0);
                widgets::centered_stat_pills(
                    ui,
                    &[
                        (format!("{done} / {total}"), "processes"),
                        (files_found.to_string(), "files"),
                    ],
                );
                ui.add_space(10.0);
                if ui.link("Cancel Scan").clicked() {
                    state.request_cancel();
                }
            }
            ScanPhase::Scanning {
                total,
                scanned,
                current_path,
            } => {
                // clamscan 启动后先加载病毒库（十几秒），这段时间 current_path 是空的、
                // 一个 FileScanned 都没来。全盘扫描在 walk 磁盘阶段也是空 current_path。
                // 用旋转不定进度环 + "Preparing scan…" 区分"准备中"和"正在逐文件扫描"；
                // 两种状态都显示实时已用时长，让用户知道进度在走、不是卡死。
                let starting = current_path.is_empty();
                let percent = if starting {
                    None
                } else {
                    total.map(|t| if t == 0 { 1.0 } else { *scanned as f32 / t as f32 })
                };
                let title_text = if starting {
                    "Starting".to_string()
                } else {
                    percent
                        .map(|p| format!("{:.0}%", p * 100.0))
                        .unwrap_or_else(|| format!("{scanned}"))
                };
                let heading = if starting {
                    format!("Starting {title}")
                } else {
                    format!("Running {title}")
                };
                widgets::bold_label(ui, &heading, 14.0, colors::TEXT_PRIMARY);
                ui.add_space(6.0);
                widgets::progress_ring(
                    ui,
                    220.0,
                    percent,
                    ring_color,
                    &title_text,
                    if starting { "scan engine" } else { "" },
                );
                ui.add_space(4.0);
                let status_line = if starting {
                    "Preparing scan…".to_string()
                } else {
                    truncate(current_path, 60)
                };
                ui.label(
                    egui::RichText::new(status_line)
                        .color(colors::TEXT_SECONDARY)
                        .small(),
                );
                ui.add_space(4.0);
                if let Some(started) = state.started_at.as_ref() {
                    ui.label(
                        egui::RichText::new(format!("Elapsed {}", format_duration(started.elapsed())))
                            .color(colors::TEXT_SECONDARY)
                            .small(),
                    );
                }
                ui.add_space(16.0);
                let first_pill = match total {
                    Some(t) => (format!("{scanned} / {t}"), "files"),
                    None => (scanned.to_string(), "scanned"),
                };
                widgets::centered_stat_pills(
                    ui,
                    &[first_pill, (state.threats.len().to_string(), "threats")],
                );
                ui.add_space(10.0);
                if ui.link("Cancel Scan").clicked() {
                    state.request_cancel();
                }
                // 旋转环靠 time 推进、已用时长每秒变化、事件限流后还有积压要继续排空——
                // 三者都需要持续重绘，否则一停下来界面就静止了。限制在 ~30fps：动画
                // 仍然流畅，但不会在老机器上按 vsync 满帧率白烧 CPU/GPU。
                ui.ctx().request_repaint_after(Duration::from_millis(33));
            }
            ScanPhase::Done {
                scanned,
                elapsed,
                cancelled,
            } => {
                let has_threats = !state.threats.is_empty();
                let color = if has_threats {
                    colors::RED
                } else {
                    colors::GREEN
                };
                const DIAMETER: f32 = 140.0;
                const GLOW_MARGIN: f32 = 50.0;
                let (response, painter) = ui.allocate_painter(
                    Vec2::splat(DIAMETER + GLOW_MARGIN * 2.0),
                    egui::Sense::hover(),
                );
                let center = response.rect.center();
                let radius = DIAMETER / 2.0 - 4.0;
                widgets::paint_glow(&painter, center, radius, color);
                painter.circle_filled(center, radius, colors::BG_CARD);
                painter.circle_stroke(center, radius, Stroke::new(3.0, color));
                let glyph_rect = egui::Rect::from_center_size(center, Vec2::splat(DIAMETER * 0.50));
                if has_threats {
                    icons::warning_triangle(&painter, glyph_rect, Stroke::new(2.2, color), None);
                } else {
                    icons::shield_check(&painter, glyph_rect, Stroke::new(2.2, color));
                }
                ui.add_space(14.0);
                let heading = if *cancelled {
                    "Scan cancelled".to_string()
                } else if has_threats {
                    format!("{} threat(s) found", state.threats.len())
                } else {
                    "No threats found".to_string()
                };
                widgets::bold_label(ui, &heading, 18.0, colors::TEXT_PRIMARY);
                ui.label(
                    egui::RichText::new(format!(
                        "{title} · Duration {} · {scanned} files scanned",
                        format_duration(*elapsed)
                    ))
                    .color(colors::TEXT_SECONDARY)
                    .small(),
                );
                ui.add_space(16.0);
                if action_button(ui, &format!("Run {title} Again"), icon) {
                    state.start(config.scan_removable_drives);
                }
            }
        }

        if let Some(err) = &state.last_error {
            ui.add_space(10.0);
            ui.label(egui::RichText::new(err).color(colors::RED));
        }

        ui.add_space(20.0);
        let mut ignore_target: Option<usize> = None;
        for (i, threat) in state.threats.iter().enumerate() {
            ui.horizontal(|ui| {
                ui.add_space((ui.available_width() - 700.0).max(0.0) / 2.0);
                ui.vertical(|ui| {
                    ui.set_width(700.0_f32.min(ui.available_width()));
                    let path_str = threat.path.display().to_string();
                    let action = widgets::threat_card(ui, &threat.virus_name, &path_str);
                    match action {
                        ThreatAction::Ignore => ignore_target = Some(i),
                        ThreatAction::Quarantine => {
                            toasts.push(Toast::new(
                                "Quarantine will be available in a future release",
                            ));
                        }
                        ThreatAction::None => {}
                    }
                    ui.add_space(8.0);
                });
            });
        }
        if let Some(i) = ignore_target {
            let t = state.threats.remove(i);
            config.add_ignored(t.path.display().to_string(), t.virus_name);
        }
    });
    state.content_height = content_height;
}

fn virus_db_page(ui: &mut egui::Ui, app: &mut App) {
    ui.add_space(28.0);
    ui.columns(2, |columns| {
        virus_db_status_column(&mut columns[0], app);
        virus_db_about_column(&mut columns[1], app);
    });
}

/// 左栏：病毒库状态 + 手动更新交互。
fn virus_db_status_column(ui: &mut egui::Ui, app: &mut App) {
    let mut core = app.core.borrow_mut();
    let mut content_height = core.virus_db.status_col_height;
    let mut pending_toast: Option<String> = None;
    widgets::vertically_centered(ui, &mut content_height, |ui| {
        let (response, painter) = ui.allocate_painter(Vec2::splat(96.0), egui::Sense::hover());
        icons::database(
            &painter,
            response.rect.shrink(6.0),
            Stroke::new(2.0, colors::ACCENT_BLUE),
        );
        ui.add_space(14.0);
        widgets::bold_label(ui, "Virus Database", 18.0, colors::TEXT_PRIMARY);
        ui.add_space(14.0);

        let available = paths::clamscan_available();
        // 第一次画这一栏时顺带查一次版本（只查这一次，之后靠"更新完成"事件刷新），
        // 避免每帧都拉起 clamscan 进程。更新成功后 `db_version` 会从旧值刷新成新值。
        if core.virus_db.db_version.is_none() {
            core.virus_db.refresh_db_version(ui.ctx().clone());
        }
        let status = if available {
            "Built-in database ready"
        } else {
            "Scan engine not found"
        };
        let detail_dir = if available {
            paths::resolved_clamav_database_dir()
                .unwrap_or_else(|| paths::clamav_database_dir())
        } else {
            paths::clamav_dir()
        };
        let path_display = detail_dir.display().to_string();

        // 状态文字之前包了一层卡片，看起来跟旁边真正能点的按钮长一个样、却点
        // 不动，容易让人以为是个坏了的按钮——改成裸文字，旁边挂一个小小的图标
        // 按钮（hover 出完整路径），不用文字说明也够直观，整体不再像个按钮。
        const INFO_ICON_SIZE: f32 = 26.0;
        const INFO_GAP: f32 = 8.0;
        let status_w = widgets::measure_text_width(ui, status, 14.0);
        // `ui.label(...)` 是个"裸"部件（跟 action_button_width 注释里说的
        // allocate_painter/allocate_exact_size 一样），egui 会在它后面自动追加
        // 一份 item_spacing，再叠加下面显式的 `INFO_GAP`——量宽度的时候得把这份
        // 自动间距也算进去，不然这一行会比按钮行整体偏左几像素。
        let status_row_w = status_w + ui.spacing().item_spacing.x + INFO_GAP + INFO_ICON_SIZE;
        ui.allocate_ui_with_layout(
            Vec2::new(status_row_w, INFO_ICON_SIZE),
            egui::Layout::left_to_right(egui::Align::Center),
            |ui| {
                ui.label(egui::RichText::new(status).color(colors::TEXT_SECONDARY));
                ui.add_space(INFO_GAP);
                widgets::icon_only_button(ui, INFO_ICON_SIZE, icons::info_circle)
                    .on_hover_text(path_display.as_str());
            },
        );

        if let Some(ver) = &core.virus_db.db_version {
            ui.label(
                egui::RichText::new(format!("Version: {ver}"))
                    .color(colors::TEXT_MUTED)
                    .small(),
            );
        }

        ui.add_space(16.0);
        // "打开所在文件夹"和"手动更新病毒库"放同一行——都是这一栏里的辅助操作，
        // 分两行意义不大，合一行更紧凑。宽度量出来再居中，见 `action_button_width`
        // 的注释。
        let update_label = if core.virus_db.updating {
            "Updating…"
        } else {
            "Update Database"
        };
        const BTN_GAP: f32 = 12.0;
        let row_width = action_button_width(ui, "Open Folder")
            + BTN_GAP
            + action_button_width(ui, update_label);
        ui.allocate_ui_with_layout(
            Vec2::new(row_width, 42.0),
            egui::Layout::left_to_right(egui::Align::Center),
            |ui| {
                if action_button(ui, "Open Folder", icons::folder) {
                    // 文件夹不一定存在（比如引擎没找到、数据库还没更新过一次）——
                    // 先确保目录存在再打开，不然系统文件管理器会直接报错。
                    paths::ensure_dir(&detail_dir);
                    if let Err(e) = paths::open_in_file_explorer(&detail_dir) {
                        pending_toast = Some(e);
                    }
                }
                ui.add_space(BTN_GAP);
                if action_button(ui, update_label, icons::database) && !core.virus_db.updating {
                    core.virus_db.start_update(ui.ctx().clone());
                    pending_toast = Some("Updating database…".to_string());
                }
            },
        );
    });
    core.virus_db.status_col_height = content_height;
    drop(core);
    if let Some(msg) = pending_toast {
        app.toast(msg);
    }
}

/// 右栏：关于（真实品牌图标 + 名称 + 版本 + 简介）。
fn virus_db_about_column(ui: &mut egui::Ui, app: &mut App) {
    let mut core = app.core.borrow_mut();
    let mut content_height = core.virus_db.about_col_height;
    widgets::vertically_centered(ui, &mut content_height, |ui| {
        const LOGO_DISPLAY_PT: f32 = 90.0;
        if let Some(tex) = &app.app_icon_texture {
            ui.add(
                egui::Image::new((tex.id(), tex.size_vec2()))
                    .fit_to_exact_size(Vec2::splat(LOGO_DISPLAY_PT))
                    .corner_radius(16.0),
            );
        }
        ui.add_space(12.0);
        widgets::bold_label(ui, "CLV3000", 17.0, colors::TEXT_PRIMARY);
        ui.add_space(4.0);
        ui.label(
            egui::RichText::new(format!("Version {}", env!("CARGO_PKG_VERSION")))
                .color(colors::TEXT_SECONDARY)
                .small(),
        );
        ui.add_space(10.0);
        ui.label(
            egui::RichText::new("Fast, reliable virus protection for even older PCs")
                .color(colors::TEXT_MUTED)
                .small(),
        );
    });
    core.virus_db.about_col_height = content_height;
}

fn truncate(s: &str, max_chars: usize) -> String {
    if s.chars().count() <= max_chars {
        s.to_string()
    } else {
        let head: String = s.chars().take(max_chars).collect();
        format!("{head}…")
    }
}

fn format_duration(d: Duration) -> String {
    let secs = d.as_secs();
    let m = secs / 60;
    let s = secs % 60;
    if m > 0 {
        format!("{m}m {s}s")
    } else {
        format!("{s}s")
    }
}
