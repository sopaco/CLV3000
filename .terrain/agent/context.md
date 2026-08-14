---
type: agent_context
project: clv3000
title: Agent Architecture Context
source: .
---

## 项目概览

CLV3000 是一个面向 Windows 的**便携式按需（on-demand）病毒扫描桌面应用**，Rust 编写，底层调用 ClamAV，致敬经典 KV3000。定位是"插 U 盘即用、免安装、低资源占用"的应急扫描器：不提供实时防护，专注扫描一件事。三条扫描入口（闪电/全盘/右键单文件）+ 完整威胁处置闭环（忽略/隔离/恢复/删除）+ 系统集成（托盘常驻、开机自启、右键菜单、单实例）。关键约束：老机器友好（release 体积优化 `opt-level="s"`+lto）、GUI 闲置时事件循环睡死（零心跳）、隐藏到托盘释放 GPU 纹理并压缩工作集、扫描走 `clamscan` 子进程而非进程内引擎。Windows/macOS 为真实行为，Linux 等其余目标为 mock 引擎（纯 UI 预览）。

## 架构设计

- **UI 壳层（egui/eframe 主线程）**：`src/app/` 下的 `App` 持有全部 UI 状态，按 `Page` 枚举分派页面；拆为 `app_shell.rs`（`ui()`/`logic()`、资源加载/释放）与 `lifecycle_view.rs`（托盘轮询、扫描请求转发、`hide_to_tray`/窗口协调）；`logic()` 处理托盘/扫描请求轮询、资源监控采样与"扫描时才重绘"的唤醒策略。
- **扫描后端（后台线程）**：`src/scan/` 以 `std::thread` + `mpsc::Sender<ScanEvent>` 单向通信上报进度/威胁，UI 每帧 `try_recv` 轮询；取消通过原子 `CancelFlag`。
- **桌面集成层**：托盘、单实例、自启、右键菜单、macOS 重开，均独立模块、按 `cfg` 分平台实现。
- **状态与配置**：`AppCore`（页面 + 三块页面状态）与 `AppConfig`（TOML 持久化，含忽略/隔离/扫描记录）。页面状态持有各自的后台线程句柄与事件接收端。
- **双轨真实/mock**：同一函数签名下用 `cfg(windows)` / `cfg(target_os="macos")` 提供真实实现，其余目标走模拟数据（~342 进程、~3000 文件、交替 OK/FOUND）。

```
┌─────────────────────────── egui/eframe 主线程 ───────────────────────────┐
│ App (app_shell.rs + lifecycle_view.rs) ─┬─ AppCore ─┬─ ScanPageState      │
│  ui()/logic()/tray/资源/窗口协调         │          │   (quick/full)      │
│  release_ui_resources()                 │          ├─ VirusDbState       │
│                                         │          └─ SettingsState      │
└──┬──────────┬─────────────┘                                              │
   │ wakeup   │ mpsc<ScanEvent> / CancelFlag                                │
   ▼          ▼                                                             │
后台线程：scan/quick_scan · scan/full_scan · scan/engine(clamscan子进程)     │
└───────────────────────────────────────────────────────────────────────────┘
```

## 模块地图

