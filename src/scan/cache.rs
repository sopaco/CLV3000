//! 文件基因扫描缓存：用文件内容的 BLAKE3 哈希作为"基因"，
//! 把"这个基因上次扫出来是干净 / 某个病毒名"记到磁盘，下次遇到相同内容直接复用结果，
//! 不浪费 ClamAV 的逐文件扫描时间。
//!
//! 关键安全约束：每条缓存都记了当时的病毒库版本号（db_revision）。
//! freshclam 更新病毒库后 db_revision 会变，旧条目一律按"未命中"处理，
//! 保证"新签名能揪出旧缓存判定为干净的文件"。再叠加一个 TTL 兜底。
//!
//! 容量控制（低端机友好）：
//! - 条目上限 `MAX_ENTRIES`，超过后按"最久未用（last_used）"淘汰，避免无限膨胀占满内存/磁盘。
//! - 每条记录带 TTL（默认 30 天），过期的在每次压缩落盘时物理删除；加载超大旧缓存时也会先裁剪。
//! - 缓存文件超过 `COMPACT_THRESHOLD` 或条目超上限时触发压缩重写。
//!
//! 存放位置：`%APPDATA%\CLV3000\scan_cache.tsv`（见 `paths::app_data_dir`）。

use std::collections::HashMap;
use std::fs::{File, OpenOptions};
use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

/// 缓存结果超过这个时长（秒）就当失效，重新扫一次。默认 30 天。
const TTL_SECS: i64 = 30 * 24 * 3600;
/// 缓存文件超过这个体积（字节）就在保存时压缩重写，避免无限膨胀。
const COMPACT_THRESHOLD: u64 = 32 * 1024 * 1024;
/// 缓存条目上限。超过后按最久未用淘汰，避免低端机内存被撑爆。
/// 估算：每条约 80~130 字节（哈希+结果+整型），20 万条约 16~26 MB。
const MAX_ENTRIES: usize = 200_000;
/// 触发淘汰后保留到这个量，留 20% 余量，避免每次插入都触发淘汰。
const TARGET_ENTRIES: usize = 160_000;

struct Record {
    result: String, // "clean" 或病毒名
    ts: i64,        // 扫描时刻（unix 秒）
    dbrev: u64,     // 当时的病毒库版本号
    last_used: i64, // 最近一次被查询/写入的时刻，用于 LRU 淘汰
}

pub struct ScanCache {
    path: PathBuf,
    index: HashMap<String, Record>,
    current_rev: u64,
    append: Option<File>,
    #[allow(dead_code)]
    disabled: bool,
}

impl ScanCache {
    /// 打开（或创建）缓存。db_dir 是 ClamAV 病毒库目录，用来算 db_revision。
    /// 任何 IO 错误都退化为空缓存——缓存只是加速，绝不能因为它坏了就阻断扫描。
    pub fn open(path: &Path, db_dir: &Path) -> Self {
        let current_rev = db_revision(db_dir).unwrap_or(0);
        let mut index: HashMap<String, Record> = HashMap::new();
        if let Ok(f) = File::open(path) {
            for line in BufReader::new(f).lines().flatten() {
                if line.is_empty() {
                    continue;
                }
                let parts: Vec<&str> = line.split('\t').collect();
                if parts.len() < 4 {
                    continue;
                }
                let Ok(ts) = parts[2].parse::<i64>() else { continue };
                let Ok(dbrev) = parts[3].parse::<u64>() else { continue };
                // 兼容旧缓存文件（4 列，无 last_used 列）：用 ts 兜底。
                let last_used = if parts.len() >= 5 {
                    parts[4].parse::<i64>().unwrap_or(ts)
                } else {
                    ts
                };
                index.insert(
                    parts[0].to_string(),
                    Record {
                        result: parts[1].to_string(),
                        ts,
                        dbrev,
                        last_used,
                    },
                );
            }
        }
        // 加载即剔除：过期(TTL)与病毒库版本不符(dbrev)的条目在 `lookup` 里永远命中不了，
        // 直接丢出内存——纯内存操作、零磁盘写，立刻降低低端机常驻内存。
        // 物理删除（重写 .tsv）仍留到下次 `compact` 落盘时做，不在这里增加启动磁盘 I/O。
        let now = now_secs();
        index.retain(|_, r| r.dbrev == current_rev && now - r.ts <= TTL_SECS);
        // 加载后先裁剪：历史版本没有容量上限，可能积累超大缓存，先按扫描时间保留最近的，
        // 避免把低端机内存一次性吃满。
        if index.len() > TARGET_ENTRIES {
            let mut items: Vec<(i64, String)> =
                index.iter().map(|(h, r)| (r.ts, h.clone())).collect();
            items.sort_by_key(|(t, _)| std::cmp::Reverse(*t));
            for (_, h) in items.into_iter().skip(TARGET_ENTRIES) {
                index.remove(&h);
            }
        }
        ScanCache { path: path.to_path_buf(), index, current_rev, append: None, disabled: false }
    }

    /// 命中且未过期且病毒库版本一致 → 返回上次结果（"clean" 或病毒名），否则 None。
    /// 命中时顺手刷新 last_used，供 LRU 淘汰参考。
    pub fn lookup(&mut self, hash: &str) -> Option<String> {
        if self.disabled {
            return None;
        }
        let now = now_secs();
        let rec = self.index.get_mut(hash)?;
        if rec.dbrev != self.current_rev {
            return None;
        }
        if now - rec.ts > TTL_SECS {
            return None;
        }
        rec.last_used = now;
        Some(rec.result.clone())
    }

