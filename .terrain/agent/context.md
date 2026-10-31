---
type: agent_context
project: clv3000
title: Agent Architecture Context
source: .
---

## 项目概览

CLV3000 是面向 Windows 与 macOS 的便携式按需杀毒扫描器，使用 Rust 编写，由 ClamAV 驱动。面向应急/救援场景：从 U 盘直接运行、免安装，发现可疑行为后立即扫描，再隔离或忽略结果。使用者包括终端用户、IT 运维和离线救援人员（在 WinPE 或脚本中通过 `--scan-path` 调用）。

关键约束：**便携优先**（路径相对可执行文件解析）；**流水线架构**（枚举 → 预过滤 → ClamAV 子进程 → UI 事件）；**非阻塞 GUI**（后台线程 + `ScanEvent` 通道）；**小体积发布**（`opt-level = "s"`、LTO、strip）。不是实时/常驻防护：扫描由用户发起，或由用户显式开启的自动计划（开机、每周定时、病毒库更新后）触发。

## 架构设计

| 容器 | 分层 | 职责 |
|------|------|------|
| App UI | 表现层 + 应用层 | egui 页面、`AppCore`/`ScanPageState`、设置、病毒库视图 |
| Auto Scan Scheduler | 应用层 | 开机/定时/更新后自动扫描的到点判定与去重 |
| Scan Engine | 领域层 | 路径枚举、预扫缓存/签名、ClamAV 封装 |
| Threat Management | 领域层 | 隔离区、忽略列表、还原 |
| Persistence | 基础设施层 | TOML 配置、TSV 扫描缓存、路径解析 |
| Platform Layer | 基础设施层 | 托盘、单实例 IPC、开机自启、右键菜单 |
| ClamAV | 外部子进程 | 可执行文件旁的 `clamscan` / `freshclam` |

**模式：** 带类型事件的扫描流水线；`ScanPhase` 状态机（Idle → Enumerating → Scanning → Done）；`AppCore::poll_background` 每帧轮询自动更新与自动扫描计划（内部节流）；`#[cfg(...)]` 平台策略，Linux 下用 mock 支持 UI 预览；单实例锁 + 扫描/显示请求转发。

**主要依赖：** eframe/egui 0.36（glow）、sysinfo、walkdir、blake3、muda/tray-icon、windows crate（Win32）、objc2（macOS）；测试用 egui_kittest 0.36.1。

```
User/CLI → lifecycle (main.rs) → App UI ←ScanEvent— Scan thread
                                    ↓   ↑            ↓
                         AppCore 自动扫描调度   quick_scan/full_scan
                                    ↓                  ↓
                            config/quarantine   engine (prescan → clamscan)
```

## 模块地图

| 模块 | 职责 | 主要路径 |
|------|------|----------|
| app | UI 外壳、页面路由、扫描编排、设置 | `src/app/`, `src/app/app_shell.rs`, `src/app/pages/` |
| app/core | 跨帧状态：路由、配置、扫描阶段、设置页、病毒库、自动扫描调度 | `src/app/core/` |
| scan | 枚举、引擎流水线、内容缓存、Authenticode 跳过 | `src/scan/` |
| quarantine | 威胁文件移入隔离区；还原 | `src/quarantine.rs` |
| persistence | 用户配置（TOML）与便携路径布局 | `src/config.rs`, `src/paths.rs` |
| lifecycle | CLI 解析、启动模式、运行/托盘切换 | `src/lifecycle.rs`, `src/main.rs` |
| platform | 托盘、IPC、开机自启、资源管理器右键菜单、唤醒 | `src/tray.rs`, `src/single_instance.rs`, `src/autostart.rs`, `src/context_menu.rs`, `src/wakeup.rs` |
| system-integration | ClamAV 探测、病毒库更新（freshclam）、版本查询 | `src/app/freshclam.rs`, `src/clamav_info.rs`, `src/app/core/virus_db.rs` |
| ui-infra | 主题、控件、资源监视、平台窗口外壳 | `src/theme.rs`, `src/widgets.rs`, `src/icons.rs`, `src/sysmon.rs`, `src/windows_chrome.rs`, `src/app/chrome.rs` |
| ui-testing | 无头 egui_kittest 测试与 PNG 视觉预览 | `src/app/ui_preview.rs`, 组件同文件 `mod tests` |
| build/packaging | macOS 打包脚本、Windows 资源 | `build.rs`, `scripts/bundle-macos.sh` |

## 核心流程

### 1. 应用启动与第二实例转发

