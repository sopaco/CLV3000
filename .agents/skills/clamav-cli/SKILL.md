---
name: "clamav-cli"
description: "ClamAV CLI tools usage guide (clamscan/freshclam/clamd/clamdscan/sigtool). Invoke when integrating ClamAV scanning, updating virus DB, parsing clamscan output, or debugging scan engine issues."
---

# ClamAV CLI 使用指南

基于 ClamAV 1.5.4（Windows便携版）实际调研，覆盖 `clamscan`、`freshclam`、`clamd`、`clamdscan`、`sigtool` 等工具的核心用法、关键参数、输出格式和常见坑。

## 工具总览

| 工具 | 用途 | 是否需要守护进程 |
|------|------|------------------|
| `clamscan` | 一次性扫描，每次启动都重新加载病毒库 | 否 |
| `clamd` | 守护进程，加载一次病毒库常驻内存 | — |
| `clamdscan` | clamd 客户端，把扫描任务发给 clamd | **是**（需要 clamd 运行） |
| `freshclam` | 更新病毒库 | 否 |
| `sigtool` | 签名数据库工具（查看/构建/验证 CVD） | 否 |
| `clamconf` | 配置查看与生成示例配置 | 否 |
| `clambc` | 字节码签名测试工具 | 否 |
| `clamsubmit` | 提交恶意样本/误报报告 | 否 |
| `clamdtop` | clamd 实例监控（ncurses TUI） | **是**（需要 clamd 运行） |

---

## clamscan — 一次性扫描

### 基本语法

```
clamscan [options] [file/directory/-]
```

- 传 `-` 作为文件名可从 stdin 扫描**单个文件内容**（不是文件列表）
- 每次运行都会重新加载病毒库（耗时 10~30 秒），不适合高频调用
- 不需要 clamd 守护进程

### 关键参数

#### 输出控制

| 参数 | 说明 |
|------|------|
| `--verbose` / `-v` | 显示每个文件的 `Scanning <path>` 进度行 |
| `--stdout` | **关键**：结果写到 stdout 而非默认的 stderr |
| `--no-summary` | 禁止末尾打印统计摘要 |
| `--infected` / `-i` | 只打印感染文件（跳过 OK 的） |
| `--quiet` | 只输出错误 |
| `--bell` | 检测到病毒时响铃 |

#### 扫描目标

| 参数 | 说明 |
|------|------|
| `--recursive` / `-r` | 递归扫描子目录 |
| `--file-list=FILE` / `-f FILE` | 从文件读取待扫描路径列表（每行一个） |
| `--database=FILE/DIR` / `-d` | 指定病毒库文件或目录 |
| `--cross-fs[=yes/no]` | 是否跨文件系统扫描（默认 yes） |
| `--follow-dir-symlinks=0/1/2` | 跟随目录符号链接 |
| `--follow-file-symlinks=0/1/2` | 跟随文件符号链接 |
| `--exclude=REGEX` | 排除匹配的文件名 |
| `--include=REGEX` | 只扫描匹配的文件名 |

#### 文件类型与扫描选项

| 参数 | 说明 |
|------|------|
| `--scan-pe[=yes/no]` | 扫描 PE 文件（默认 yes） |
| `--scan-elf[=yes/no]` | 扫描 ELF 文件（默认 yes） |
| `--scan-archive[=yes/no]` | 扫描压缩包（默认 yes） |
| `--scan-mail[=yes/no]` | 扫描邮件文件（默认 yes） |
| `--scan-pdf[=yes/no]` | 扫描 PDF（默认 yes） |
| `--scan-html[=yes/no]` | 扫描 HTML（默认 yes） |
| `--bytecode[=yes/no]` | 加载字节码签名（默认 yes） |
| `--detect-pua[=yes/no]` | 检测 PUA（潜在不需要的应用） |

#### 大小与超时限制

| 参数 | 说明 |
|------|------|
| `--max-filesize=#n` | 超过此大小的文件跳过（视为干净） |
| `--max-scansize=#n` | 容器文件最大扫描数据量 |
| `--max-files=#n` | 容器文件内最大扫描文件数 |
| `--max-recursion=#n` | 最大归档递归层级 |
| `--max-scantime=#n` | 单文件最大扫描时间（毫秒），超时视为干净 |

#### 处置动作

| 参数 | 说明 |
|------|------|
| `--move=DIRECTORY` | 移动感染文件到指定目录 |
| `--copy=DIRECTORY` | 复制感染文件到指定目录 |
| `--remove[=yes/no]` | **删除**感染文件（危险！） |

