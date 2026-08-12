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
//! - 每次扫描结束 `save()` 都会把内存索引整体重写落盘（compact）；条目超 `MAX_ENTRIES` 时额外触发 LRU 淘汰。
//!
//! 存放位置：由 `paths::app_data_dir()` 决定——Windows 为
//! `%APPDATA%\CLV3000\scan_cache.tsv`，macOS 为
//! `~/Library/Application Support/CLV3000\scan_cache.tsv`（Linux 为假引擎占位目录）。
//!
//! 第二张表 `scan_cache_paths.tsv`（同目录）：路径 → 上次的 `(size, mtime, 内容
//! 哈希)`，命中时跳过重新读整个文件算哈希（见 `quick_hash`）。这是**启发式
//! 加速**而非安全保证——细节和权衡见 `quick_hash` 的文档注释，与 `authenticode.rs`
//! 的可信签名预筛是同一类取舍。

use std::collections::HashMap;
use std::fs::{File, OpenOptions};
use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

/// 缓存结果超过这个时长（秒）就当失效，重新扫一次。默认 30 天。
const TTL_SECS: i64 = 30 * 24 * 3600;
/// 缓存条目上限。超过后按最久未用淘汰，避免低端机内存被撑爆。
/// 估算：每条约 80~130 字节（哈希+结果+整型），20 万条约 16~26 MB。
const MAX_ENTRIES: usize = 200_000;
/// 触发淘汰后保留到这个量，留 20% 余量，避免每次插入都触发淘汰。
const TARGET_ENTRIES: usize = 160_000;
/// `path_index`（路径→哈希 快查表）的容量上限/回收目标，跟主索引用同样的量级：
/// 一次全盘扫描覆盖的可执行文件数通常同一个数量级，没必要单独调一套参数。
const MAX_PATH_ENTRIES: usize = 200_000;
const TARGET_PATH_ENTRIES: usize = 160_000;

/// 一条记录是否"新鲜"（dbrev 匹配当前病毒库版本 + 没超 TTL）——`ScanCache::lookup`
/// （会顺手刷新 `last_used`）和 `CacheSnapshot::lookup`（只读快照，不刷新）共用
/// 这同一份判定，避免这条安全相关的逻辑在两个地方各写一份、以后改 TTL/dbrev
/// 规则时漏改一处。
fn is_fresh(dbrev: u64, ts: i64, current_rev: u64) -> bool {
    dbrev == current_rev && now_secs() - ts <= TTL_SECS
}

/// `path_index` 落盘用的文件路径：跟主缓存文件同目录、固定文件名
/// `scan_cache_paths.tsv`。故意跟主缓存分开成两个文件，而不是塞进同一份
/// `scan_cache.tsv`——两张表的字段结构、失效规则（是否受 dbrev/TTL 约束）都不
/// 一样，混在一份文件里既要多加一列区分"这行是哪种记录"，`compact()` 重写时
/// 还要同时兼顾两种保留策略，不如分成两个独立、各自逻辑简单的文件。
fn path_index_file(main_path: &Path) -> PathBuf {
    main_path
        .parent()
        .map(|d| d.join("scan_cache_paths.tsv"))
        .unwrap_or_else(|| PathBuf::from("scan_cache_paths.tsv"))
}

struct Record {
    result: String, // "clean" 或病毒名
    ts: i64,        // 扫描时刻（unix 秒）
    dbrev: u64,     // 当时的病毒库版本号
    last_used: i64, // 最近一次被查询/写入的时刻，用于 LRU 淘汰
}

/// 路径级别的"上次算出来的内容哈希"记录，配合 `(size, mtime)` 判断文件自那时起
/// 是否可能变过。见 `ScanCache::quick_hash` 顶部的安全权衡说明。
struct PathStamp {
    size: u64,
    mtime_ns: i128,
    hash: String,
    last_used: i64,
}

