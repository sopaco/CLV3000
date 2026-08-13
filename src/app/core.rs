//! 跨帧保留的业务状态：页面路由、配置、两个扫描页各自的状态机、病毒库页状态。
//!
//! `App`（`app/mod.rs`）直接拥有一个 `AppCore`——不再是 `Rc<RefCell<AppCore>>`。
//! 早先用 `Rc<RefCell<>>` 是因为设想 eframe 会话可能被销毁重建、状态要跨会话
//! 存活；但 `main.rs` 里 `eframe::run_native` 现在只调用一次、贯穿整个进程
//! 生命周期，会话根本不会重建，`App` 直接 own 这份状态即可，`borrow()`/
//! `borrow_mut()` 与运行时 panic 风险随之整类消失。

use super::freshclam::run_freshclam;
use super::Page;
use crate::clamav_info::ClamAvInfo;
use crate::config::{AppConfig, ScanRecord};
use crate::localtime::Timestamp;
use crate::paths;
use crate::scan::{self, CancelFlag, ScanEvent, ScanKind, Threat};
use eframe::egui;
use std::path::PathBuf;
use std::sync::atomic::Ordering;
use std::sync::mpsc::Receiver;
use std::time::{Duration, Instant};

pub(crate) enum ScanPhase {
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

pub(crate) struct ScanPageState {
    pub(crate) kind: ScanKind,
    pub(crate) phase: ScanPhase,
    cancel: Option<CancelFlag>,
    rx: Option<Receiver<ScanEvent>>,
    pub(crate) threats: Vec<Threat>,
    pub(crate) last_error: Option<String>,
    /// 扫描启动时刻；用于在 Scanning 阶段实时显示已用时长（clamscan 加载病毒库的
    /// 十几秒里没有任何 FileScanned 事件，靠这个让用户知道没卡死）。
    pub(crate) started_at: Option<Instant>,
    /// 上一帧这个页面实际画出来的内容高度，给 `widgets::vertically_centered` 用来
    /// 算这一帧该留多少顶部空白。见该函数文档注释。
    pub(crate) content_height: f32,
    /// clamscan 已 spawn、病毒库加载中（`EngineLoading` 置位，`ScanningFile` 清除）。
    pub(crate) engine_loading: bool,
    /// `EngineLoading` 携带的待引擎扫描文件数（仅用于状态行提示）。
    pub(crate) engine_loading_remaining: usize,
    /// 全盘扫描磁盘 walk 阶段已发现的可扫文件数。
    pub(crate) walk_files_found: usize,
    /// clamscan `-v` 的 `Scanning <path>` 行：当前正在引擎内检测的文件。
    pub(crate) engine_scanning_path: Option<String>,
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
            engine_loading: false,
            engine_loading_remaining: 0,
            walk_files_found: 0,
            engine_scanning_path: None,
        }
    }

    pub(crate) fn is_running(&self) -> bool {
        matches!(
            self.phase,
            ScanPhase::Enumerating { .. } | ScanPhase::Scanning { .. }
        )
    }

    pub(crate) fn start(&mut self, scan_removable: bool) {
        if self.is_running() {
            return;
        }
        self.threats.clear();
        self.last_error = None;
        self.started_at = Some(Instant::now());
        self.engine_loading = false;
        self.engine_loading_remaining = 0;
        self.walk_files_found = 0;
        self.engine_scanning_path = None;
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

    /// 跟 `start()` 几乎一样，只是扫描目标从"枚举出来的一大批路径"换成"用户指定
    /// 的单个文件/文件夹"——右键菜单"用 CLV3000 扫描"/`--scan-path` 触发（见
    /// `App::begin_path_scan`）。只在 `full`（kind=Full）状态上调用：复用现成的
    /// 全盘扫描页面渲染，进度环/威胁列表/Done 态都已经支持"total 未知"和"任意
    /// 威胁列表"，不用新造一套 UI。
    pub(crate) fn start_path(&mut self, target: std::path::PathBuf) {
        if self.is_running() {
            return;
        }
        self.threats.clear();
        self.last_error = None;
        self.started_at = Some(Instant::now());
        self.engine_loading = false;
        self.engine_loading_remaining = 0;
        self.walk_files_found = 0;
        self.engine_scanning_path = None;
        let cancel = scan::new_cancel_flag();
        let (tx, rx) = std::sync::mpsc::channel();
        self.cancel = Some(cancel.clone());
        self.rx = Some(rx);
        self.phase = ScanPhase::Scanning {
            total: None,
            scanned: 0,
            current_path: String::new(),
        };
        std::thread::spawn(move || scan::full_scan::run_single_target(tx, cancel, target));
    }

    pub(crate) fn request_cancel(&self) {
        if let Some(c) = &self.cancel {
            c.store(true, Ordering::SeqCst);
        }
    }

    /// 返回 `Some((scanned, elapsed, cancelled))` 当这一批事件里出现了 `Finished`。
    ///
    /// 每一帧把 channel 里**所有**事件都排空并处理：clamscan 的 stdout 在管道上是块
    /// 缓冲的，整个扫描的结果常常在进程退出时一次性 flush 进 channel。早期版本用
    /// `MAX_EVENTS_PER_FRAME` 把每帧处理的事件数限死（比如 4 个），上千个结果就被
    /// 分摊到几百帧才排完，`Finished` 也跟着晚好几秒到达——进度环明明已扫完却还卡在
    /// 很低的数、Done 页迟迟不出现，这就是"扫描页尾部滞后"。
    ///
    /// 现在每帧全排空：正常流式扫描时每帧本来只有 1~2 个事件，进度环照常逐帧爬升；
    /// 只有进程退出那次突发 flush 会在一帧内把剩余结果全处理完，进度直接跳到 100%
    /// 并立刻进入 Done——这正是我们想要的行为，滞后消失。
    fn poll(&mut self, config: &AppConfig) -> Option<(usize, Duration, bool)> {
        let mut finished = None;
        let Some(rx) = &self.rx else { return None };
        while let Ok(event) = rx.try_recv() {
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
                    self.engine_loading = false;
                    self.engine_loading_remaining = 0;
                    self.engine_scanning_path = None;
                    self.walk_files_found = 0;
                    self.phase = ScanPhase::Scanning {
                        total,
                        scanned: 0,
                        current_path: String::new(),
                    };
                }
                ScanEvent::WalkProgress { files_found } => {
                    self.walk_files_found = files_found;
                }
                ScanEvent::EngineLoading { remaining } => {
                    self.engine_loading = true;
                    self.engine_loading_remaining = remaining;
                    self.engine_scanning_path = None;
                }
                ScanEvent::ScanningFile { path } => {
                    self.engine_loading = false;
                    self.engine_scanning_path = Some(path.clone());
                    if let ScanPhase::Scanning { current_path, .. } = &mut self.phase {
                        *current_path = path;
                    }
                }
                ScanEvent::FileScanned { path, infected } => {
                    self.engine_loading = false;
                    self.engine_scanning_path = None;
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
/// 两个变体分别在 `freshclam::run_freshclam`（Windows 真更新 / 未变）与开发
/// mock 中构造，并由 `poll_background` 的 `match` 消费——无 dead_code 警告。
#[derive(Debug, Clone, Copy)]
pub(crate) enum UpdateOutcome {
    Updated,
    AlreadyUpToDate,
}

/// `paths::clamscan_available()` + `resolved_clamav_database_dir()` 的探测结果。
/// 这两个查询要 `is_file()`/`read_dir()`，macOS 上 `clamscan_available` 在内置/系统
/// 路径都没找到时还要整个扫一遍 `PATH` 逐目录 `is_file()`——这些结果在一次运行里
/// 几乎不会变，之前每次画病毒库页都会重新查一遍（哪怕只是鼠标在窗口里晃一下触发
/// 的重绘），纯属浪费。见 `VirusDbState::engine_probe`。
pub(crate) struct EngineProbe {
    pub(crate) available: bool,
    pub(crate) detail_dir: PathBuf,
}

pub(crate) struct VirusDbState {
    pub(crate) updating: bool,
    rx: Option<Receiver<Result<UpdateOutcome, String>>>,
    /// 后台版本查询（`refresh_db_version`）的回传通道。`db_version` 在 `poll` 里
    /// 从这条通道接收结果写回，保证版本刷新不阻塞 UI 线程。
    version_rx: Option<Receiver<String>>,
    /// 左右两栏各自的上一帧内容高度，给 `widgets::vertically_centered` 用。两栏
    /// 内容不一样高，各自居中，不能共用一个高度。
    pub(crate) status_col_height: f32,
    pub(crate) about_col_height: f32,
    /// 当前病毒库版本（来自 `clamscan -V`），更新完成后刷新，避免界面一直显示旧版本。
    pub(crate) db_version: Option<String>,
    /// 引擎可用性 + 病毒库目录探测结果的缓存，`None` 表示需要（重新）探测一次。
    engine_probe: Option<EngineProbe>,
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
            engine_probe: None,
        }
    }

    /// 懒探测一次引擎可用性 + 病毒库目录，之后每帧直接复用缓存结果。
    pub(crate) fn engine_probe(&mut self) -> &EngineProbe {
        if self.engine_probe.is_none() {
            let available = paths::clamscan_available();
            let detail_dir = if available {
                paths::resolved_clamav_database_dir().unwrap_or_else(paths::clamav_database_dir)
            } else {
                paths::clamav_dir()
            };
            self.engine_probe = Some(EngineProbe {
                available,
                detail_dir,
            });
        }
        self.engine_probe.as_ref().expect("just set above")
    }

    /// 更新成功真的落了新文件时，之前"未找到病毒库"的探测结果可能已经过期
    /// （比如首次更新前候选目录都是空的）——清掉缓存，下次画面重新探测一次。
    /// "已是最新"没有文件变化，不需要重新探测。
    fn invalidate_engine_probe(&mut self) {
        self.engine_probe = None;
    }

    /// 异步重新查询病毒库版本：后台线程跑 `clamscan -V`，结果经 `version_rx` 回传，
    /// 不在 UI 线程阻塞（之前是同步调 `database_version()`，会卡住一帧）。
    /// 已有查询在飞（`version_rx` 挂着）就跳过，避免重复发起。
    /// `ctx` 用于查询完成时唤醒 UI（闲置时 UI 没有定时心跳，不唤醒结果就一直没人收）。
    pub(crate) fn refresh_db_version(&mut self, ctx: egui::Context) {
        if self.version_rx.is_some() {
            return;
        }
        let (tx, rx) = std::sync::mpsc::channel();
        self.version_rx = Some(rx);
        std::thread::spawn(move || {
            let v = ClamAvInfo::database_version();
            let _ = tx.send(v);
            ctx.request_repaint();
        });
    }

    pub(crate) fn start_update(&mut self, ctx: egui::Context) {
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

/// 设置页两个 tab：隔离区/忽略列表管理，和一个统一的"应用与引擎设置"（自启动+
/// 右键菜单，未来同类的应用级开关也归这里，不再按功能各开一个 tab）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SettingsTab {
    QuarantineIgnore,
    General,
}

/// 设置页跨帧状态：当前 tab + 自启动/右键菜单的懒探测缓存（跟
/// `VirusDbState::engine_probe` 同一个模式——真的碰文件系统/注册表只在首次
/// 渲染或用户操作后失效重探时做一次，其余帧直接读缓存）。
pub(crate) struct SettingsState {
    pub(crate) tab: SettingsTab,
    pub(crate) autostart_enabled: Option<bool>,
    /// 只在 Windows 上被 `app/settings.rs` 的 `#[cfg(windows)]` 分支读写——右键
    /// 菜单功能仅 Windows 实现（已与用户确认，见计划文档）。非 Windows 编译目标
    /// 上这是预期内的 dead code。
    #[allow(dead_code)]
    pub(crate) context_menu_enabled: Option<bool>,
}

impl SettingsState {
    fn new() -> Self {
        Self {
            tab: SettingsTab::QuarantineIgnore,
            autostart_enabled: None,
            context_menu_enabled: None,
        }
    }
}

/// 跨帧保留的业务状态（配置、扫描、页面路由）。纯托盘模式下也存在。
pub(crate) struct AppCore {
    pub(crate) page: Page,
    pub(crate) config: AppConfig,
    pub(crate) quick: ScanPageState,
    pub(crate) full: ScanPageState,
    pub(crate) virus_db: VirusDbState,
    pub(crate) settings: SettingsState,
    pub(crate) dashboard_content_height: f32,
}

impl AppCore {
    pub(crate) fn new() -> Self {
        Self {
            page: Page::Dashboard,
            config: AppConfig::load(),
            quick: ScanPageState::new(ScanKind::Quick),
            full: ScanPageState::new(ScanKind::Full),
            virus_db: VirusDbState::new(),
            settings: SettingsState::new(),
            dashboard_content_height: 0.0,
        }
    }

    /// 闪电扫描和全盘扫描是否有任意一个正在跑。两者共享临时扫描列表文件命名
    /// （`/tmp/clv3000_scanlist_<pid>.txt`，见 `engine.rs`）与文件基因缓存
    /// （`ScanCache`，见 `cache.rs`）——`ScanPageState::start` 只检查自己这一页，
    /// 不知道另一页也在跑，两个扫描并发会互相覆盖对方的临时文件、缓存落盘时
    /// 后写者整体覆盖前写者。所有触发扫描的入口都必须先查这个再决定是否放行。
    pub(crate) fn any_scan_running(&self) -> bool {
        self.quick.is_running() || self.full.is_running()
    }

    /// 轮询扫描/病毒库后台线程；返回需要弹 Toast 的消息（仅窗口会话消费）。
    /// `ctx` 透传给更新完成后的版本号刷新（后台线程完成时要能唤醒 UI）。
    pub(crate) fn poll_background(&mut self, ctx: &egui::Context) -> Vec<String> {
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
                    self.virus_db.invalidate_engine_probe();
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