1. 解析 CLI（`--tray-only`、`--show`、`--scan-path`，Windows 专用 `--force-quarantine` 在单实例检查前直接执行提权隔离子进程并退出）。
2. 尝试 `single_instance::acquire`；若锁被占用，则把扫描/显示请求写入 IPC（Windows：`scan_request.txt` + 事件；macOS：Unix socket）后退出，转发失败时提示“已在运行”。
3. 初始化 `wakeup`、启动请求监听线程、构建系统托盘。
4. 解析 `InitialMode`；Windows 托盘启动时在 eframe 之外运行 Win32 消息循环（`wait_in_tray`），避免闪窗。
5. 启动 `eframe::run_native`，用 `&egui::Context` 构造 `App`；首帧按生命周期对账窗口可见性，之后每帧 `App::logic` 调用 `AppCore::poll_background` 处理自动更新与自动扫描。

### 2. 闪电扫描

1. 用户触发闪电扫描；`ScanPageState` 启动带取消标志的后台线程。
2. `quick_scan::run` 快照进程列表、枚举已加载模块/DLL，去重路径。
3. 发出 `ScanEvent::Enumerating` / `ScanStarted`，以 `PathSource::InMemory` 进入引擎。
4. `engine::prescan` 并行检查 blake3 缓存与代码签名；命中时直接发 `FileScanned`。
5. 剩余路径批量写入临时文件列表，启动单个 `clamscan` 子进程，逐行解析 stdout 为事件。
6. UI 通过 `apply_scan_event` 更新状态；`ScanPhase` 进入 Done，威胁可隔离/忽略。

### 3. 全盘扫描

1. 用户启动全盘扫描；是否包含可移动盘由 `AppConfig.scan_removable_drives` 决定。
2. `full_scan::run` 遍历固定磁盘，通过 `WalkListWriter` 流式写入临时文件（避免 10 万+ 路径常驻内存）。
3. 枚举阶段发出 `WalkProgress`；随后复用引擎预扫 → 单批 `clamscan`。
4. 结果写入配置（`last_full_scan`）与 TSV 缓存，供后续复扫。

### 4. 自动扫描调度（开机 / 定时 / 更新后）

1. `AutoScanState` 挂在 `AppCore`，由 `poll_background` 每帧询问；时间判定节流为 10 秒一次。
2. 开机扫描：启动 5 秒后若 `auto_scan_on_boot` 开启则触发一次（每次运行仅一次）。
3. 定时扫描：`ScheduleConfig` 用星期位掩码（bit0=周一）+ 时/分描述；到点后 5 分钟宽限窗口内补扫，同一天同一档位用日期戳去重。
4. 病毒库更新：`freshclam` 成功后若 `auto_scan_after_update` 开启则触发；自动检查每次运行只发起一次。
5. `start_auto_scan` 在已有扫描运行时跳过；定时计划按 `scan_full` 选择全盘/闪电，其余两种固定闪电扫描，并通过 Toast 提示触发来源。

## 技术选型

- **语言：** Rust 2024 edition（v0.7.8）
- **GUI：** eframe 0.36 + egui 0.36（glow 后端，默认字体）
- **测试：** egui_kittest 0.36.1（dev-dependency，`snapshot` feature）；组件同文件无头 UI 测试；`src/app/ui_preview.rs` 将真实 `App` 的 13 个状态渲染为 `target/ui_preview/*.png`（`#[ignore]`）
- **扫描引擎：** ClamAV 子进程（`clamscan`、`freshclam`）——不走 libclamav FFI
- **进程枚举：** sysinfo；Windows Toolhelp32 通过 `windows` crate
- **签名预过滤：** Windows WinVerifyTrust / macOS codesign（`src/scan/authenticode.rs`）
- **内容缓存：** blake3 哈希，扁平 TSV，按病毒库修订号做 TTL
- **持久化：** TOML（`serde` + `toml`）保存配置；TSV 保存扫描缓存
- **时间：** Windows `GetLocalTime`；非 Windows 用 `SystemTime` + Howard Hinnant 公历换算（UTC，仅预览用），`Timestamp::weekday` 支持定时计划
- **系统集成：** tray-icon、muda 菜单；Win32 注册表；macOS LaunchAgents + objc2
- **发布：** `opt-level = "s"`、LTO、`panic = "abort"`、strip；winresource 写入 Windows 元数据
- **开发基础设施：** Terrain 知识库 `.terrain/`；CodeGraph 索引 `.codegraph/`

## 系统边界