/// `ScanCache` 在某一时刻的不可变只读快照——`snapshot()` 拷贝出当时的两张表
/// （不含 `last_used`，只读路径不需要它）。多个线程可以各自持有 `&CacheSnapshot`
/// 并发读取，不需要任何锁：这是并行预筛（`engine.rs` 的 `prescan`）刻意选择的
/// 设计，用来彻底避开一个真实踩过的坑——早期实现让多个 worker 线程共享
/// `Mutex<ScanCache>`、每个文件上锁 2~4 次，在这台 macOS 开发机上用 6 个
/// worker 线程实测直接卡死（`sample` 抓到全部 worker 都停在
/// `pthread_mutex_firstfit_lock_wait`，没有任何线程在推进——推测是高频次、
/// 短临界区、强竞争场景下触发了 pthread mutex 的优先级反转）。
///
/// 因此这里不共享任何可变状态：每个 worker 只读 `&CacheSnapshot`（`lookup`/
/// `quick_hash` 的判定逻辑跟 `ScanCache` 完全一致，只是不刷新 `last_used`——
/// 快照本来就是某一时刻的静态拷贝，晚一点点淘汰无所谓，不影响正确性），新学到
/// 的记录（新哈希、新的路径→哈希映射）由调用方本地攢起来，等所有 worker 都
/// 跑完之后单线程一次性写回真正的 `ScanCache`（见 `engine.rs` 的 `CacheWrite`）。
pub struct CacheSnapshot {
    /// hash → (result, dbrev, ts)
    index: HashMap<String, (String, u64, i64)>,
    /// path → (size, mtime_ns, hash)
    path_index: HashMap<String, (u64, i128, String)>,
    current_rev: u64,
}

impl CacheSnapshot {
    /// 跟 `ScanCache::lookup` 同样的 dbrev/TTL 判定（共用 `is_fresh`），只是
    /// 不可变、不刷新 `last_used`。
    pub fn lookup(&self, hash: &str) -> Option<String> {
        let (result, dbrev, ts) = self.index.get(hash)?;
        if !is_fresh(*dbrev, *ts, self.current_rev) {
            return None;
        }
        Some(result.clone())
    }

    /// 跟 `ScanCache::quick_hash` 同样的判定，见其安全权衡说明（`(size, mtime)`
    /// 匹配才命中）。
    pub fn quick_hash(&self, path_key: &str, size: u64, mtime_ns: i128) -> Option<String> {
        let (s, m, hash) = self.path_index.get(path_key)?;
        if *s != size || *m != mtime_ns {
            return None;
        }
        Some(hash.clone())
    }
}

pub struct ScanCache {
    path: PathBuf,
    index: HashMap<String, Record>,
    /// 路径 → 上次记录的 (size, mtime, 内容哈希)，命中时跳过整份文件的 blake3
    /// 读取。跟 `index`（内容哈希 → 扫描结果）是两张独立的表：这张表只加速"算
    /// 哈希"这一步，最终判定"clean/病毒"仍然要经过 `index` 那套 dbrev/TTL 校验，
    /// 不会绕过病毒库版本失效机制。
    path_index: HashMap<String, PathStamp>,
    current_rev: u64,
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

        let mut path_index: HashMap<String, PathStamp> = HashMap::new();
        if let Ok(f) = File::open(path_index_file(path)) {
            for line in BufReader::new(f).lines().flatten() {
                if line.is_empty() {
                    continue;
                }
                // path 放最后一列、用 splitn(5, ..) 切：前 4 列（hash/size/mtime_ns/
                // last_used）都是不含 tab 的数字/哈希，剩下的全部（哪怕真的含 tab）
                // 都归给 path，不会因为路径里偶然出现的字符切错列。
                let parts: Vec<&str> = line.splitn(5, '\t').collect();
                if parts.len() < 5 {
                    continue;
                }
                let Ok(size) = parts[1].parse::<u64>() else { continue };
                let Ok(mtime_ns) = parts[2].parse::<i128>() else { continue };
                let Ok(last_used) = parts[3].parse::<i64>() else { continue };
                path_index.insert(
                    parts[4].to_string(),
                    PathStamp { size, mtime_ns, hash: parts[0].to_string(), last_used },
                );
            }
        }
        if path_index.len() > TARGET_PATH_ENTRIES {
            let mut items: Vec<(i64, String)> = path_index
                .iter()
                .map(|(p, s)| (s.last_used, p.clone()))
                .collect();
            items.sort_by_key(|(t, _)| std::cmp::Reverse(*t));
            for (_, p) in items.into_iter().skip(TARGET_PATH_ENTRIES) {
                path_index.remove(&p);
            }
        }