    /// 记录一次扫描结果。失败静默忽略。插入后若超容量则触发 LRU 淘汰 + 压缩。
    pub fn insert(&mut self, hash: &str, result: &str) {
        if self.disabled {
            return;
        }
        let ts = now_secs();
        self.index.insert(
            hash.to_string(),
            Record { result: result.to_string(), ts, dbrev: self.current_rev, last_used: ts },
        );
        self.evict_if_needed();
    }

    /// 超过容量上限时：先丢过期条目，仍超限则按 last_used 升序丢最久未用的。
    fn evict_if_needed(&mut self) {
        if self.index.len() <= MAX_ENTRIES {
            return;
        }
        let now = now_secs();
        self.index.retain(|_, r| now - r.ts <= TTL_SECS);
        if self.index.len() > TARGET_ENTRIES {
            let mut order: Vec<(i64, String)> =
                self.index.iter().map(|(h, r)| (r.last_used, h.clone())).collect();
            order.sort_by_key(|(t, _)| *t);
            let drop = order.len() - TARGET_ENTRIES;
            for (i, (_, h)) in order.into_iter().enumerate() {
                if i >= drop {
                    break;
                }
                self.index.remove(&h);
            }
        }
        let _ = self.compact();
    }

    /// 收尾：关闭追加句柄；若文件过大或条目超上限则压缩重写（同时物理删除过期条目）。
    pub fn save(&mut self) {
        if self.disabled {
            return;
        }
        self.append = None; // drop → flush + close
        let over_size = std::fs::metadata(&self.path)
            .map(|m| m.len() > COMPACT_THRESHOLD)
            .unwrap_or(false);
        if over_size || self.index.len() > MAX_ENTRIES {
            let _ = self.compact();
        }
    }

    /// 压缩重写：只保留未过期条目，并把它们落盘（5 列：hash\tresult\tts\tdbrev\tlast_used）。
    /// 全部过期则清空缓存文件并释放内存。
    fn compact(&mut self) -> bool {
        let now = now_secs();
        let tmp = self.path.with_extension("compact.tmp");
        {
            let mut f = match OpenOptions::new().create(true).truncate(true).open(&tmp) {
                Ok(f) => f,
                Err(_) => return false,
            };
            let mut kept = 0;
            for (h, rec) in &self.index {
                if now - rec.ts > TTL_SECS {
                    continue;
                }
                let _ = writeln!(
                    f,
                    "{}\t{}\t{}\t{}\t{}",
                    h, rec.result, rec.ts, rec.dbrev, rec.last_used
                );
                kept += 1;
            }
            let _ = f.flush();
            if kept == 0 {
                // 全过期：清掉文件与内存，下次从空缓存开始。
                let _ = std::fs::remove_file(&tmp);
                let _ = std::fs::remove_file(&self.path);
                self.index.clear();
                self.append = None;
                return true;
            }
        }
        let _ = std::fs::rename(&tmp, &self.path);
        self.append = None;
        true
    }
}

/// 算病毒库目录的版本号：目录里每个 .cvd/.cld 的"文件名:大小:修改时间"排序后拼起来再 BLAKE3。
/// 任一库文件更新（freshclam）→ 版本号变 → 旧缓存条目自动失效。
fn db_revision(db_dir: &Path) -> std::io::Result<u64> {
    let mut entries: Vec<(String, u64, u128)> = Vec::new();
    for e in std::fs::read_dir(db_dir)?.flatten() {
        let p = e.path();
        let is_db = p
            .extension()
            .and_then(|x| x.to_str())
            .map(|x| x.eq_ignore_ascii_case("cvd") || x.eq_ignore_ascii_case("cld"))
            .unwrap_or(false);
        if !is_db {
            continue;
        }
        let m = e.metadata()?;
        let mtime_ns = m
            .modified()?
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let name = p.file_name().unwrap_or_default().to_string_lossy().to_string();
        entries.push((name, m.len(), mtime_ns));
    }
    entries.sort();
    let mut s = String::new();
    for (name, len, mtime) in entries {
        s.push_str(&name);
        s.push(':');
        s.push_str(&len.to_string());
        s.push(':');
        s.push_str(&mtime.to_string());
        s.push('\n');
    }
    let mut hasher = blake3::Hasher::new();
    hasher.update(s.as_bytes());
    let h = hasher.finalize();
    let mut bytes = [0u8; 8];
    bytes.copy_from_slice(&h.as_bytes()[..8]);
    Ok(u64::from_le_bytes(bytes))
}

/// 计算文件内容的 BLAKE3 哈希（十六进制）。读不了（被锁/无权限）返回 None，
/// 调用方应退回"交给 ClamAV 扫"。
pub fn file_hash(path: &Path) -> Option<String> {
    let file = std::fs::File::open(path).ok()?;
    let mut reader = BufReader::with_capacity(256 * 1024, file);
    let mut hasher = blake3::Hasher::new();
    let mut buf = [0u8; 64 * 1024];
    loop {
        match reader.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => {
                hasher.update(&buf[..n]);
            }
            Err(_) => return None,
        }
    }
    Some(hasher.finalize().to_hex().to_string())
}

fn now_secs() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}
