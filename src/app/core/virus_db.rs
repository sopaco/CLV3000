//! 病毒库页状态：freshclam 更新、版本查询、引擎探测缓存。

use crate::clamav_info::ClamAvInfo;
use crate::paths;
use eframe::egui;
use std::path::PathBuf;
use std::sync::mpsc::Receiver;

use super::super::freshclam::run_freshclam;

/// 病毒库更新结果：区分"真的更新了"和"已经是最新、无需更新"。
#[derive(Debug, Clone, Copy)]
pub(crate) enum UpdateOutcome {
    Updated,
    AlreadyUpToDate,
}

pub(crate) struct EngineProbe {
    pub(crate) available: bool,
    pub(crate) detail_dir: PathBuf,
}

pub(crate) struct VirusDbState {
    pub(crate) updating: bool,
    rx: Option<Receiver<Result<UpdateOutcome, String>>>,
    version_rx: Option<Receiver<String>>,
    /// 页面内容高度缓存（垂直居中用，见 `widgets::vertically_centered`）。
    pub(crate) content_height: f32,
    pub(crate) db_version: Option<String>,
    engine_probe: Option<EngineProbe>,
    /// 本次运行是否已经发起过一次自动检查。自动检查只在启动后做一次
    /// （结果与「上次检查」时间一起展示），不做高频轮询。
    auto_check_done: bool,
}

impl VirusDbState {
    pub(super) fn new() -> Self {
        Self {
            updating: false,
            rx: None,
            version_rx: None,
            content_height: 0.0,
            db_version: None,
            engine_probe: None,
            auto_check_done: false,
        }
    }

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

    pub(super) fn invalidate_engine_probe(&mut self) {
        self.engine_probe = None;
    }

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
            ctx.request_repaint();
        });
    }

    /// 自动检查：本次运行只发起一次（`auto_check_done` 置位后不再重复）。
    /// 放在 `poll_background` 每帧调用里，返回 true 表示这一帧发起了检查。
    pub(crate) fn maybe_auto_update(&mut self, ctx: egui::Context) -> bool {
        if self.auto_check_done || self.updating {
            return false;
        }
        self.auto_check_done = true;
        if !paths::freshclam_available() {
            return false;
        }
        self.start_update(ctx);
        true
    }

    pub(super) fn poll(&mut self) -> Option<Result<UpdateOutcome, String>> {
        let update_result = match self.rx.as_ref().map(|rx| rx.try_recv()) {
            Some(Ok(r)) => {
                self.updating = false;
                self.rx = None;
                Some(r)
            }
            Some(Err(std::sync::mpsc::TryRecvError::Disconnected)) => {
                self.updating = false;
                self.rx = None;
                Some(Err("Update thread stopped unexpectedly".to_string()))
            }
            _ => None,
        };

        match self.version_rx.as_ref().map(|rx| rx.try_recv()) {
            Some(Ok(v)) => {
                self.db_version = Some(v);
                self.version_rx = None;
            }
            Some(Err(std::sync::mpsc::TryRecvError::Disconnected)) => {
                self.version_rx = None;
            }
            _ => {}
        }

        update_result
    }
}