#### 进程内存扫描

| 参数 | 说明 |
|------|------|
| `--memory` | 扫描运行中进程的内存模块（需管理员权限） |
| `--kill` | 杀死/卸载感染进程的模块 |
| `--unload` | 从进程卸载感染模块 |

### 输出格式

使用 `--verbose --stdout` 时，stdout 每个文件输出两行：

```
Scanning C:\path\to\file.exe           ← -v 的进度提示行
C:\path\to\file.exe: OK                ← 结果行
```

感染时：

```
Scanning C:\path\to\file.exe
C:\path\to\file.exe: Win.Test.EICAR_HDB-1 FOUND
```

无法访问时：

```
Scanning C:\path\to\file.exe
C:\path\to\file.exe: No such file or directory ERROR
```

**解析方法**：用 `line.rsplit_once(": ")` 从右分割。Windows 路径 `C:\` 中的冒号后跟 `\` 不是 `: `（冒号+空格），不会误分割。`Scanning <path>` 行不含 `: `，会被自然过滤。

### 退出码

| 退出码 | 含义 |
|--------|------|
| 0 | 未发现病毒 |
| 1 | 发现病毒 |
| 2 | 发生错误（参数错误、数据库损坏等） |
| 40+ | 各种特定错误（数据库过旧、文件不足等） |

### 从文件列表扫描

```
clamscan --file-list=paths.txt --stdout -v --no-summary --database=<db_dir>
```

**重要**：
- `--file-list=-`（stdin）**在 ClamAV 1.5.x 不支持**，会报 `Can't open file -`。必须使用实际临时文件。
- 文件列表每行一个路径，UTF-8 编码（无 BOM），LF 或 CRLF 换行均可。
- 路径中的反斜杠 `\` 是 Windows 标准分隔符，无需转义。

### 性能特征

- **病毒库加载**：每次启动加载 `main.cvd` + `daily.cvd` + `bytecode.cvd`，耗时 10~30 秒。这是 `clamscan` 每次调用的固定开销。
- **单文件扫描**：加载完 DB 后，单个文件扫描通常很快（毫秒级）。
- **不适合高频调用**：如需频繁扫描，应使用 `clamd` 守护进程模式。

---

## clamd + clamdscan — 守护进程模式

### clamd 守护进程

`clamd` 是多线程守护进程，加载一次病毒库常驻内存，通过 socket 接收扫描命令。适合需要频繁扫描的场景。

```
clamd --config-file=clamd.conf
```

- 监听 Unix socket 或 TCP socket（由 `clamd.conf` 配置）
- **安全警告**：TCP socket 无认证，不要暴露到公网
- 信号处理：`SIGTERM` 退出、`SIGHUP` 重开日志、`SIGUSR2` 重载病毒库

### clamdscan 客户端

`clamdscan` 是 `clamd` 的客户端，把扫描任务发给运行中的 `clamd`：

```
clamdscan [options] [file/directory/-]
```

**需要 clamd 已在运行**。支持 `--fdpass`（传文件描述符）、`--stream`（传文件内容）、`--multiscan`（多线程扫描）等模式。

### clamd vs clamscan 选择

| 场景 | 推荐 |
|------|------|
| 一次性扫描、脚本调用 | `clamscan` |
| 频繁扫描、实时保护 | `clamd` + `clamdscan` |
| 需要避免每次加载 DB 的开销 | `clamd` |
| 简单部署、无需常驻 | `clamscan` |

---

## freshclam — 病毒库更新

### 基本用法

```
freshclam [options]
```

### 关键参数

| 参数 | 说明 |
|------|------|
| `--config-file=FILE` | 指定配置文件 |
| `--datadir=DIRECTORY` | 下载到指定目录（目录必须已存在） |
| `--log=FILE` / `-l FILE` | 日志文件 |
| `--daemon` / `-d` | 守护进程模式，定期检查更新 |
| `--checks=#n` / `-c #n` | 每天检查次数（1~50） |
| `--update-db=DBNAME` | 只更新指定数据库 |
| `--show-progress` | 显示下载进度 |
| `--stdout` | 输出到 stdout 而非 stderr |
| `--install-service` | 安装为 Windows 服务 |
| `--no-dns` | 强制使用非 DNS 验证方式 |

### 配置文件

默认读取 `freshclam.conf`，关键配置项：

```
DatabaseDirectory <path>        # 病毒库目录
UpdateLogFile <path>            # 日志文件
DatabaseMirror database.clamav.net  # 镜像服务器
Checks 24                       # 每天检查次数
```