| 模块 | 职责 | 主要路径 |
|------|------|----------|
| 入口/启动 | CLI 参数解析（`--tray-only`/`--scan-path`）、viewport 构建、托盘初始化 | `src/main.rs`、`src/lifecycle.rs` |
| UI 壳层 | App 装配、事件循环、页面分派、托盘/请求轮询、资源生命周期 | `src/app/app_shell.rs`、`src/app/lifecycle_view.rs` |
| 页面 | 仪表盘、闪电/全盘扫描页、病毒库页、设置页 | `src/app/pages/*`、`src/app/settings.rs` |
| 窗口框架 | 侧边栏、资源条、标题栏（非 Windows）、Toast | `src/app/chrome.rs`、`src/widgets.rs`、`src/theme.rs` |
| 核心状态 | `AppCore`、扫描状态机（Idle/Enumerating/Scanning/Done）、设置、病毒库状态 | `src/app/core/*` |
| 扫描编排 | 预扫描（blake3 哈希 + 缓存查重）、clamscan 批处理、结果解析、缓存写入 | `src/scan/engine.rs`、`src/scan/cache.rs` |
| 闪电扫描 | 进程/模块枚举（Win Toolhelp32 / macOS sysinfo） | `src/scan/quick_scan.rs` |
| 全盘扫描 | 固定盘遍历、可执行扩展名/Mach-O 筛选、流式写入临时清单 | `src/scan/full_scan.rs` |
| 签名预过滤 | WinVerifyTrust（PE/catalog）、macOS codesign 校验，跳过可信签名 | `src/scan/authenticode.rs` |
| 病毒库管理 | freshclam 子进程更新、引擎/库版本探测 | `src/app/freshclam.rs`、`src/clamav_info.rs` |
| 威胁处置 | 隔离/恢复/删除、强制隔离（杀占用进程+提权移动） | `src/quarantine.rs` |
| 桌面集成 | 托盘、单实例、自启、右键菜单、macOS 重开 | `src/tray.rs`、`src/single_instance.rs`、`src/autostart.rs`、`src/context_menu.rs`、`src/macos_reopen.rs` |
| 基础设施 | 路径解析、配置持久化、系统监控、图标资源 | `src/paths.rs`、`src/config.rs`、`src/sysmon.rs`、`src/icon_data.rs`、`src/icons.rs` |

## 核心流程

**闪电扫描 / 全盘扫描（后台任务）**
1. 页面启动 → 创建 `CancelFlag` + `mpsc` 通道，`spawn` 后台线程（quick→`quick_scan::run`，full→`full_scan::run`）。
2. 枚举阶段（quick 枚举进程+模块去重；full 遍历固定盘把可执行文件流式写入临时 walklist），UI 显示"已发现 N 个文件"。
3. 引擎阶段：多线程预扫描——blake3 内容哈希 + 查 `ScanCache`（DB 版本变化即失效）；未命中缓存 → 组装 `clamscan` 子进程批处理，解析 OK/FOUND 输出，回写缓存。
4. 全程经 `ScanEvent` 上报进度/当前路径/威胁；UI 每帧 `try_recv`，`apply_scan_event` 推进状态机。
5. 取消：置 `CancelFlag`，后台终止子进程并退出；完成时返回扫描数/耗时摘要。

**右键"用 CLV3000 扫描"（冷启动/转发）**
1. Shell 启动第二实例 → `single_instance::acquire` 探测已有实例（Win 命名 Mutex / Unix socket）。
2. 已有实例 → 经 `wakeup::push_scan_request`/socket 把路径转发给运行中实例，本实例退出。
3. 无实例 → 解析 `--scan-path`，直接进入 FullScan 页并对该路径发起 `start_path` 扫描。

**生命周期与托盘**
1. 启动按 `InitialMode` 分派（ShowWindow / TrayOnly / QuickScan / About / ScanPath）；`--tray-only` 不显示窗口。
2. 关闭按钮/`RunMode::Quit` 判定：非退出 → 取消关闭、`hide_to_tray`（释放 sysmon、清 egui 缓存/纹理、`trim_working_set` 压缩工作集）。
3. 扫描中每帧 `request_repaint_after(250–500ms)` 推进进度动画；闲置时零重绘请求，事件循环睡死。

**威胁处置闭环**
1. 检测到威胁 → 页面列出，用户可选忽略（写入 `AppConfig.ignored`，后续扫描跳过）或隔离（移入隔离目录）。
2. 文件被占用/权限不足 → Windows 强制隔离：枚举占用进程、终止、必要时提权移动。
3. 设置页隔离列表支持恢复（移回原路径）与永久删除；全部经 `AppConfig`（TOML）持久化。

## 技术选型