| 边界 | 参与者 / 系统 | 接口 | 信任模型 |
|------|----------------|------|----------|
| ClamAV 引擎 | 外部子进程 | 启动 `clamscan`/`freshclam`；位于 `<exe>/clamav/` 或 macOS `Contents/Resources/clamav/` | 受信任的扫描权威；引擎缺失时 UI 报错、不崩溃 |
| 本地文件系统 | 操作系统 | 读取扫描目标；写入隔离区、缓存 TSV、临时文件列表 | 用户文件；错误以 Toast/`ScanEvent::Error` 呈现 |
| 用户配置 | Windows `%APPDATA%/CLV3000/config.toml`；macOS `~/Library/Application Support/CLV3000/` | TOML：忽略/隔离记录、扫描历史、可移动盘/自动更新/自动扫描计划等设置 | 应用自有；不加密 |
| 系统本地时钟 | 操作系统 | `GetLocalTime` / `SystemTime`；定时扫描按本地星期几与时刻判定 | 信任系统时钟；时间异常只会导致不触发或宽限窗口内补触发 |
| CLI | 用户 / 资源管理器 / 脚本 | `--tray-only`、`--show`、`--scan-path`、`--force-quarantine`（Windows UAC 内部助手） | `--force-quarantine` 不面向直接用户 |
| 单实例 IPC | 第二个进程 | 互斥量 + 事件 + `scan_request.txt`（Windows）；`clv3000.sock`（macOS） | 仅同用户本地 |
| 注册表 / LaunchAgents | 设置页开关 | 开机自启（`--tray-only`）；资源管理器右键菜单（Windows） | 需用户通过设置页授权 |
| 环境变量 | 仅开发者 | `CLV3000_ALLOW_MULTIPLE_INSTANCES` 绕过单实例锁 | 仅调试 |

**范围之外：** 网络/云扫描、常驻实时防护、Linux 生产扫描（Linux 构建使用 mock 供 UI 开发）。

## 代码映射索引

| 概念 | 位置 | 说明 |
|------|------|------|
| 进程入口与 eframe 启动 | `src/main.rs` | 托盘循环、视口、强制隔离助手 |
| CLI 参数与运行模式 | `src/lifecycle.rs` | `InitialMode`、`RunMode`、`Lifecycle` |
| 便携路径解析 | `src/paths.rs` | `exe_dir`、`clamscan_path`、病毒库目录 |
| 用户配置 | `src/config.rs` | `AppConfig`、`ScheduleConfig`、忽略/隔离记录、自动扫描开关 |
| 扫描事件类型 | `src/scan/mod.rs` | `ScanEvent`、`ScanKind`、`Threat`、`PathSource` |
| ClamAV 子进程封装 | `src/scan/engine.rs` | 预扫、批量 clamscan、取消看门狗 |
| 内容缓存与签名跳过 | `src/scan/cache.rs`, `src/scan/authenticode.rs` | `ScanCache`、不可变 `CacheSnapshot`、受信任发布者判断 |
| 闪电/全盘枚举 | `src/scan/quick_scan.rs`, `src/scan/full_scan.rs` | 进程模块枚举；磁盘遍历与流式路径列表 |
| 扫描 UI 状态机 | `src/app/core/scan_state.rs` | `ScanPhase`、`apply_scan_event`、`threats_modal_open` |
| 自动扫描调度 | `src/app/core/auto_scan.rs` | `AutoScanState`、开机/定时/更新后触发与去重 |
| 应用外壳与导航 | `src/app/app_shell.rs`, `src/app/chrome.rs`, `src/app/mod.rs` | `App`、标题栏/侧栏/资源条、`Page` 路由 |
| 页面与威胁弹窗 | `src/app/pages/` | 仪表盘、扫描页、病毒库页、设置页；`threats_modal` |
| 病毒库更新与版本查询 | `src/app/freshclam.rs`, `src/app/core/virus_db.rs`, `src/clamav_info.rs`, `src/app/pages/virus_db.rs` | freshclam 编排、自动检查、版本展示 |
| 平台集成 | `src/quarantine.rs`, `src/single_instance.rs`, `src/tray.rs`, `src/wakeup.rs`, `src/autostart.rs`, `src/context_menu.rs` | 隔离/还原、IPC、托盘、唤醒、自启、右键菜单 |
| UI 基础设施与视觉预览 | `src/theme.rs`, `src/widgets.rs`, `src/icons.rs`, `src/app/ui_preview.rs` | 设计令牌、复用控件、图标、无头渲染预览 |