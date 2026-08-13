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

/// 隔离区里的一条记录：原始路径 + 病毒名 + 隔离时间 + 隔离区里实际的文件名
/// （`stored_name`，不含目录，配合 `paths::quarantine_dir()` 定位真实文件，见 `quarantine.rs`）。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct QuarantineEntry {
    pub original_path: String,
    pub virus_name: String,
    pub quarantined_at: Timestamp,
    pub stored_name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AppConfig {
    pub last_quick_scan: Option<ScanRecord>,
    pub last_full_scan: Option<ScanRecord>,
    #[serde(default)]
    pub ignored: Vec<IgnoredEntry>,
    /// 已隔离的威胁文件记录，设置页「Quarantine」tab 用来渲染列表 + 支持还原/删除。
    #[serde(default)]
    pub quarantined: Vec<QuarantineEntry>,
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

    /// 设置页「Ignored」列表的"移除"按钮用：把某条忽略记录删掉，下次扫描到同一
    /// 文件路径 + 同一病毒名会重新报出来。
    pub fn remove_ignored(&mut self, path: &str, virus_name: &str) {
        self.ignored
            .retain(|e| !(e.path == path && e.virus_name == virus_name));
        self.save();
    }

    pub fn add_quarantined(&mut self, entry: QuarantineEntry) {
        self.quarantined.push(entry);
        self.save();
    }

    /// 按 `stored_name`（隔离区里的实际文件名，唯一）移除一条隔离记录并落盘；
    /// 返回被移除的记录，供调用者（还原/彻底删除）拿到 `original_path` 等字段。
    pub fn remove_quarantined(&mut self, stored_name: &str) -> Option<QuarantineEntry> {
        let idx = self
            .quarantined
            .iter()
            .position(|e| e.stored_name == stored_name)?;
        let entry = self.quarantined.remove(idx);
        self.save();
        Some(entry)
    }
}
