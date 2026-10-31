//! 跨帧保留的业务状态：页面路由、配置、扫描页、病毒库页、设置页。

mod auto_scan;
mod scan_state;
mod settings_state;
mod virus_db;

pub(crate) use auto_scan::{AutoScanKind, AutoScanState};
pub(crate) use scan_state::{ScanPageState, ScanPhase};
// PendingForceQuarantine 只在 Windows 扫码页强制隔离逻辑里用到（cfg(windows)），
// 在非 Windows 构建下这个重导出会报 unused import，所以按平台条件导出。
#[cfg(windows)]
pub(crate) use scan_state::PendingForceQuarantine;
pub(crate) use settings_state::{SettingsState, SettingsTab};
pub(crate) use virus_db::{UpdateOutcome, VirusDbState};

use super::Page;
use crate::config::{AppConfig, ScanRecord};
use crate::localtime::Timestamp;
use crate::scan::ScanKind;
use eframe::egui;

pub(crate) struct AppCore {
    pub(crate) page: Page,
    pub(crate) config: AppConfig,
    pub(crate) quick: ScanPageState,
    pub(crate) full: ScanPageState,
    pub(crate) virus_db: VirusDbState,
    pub(crate) settings: SettingsState,
    /// 自动扫描调度（开机 / 定时 / 更新后）。
    pub(crate) auto_scan: AutoScanState,
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
            auto_scan: AutoScanState::new(),
        }
    }

    pub(crate) fn any_scan_running(&self) -> bool {
        self.quick.is_running() || self.full.is_running()
    }

    pub(crate) fn poll_background(&mut self, ctx: &egui::Context) -> Vec<String> {
        let mut toasts = Vec::new();
        // 自动更新：启动后检查一次（本次运行只做一次，见 VirusDbState）。
        // 关闭开关后完全由用户手动点「Update Database」触发。
        if self.config.auto_update_definitions && self.virus_db.maybe_auto_update(ctx.clone()) {
            // 静默进行，不打扰用户；结果通过「上次检查」时间和版本号体现。
        }
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
                    self.config.last_update_check = Some(Timestamp::now());
                    self.config.save();
                    toasts.push("Virus database updated".to_string());
                    // 更新完病毒库顺手扫一遍：新装的定义正是最该用的时候。
                    if self.config.auto_scan_after_update {
                        self.start_auto_scan(AutoScanKind::AfterUpdate, &mut toasts);
                    }
                }
                Ok(UpdateOutcome::AlreadyUpToDate) => {
                    self.virus_db.refresh_db_version(ctx.clone());
                    self.config.last_update_check = Some(Timestamp::now());
                    self.config.save();
                    toasts.push("Virus database already up to date".to_string());
                }
                Err(e) => toasts.push(format!("Database update failed: {e}")),
            }
        }
        // 开机扫描 / 定时扫描：到点了就发起一次（已有扫描在跑时跳过，等下一轮）。
        if let Some(kind) = self.auto_scan.poll(&self.config) {
            self.start_auto_scan(kind, &mut toasts);
        }
        toasts
    }

    /// 发起一次自动扫描。定时计划按用户选的"闪电 / 全盘"走，其余两种固定
    /// 闪电扫描（开机与更新后都是"快速确认一下"，不该占住机器几十分钟）。
    fn start_auto_scan(&mut self, kind: AutoScanKind, toasts: &mut Vec<String>) {
        if self.any_scan_running() {
            return;
        }
        let full = match kind {
            AutoScanKind::Scheduled => self
                .config
                .scheduled_scan
                .map(|s| s.scan_full)
                .unwrap_or(false),
            _ => false,
        };
        let removable = self.config.scan_removable_drives;
        if full {
            self.full.start(removable);
        } else {
            self.quick.start(removable);
        }
        toasts.push(format!("{} started", kind.label()));
    }
}
