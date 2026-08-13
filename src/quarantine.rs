//! 隔离区：把检出的威胁文件从原位置搬到应用私有的 `paths::quarantine_dir()`，
//! 不是删除——保留内容，配合 `config::QuarantineEntry` 记录原路径，让设置页能
//! 「还原」或「彻底删除」。这是本项目第一次真正对用户文件做写操作（此前"隔离"
//! 按钮只是占位 toast），所以每一步失败都要优雅报错、不能 panic：文件正被进程
//! 占用（Windows 上闪电扫描本来就是扫"正在运行进程加载的模块"，占用中删不掉是
//! 预期会遇到的真实场景）、跨盘搬移、原文件已被用户手动删掉等，都只是普通业务
//! 失败，交回 `Result` 让 UI 弹 toast。

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
