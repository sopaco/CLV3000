//! 扫描页状态机：`ScanPhase` 与 `ScanPageState`，以及可单测的 `apply_scan_event`。

use crate::config::AppConfig;
use crate::scan::{self, CancelFlag, ScanEvent, ScanKind, Threat};
use std::path::PathBuf;
use std::sync::atomic::Ordering;
use std::sync::mpsc::Receiver;
use std::time::{Duration, Instant};

#[derive(Debug)]
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
    pub(crate) started_at: Option<Instant>,
    pub(crate) content_height: f32,
    pub(crate) engine_loading: bool,
    pub(crate) engine_loading_remaining: usize,
    pub(crate) walk_files_found: usize,
    pub(crate) engine_scanning_path: Option<String>,
    #[allow(dead_code)]
    pub(crate) pending_force_quarantine: Option<PendingForceQuarantine>,
    #[allow(dead_code)]
    force_quarantine_rx: Option<Receiver<Result<crate::config::QuarantineEntry, String>>>,
    #[allow(dead_code)]
    force_quarantine_path: Option<PathBuf>,
}

#[allow(dead_code)]
pub(crate) struct PendingForceQuarantine {
    pub(crate) path: PathBuf,
    pub(crate) virus_name: String,
}

/// `apply_scan_event` 的可变视图：把 `ScanPageState` 里会被事件更新的字段
/// 集中传给纯函数，便于单测而不构造完整的 `ScanPageState`。
pub(crate) struct ScanEventContext<'a> {
    pub phase: &'a mut ScanPhase,
    pub engine_loading: &'a mut bool,
    pub engine_loading_remaining: &'a mut usize,
    pub walk_files_found: &'a mut usize,
    pub engine_scanning_path: &'a mut Option<String>,
    pub started_at: &'a mut Option<Instant>,
    pub last_error: &'a mut Option<String>,
    pub threats: &'a mut Vec<Threat>,
    pub config: &'a AppConfig,
}

/// 把单个 `ScanEvent` 应用到扫描 UI 状态。返回 `Some((scanned, elapsed, cancelled))`
/// 当事件为 `Finished`。
pub(crate) fn apply_scan_event(
    ctx: &mut ScanEventContext<'_>,
    event: ScanEvent,
) -> Option<(usize, Duration, bool)> {
    match event {
        ScanEvent::Enumerating {
            processes_done,
            processes_total,
            files_found,
        } => {
            *ctx.phase = ScanPhase::Enumerating {
                done: processes_done,
                total: processes_total,
                files_found,
            };
            None
        }
        ScanEvent::ScanStarted { total } => {
            *ctx.engine_loading = false;
            *ctx.engine_loading_remaining = 0;
            *ctx.engine_scanning_path = None;
            *ctx.walk_files_found = 0;
            *ctx.phase = ScanPhase::Scanning {
                total,
                scanned: 0,
                current_path: String::new(),
            };
            None
        }
        ScanEvent::WalkProgress { files_found } => {
            *ctx.walk_files_found = files_found;
            None
        }
        ScanEvent::EngineLoading { remaining } => {
            *ctx.engine_loading = true;
            *ctx.engine_loading_remaining = remaining;
            *ctx.engine_scanning_path = None;
            None
        }
        ScanEvent::ScanningFile { path } => {
            *ctx.engine_loading = false;
            *ctx.engine_scanning_path = Some(path.clone());
            if let ScanPhase::Scanning { current_path, .. } = ctx.phase {
                *current_path = path;
            }
            None
        }
        ScanEvent::FileScanned { path, infected } => {
            *ctx.engine_loading = false;
            *ctx.engine_scanning_path = None;
            let total_hint = match ctx.phase {
                ScanPhase::Enumerating { files_found, .. } => Some(*files_found),
                ScanPhase::Scanning { total, .. } => *total,
                _ => None,
            };
            let scanned_before = match ctx.phase {
                ScanPhase::Scanning { scanned, .. } => *scanned,
                _ => 0,
            };
            if let Some(name) = infected
                && !ctx.config.is_ignored(&path, &name)
            {
                ctx.threats.push(Threat {
                    path: PathBuf::from(&path),
                    virus_name: name,
                });
            }
            *ctx.phase = ScanPhase::Scanning {
                total: total_hint,
                scanned: scanned_before + 1,
                current_path: path,
            };
            None
        }
        ScanEvent::Finished {
            scanned,
            elapsed,
            cancelled,
        } => {
            *ctx.phase = ScanPhase::Done {
                scanned,
                elapsed,
                cancelled,
            };
            *ctx.started_at = None;
            Some((scanned, elapsed, cancelled))
        }
        ScanEvent::Error(e) => {
            *ctx.last_error = Some(e);
            None
        }
    }
}

