//! 隔离区：把检出的威胁文件从原位置搬到应用私有的 `paths::quarantine_dir()`，
//! 不是删除——保留内容，配合 `config::QuarantineEntry` 记录原路径，让设置页能
//! 「还原」或「彻底删除」。这是本项目第一次真正对用户文件做写操作（此前"隔离"
//! 按钮只是占位 toast），所以每一步失败都要优雅报错、不能 panic：文件正被进程
//! 占用（Windows 上闪电扫描本来就是扫"正在运行进程加载的模块"，占用中删不掉是
//! 预期会遇到的真实场景）、跨盘搬移、原文件已被用户手动删掉等，都只是普通业务
//! 失败，交回 `Result` 让 UI 弹 toast。
//!
//! # 强制隔离（仅 Windows）
//!
//! 普通 `quarantine_file` 失败后（最常见原因是文件被进程占用），用户可选择
//! 「强制隔离」：杀掉占用该文件的进程，然后重试搬移。如果占用进程的权限高于
//! 当前后台（如系统服务进程），会通过 UAC 提权（`ShellExecuteExW` + `runas`）
//! 重新启动一个提权后的 CLV3000 子进程来完成杀进程 + 搬移操作。
//!
//! 提权子进程通过 `--force-quarantine <original> <dest>` 命令行参数触发，
//! 在 `main.rs` 中于单实例检查之前拦截，避免与已运行的实例冲突。

use crate::config::QuarantineEntry;
use crate::localtime::Timestamp;
use crate::paths;
use std::path::Path;

/// 把 `original` 移进隔离区，返回记录（写进 `AppConfig.quarantined` 由调用者负责）。
///
/// `stored_name` 用 blake3(原路径字符串 + 当前纳秒时间戳) 取前 16 位 hex + 固定
/// `.quarantined` 后缀——故意不保留原扩展名，避免用户在隔离目录里手滑双击、误跑
/// 起恶意程序；哈希输入带时间戳是为了同一个路径被反复隔离（比如威胁复发）时
/// 也不会撞名互相覆盖。
pub fn quarantine_file(original: &Path, virus_name: &str) -> Result<QuarantineEntry, String> {
    let dir = paths::quarantine_dir();
    paths::ensure_dir(&dir);

    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let seed = format!("{}|{}", original.display(), nanos);
    let hash = blake3::hash(seed.as_bytes()).to_hex();
    let stored_name = format!("{}.quarantined", &hash[..16]);
    let dest = dir.join(&stored_name);

    move_file(original, &dest)
        .map_err(|e| format!("Failed to quarantine {}: {e}", original.display()))?;

    Ok(QuarantineEntry {
        original_path: original.display().to_string(),
        virus_name: virus_name.to_string(),
        quarantined_at: Timestamp::now(),
        stored_name,
    })
}

/// 把隔离区里的文件挪回原路径。原目录必须还存在、原路径上不能已经有文件
/// （拒绝覆盖，不猜用户想不想覆盖）——都只是普通失败，返回 `Err` 由调用者弹 toast，
/// 隔离记录/文件本身在失败时保持不变，用户可以再试一次。
pub fn restore_file(entry: &QuarantineEntry) -> Result<(), String> {
    let stored = paths::quarantine_dir().join(&entry.stored_name);
    let original = Path::new(&entry.original_path);

    let Some(parent) = original.parent() else {
        return Err("Original path has no parent directory".to_string());
    };
    if !parent.is_dir() {
        return Err(format!(
            "Original folder no longer exists: {}",
            parent.display()
        ));
    }
    if original.exists() {
        return Err(format!(
            "A file already exists at the original location: {}",
            original.display()
        ));
    }

    move_file(&stored, original).map_err(|e| format!("Failed to restore file: {e}"))
}

/// 彻底删除隔离区里的文件（真删除，不可撤销）。调用者负责同时从
/// `AppConfig.quarantined` 里移除对应记录。
pub fn delete_permanently(entry: &QuarantineEntry) -> Result<(), String> {
    let stored = paths::quarantine_dir().join(&entry.stored_name);
    std::fs::remove_file(&stored).map_err(|e| format!("Failed to delete quarantined file: {e}"))
}

