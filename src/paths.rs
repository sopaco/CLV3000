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

// 这两个函数 Windows 与 macOS 真实引擎都会用到，mock 路径（Linux 等）用不到，
// 但对其它目标仍定义（返回 bare 名字），编译期不会报 never-used。

/// 内置便携版 ClamAV 里的 `clamscan` 可执行文件。macOS 上若随包内置了
/// `clamav/clamscan` 就用它，否则退回 PATH 上的 `clamscan`（如 Homebrew 安装）。
/// 在 Windows / macOS 真实路径里用到；其它目标（mock 引擎）不会引用，允许 dead_code。
#[allow(dead_code)]
pub fn clamscan_path() -> PathBuf {
    #[cfg(windows)]
    {
        clamav_dir().join("clamscan.exe")
    }
    #[cfg(target_os = "macos")]
    {
        let bundled = clamav_dir().join("clamscan");
        if bundled.is_file() {
            return bundled;
        }
        // 系统安装位置（手动装到 /usr/local/clamav，或 Homebrew 的等价前缀）：
        // 识别到就直接返回绝对路径，这样 App 不依赖用户把 bin 加进 PATH。
        let system = PathBuf::from("/usr/local/clamav/bin/clamscan");
        if system.is_file() {
            return system;
        }
        PathBuf::from("clamscan")
    }
    #[cfg(not(any(windows, target_os = "macos")))]
    {
        PathBuf::from("clamscan")
    }
}

/// 内置便携版 ClamAV 里的 `freshclam` 可执行文件。规则同 `clamscan_path`。
#[allow(dead_code)]
pub fn freshclam_path() -> PathBuf {
    #[cfg(windows)]
    {
        clamav_dir().join("freshclam.exe")
    }
    #[cfg(target_os = "macos")]
    {
        let bundled = clamav_dir().join("freshclam");
        if bundled.is_file() {
            return bundled;
        }
        let system = PathBuf::from("/usr/local/clamav/bin/freshclam");
        if system.is_file() {
            return system;
        }
        PathBuf::from("freshclam")
    }
    #[cfg(not(any(windows, target_os = "macos")))]
    {
        PathBuf::from("freshclam")
    }
}

/// 在 PATH 里查找某个命令是否存在（不 spawn 子进程，纯路径判断，更轻）。
/// 仅 macOS 真实路径用到。
#[cfg(target_os = "macos")]
fn command_exists(name: &str) -> bool {
    if let Ok(paths) = std::env::var("PATH") {
        for dir in std::env::split_paths(&paths) {
            if dir.join(name).is_file() {
                return true;
            }
        }
    }
    false
}

/// 病毒库目录：`<exe目录>\clamav\database\`
pub fn clamav_database_dir() -> PathBuf {
    clamav_dir().join("database")
}

/// 解析出"真实可用"的病毒库目录，供 `engine` 调 `clamscan` 时显式传 `--database=`。
///
/// 优先级（返回第一个「真实存在」的目录）：
/// 1. 随包内置的 `<exe>/clamav/database` —— 打包分发 ClamAV 时用（Windows/macOS 通用）。
/// 2. macOS 系统安装默认位置 `/usr/local/clamav/share/clamav` —— 手动安装 ClamAV 的默认库目录。
/// 3. macOS 用户级目录 `~/.clamav` —— 开发机免 sudo 方案（`freshclam.conf` 里指定的库目录）。
///
/// 这样无论用哪种分发/部署方式，App 都能自动找到已有的病毒库，而不必依赖
/// `clamscan` 的编译期默认目录（那在开发机上往往是空的、或 root 所有无权限）。
/// 都不存在时返回 `None`，此时 `engine` 不传 `--database=`，交给 clamscan 用其自身默认目录
/// （通常也扫不了，会被可用性/错误路径正常拦下）。
#[cfg(any(windows, target_os = "macos"))]
pub fn resolved_clamav_database_dir() -> Option<PathBuf> {
    let mut candidates: Vec<PathBuf> = Vec::new();
    // 1. 内置（所有平台通用，优先级最高）
    candidates.push(clamav_database_dir());
    #[cfg(target_os = "macos")]
    {
        // 2. 系统安装默认位置
        candidates.push(PathBuf::from("/usr/local/clamav/share/clamav"));
        // 3. 用户级目录
        if let Some(home) = directories::BaseDirs::new().map(|b| b.home_dir().to_path_buf()) {
            candidates.push(home.join(".clamav"));
        }
    }
    candidates.into_iter().find(|p| p.is_dir())
}

/// Windows / macOS 真实检查 `clamscan` 可执行文件（或 PATH 上的命令）是否存在；
/// 其它目标（mock 引擎路径）直接报"可用"，让病毒库页看起来是正常状态。
#[cfg(windows)]
pub fn clamscan_available() -> bool {
    clamscan_path().is_file()
}

#[cfg(target_os = "macos")]
pub fn clamscan_available() -> bool {
    let p = clamscan_path();
    p.is_file() || command_exists("clamscan")
}

#[cfg(not(any(windows, target_os = "macos")))]
pub fn clamscan_available() -> bool {
    true
}

/// 同 `clamscan_available`：Windows / macOS 真查文件，其它目标直接放行给 mock 用。
#[cfg(windows)]
pub fn freshclam_available() -> bool {
    freshclam_path().is_file()
}

#[cfg(target_os = "macos")]
pub fn freshclam_available() -> bool {
    let p = freshclam_path();
    p.is_file() || command_exists("freshclam")
}

#[cfg(not(any(windows, target_os = "macos")))]
pub fn freshclam_available() -> bool {
    true
}

/// `%APPDATA%\CLV3000\`（Windows）。不套公司/组织名子目录，直接落在 APPDATA 下的 CLV3000。
pub fn app_data_dir() -> PathBuf {
    directories::BaseDirs::new()
        .map(|b| b.config_dir().join("CLV3000"))
        .unwrap_or_else(|| exe_dir().join("config"))
}

pub fn config_file_path() -> PathBuf {
    app_data_dir().join("config.toml")
}

pub fn ensure_dir(dir: &Path) {
    let _ = std::fs::create_dir_all(dir);
}

/// 用系统文件管理器打开一个目录——病毒库页"打开所在文件夹"按钮用。跟双击
/// 桌面上的文件夹图标是同一个操作，不是执行任意程序，风险可控。
///
/// 用 `spawn` 而不是 `status`：Windows 的 `explorer.exe` 在成功打开窗口后
/// 经常还是返回非零退出码（这是它自己的老毛病，跟这次操作是否成功没关系），
/// 用 `status` 判断成功与否反而会把"明明打开了"误判成失败。
#[cfg(windows)]
pub fn open_in_file_explorer(dir: &Path) -> Result<(), String> {
    use std::os::windows::process::CommandExt;
    std::process::Command::new("explorer")
        .arg(dir)
        .creation_flags(0x0800_0000)
        .spawn()
        .map(|_| ())
        .map_err(|e| format!("Failed to open folder: {e}"))
}

/// macOS 开发机预览用：`open` 是 macOS 上等价的"用 Finder 打开这个目录"命令。
#[cfg(not(windows))]
pub fn open_in_file_explorer(dir: &Path) -> Result<(), String> {
    std::process::Command::new("open")
        .arg(dir)
        .spawn()
        .map(|_| ())
        .map_err(|e| format!("Failed to open folder: {e}"))
}
