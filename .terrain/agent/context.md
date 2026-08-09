---
type: agent_context
project: clv3000
title: Agent Architecture Context
source: .
---

## 项目概览

CLV3000 是一个纯 Rust 实现的极简 Windows **手动杀毒工具**，专为跑不动大厂常驻杀软的老旧机器设计。核心能力是"两类扫描 + 四项辅助"：**闪电扫描**（枚举活跃进程加载的模块）与**全盘扫描**（遍历本地磁盘可执行文件），均交由随程序分发的便携版 ClamAV（`clamscan.exe` 子进程）比对签名；另提供病毒库手动更新、系统托盘常驻、CPU/内存状态条、单实例保护。刻意**不做**实时防护、不做任何文件写操作——检出的威胁只报告，"隔离"按钮为占位；忽略记忆写配置。项目仅 18 个源文件、约 2200 行，面向 `x86_64-pc-windows-gnu` 交叉编译，产物体积优先。

## 架构设计

单进程桌面应用，`eframe/egui` 事件循环驱动 UI，扫描/监控/更新均通过**后台线程 + `std::sync::mpsc` 通道 + `Arc<AtomicBool>` 取消标志**与 UI 异步协作；UI 每帧轮询各通道。

| 层 | 组件 | 说明 |
|---|---|---|
| 入口 | `main.rs` | 单实例锁 → 构建托盘 → 创建 eframe 窗口 |
| UI | `app.rs` | 四页面路由、自绘标题栏/侧边栏、状态轮询、托盘事件分发 |
| UI 组件 | `widgets.rs` `theme.rs` `icons.rs` | 圆环进度/威胁卡片/Toast/胶囊；配色与圆点背景；手绘矢量图标 |
| 扫描编排 | `app.rs` → `scan/mod.rs` | `ScanPageState` 管理扫描生命周期；`ScanEvent` 为统一事件协议 |
| 扫描实现 | `scan/quick_scan.rs` `scan/full_scan.rs` | 进程/模块枚举（ToolHelp）；磁盘遍历（walkdir）流式喂给引擎 |
| 引擎桥接 | `scan/engine.rs` | `clamscan.exe` 子进程封装，解析 stdout 行判定感染 |
| 病毒库 | `app.rs`(`VirusDbState`) + `paths.rs` | 手动触发 `freshclam.exe` 更新签名 |
| 辅助服务 | `tray.rs` `sysmon.rs` `single_instance.rs` | 托盘+菜单、每秒资源采样线程、命名互斥锁 |
| 持久化 | `config.rs` `paths.rs` `localtime.rs` | TOML 配置（%APPDATA%）、路径解析、极简本地时间 |

依赖方向：UI 层依赖扫描/引擎/托盘/系统监控；扫描层依赖 `paths.rs` 与 `engine.rs`；各层经模块边界由 `scan/mod.rs` 共享类型耦合。

## 模块地图

