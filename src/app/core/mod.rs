//! 跨帧保留的业务状态：页面路由、配置、扫描页、病毒库页、设置页。

mod scan_state;
mod settings_state;
mod virus_db;

pub(crate) use scan_state::{PendingForceQuarantine, ScanPageState, ScanPhase};
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

    pub(crate) fn any_scan_running(&self) -> bool {
        self.quick.is_running() || self.full.is_running()
    }

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
