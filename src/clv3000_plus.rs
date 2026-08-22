//! 托盘菜单「Optimize PC」：尝试启动 CLV3000 Plus，失败则打开 GitHub Releases 页。

use std::path::{Path, PathBuf};

const RELEASES_URL: &str = "https://github.com/sopaco/CLV3000-Plus/releases";

#[cfg(windows)]
fn plus_install_path() -> PathBuf {
    crate::paths::exe_dir().join("clv3000-plus.exe")
}

#[cfg(target_os = "macos")]
fn plus_install_path() -> PathBuf {
    PathBuf::from("/Applications/CLV3000 Plus.app")
}

#[cfg(not(any(windows, target_os = "macos")))]
fn plus_install_path() -> PathBuf {
    PathBuf::from("clv3000-plus")
}

/// 托盘「Optimize PC」入口：能启动 Plus 就启动，否则用默认浏览器打开 Releases。
pub fn launch_or_open_releases() {
    if try_launch_plus() {
        return;
    }
    let _ = open_releases_page();
}

fn try_launch_plus() -> bool {
    let path = plus_install_path();
    if !path.exists() {
        return false;
    }
    launch_plus_at(&path)
}

#[cfg(windows)]
fn launch_plus_at(path: &Path) -> bool {
    use std::os::windows::process::CommandExt;
    std::process::Command::new(path)
        .current_dir(
            path.parent()
                .unwrap_or_else(|| Path::new(".")),
        )
        .creation_flags(0x0800_0000)
        .spawn()
        .is_ok()
}

#[cfg(target_os = "macos")]
fn launch_plus_at(path: &Path) -> bool {
    std::process::Command::new("open")
        .arg(path)
        .spawn()
        .is_ok()
}

#[cfg(not(any(windows, target_os = "macos")))]
fn launch_plus_at(_path: &Path) -> bool {
    false
}

fn open_releases_page() -> Result<(), String> {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        std::process::Command::new("cmd")
            .args(["/C", "start", "", RELEASES_URL])
            .creation_flags(0x0800_0000)
            .spawn()
            .map(|_| ())
            .map_err(|e| format!("Failed to open browser: {e}"))
    }
    #[cfg(target_os = "macos")]
    {
        std::process::Command::new("open")
            .arg(RELEASES_URL)
            .spawn()
            .map(|_| ())
            .map_err(|e| format!("Failed to open browser: {e}"))
    }
    #[cfg(not(any(windows, target_os = "macos")))]
    {
        std::process::Command::new("xdg-open")
            .arg(RELEASES_URL)
            .spawn()
            .map(|_| ())
            .map_err(|e| format!("Failed to open browser: {e}"))
    }
}