| 模块 | 职责 | 主要路径 |
|---|---|---|
| 扫描事件协议 | 共享类型：`ScanKind`、`Threat`、`ScanEvent`、`CancelFlag` | `src/scan/mod.rs` |
| 闪电扫描 | ToolHelp 快照 PID → 枚举各进程模块去重 → 喂引擎 | `src/scan/quick_scan.rs` |
| 全盘扫描 | 枚举固定磁盘（可含可移动盘）→ 按可执行扩展名过滤 → 流式喂引擎 | `src/scan/full_scan.rs` |
| 扫描引擎 | 消费路径通道，调用 `clamscan.exe`，解析输出行 → `FileScanned` 事件 | `src/scan/engine.rs` |
| 应用状态与 UI | 四页面路由、标题栏/侧边栏/状态条、扫描与病毒库状态机、托盘事件分发 | `src/app.rs` |
| 配置持久化 | `AppConfig`（扫描记录/忽略项/可移动盘开关）TOML 读写 | `src/config.rs` |
| 路径定位 | exe 目录、`clamav\`、`clamscan`/`freshclam`、`%APPDATA%\CLV3000` | `src/paths.rs` |
| 系统托盘 | `tray-icon` 图标 + `muda` 菜单（显示/闪电扫描/关于/退出） | `src/tray.rs` |
| 资源监控 | 后台线程每秒采集 CPU/内存，通道送 UI | `src/sysmon.rs` |
| 单实例 | Win32 命名 Mutex，二次启动直接退出 | `src/single_instance.rs` |
| 主题与视觉 | 深色配色、全局样式、圆点背景、强调色 | `src/theme.rs` |
| 通用控件 | 圆环进度、状态胶囊、威胁卡片、Toast | `src/widgets.rs` |
| 图标与时间 | 手绘矢量图标 + RGBA 程序图标；`GetLocalTime` 极简时间 | `src/icons.rs` `src/icon_data.rs` `src/localtime.rs` |

## 核心流程

**1. 闪电扫描**
1. UI 点击触发 `ScanPageState::start`，新建 `CancelFlag` 与 mpsc 通道，spawn `quick_scan::run`。
2. 线程用 Win32 ToolHelp 拍进程快照，逐个枚举模块并去重，期间发 `Enumerating` 事件上报进度。
3. 去重后的文件路径写入共享路径通道；`engine::run` 消费通道，逐路径调用 `clamscan.exe`。
4. 引擎解析子进程输出，发 `FileScanned{path, infected}`；UI 轮询聚合威胁（过滤配置忽略项）。
5. `Finished` 事件落盘 `last_quick_scan`，页面显示结果；取消经 `CancelFlag` 终止子进程与扫描。

**2. 全盘扫描**
1. 与闪电扫描同一状态机；`full_scan::run` 先枚举本地固定磁盘根（`scan_removable_drives` 决定是否含 U 盘）。
2. 用 `walkdir` 遍历目录树，按 `.exe/.dll/.sys/.scr/.com/.cpl/.ocx/.drv` 过滤，流式写入路径通道（不攒列表，进度实时可见）。
3. 后续走同一引擎/事件/聚合路径；无 `clamscan` 时 UI 提示"找不到扫描引擎"而不崩溃。

**3. 病毒库手动更新**
1. 病毒库页点击更新 → `VirusDbState::start_update`（先检查 `freshclam_available`）。
2. spawn 线程执行 `run_freshclam`（`freshclam.exe` 联网拉取签名）。
3. 结果经通道回传，UI 以 Toast 提示成功/失败；无自动定时更新。

**4. 生命周期与托盘**
1. 启动时 `single_instance::acquire` 抢占互斥锁，失败即退出；成功后建托盘。
2. 窗口关闭按钮默认 `CancelClose` + 隐藏（最小化到托盘）；托盘双击/菜单"显示"唤回窗口。
3. 托盘事件每帧轮询（`request_repaint_after(250ms)` 维持，最坏延迟数百 ms）。
4. 仅托盘"退出"置 `allow_exit` 并真正关闭进程。

## 技术选型

- 语言/工具链：Rust edition 2024；`x86_64-pc-windows-gnu` 交叉编译（macOS 开发机 + mingw-w64）
- GUI：`eframe`/`egui` 0.36（glow 后端、`default_fonts`、`persistence`），全自绘深色 UI
- 托盘/菜单：`tray-icon` 0.24 + `muda` 0.19
- Win32 互操作：`windows` 0.62（ToolHelp/ProcessStatus/Threading/FileSystem/LibraryLoader 等特性）
- 资源监控：`sysinfo` 0.39；目录遍历：`walkdir` 2.5
- 配置：`serde` + `toml` 持久化到 `%APPDATA%\CLV3000\config.toml`；`directories` 定位目录
- 错误处理：`anyhow`；时间：自研 `localtime`（`GetLocalTime`，不引入 chrono）
- 第三方病毒引擎：便携版 **ClamAV**（`clamscan.exe`/`freshclam.exe`/libclamav DLL/`database\*.cvd`），随 exe 以 `clamav\` 子目录分发，**不在代码仓库内**
- Release 优化：`opt-level=s` + `lto` + `codegen-units=1` + `panic=abort` + `strip`

## 系统边界

- **外部子进程（信任边界）**：`clamscan.exe`（扫描比对，解析其 stdout 行）、`freshclam.exe`（联网更新签名）。二者缺失时优雅降级提示，不崩溃。
- **Win32 API**：ToolHelp（进程/模块枚举）、`GetLocalTime`、命名 Mutex、托盘消息。
- **外部存储（只读）**：活跃进程模块、本地磁盘可执行文件；**唯一写路径**为 `%APPDATA%\CLV3000\config.toml`。
- **网络**：仅经 `freshclam.exe` 手动触发访问 ClamAV 签名源；无其他网络行为。
- **非目标**：无实时防护/后台服务；无隔离写入（按钮为占位）；无文件删除。
- **信任假设**：`clamav\` 目录与签名库视为可信来源；程序图标为占位美术（`icon_data.rs` 光栅生成）。

## 代码映射索引

| 概念 | 位置 | 备注 |
|---|---|---|
| 程序入口 | `src/main.rs` | 单实例→托盘→窗口 |
| 应用状态/页面路由 | `src/app.rs` | `Page`、`ScanPageState`、`VirusDbState`、标题栏/侧边栏/状态条 |
| 扫描事件协议 | `src/scan/mod.rs` | `ScanKind`、`Threat`、`ScanEvent`、取消标志 |
| 闪电扫描 | `src/scan/quick_scan.rs` | PID 快照、模块枚举、去重 |
| 全盘扫描 | `src/scan/full_scan.rs` | 磁盘根枚举、walkdir、扩展名过滤 |
| 引擎桥接 | `src/scan/engine.rs` | clamscan 子进程、输出解析 |
| 配置 | `src/config.rs` | `AppConfig`/`ScanRecord`/`IgnoredEntry` |
| 路径 | `src/paths.rs` | clamav 目录、appdata、可用性检查 |
| 托盘 | `src/tray.rs` | `Tray`、`TrayMenuIds` |
| 资源监控 | `src/sysmon.rs` | `SysMonHandle`、采样线程 |
| 单实例 | `src/single_instance.rs` | Win32 Mutex |
| 主题/控件/图标 | `src/theme.rs` `src/widgets.rs` `src/icons.rs` `src/icon_data.rs` | 自绘视觉体系 |
| 本地时间 | `src/localtime.rs` | `Timestamp`、`GetLocalTime` |
| 构建配置 | `Cargo.toml` `.cargo/config.toml` | 交叉编译 target 与 release 优化 |