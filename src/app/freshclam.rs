//! freshclam 子进程调用：病毒库页"手动更新病毒库"按钮背后的实现。
//!
//! 三份 `run_freshclam`（Windows / macOS / 其它开发机 mock）按 `#[cfg]` 互斥，
//! 逻辑跟 `scan/engine.rs` 里 clamscan 的平台分派是同一个思路：真实实现只在
//! Windows/macOS 编译，其它目标退化成一个不联网的假实现方便看 UI。

use super::core::UpdateOutcome;
use crate::paths;

#[cfg(windows)]
pub(super) fn run_freshclam() -> Result<UpdateOutcome, String> {
    use std::os::windows::process::CommandExt;
    use std::process::{Command, Stdio};

    let db_dir =
        paths::resolved_clamav_database_dir().unwrap_or_else(|| paths::clamav_database_dir());
    // 跑之前先记一份数据库目录签名，跑完再比对——freshclam 在"已是最新"时
    // 也返回退出码 0，光看退出码会把"没变化"误判成"更新成功"。
    let before = database_signature(&db_dir);

    let mut cmd = Command::new(paths::freshclam_path());
    cmd.arg(format!("--datadir={}", db_dir.display()))
        .stdout(Stdio::null())
        // 保留 stderr 管道：freshclam 的报错（配置文件缺失、连不上镜像源等）都走
        // stderr，失败时把它塞进错误提示，比只报退出码有用得多。
        .stderr(Stdio::piped())
        .creation_flags(0x0800_0000);

    match cmd.output() {
        Ok(out) if out.status.success() => {
            let after = database_signature(&db_dir);
            if after != before {
                Ok(UpdateOutcome::Updated)
            } else {
                Ok(UpdateOutcome::AlreadyUpToDate)
            }
        }
        Ok(out) => {
            let stderr = String::from_utf8_lossy(&out.stderr).trim().to_string();
            if stderr.is_empty() {
                Err(format!(
                    "Database update failed with exit code {}",
                    out.status
                ))
            } else {
                Err(format!(
                    "Database update failed (exit {}): {}",
                    out.status, stderr
                ))
            }
        }
        Err(e) => Err(format!("Failed to start freshclam: {e}")),
    }
}

/// 数据库目录签名：把所有签名文件（.cvd/.cld/.cud）的「文件名:大小:修改时间」
/// 拼成一段稳定字符串，用来判断 freshclam 跑完之后文件到底有没有变。
#[cfg(any(windows, target_os = "macos"))]
fn database_signature(dir: &std::path::Path) -> String {
    use std::collections::BTreeMap;
    let mut map: BTreeMap<String, (u64, std::time::SystemTime)> = BTreeMap::new();
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            let p = entry.path();
            let ext = p.extension().and_then(|e| e.to_str()).unwrap_or("");
            if matches!(ext, "cvd" | "cld" | "cud") {
                if let Ok(meta) = std::fs::metadata(&p) {
                    let mtime = meta.modified().unwrap_or(std::time::SystemTime::UNIX_EPOCH);
                    if let Some(name) = p.file_name().and_then(|n| n.to_str()) {
                        map.insert(name.to_string(), (meta.len(), mtime));
                    }
                }
            }
        }
    }
    let mut sig = String::new();
    for (name, (len, mtime)) in map {
        sig.push_str(&format!("{name}:{len}:{mtime:?}|"));
    }
    sig
}