/// `fs::rename` 优先（同盘几乎零成本），失败（最常见是跨盘，比如恶意文件在 D 盘、
/// 隔离区在 C 盘的 APPDATA 下）时退化成 `copy` + `remove_file`；两者都失败就是
/// 真的失败（文件被占用/权限不足等），原样把系统错误信息交出去。
fn move_file(from: &Path, to: &Path) -> std::io::Result<()> {
    if std::fs::rename(from, to).is_ok() {
        return Ok(());
    }
    std::fs::copy(from, to)?;
    std::fs::remove_file(from).inspect_err(|_| {
        // 复制已经成功、原文件删不掉（比如权限问题）：把复制出来的那份也清掉，
        // 避免隔离区里留一个"看起来隔离成功了"但原文件其实还在原地的假象。
        let _ = std::fs::remove_file(to);
    })
}

// ── 强制隔离（仅 Windows）──────────────────────────────────────────────

#[cfg(windows)]
mod force {
    use super::move_file;
    use crate::config::QuarantineEntry;
    use crate::localtime::Timestamp;
    use crate::paths;
    use std::path::{Path, PathBuf};
    use std::time::Duration;
    use windows::core::{HSTRING, PCWSTR};
    use windows::Win32::Foundation::{CloseHandle, INVALID_HANDLE_VALUE};
    use windows::Win32::System::Diagnostics::ToolHelp::{
        CreateToolhelp32Snapshot, Module32FirstW, Module32NextW, Process32FirstW, Process32NextW,
        MODULEENTRY32W, PROCESSENTRY32W, TH32CS_SNAPMODULE, TH32CS_SNAPMODULE32,
        TH32CS_SNAPPROCESS,
    };
    use windows::Win32::System::Threading::{
        OpenProcess, TerminateProcess, WaitForSingleObject, PROCESS_TERMINATE,
    };
    use windows::Win32::UI::Shell::{ShellExecuteExW, SEE_MASK_NOCLOSEPROCESS, SHELLEXECUTEINFOW};

    /// 杀掉这些进程后蓝屏 / 系统崩溃的系统关键进程名（小写比较）。
    /// 不会终止这些进程——即使它们恰好加载了恶意 DLL，也只是跳过，让用户手动处理。
    const PROTECTED_NAMES: &[&str] = &[
        "smss.exe",
        "csrss.exe",
        "wininit.exe",
        "services.exe",
        "lsass.exe",
        "winlogon.exe",
        "fontdrvhost.exe",
    ];

    /// PID 0（System Idle Process）和 PID 4（System / Kernel）永远不能杀。
    const PROTECTED_PIDS: &[u32] = &[0, 4];

    /// 强制隔离入口：尝试杀掉占用 `original` 的进程，然后搬移文件。
    ///
    /// 如果杀进程时遇到权限不足（`ERROR_ACCESS_DENIED`），通过 UAC 提权重启
    /// 一个提权子进程来完成操作。提权子进程退出后检查目标文件是否就位。
    pub fn force_quarantine_file(
        original: &Path,
        virus_name: &str,
    ) -> Result<QuarantineEntry, String> {
        let dir = paths::quarantine_dir();
        paths::ensure_dir(&dir);

        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let seed = format!("{}|{}", original.display(), nanos);
        let hash = blake3::hash(seed.as_bytes()).to_hex();
        let stored_name = format!("{}.quarantined", &hash[..16]);
        let dest = dir.join(&stored_name);

        // 先尝试当前权限下杀进程 + 搬移。
        match try_kill_and_move(original, &dest) {
            Ok(()) => Ok(QuarantineEntry {
                original_path: original.display().to_string(),
                virus_name: virus_name.to_string(),
                quarantined_at: Timestamp::now(),
                stored_name,
            }),
            Err(KillMoveError::AccessDenied) => {
                // 权限不足 → UAC 提权重试。提权子进程做同样的 kill + move。
                run_elevated(original, &dest)?;
                if dest.exists() {
                    Ok(QuarantineEntry {
                        original_path: original.display().to_string(),
                        virus_name: virus_name.to_string(),
                        quarantined_at: Timestamp::now(),
                        stored_name,
                    })
                } else {
                    Err("Force quarantine failed after elevation".to_string())
                }
            }
            Err(KillMoveError::Other(e)) => Err(e),
        }
    }