impl ScanPageState {
    pub(super) fn new(kind: ScanKind) -> Self {
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
            pending_force_quarantine: None,
            force_quarantine_rx: None,
            force_quarantine_path: None,
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

    #[cfg(windows)]
    pub(crate) fn start_force_quarantine(
        &mut self,
        ctx: &eframe::egui::Context,
        pending: PendingForceQuarantine,
    ) {
        let (tx, rx) = std::sync::mpsc::channel();
        self.force_quarantine_rx = Some(rx);
        self.force_quarantine_path = Some(pending.path.clone());
        self.pending_force_quarantine = None;
        let path = pending.path.clone();
        let virus_name = pending.virus_name.clone();
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let result = crate::quarantine::force_quarantine_file(&path, &virus_name);
            let _ = tx.send(result);
            ctx.request_repaint();
        });
    }

    #[cfg(windows)]
    pub(crate) fn poll_force_quarantine(
        &mut self,
    ) -> Option<Result<(crate::config::QuarantineEntry, PathBuf), String>> {
        let rx = self.force_quarantine_rx.as_ref()?;
        match rx.try_recv() {
            Ok(Ok(entry)) => {
                let path = self.force_quarantine_path.take();
                self.force_quarantine_rx = None;
                Some(Ok((entry, path.unwrap_or_default())))
            }
            Ok(Err(e)) => {
                self.force_quarantine_rx = None;
                self.force_quarantine_path = None;
                Some(Err(e))
            }
            Err(std::sync::mpsc::TryRecvError::Empty) => None,
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                self.force_quarantine_rx = None;
                self.force_quarantine_path = None;
                Some(Err("Force quarantine thread stopped unexpectedly".to_string()))
            }
        }
    }

    #[allow(dead_code)]
    pub(crate) fn is_force_quarantining(&self) -> bool {
        self.force_quarantine_rx.is_some()
    }

    pub(super) fn poll(&mut self, config: &AppConfig) -> Option<(usize, Duration, bool)> {
        let mut finished = None;
        let Some(rx) = &self.rx else {
            return None;
        };
        while let Ok(event) = rx.try_recv() {
            let mut ctx = ScanEventContext {
                phase: &mut self.phase,
                engine_loading: &mut self.engine_loading,
                engine_loading_remaining: &mut self.engine_loading_remaining,
                walk_files_found: &mut self.walk_files_found,
                engine_scanning_path: &mut self.engine_scanning_path,
                started_at: &mut self.started_at,
                last_error: &mut self.last_error,
                threats: &mut self.threats,
                config,
            };
            if let Some(result) = apply_scan_event(&mut ctx, event) {
                finished = Some(result);
            }
        }
        finished
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::AppConfig;

    struct TestScanCtx<'a> {
        phase: ScanPhase,
        engine_loading: bool,
        engine_loading_remaining: usize,
        walk_files_found: usize,
        engine_scanning_path: Option<String>,
        started_at: Option<Instant>,
        last_error: Option<String>,
        threats: Vec<Threat>,
        config: &'a AppConfig,
    }

    impl<'a> TestScanCtx<'a> {
        fn new(config: &'a AppConfig) -> Self {
            Self {
                phase: ScanPhase::Idle,
                engine_loading: false,
                engine_loading_remaining: 0,
                walk_files_found: 0,
                engine_scanning_path: None,
                started_at: Some(Instant::now()),
                last_error: None,
                threats: Vec::new(),
                config,
            }
        }

        fn apply(&mut self, event: ScanEvent) -> Option<(usize, Duration, bool)> {
            let mut ctx = ScanEventContext {
                phase: &mut self.phase,
                engine_loading: &mut self.engine_loading,
                engine_loading_remaining: &mut self.engine_loading_remaining,
                walk_files_found: &mut self.walk_files_found,
                engine_scanning_path: &mut self.engine_scanning_path,
                started_at: &mut self.started_at,
                last_error: &mut self.last_error,
                threats: &mut self.threats,
                config: self.config,
            };
            apply_scan_event(&mut ctx, event)
        }
    }

    #[test]
    fn enumerating_to_scanning_via_scan_started() {
        let config = AppConfig::default();
        let mut state = TestScanCtx::new(&config);
        state.apply(ScanEvent::Enumerating {
            processes_done: 2,
            processes_total: 10,
            files_found: 42,
        });
        state.apply(ScanEvent::ScanStarted {
            total: Some(42),
        });
        match state.phase {
            ScanPhase::Scanning {
                total,
                scanned,
                current_path,
            } => {
                assert_eq!(total, Some(42));
                assert_eq!(scanned, 0);
                assert!(current_path.is_empty());
            }
            other => panic!("expected Scanning, got {other:?}"),
        }
    }

    #[test]
    fn file_scanned_records_threat_when_not_ignored() {
        let config = AppConfig::default();
        let mut state = TestScanCtx::new(&config);
        state.phase = ScanPhase::Scanning {
            total: Some(1),
            scanned: 0,
            current_path: String::new(),
        };
        state.apply(ScanEvent::FileScanned {
            path: "/tmp/evil.exe".to_string(),
            infected: Some("EICAR".to_string()),
        });
        assert_eq!(state.threats.len(), 1);
        assert_eq!(state.threats[0].virus_name, "EICAR");
        match state.phase {
            ScanPhase::Scanning {
                scanned, total, ..
            } => {
                assert_eq!(scanned, 1);
                assert_eq!(total, Some(1));
            }
            other => panic!("expected Scanning, got {other:?}"),
        }
    }

    #[test]
    fn file_scanned_skips_ignored_threat() {
        let mut config = AppConfig::default();
        config.add_ignored("/tmp/evil.exe".to_string(), "EICAR".to_string());
        let mut state = TestScanCtx::new(&config);
        state.phase = ScanPhase::Scanning {
            total: Some(1),
            scanned: 0,
            current_path: String::new(),
        };
        state.apply(ScanEvent::FileScanned {
            path: "/tmp/evil.exe".to_string(),
            infected: Some("EICAR".to_string()),
        });
        assert!(state.threats.is_empty());
    }

    #[test]
    fn finished_clears_started_at_and_returns_summary() {
        let config = AppConfig::default();
        let mut state = TestScanCtx::new(&config);
        state.phase = ScanPhase::Scanning {
            total: Some(5),
            scanned: 5,
            current_path: String::new(),
        };
        let elapsed = Duration::from_secs(3);
        let result = state.apply(ScanEvent::Finished {
            scanned: 5,
            elapsed,
            cancelled: false,
        });
        assert_eq!(result, Some((5, elapsed, false)));
        assert!(state.started_at.is_none());
        match state.phase {
            ScanPhase::Done {
                scanned,
                cancelled,
                ..
            } => {
                assert_eq!(scanned, 5);
                assert!(!cancelled);
            }
            other => panic!("expected Done, got {other:?}"),
        }
    }

    #[test]
    fn walk_progress_updates_counter() {
        let config = AppConfig::default();
        let mut state = TestScanCtx::new(&config);
        state.apply(ScanEvent::WalkProgress { files_found: 128 });
        assert_eq!(state.walk_files_found, 128);
    }

    #[test]
    fn engine_loading_sets_flags() {
        let config = AppConfig::default();
        let mut state = TestScanCtx::new(&config);
        state.apply(ScanEvent::EngineLoading { remaining: 7 });
        assert!(state.engine_loading);
        assert_eq!(state.engine_loading_remaining, 7);
    }
}