/// macOS：真实调用 `freshclam` 更新病毒库，逻辑与 Windows 版一致（跑前/跑后比对
/// 数据库目录签名，区分"已更新"与"已是最新"），只是不需要 `creation_flags`。
#[cfg(target_os = "macos")]
pub(super) fn run_freshclam() -> Result<UpdateOutcome, String> {
    use std::process::{Command, Stdio};

    let db_dir =
        paths::resolved_clamav_database_dir().unwrap_or_else(|| paths::clamav_database_dir());
    // 跑之前先记一份数据库目录签名，跑完再比对——freshclam 在"已是最新"时
    // 也返回退出码 0，光看退出码会把"没变化"误判成"更新成功"。
    let before = database_signature(&db_dir);

    let mut cmd = Command::new(paths::freshclam_path());
    // macOS 上没有系统默认 freshclam.conf，必须显式指定，否则 freshclam 直接报
    // "Can't open/parse the config file" 退出。config 里已含 DatabaseDirectory，
    // 这里再补一个 --datadir（与 config 一致）兜底，确保写到 resolved 目录。
    if let Some(cfg) = paths::freshclam_config_path() {
        cmd.arg("--config-file").arg(cfg);
    }
    cmd.arg(format!("--datadir={}", db_dir.display()))
        // 保留 stdout / stderr 管道：freshclam 的更新进度走 stdout，报错（配置文件缺失、
        // 连不上镜像源等）走 stderr。失败时把它们写进调试日志，比只报退出码有用得多。
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    let mut spawned_args: Vec<String> = vec![paths::freshclam_path().display().to_string()];
    if let Some(cfg) = paths::freshclam_config_path() {
        spawned_args.push("--config-file".to_string());
        spawned_args.push(cfg.display().to_string());
    }
    spawned_args.push(format!("--datadir={}", db_dir.display()));

    // 先算出结果，再统一写调试日志（含 stdout/stderr/退出码），最后返回。
    // stderr_log 在 match 的两个分支里都会被赋值，故用延迟初始化声明。
    let mut stdout_log = String::new();
    let stderr_log: String;
    let result = match cmd.output() {
        Ok(out) => {
            stdout_log = String::from_utf8_lossy(&out.stdout).trim().to_string();
            stderr_log = String::from_utf8_lossy(&out.stderr).trim().to_string();
            if out.status.success() {
                let after = database_signature(&db_dir);
                if after != before {
                    Ok(UpdateOutcome::Updated)
                } else {
                    Ok(UpdateOutcome::AlreadyUpToDate)
                }
            } else if stderr_log.is_empty() {
                Err(format!(
                    "Database update failed with exit code {}",
                    out.status
                ))
            } else {
                Err(format!(
                    "Database update failed (exit {}): {}",
                    out.status, stderr_log
                ))
            }
        }
        Err(e) => {
            let msg = format!("Failed to start freshclam: {e}");
            stderr_log = msg.clone();
            Err(msg)
        }
    };
    debug_log_freshclam(&spawned_args, &result, &stdout_log, &stderr_log);
    result
}

/// 开发预览用（Linux 等）：不真的联网更新，睡一下模拟"正在更新"的等待感，然后报成功。
/// 用原子计数器在 `Updated` / `AlreadyUpToDate` 之间来回切，方便开发时预览两种
/// 提示文案。
#[cfg(not(any(windows, target_os = "macos")))]
pub(super) fn run_freshclam() -> Result<UpdateOutcome, String> {
    use std::sync::atomic::{AtomicUsize, Ordering};
    static TOGGLE: AtomicUsize = AtomicUsize::new(0);
    std::thread::sleep(std::time::Duration::from_millis(1200));
    if TOGGLE.fetch_add(1, Ordering::Relaxed) % 2 == 0 {
        Ok(UpdateOutcome::Updated)
    } else {
        Ok(UpdateOutcome::AlreadyUpToDate)
    }
}

/// macOS 调试用：把 freshclam 的命令行、退出码、stdout、stderr 追加写到
/// `/tmp/clv3000_freshclam.log`，方便在 GUI 子进程里复现失败时拿到完整现场
/// （GUI 跑的命令和终端里手敲的偶尔会因环境不同而出错，光看弹窗里的简短报错不够）。
#[cfg(target_os = "macos")]
fn debug_log_freshclam(
    args: &[String],
    result: &Result<UpdateOutcome, String>,
    stdout: &str,
    stderr: &str,
) {
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let line = format!(
        "[{stamp}] args={args:?}\n  result={result:?}\n  stdout={stdout}\n  stderr={stderr}\n"
    );
    let _ = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open("/tmp/clv3000_freshclam.log")
        .and_then(|mut f| std::io::Write::write_all(&mut f, line.as_bytes()));
}