    /// 提权子进程的入口：杀掉占用 `original` 的进程，搬移到 `dest`。
    /// 不再尝试提权（已经提权过了），失败直接返回错误。
    pub fn run_force_quarantine_helper(original: &Path, dest: &Path) -> Result<(), String> {
        try_kill_and_move(original, dest).map_err(|e| match e {
            KillMoveError::AccessDenied => {
                "Access denied even after elevation".to_string()
            }
            KillMoveError::Other(e) => e,
        })
    }

    enum KillMoveError {
        /// `OpenProcess(PROCESS_TERMINATE)` 返回 `ERROR_ACCESS_DENIED`——需要提权。
        AccessDenied,
        Other(String),
    }

    /// 杀掉所有加载了 `original` 的非系统进程，等待它们退出，然后搬移文件。
    fn try_kill_and_move(original: &Path, dest: &Path) -> Result<(), KillMoveError> {
        let holders = find_processes_holding(original);

        let mut access_denied = false;
        for (pid, name) in &holders {
            if is_protected(pid, name) {
                continue;
            }
            match kill_process(*pid) {
                Ok(()) => {}
                Err(KillError::AccessDenied) => access_denied = true,
                Err(KillError::Other(e)) => {
                    return Err(KillMoveError::Other(format!(
                        "Failed to stop process {name} (PID {pid}): {e}"
                    )))
                }
            }
        }

        // 给被杀的进程一点时间完全退出、释放文件句柄。
        std::thread::sleep(Duration::from_millis(200));

        match move_file(original, dest) {
            Ok(()) => Ok(()),
            Err(e) => {
                if access_denied {
                    Err(KillMoveError::AccessDenied)
                } else {
                    Err(KillMoveError::Other(format!(
                        "Failed to move file after stopping processes: {e}"
                    )))
                }
            }
        }
    }