---

## sigtool — 签名工具

### 常用命令

| 命令 | 说明 |
|------|------|
| `--info=FILE` / `-i FILE` | 查看 CVD 数据库信息 |
| `--list-sigs[=FILE]` / `-l` | 列出签名名称 |
| `--find-sigs=REGEX` / `-f` | 查找匹配的签名 |
| `--unpack=FILE` / `-u FILE` | 解包 CVD/CLD 文件 |
| `--md5 [FILES]` | 生成 MD5 哈希签名 |
| `--sha2-256 [FILES]` | 生成 SHA2-256 哈希签名 |
| `--vba=FILE` | 提取 VBA 宏代码 |
| `--print-certs=FILE` | 打印 PE 的 Authenticode 证书 |

### 查看 CVD 信息示例

```
sigtool --info=main.cvd
```

输出包含：版本号、签名数、功能级别、构建时间、MD5 等。

---

## Windows 特有注意事项

### 编码
- **所有输入必须 UTF-8 编码**（API、socket 命令、文件列表）
- **控制台输出始终 OEM 编码**，即使重定向到文件
- 文件列表临时文件用 UTF-8 无 BOM 写入

### 路径
- 使用反斜杠 `\` 作为路径分隔符
- 支持 SMB 网络共享和 UNC 路径
- 通配符 `*` 和 `?` 由 clamscan 内部模拟（Windows shell 不展开）

### 创建无窗口子进程
从 GUI 应用调用 clamscan 时，设置 `CREATE_NO_WINDOW`（0x08000000）标志避免弹出黑色控制台窗口：

```rust
use std::os::windows::process::CommandExt;
const CREATE_NO_WINDOW: u32 = 0x0800_0000;
cmd.creation_flags(CREATE_NO_WINDOW);
```

### PowerShell 调用陷阱
PowerShell 的原生命令调用会混淆 stdout/stderr（错误流被混入）。调试 clamscan 输出时，使用 `System.Diagnostics.Process` 直接重定向，或用 `cmd /c` + `2>` 分离。

---

## 病毒库结构

ClamAV 病毒库目录通常包含：

| 文件 | 说明 |
|------|------|
| `main.cvd` | 主病毒库（大，稳定） |
| `daily.cvd` | 每日更新库（增量更新） |
| `bytecode.cvd` | 字节码签名 |
| `*.cvd.sign` | CVD 数字签名文件（验证完整性） |

CVD = ClamAV Virus Database，是签名数据库的打包格式。

---

## 集成模式：从程序调用 clamscan

### 推荐：临时文件模式

适合需要批量扫描多个文件的场景：

```
clamscan --database=<db_dir> --file-list=<temp_file> --stdout -v --no-summary
```

流程：
1. 收集待扫描路径列表
2. 写入临时文件（UTF-8 无 BOM，每行一个路径）
3. spawn clamscan，重定向 stdout
4. 逐行解析 stdout，用 `rsplit_once(": ")` 提取 path 和 status
5. 扫描结束后删除临时文件

### 推荐：守护进程模式

适合需要频繁扫描或实时保护的场景：

1. 启动时 spawn `clamd`（后台守护进程）
2. 通过 `clamdscan` 或直接 socket 协议发送扫描命令
3. 病毒库只加载一次，后续扫描无 DB 加载开销

---

## 常见问题排查

### `--file-list=-` 报 "Can't open file -"
ClamAV 1.5.x 不支持从 stdin 读文件列表。改用临时文件 `--file-list=<tempfile>`。

### 扫描结果不出现在 stdout
默认 clamscan 把结果写到 **stderr** 而非 stdout。加 `--stdout` 标志。

### 病毒库加载很慢
`main.cvd` + `daily.cvd` + `bytecode.cvd` 合计较大，每次 clamscan 启动都要加载。如需频繁扫描，改用 `clamd` 守护进程模式（DB 只加载一次）。

### 扫描速度慢
- 病毒库加载是固定开销（10~30 秒），与文件数无关
- 关闭不需要的扫描类型可加速：`--scan-archive=no`、`--scan-mail=no` 等
- 用 `--max-filesize` / `--max-scansize` 跳过大文件
- 守护进程模式避免重复加载 DB

### 部分文件报 "Can't access file"
系统/受保护进程的文件在普通用户权限下无法访问，属正常现象。需管理员权限才能扫描。

### 退出码非 0 但无病毒
退出码 2 = 发生错误。检查 stderr 输出，可能是数据库损坏、路径不存在等。
