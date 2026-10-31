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

/// 数量加千分位：128450 → "128,450"。全盘扫描动辄几十万文件，不加分隔符
/// 一眼看不出量级（"128450 files" 很容易读成 "12845"）。
pub fn format_count(n: usize) -> String {
    let s = n.to_string();
    let mut out = String::with_capacity(s.len() + s.len() / 3);
    for (i, ch) in s.chars().enumerate() {
        if i > 0 && (s.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(ch);
    }
    out
}

/// 定时扫描计划：每周几（位掩码，bit0=周一 … bit6=周日）+ 每天固定时刻。
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub struct ScheduleConfig {
    /// 生效的星期几，bit i 表示"周一 + i"生效。
    pub days_mask: u8,
    /// 触发时刻（0-23 时）。
    pub hour: u8,
    /// 触发时刻（0-59 分）。
    pub minute: u8,
    /// 扫全盘（false = 闪电扫描）。
    pub scan_full: bool,
}

impl Default for ScheduleConfig {
    fn default() -> Self {
        Self {
            days_mask: 1 << 0, // 默认周一
            hour: 9,
            minute: 0,
            scan_full: false,
        }
    }
}

impl ScheduleConfig {
    /// 某一天（0=周一）是否在计划内。
    pub fn includes_day(&self, weekday: u8) -> bool {
        self.days_mask & (1 << (weekday % 7)) != 0
    }

    /// 计划是否有效：至少选了一天，且时刻在合法范围内。
    pub fn is_valid(&self) -> bool {
        self.days_mask != 0 && self.hour < 24 && self.minute < 60
    }
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

#[derive(Debug, Clone, Serialize, Deserialize)]
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
    /// 是否自动检查病毒库更新（默认开）。关闭后只能手动点「Update Database」。
    #[serde(default = "default_true")]
    pub auto_update_definitions: bool,
    /// 上次自动/手动检查病毒库更新的时间，用于界面展示"上次检查"。
    #[serde(default)]
    pub last_update_check: Option<Timestamp>,
    /// 开机时自动跑一次闪电扫描（Boot Scan）。
    #[serde(default)]
    pub auto_scan_on_boot: bool,
    /// 定时扫描计划（按周内天 + 固定时刻）。`None` = 不启用。
    #[serde(default)]
    pub scheduled_scan: Option<ScheduleConfig>,
    /// 病毒库更新完成后自动跑一次扫描。
    #[serde(default)]
    pub auto_scan_after_update: bool,
}

fn default_true() -> bool {
    true
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            last_quick_scan: None,
            last_full_scan: None,
            ignored: Vec::new(),
            quarantined: Vec::new(),
            scan_removable_drives: false,
            // 自动更新默认开：病毒库过期是安全风险，不该让用户自己记得点。
            auto_update_definitions: true,
            last_update_check: None,
            // 三个自动扫描开关默认全关：开机就扫、定时扫都是"用户不在场时
            // 发生的事"，必须由用户明确开启。
            auto_scan_on_boot: false,
            scheduled_scan: None,
            auto_scan_after_update: false,
        }
    }
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
