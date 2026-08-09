//! 统一定位内置 ClamAV 目录、%APPDATA% 配置目录等路径。
//! 所有路径都以"exe 所在目录"为基准，方便绿色版/安装版共用同一套代码。

use std::path::{Path, PathBuf};

/// 可执行文件所在目录。
pub fn exe_dir() -> PathBuf {
    std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|p| p.to_path_buf()))
        .unwrap_or_else(|| PathBuf::from("."))
}

/// 内置便携版 ClamAV 所在目录：`<exe目录>\clamav\`
pub fn clamav_dir() -> PathBuf {
    exe_dir().join("clamav")
}

// 这两个只在 Windows 真实代码路径里用到（engine.rs / app.rs 的 run_freshclam），
// 非 Windows mock 路径不需要，允许 dead_code 而不是特地给它们也分平台。
#[allow(dead_code)]
pub fn clamscan_path() -> PathBuf {
    clamav_dir().join("clamscan.exe")
}

#[allow(dead_code)]
pub fn freshclam_path() -> PathBuf {
    clamav_dir().join("freshclam.exe")
}

/// 病毒库目录：`<exe目录>\clamav\database\`
pub fn clamav_database_dir() -> PathBuf {
    clamav_dir().join("database")
}

/// Windows 下真的检查 `clamscan.exe` 是否存在；非 Windows（macOS/Linux 开发机预览）
/// 反正走的是 mock 引擎，不依赖这个文件，直接报"可用"，让病毒库页看起来是正常状态，
/// 不用特地摆一个假的 clamscan.exe 才能看 UI。
#[cfg(windows)]
pub fn clamscan_available() -> bool {
    clamscan_path().is_file()
}

#[cfg(not(windows))]
pub fn clamscan_available() -> bool {
    true
}

/// 同 `clamscan_available`：Windows 下真查文件，非 Windows 直接放行给 mock 用。
#[cfg(windows)]
pub fn freshclam_available() -> bool {
    freshclam_path().is_file()
}

#[cfg(not(windows))]
pub fn freshclam_available() -> bool {
    true
}

/// `%APPDATA%\CLV3000\`
pub fn app_data_dir() -> PathBuf {
    directories::ProjectDirs::from("com", "hytechc", "CLV3000")
        .map(|p| p.config_dir().to_path_buf())
        .unwrap_or_else(|| exe_dir().join("config"))
}

pub fn config_file_path() -> PathBuf {
    app_data_dir().join("config.toml")
}

pub fn ensure_dir(dir: &Path) {
    let _ = std::fs::create_dir_all(dir);
}
