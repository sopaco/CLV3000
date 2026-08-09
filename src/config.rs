//! 持久化配置：上次扫描摘要、已忽略的威胁条目。
//! 存放在 `%APPDATA%\CLV3000\config.toml`。

use crate::localtime::Timestamp;
use crate::paths;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScanRecord {
    pub time: Timestamp,
    pub threats_found: usize,
    pub scanned_count: usize,
}

/// 用户点了"忽略"的威胁条目：同一个文件路径 + 同一个病毒名下次扫描不再打扰。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct IgnoredEntry {
    pub path: String,
    pub virus_name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AppConfig {
    pub last_quick_scan: Option<ScanRecord>,
    pub last_full_scan: Option<ScanRecord>,
    #[serde(default)]
    pub ignored: Vec<IgnoredEntry>,
    /// 全盘扫描是否包含可移动盘（U盘等），默认不包含，避免扫描时间不可控。
    #[serde(default)]
    pub scan_removable_drives: bool,
}

impl AppConfig {
    pub fn load() -> Self {
        let path = paths::config_file_path();
        match std::fs::read_to_string(&path) {
            Ok(text) => toml::from_str(&text).unwrap_or_default(),
            Err(_) => Self::default(),
        }
    }

    pub fn save(&self) {
        let dir = paths::app_data_dir();
        paths::ensure_dir(&dir);
        if let Ok(text) = toml::to_string_pretty(self) {
            let _ = std::fs::write(paths::config_file_path(), text);
        }
    }

    pub fn is_ignored(&self, path: &str, virus_name: &str) -> bool {
        self.ignored
            .iter()
            .any(|e| e.path == path && e.virus_name == virus_name)
    }

    pub fn add_ignored(&mut self, path: String, virus_name: String) {
        let entry = IgnoredEntry { path, virus_name };
        if !self.ignored.contains(&entry) {
            self.ignored.push(entry);
        }
        self.save();
    }
}
