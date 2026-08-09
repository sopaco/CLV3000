//! 查询内置 ClamAV 引擎与病毒库版本信息（供「关于」对话框展示）。

use crate::paths;
#[cfg(windows)]
use std::process::Command;

#[derive(Debug, Clone)]
pub struct ClamAvInfo {
    pub engine: String,
    pub database: String,
}

impl ClamAvInfo {
    pub fn gather() -> Self {
        Self {
            engine: query_engine_version(),
            database: query_database_version(),
        }
    }
}

fn query_engine_version() -> String {
    if !paths::clamscan_available() {
        return "Not found (clamscan.exe missing)".to_string();
    }
    match run_clamscan_version_flag() {
        Ok(text) => parse_engine_from_clamscan_v(&text).unwrap_or_else(|| first_line(&text)),
        Err(e) => format!("Unavailable ({e})"),
    }
}

fn query_database_version() -> String {
    if !paths::clamscan_available() {
        return summarize_database_files();
    }
    match run_clamscan_version_flag() {
        Ok(text) => parse_database_from_clamscan_v(&text).unwrap_or_else(summarize_database_files),
        Err(_) => summarize_database_files(),
    }
}

/// `clamscan -V` 典型输出：`ClamAV 1.0.6/28901/Mon Jan  1 12:00:00 2024`
#[cfg(windows)]
fn run_clamscan_version_flag() -> Result<String, String> {
    use std::os::windows::process::CommandExt;

    let output = Command::new(paths::clamscan_path())
        .arg("-V")
        .creation_flags(0x0800_0000)
        .output()
        .map_err(|e| e.to_string())?;
    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    if !stdout.is_empty() {
        Ok(stdout)
    } else if !stderr.is_empty() {
        Ok(stderr)
    } else if output.status.success() {
        Ok(String::new())
    } else {
        Err(format!("exit {}", output.status))
    }
}

#[cfg(not(windows))]
fn run_clamscan_version_flag() -> Result<String, String> {
    Ok("ClamAV 0.7.0 (dev preview)/10001/Thu Jan  1 00:00:00 2031".to_string())
}

fn parse_engine_from_clamscan_v(line: &str) -> Option<String> {
    let head = line.split('/').next()?.trim();
    head.strip_prefix("ClamAV")
        .map(|v| v.trim().to_string())
        .filter(|s| !s.is_empty())
        .or_else(|| Some(head.to_string()))
}

fn parse_database_from_clamscan_v(line: &str) -> Option<String> {
    let mut parts = line.split('/');
    let _engine = parts.next()?;
    let db_ver = parts.next()?.trim();
    let date = parts.next().map(str::trim).filter(|s| !s.is_empty());
    match date {
        Some(d) => Some(format!("{db_ver} ({d})")),
        None if !db_ver.is_empty() => Some(db_ver.to_string()),
        None => None,
    }
}

fn first_line(text: &str) -> String {
    text.lines().next().unwrap_or(text).trim().to_string()
}

/// 引擎不可用时，尽量从 `database\` 目录里的签名文件推断状态。
fn summarize_database_files() -> String {
    let dir = paths::clamav_database_dir();
    if !dir.is_dir() {
        return format!("Not found ({})", dir.display());
    }
    let mut names: Vec<String> = std::fs::read_dir(&dir)
        .ok()
        .into_iter()
        .flatten()
        .filter_map(|e| e.ok())
        .filter_map(|e| {
            let p = e.path();
            let ext = p.extension()?.to_str()?;
            if matches!(ext, "cvd" | "cld" | "cud") {
                p.file_name().map(|n| n.to_string_lossy().into_owned())
            } else {
                None
            }
        })
        .collect();
    if names.is_empty() {
        return format!("No signature files in {}", dir.display());
    }
    names.sort();
    names.join(", ")
}