    /// 枚举所有进程，找出哪些进程加载了 `file` 作为模块（含主 exe 本体）。
    /// 返回 `(pid, 进程名)` 列表。路径比较大小写不敏感（Windows 文件系统不区分）。
    fn find_processes_holding(file: &Path) -> Vec<(u32, String)> {
        let target_lower = file.to_string_lossy().to_lowercase();
        let mut result = Vec::new();

        let Ok(snapshot) = (unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) }) else {
            return result;
        };

        let mut entry = PROCESSENTRY32W {
            dwSize: std::mem::size_of::<PROCESSENTRY32W>() as u32,
            ..Default::default()
        };

        unsafe {
            if Process32FirstW(snapshot, &mut entry).is_ok() {
                loop {
                    let pid = entry.th32ProcessID;
                    let name = wide_to_string(&entry.szExeFile);
                    let exe_name = name
                        .rsplit('\\')
                        .next()
                        .unwrap_or(&name)
                        .to_lowercase();

                    // 检查进程的主 exe 路径是否匹配。
                    let main_match = name.to_lowercase() == target_lower;

                    // 检查进程加载的模块是否匹配。
                    let module_match = if main_match {
                        true
                    } else {
                        modules_contain(pid, &target_lower)
                    };

                    if module_match {
                        result.push((pid, exe_name));
                    }

                    if Process32NextW(snapshot, &mut entry).is_err() {
                        break;
                    }
                }
            }
            let _ = CloseHandle(snapshot);
        }

        result
    }

    /// 检查 `pid` 的模块列表中是否有路径匹配 `target_lower`（已小写化）的模块。
    fn modules_contain(pid: u32, target_lower: &str) -> bool {
        let flags = TH32CS_SNAPMODULE | TH32CS_SNAPMODULE32;
        let Ok(snapshot) = (unsafe { CreateToolhelp32Snapshot(flags, pid) }) else {
            return false;
        };

        let mut entry = MODULEENTRY32W {
            dwSize: std::mem::size_of::<MODULEENTRY32W>() as u32,
            ..Default::default()
        };

        let mut found = false;
        unsafe {
            if Module32FirstW(snapshot, &mut entry).is_ok() {
                loop {
                    if let Some(path) = wide_to_path(&entry.szExePath) {
                        if path.to_string_lossy().to_lowercase() == target_lower {
                            found = true;
                            break;
                        }
                    }
                    if Module32NextW(snapshot, &mut entry).is_err() {
                        break;
                    }
                }
            }
            let _ = CloseHandle(snapshot);
        }
        found
    }

    enum KillError {
        AccessDenied,
        Other(String),
    }

    /// `HRESULT_FROM_WIN32(ERROR_ACCESS_DENIED)` = `0x80070005`。
    const E_ACCESSDENIED: i32 = 0x80070005u32 as i32;

    fn kill_process(pid: u32) -> Result<(), KillError> {
        let handle = unsafe { OpenProcess(PROCESS_TERMINATE, false, pid) };
        match handle {
            Ok(h) => {
                let result = unsafe { TerminateProcess(h, 1) };
                let _ = unsafe { CloseHandle(h) };
                result.map_err(|e| {
                    if e.code().0 == E_ACCESSDENIED {
                        KillError::AccessDenied
                    } else {
                        KillError::Other(format!("{e}"))
                    }
                })
            }
            Err(e) => {
                if e.code().0 == E_ACCESSDENIED {
                    Err(KillError::AccessDenied)
                } else {
                    Err(KillError::Other(format!("{e}")))
                }
            }
        }
    }

    fn is_protected(pid: &u32, name: &str) -> bool {
        PROTECTED_PIDS.contains(pid) || PROTECTED_NAMES.contains(&name)
    }

    /// 通过 `ShellExecuteExW` + `runas` 启动提权子进程，等待其完成。
    fn run_elevated(original: &Path, dest: &Path) -> Result<(), String> {
        let exe = std::env::current_exe().map_err(|e| format!("Cannot find exe path: {e}"))?;
        let params = format!(
            "--force-quarantine \"{}\" \"{}\"",
            original.display(),
            dest.display()
        );

        let verb_h = HSTRING::from("runas");
        let exe_h = HSTRING::from(exe.as_os_str());
        let params_h = HSTRING::from(&params);

        let mut info = SHELLEXECUTEINFOW {
            cbSize: std::mem::size_of::<SHELLEXECUTEINFOW>() as u32,
            fMask: SEE_MASK_NOCLOSEPROCESS,
            lpVerb: PCWSTR(verb_h.as_ptr()),
            lpFile: PCWSTR(exe_h.as_ptr()),
            lpParameters: PCWSTR(params_h.as_ptr()),
            nShow: 0, // SW_HIDE
            ..Default::default()
        };

        let success = unsafe { ShellExecuteExW(&mut info) };
        if !success.is_ok() {
            return Err("Elevation request failed (user may have declined the UAC prompt)".to_string());
        }

        // ShellExecuteExW 成功 + SEE_MASK_NOCLOSEPROCESS → hProcess 是有效句柄。
        // 等待提权子进程完成（最多 60 秒，含 UAC 弹窗 + 实际操作），然后关闭句柄。
        if info.hProcess != INVALID_HANDLE_VALUE && !info.hProcess.is_invalid() {
            unsafe {
                let _ = WaitForSingleObject(info.hProcess, 60_000);
                let _ = CloseHandle(info.hProcess);
            }
        }

        Ok(())
    }

    fn wide_to_string(buf: &[u16]) -> String {
        let end = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
        String::from_utf16_lossy(&buf[..end])
    }

    fn wide_to_path(buf: &[u16]) -> Option<PathBuf> {
        let end = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
        if end == 0 {
            return None;
        }
        Some(PathBuf::from(String::from_utf16_lossy(&buf[..end])))
    }
}

#[cfg(windows)]
pub use force::{force_quarantine_file, run_force_quarantine_helper};