        ScanCache {
            path: path.to_path_buf(),
            index,
            path_index,
            current_rev,
            disabled: false,
        }
    }

    /// 快速哈希：路径 + 当前 `(size, mtime)` 与上次记录的完全一致时，直接返回
    /// 上次算出的内容哈希，跳过重新读取整个文件——重复扫描（"Run Again"、
    /// 隔天再跑一次全盘扫描）里绝大多数文件都没变过，这一步能省掉最贵的
    /// 那部分开销（读盘 + blake3）。
    ///
    /// ⚠️ 安全权衡（务必读完再决定是否需要关掉）：这是**启发式加速**，跟
    /// `authenticode.rs` 的可信签名预筛属于同一类风险——如果攻击者能在保留
    /// 文件大小与修改时间的前提下替换文件内容（"timestomping"，需要对目标机器
    /// 有写权限并刻意构造），这里会误用旧内容的哈希，进而复用旧的"clean"判定，
    /// 让新内容躲开这一次 ClamAV 扫描。这与"可信签名预筛可被盗用证书绕过"是
    /// 同一等级的已知取舍，项目现有立场是接受它、靠定期全盘扫描 + 病毒库版本
    /// 失效兜底（换了库版本后 `index` 那张表照样会重新校验）。如果这个权衡对
    /// 你的场景不可接受，把调用点的 `quick_hash` 短路去掉、永远走 `file_hash`
    /// 即可完全禁用，不影响其它任何功能。
    ///
    /// 生产代码目前走的是只读、可多线程安全共享的 `CacheSnapshot::quick_hash`
    /// （见 `engine.rs` 的 `prescan`——并行预筛不能共享这个需要 `&mut self` 的
    /// 版本）；这个方法保留给单线程场景（未来的调用点）和下面的单元测试直接用，
    /// 两者判定逻辑完全一致，`allow(dead_code)` 只是因为当前没有非测试调用点。
    #[allow(dead_code)]
    pub fn quick_hash(&mut self, path_key: &str, size: u64, mtime_ns: i128) -> Option<String> {
        if self.disabled {
            return None;
        }
        let rec = self.path_index.get_mut(path_key)?;
        if rec.size != size || rec.mtime_ns != mtime_ns {
            return None;
        }
        rec.last_used = now_secs();
        Some(rec.hash.clone())
    }

    /// `quick_hash` 未命中、真的读了整个文件算出哈希之后，记下这次的
    /// `(size, mtime_ns) → hash`，供下次同一路径命中 `quick_hash`。
    pub fn remember_path_hash(&mut self, path_key: &str, size: u64, mtime_ns: i128, hash: &str) {
        if self.disabled {
            return;
        }
        let ts = now_secs();
        self.path_index.insert(
            path_key.to_string(),
            PathStamp { size, mtime_ns, hash: hash.to_string(), last_used: ts },
        );
        if self.path_index.len() > MAX_PATH_ENTRIES {
            let mut order: Vec<(i64, String)> = self
                .path_index
                .iter()
                .map(|(p, s)| (s.last_used, p.clone()))
                .collect();
            order.sort_by_key(|(t, _)| *t);
            let drop = order.len() - TARGET_PATH_ENTRIES;
            for (i, (_, p)) in order.into_iter().enumerate() {
                if i >= drop {
                    break;
                }
                self.path_index.remove(&p);
            }
        }
    }

    /// 拷贝出一份不可变只读快照，供多个线程并发只读共享（见 `CacheSnapshot`
    /// 的文档）。
    pub fn snapshot(&self) -> CacheSnapshot {
        CacheSnapshot {
            index: self
                .index
                .iter()
                .map(|(h, r)| (h.clone(), (r.result.clone(), r.dbrev, r.ts)))
                .collect(),
            path_index: self
                .path_index
                .iter()
                .map(|(p, s)| (p.clone(), (s.size, s.mtime_ns, s.hash.clone())))
                .collect(),
            current_rev: self.current_rev,
        }
    }

    /// 命中且未过期且病毒库版本一致 → 返回上次结果（"clean" 或病毒名），否则 None。
    /// 命中时顺手刷新 last_used，供 LRU 淘汰参考。
    ///
    /// 同 `quick_hash`：生产代码走的是只读的 `CacheSnapshot::lookup`，这个
    /// 需要 `&mut self` 的版本保留给单线程调用点和单元测试。
    #[allow(dead_code)]
    pub fn lookup(&mut self, hash: &str) -> Option<String> {
        if self.disabled {
            return None;
        }
        let now = now_secs();
        let rec = self.index.get_mut(hash)?;
        if !is_fresh(rec.dbrev, rec.ts, self.current_rev) {
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

    /// 收尾：把内存里的索引整体落盘（compact 跳过已过期条目、其余全量重写），
    /// 保证"文件基因缓存"能真正跨次复用。
    ///
    /// 注意：这里**无条件**重写，而不是只在"文件已 >32MB 或条目 >20 万"时才写——
    /// 旧逻辑下普通闪电/全盘扫描只碰几百~几千个文件，永远触不到压缩阈值，
    /// 导致 `save()` 直接 return、缓存只活在内存里、扫描线程结束后被丢弃，
    /// 加速特性彻底失效。每次扫描结束落一次盘（最多 20 万条、几十 MB）开销可忽略。
    pub fn save(&mut self) {
        if self.disabled {
            return;
        }
        let _ = self.compact();
        let _ = self.compact_path_index();
    }

    /// 压缩重写：只保留未过期条目，并把它们落盘（5 列：hash\tresult\tts\tdbrev\tlast_used）。
    /// 全部过期则清空缓存文件并释放内存。
    fn compact(&mut self) -> bool {
        let now = now_secs();
        let tmp = self.path.with_extension("compact.tmp");
        {
            // 关键：必须带 `.write(true)`，否则 `create`/`truncate` 会因"无写权限"直接失败、
            // 导致本条缓存永远写不进磁盘（旧代码就漏了这步，缓存从不存在任何落盘）。
            let mut f = match OpenOptions::new()
                .create(true)
                .write(true)
                .truncate(true)
                .open(&tmp)
            {
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
                return true;
            }
        }
        let _ = std::fs::rename(&tmp, &self.path);
        true
    }

    /// `path_index` 的落盘压缩：跟 `compact()` 是同一套思路，独立成一份文件
    /// （`path_index_file`），格式 `hash\tsize\tmtime_ns\tlast_used\t路径`——路径
    /// 放最后一列，即使路径本身含 tab 字符也不会切错前 4 列（读的时候
    /// `splitn(5, '\t')` 与此对应）。用 `last_used` 而非独立 TTL 判过期：一个
    /// 路径长期没在扫描里出现（用户没再扫到这个文件）就没必要继续占位置。
    fn compact_path_index(&mut self) -> bool {
        let now = now_secs();
        let dest = path_index_file(&self.path);
        let tmp = dest.with_extension("compact.tmp");
        {
            let mut f = match OpenOptions::new()
                .create(true)
                .write(true)
                .truncate(true)
                .open(&tmp)
            {
                Ok(f) => f,
                Err(_) => return false,
            };
            let mut kept = 0;
            for (p, s) in &self.path_index {
                if now - s.last_used > TTL_SECS {
                    continue;
                }
                let _ = writeln!(f, "{}\t{}\t{}\t{}\t{}", s.hash, s.size, s.mtime_ns, s.last_used, p);
                kept += 1;
            }
            let _ = f.flush();
            if kept == 0 {
                let _ = std::fs::remove_file(&tmp);
                let _ = std::fs::remove_file(&dest);
                self.path_index.clear();
                return true;
            }
        }
        let _ = std::fs::rename(&tmp, &dest);
        true
    }
}

/// 算某个 `Metadata` 修改时间的纳秒时间戳（自 UNIX_EPOCH），供 `quick_hash`/
/// `remember_path_hash` 的调用方使用——引擎那边本来就要 `metadata()` 一次判断
/// "文件是否过大"，这里复用同一次 `stat`，不为了拿 mtime 再多一次系统调用。
pub fn mtime_ns(meta: &std::fs::Metadata) -> i128 {
    meta.modified()
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map(|d| d.as_nanos() as i128)
        .unwrap_or(0)
}

/// 算病毒库目录的版本号：目录里每个 .cvd/.cld/.cud 的"文件名:大小:修改时间"排序后
/// 拼起来再 BLAKE3。任一库文件更新（freshclam）→ 版本号变 → 旧缓存条目自动失效。
///
/// 必须把 `.cud`（增量/自定义签名库）也纳入——`app.rs` 里 `database_signature`
/// 判断"freshclam 是否真的更新了库"时看的是 `.cvd/.cld/.cud` 三种后缀，这里如果
/// 只看两种，一次只更新了 `.cud` 的增量更新就不会改变这里算出的版本号，旧缓存
/// 条目（可能判定为"clean"）会继续被复用到 TTL 到期，新签名扫不出旧文件——两处
/// 判据必须完全一致，否则缓存失效这道安全兜底就出现了缝隙。
fn db_revision(db_dir: &Path) -> std::io::Result<u64> {
    let mut entries: Vec<(String, u64, u128)> = Vec::new();
    for e in std::fs::read_dir(db_dir)?.flatten() {
        let p = e.path();
        let is_db = p
            .extension()
            .and_then(|x| x.to_str())
            .map(|x| matches!(x.to_ascii_lowercase().as_str(), "cvd" | "cld" | "cud"))
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    /// 验证修复：insert 后 `save()` 必须真正把索引落盘，且重新打开能命中。
    /// 这正是之前坏掉的行为——旧 `save()` 只在文件 >32MB 或条目 >20 万时才写。
    #[test]
    fn save_persists_and_reload_hits() {
        let dir = std::env::temp_dir().join("clv3000_cache_test");
        let _ = fs::create_dir_all(&dir);
        let db = dir.join("db");
        let _ = fs::create_dir_all(&db);
        let path = dir.join("scan_cache.tsv");
        let _ = fs::remove_file(&path);

        // 首次：空缓存 → 插入 → 保存
        {
            let mut cache = ScanCache::open(&path, &db);
            assert!(!path.exists(), "保存前不应有缓存文件");
            cache.insert("deadbeef", "clean");
            cache.insert("cafebabe", "Win.Test.EICAR_HDB-1");
            cache.save();
        }
        assert!(path.exists(), "save() 后必须生成 scan_cache.tsv");

        // 二次打开：应能命中上次结果（同一病毒库目录 → 同 dbrev）
        let mut cache2 = ScanCache::open(&path, &db);
        assert_eq!(cache2.lookup("deadbeef"), Some("clean".to_string()));
        assert_eq!(
            cache2.lookup("cafebabe"),
            Some("Win.Test.EICAR_HDB-1".to_string())
        );

        let _ = fs::remove_file(&path);
        let _ = fs::remove_file(path_index_file(&path));
        let _ = fs::remove_dir_all(&dir);
    }

    /// `quick_hash`：(size, mtime) 匹配才命中；任一变了都必须 miss（否则会拿着
    /// 旧内容的哈希去查 `index`，把"文件已经变了"误判成"还是老样子"）。
    /// 落盘后重新 `open` 也要能命中——这是 `save()` 里 `compact_path_index` 要
    /// 保证的事。
    #[test]
    fn quick_hash_matches_size_and_mtime_and_persists() {
        let dir = std::env::temp_dir().join("clv3000_cache_test_pathidx");
        let _ = fs::create_dir_all(&dir);
        let db = dir.join("db");
        let _ = fs::create_dir_all(&db);
        let path = dir.join("scan_cache.tsv");
        let _ = fs::remove_file(&path);
        let _ = fs::remove_file(path_index_file(&path));

        {
            let mut cache = ScanCache::open(&path, &db);
            assert_eq!(cache.quick_hash("/fake/a.exe", 100, 111), None, "从没记过，必须 miss");
            cache.remember_path_hash("/fake/a.exe", 100, 111, "deadbeef");
            assert_eq!(
                cache.quick_hash("/fake/a.exe", 100, 111),
                Some("deadbeef".to_string()),
                "size/mtime 都对得上，必须命中"
            );
            assert_eq!(
                cache.quick_hash("/fake/a.exe", 101, 111),
                None,
                "size 变了，必须 miss——否则会误用旧内容的哈希"
            );
            assert_eq!(
                cache.quick_hash("/fake/a.exe", 100, 222),
                None,
                "mtime 变了，必须 miss"
            );
            cache.save();
        }

        // 重新打开：落盘的 path_index 应该还能命中。
        let mut cache2 = ScanCache::open(&path, &db);
        assert_eq!(
            cache2.quick_hash("/fake/a.exe", 100, 111),
            Some("deadbeef".to_string()),
            "save() 后重新 open 必须还能命中"
        );

        let _ = fs::remove_file(&path);
        let _ = fs::remove_file(path_index_file(&path));
        let _ = fs::remove_dir_all(&dir);
    }
}