- **语言/版本**：Rust，edition 2024；release 用体积优化（`opt-level="s"`、`lto`、`panic="abort"`、`strip`）。
- **GUI**：egui / eframe 0.36（glow 渲染、默认字体，非默认特性）；无自定义依赖重量级 UI。
- **Windows 原生**：`windows` crate 0.62（Toolhelp、WinTrust、Shell、注册表、进程管理等）；资源文件用 `winresource`（build.rs）。
- **macOS 原生**：`objc2` 系列（AppKit/Foundation）；`macos_reopen.rs` 处理重开事件。
- **系统托盘/菜单**：`tray-icon` + `muda`。
- **扫描外部依赖**：ClamAV 便携目录（`clamscan`/`freshclam` + `database/*.cvd`），以子进程方式调用，**不**在进程内加载引擎。
- **辅助**：`sysinfo`（资源监控）、`blake3`（文件基因哈希）、`walkdir`（磁盘遍历）、`serde`+`toml`（配置）、`image`（图标解码）、`directories`（目录定位）。

## 系统边界

- **外部进程契约**：`clamscan`（扫描/`--version`）、`freshclam`（更新/`--datadir`）。两者必须位于 exe 旁 `clamav/` 目录或 PATH；缺失不崩溃，UI 显示"engine not found"。
- **文件系统**：exe 旁 `clamav/database/`（签名库）；AppData 下 `config.toml`、扫描缓存、隔离目录；全盘扫描临时 walklist 写系统 temp。
- **注册表**：自启 Run 键、右键菜单 `ShellEx`/verb 键（读写）；单实例命名 Mutex。
- **网络**：仅 freshclam 更新签名库时访问 ClamAV 服务器（不经应用 HTTP 栈）。
- **信任边界**：隔离/强制隔离会终止占用进程并可能触发 UAC 提权，属高风险操作；签名预过滤信任系统证书链；mock 模式（非 Win/mac）数据全部为合成，仅用于 UI 预览。

## 代码映射索引

| 概念 | 位置 | 备注 |
|------|------|------|
| 入口、CLI 模式 | `src/main.rs`、`src/lifecycle.rs` | `InitialMode`/`RunMode` |
| App 壳、事件循环 | `src/app/app_shell.rs`、`src/app/lifecycle_view.rs` | `ui()`/`logic()`（app_shell）；`hide_to_tray`/`reconcile_lifecycle`/托盘与扫描请求轮询（lifecycle_view） |
| 页面与框架 | `src/app/pages/`、`src/app/chrome.rs`、`src/app/settings.rs` | `Page` 枚举分派；`settings_page` 在 settings.rs |
| 核心状态 | `src/app/core/mod.rs`、`scan_state.rs`、`settings_state.rs`、`virus_db.rs` | `AppCore`、`ScanPhase` 状态机、`apply_scan_event` |
| 扫描编排 | `src/scan/engine.rs`、`src/scan/cache.rs` | 预扫描、clamscan 批处理、缓存 |
| 闪电/全盘扫描 | `src/scan/quick_scan.rs`、`src/scan/full_scan.rs` | 进程枚举 / 磁盘遍历 |
| 签名预过滤 | `src/scan/authenticode.rs` | WinVerifyTrust / codesign |
| 病毒库更新 | `src/app/freshclam.rs`、`src/clamav_info.rs` | freshclam 子进程、版本解析 |
| 威胁处置 | `src/quarantine.rs`、`src/config.rs` | 隔离/恢复/强制隔离（杀占用进程+提权） |
| 托盘/单实例/自启/右键 | `src/tray.rs`、`src/single_instance.rs`、`src/autostart.rs`、`src/context_menu.rs` | 分平台 cfg 实现 |
| 后台唤醒与请求桥 | `src/wakeup.rs`、`src/macos_reopen.rs` | 线程→UI 重绘、扫描请求转发 |
| 路径/配置/监控 | `src/paths.rs`、`src/config.rs`、`src/sysmon.rs` | clamav 目录解析、TOML 持久化、资源采样 |
| 主题/图标/组件 | `src/theme.rs`、`src/icons.rs`、`src/icon_data.rs`、`src/widgets.rs`、`src/about_dialog.rs` | 设计 token、图标、通用控件 |
```