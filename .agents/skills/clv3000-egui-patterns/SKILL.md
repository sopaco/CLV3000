---
name: clv3000-egui-patterns
description: Use when writing or reviewing CLV3000 egui/eframe 0.36 UI code — pages, dialogs/modals, background task wiring, repaint scheduling, or UI tests (src/app/, src/app/pages/, src/widgets.rs, src/wakeup.rs). Encodes architecture patterns benchmarked against storytold/craft-launcher (same eframe 0.36 stack) UI/backend boundaries, dialog state machine, event-driven repaint, egui_kittest testability, never-crash rules under panic=abort. Complements clv3000-design (visual tokens and rendering pitfalls); this one covers code organization.
version: 0.1.0
---

# CLV3000 egui 架构模式（对标 craft-launcher）

来源：对 `github.com/storytold/craft-launcher`（ArtCraft Launcher，egui/eframe 0.36，与本项目同版本栈）的深度调研 + 与 CLV3000 现状逐项对比。craft-launcher 侧结论出自其源码结构与工程文档；CLV3000 侧结论全部对照本项目源码核实过（标注"已核实"处）。

定位：与 `clv3000-design` 互补——那份管"长什么样"（设计 token / 渲染坑），这份管"代码怎么组织、边界在哪、怎么测"。写新 UI 前两份都读。

标记：✅ 已对齐保持 / 🔶 建议采纳 / ⏸ 暂缓（记录触发条件，避免重复论证）

## 1. 对比总表

| 维度 | craft-launcher | CLV3000 现状 | 结论 |
|---|---|---|---|
| 帧模型 | eframe 0.36 `logic()`+`ui()` 分离，Shell 薄壳委托 `LauncherUi` | 同一 API；`App::logic` 排事件/对账生命周期，`App::ui` 纯绘制 | ✅ |
| 状态组织 | `LauncherUi` 集中状态 + `views.rs`/`dialogs.rs` 方法 | `App`（shell 关注点）+ `AppCore`（业务状态）+ `pages/` 自由函数 | ✅ 等价 |
| UI↔后台 | `Backend` trait（`send`/`poll`）+ `Command`/`Event` 枚举，Native/Web/Fake 三实现 | 每页状态对象持 mpsc + `start`/`poll` 方法对 | ✅ 形状一致；Fake 替身未引入（见 §2 末尾） |
| 重绘调度 | Waker（`Arc<dyn Fn()>`）事件唤醒 + 任务期显式 repaint | 托盘/IPC 唤醒（wakeup）+ 扫描定时 33/250ms + sysmon/freshclam 完成点唤醒 | ✅ 同构；决策表见 §4 |
| 弹窗 | `Dialog` 枚举 + `egui::Modal` + 每弹窗唯一 Id，渲染函数返回"是否关闭" | `threats_modal_open: bool`；`about_dialog` 模块级 `AtomicBool` + `take_closed()` | 🔶 §3 |
| UI 测试 | `egui_kittest::Harness` 驱动**真实** UI + Fake backend + 快照回归 | ✅ 已落地：kittest 驱动真实组件（`pages/scan.rs::tests`），见 AGENTS.md「UI 代码验证流程」 | ✅ §2 |
| 分层 | Cargo workspace 强制：core 零 UI 依赖、可编译 wasm；"UI 不下沉"写进 AGENTS.md | 模块纪律：`src/scan` 等引擎模块零 egui import（已核实） | ✅ 等价；纪律见 §5 |
| 主题 | `Tokens` 明暗双套 + 存 ctx data 运行时切换 | `colors` 常量 + `theme::apply` 一次，固定深色 | ⏸ §6 |
| i18n | TSV catalog + `tl!`/`tn!` 宏 + 每语言复数规则 + 运行时查系统 CJK 字体 | 内联英文字符串 | ⏸ §6 |
| 健壮性 | "Never crash"、no unsafe 写进 AGENTS.md | release `panic="abort"` 下仍有 `lock().unwrap()` 存量 | 🔶 §7 |

## 2. ✅ UI 测试：egui_kittest 驱动真实 UI（已落地）

**已集成**（2026-10）：`egui_kittest = "0.36.1"`（dev-dependency，与 egui 同线），
AGENTS.md「UI 代码验证流程」是权威操作指南。落地要点：

- `App::new` 收 `&egui::Context`（不再依赖 `CreationContext`），UI 不依赖 eframe 即可构造；
- `App::run_logic(ctx)` / `App::run_ui(ui)` 从 `impl eframe::App` 壳里抽出（Windows 的
  `poll_chrome_tune` 需 eframe::Frame 借 HWND，留在壳里）；
- `pages/scan.rs::tests` 三个 kittest 测试驱动**真实** `threats_modal`：关闭按钮手型光标、
  卡片等宽/弹窗不撑宽、关闭交互（点击/Escape）；旧"复刻 UI"式测试已删；
- 自绘按钮（`pill_button`、弹窗 × 按钮）已补 `Response::widget_info`——进 accesskit 树
  后 kittest 可查、屏幕阅读器可读，一石二鸟。

**写法速查**（详情看 AGENTS.md）：`Harness::builder().with_size(..).build_ui_state(闭包, 状态元组)`
→ `theme::apply(&harness.ctx)` → `run()` → `get_by_label(...)` 查询 → `.click()/.hover()`
→ `harness.state()` 断言状态 / `node.rect()` 断言几何 / `output().platform_output.cursor_icon` 断言光标。

**后续扩展方向**（非必须）：侧栏导航切换、设置 tab、toast 过期；如需命令录制/脚本化事件驱动的
Fake backend（对标 craft-launcher 的 `Fake`），先给扫描引擎引入可替换的后台 trait 边界（§1 表中
"UI↔后台"行仍是 🔶 的部分仅指这一点）。

**禁止事项**（AGENTS.md 有完整版）：复刻 UI 结构式测试；在 harness 里触发文件系统/落盘副作用
（`quarantine_file`、`AppConfig::save` 无测试隔离）；起真 clamscan/freshclam。

## 3. 🔶 弹窗：`Dialog` 枚举 + `egui::Modal`

**现状三个问题**（均已核实）：

- `ScanPageState::threats_modal_open: bool`：每加一个弹窗加一个布尔字段，互斥关系没有编译器保证（`lifecycle.about_open` 与它理论上可同时为 true，渲染层各自为政）；
- `about_dialog.rs` 用模块级 `AtomicBool` + `take_closed()`：进程级全局状态，测试之间会串，多会话语义靠约定；
- 威胁弹窗用 `egui::Window` 手工锚定居中（`anchor(CENTER_CENTER)`），egui 0.36 有现成的 `egui::Modal`（自动居中、遮罩、阻止背景交互）。

**目标形状**（craft-launcher 同款）：

```rust
enum Dialog {
    Threats { which: ScanKind },
    About,
    // 未来扩展：ConfirmDelete { .. }、ForceQuarantine { .. }
}

// App（或 AppCore）上唯一的弹窗状态：
dialog: Option<Dialog>,

// 每帧渲染；渲染函数返回"是否请求关闭"，由调用方统一收口：
fn threats_dialog(ctx: &egui::Context, state: &mut ScanPageState) -> bool {
    let mut open = true;
    egui::Modal::new(egui::Id::new("threats_modal"))
        .show(ctx, |ui| { /* 原 threats_modal 内容，open=false 时关闭 */ });
    open
}
```

收益：`Option<Dialog>` 编译期保证同时最多一个弹窗；删掉 `about_dialog` 的全局状态；每个弹窗有唯一 `egui::Id`（尺寸记忆互不干扰，craft-launcher 用 `Dialog::key()` 生成）。

## 4. 重绘调度决策表（两个项目的实测策略合并）

| 事件类型 | 正确策略 | CLV3000 现状 |
|---|---|---|
| 高频进度（`ScanningFile`/`FileScanned`，全盘扫描时每秒成百上千条） | **不要**逐事件唤醒；页面动画定时器统一驱动 | ✅ `scan_page` 的 `request_repaint_after(33ms)` |
| 后台有任务但窗口隐藏/页面不可见 | 低频保活定时器（几百 ms），只为捞完成事件 | ✅ `logic` 的 250ms（可见）/500ms（隐藏） |
| 低频结果（DB 更新完成、资源采样到达、强制隔离完成） | 事件点直接 `ctx.request_repaint()` | ✅ `virus_db::start_update`、`sysmon::spawn`、`start_force_quarantine` 已做 |
| 扫描 Finished | 事件唤醒更即时；定时器也可接受 | ⏸ 目前靠定时器（≤250ms 延迟，无感；若未来要"完成即弹 toast"再补唤醒） |
| 外部输入（托盘/菜单/第二实例 IPC） | 阻塞转发线程 + 唤醒 | ✅ `wakeup.rs`（模块注释即完整论证） |
| 闲置 | 零重绘，事件循环真睡死 | ✅（底线：不许新增常驻心跳） |

## 5. 分层纪律（单 crate 版的"UI 不下沉"）

craft-launcher 用 Cargo workspace 强制分层；CLV3000 用模块纪律达成同一效果。规则：

- `src/scan/`、`src/quarantine.rs`、`src/config.rs`、`src/paths.rs`、`src/clamav_info.rs`：**禁止** import egui/eframe（现状已核实为零依赖，保持）。
- UI 层（`src/app/**`、`widgets.rs`、`theme.rs`、`icons.rs`、`about_dialog.rs`）可以读业务类型（`ScanEvent`/`Threat`/`AppConfig`），但**不得**直接 spawn clamscan/freshclam/提权进程——一律调 core 状态对象的方法（`start`/`poll`），线程细节封在里面。
- 引擎 → UI 的唯一合法通道：mpsc 事件 + 完成点 `request_repaint`。反向（UI 把 ctx 带进后台线程）只允许"clone `Context` 传进去、且只用来 `request_repaint`"这一种用法（`virus_db`/`sysmon` 即此）。

## 6. ⏸ 暂缓项（记录触发条件，避免重新论证）

- **i18n**：触发 = 确定做多语言。路线：英文串即 key、TSV catalog `include_str!` 编译期内置、每语言复数规则函数、`tn!` 宏；**CJK 字体必须运行时查系统字体**（craft-launcher 不打包 NotoSansCJK 的理由同样适用于本项目的 portable/小体积定位）。硬前提：当前 `default_fonts` 字体集不含 CJK 字形，中文会渲染成方框——做中文支持前先解决这个。
- **Tokens / 运行时主题切换**：触发 = 设计稿出浅色版。做法可抄 `Tokens::get(ctx)`（主题存 ctx data，widget 自取）。
- **Cargo workspace 拆 crate**：触发 = 出现第二个 shell（wasm/CLI）。当前单 crate + §5 纪律等价，拆了只多维护成本。
- **`Host` trait**（托盘/通知/退出的平台抽象）：触发 = 非桌面 shell，或 UI 测试需要模拟平台行为。

## 7. 🔶 Never crash（本项目比一般 egui 应用更致命）

`profile.release` 是 `panic = "abort"`（Cargo.toml 已核实）：UI 线程任何 panic = 进程当场蒸发，正在展示的扫描结果/隔离操作全部丢失。把 craft-launcher 的 "Never crash" 规则收紧为本项目规则：

1. 运行时数据（文件/注册表/锁/网络/纹理）禁止 `unwrap`/`expect`/`panic!`，一律 `Result` → `app.toast(e)`。正面榜样：`theme::paint_dotted_background` 收 `Option<&TextureHandle>`（其注释写明了为什么不能硬取）。
2. `unwrap`/`expect` 只允许"启动期顺序不变量"（如 main 里 `wakeup::init` 必先于 `App::new` 这类由 main 保证的顺序）。
3. UI 路径上的 `Mutex::lock().unwrap()` 遇锁中毒即炸；新代码用 `.unwrap_or_else(|e| e.into_inner())` 或直接消掉共享 Mutex（`wakeup.rs` 的静态队列是历史存量，重构时再治）。

## 8. 检查清单（动 UI 代码前过一遍）

1. 新页面？状态：跨帧的进 `AppCore`，单帧的留局部；渲染 = `pages/` 里拿 `&mut App` 的自由函数；引擎层不许 import egui（§5）。
2. 新弹窗？加 `Dialog` 变体 + 唯一 `egui::Id` + `egui::Modal`；**禁止**新增模块级全局 bool（§3）。
3. 新后台任务？mpsc + 完成点 `request_repaint()`（抄 `virus_db::start_update` / `sysmon::spawn` 的样子）；高频进度交给页面动画定时器，不逐事件唤醒（§4）。
4. 新增 `unwrap`/`expect`？对照 §7 三条。
5. 写 UI 测试？kittest 驱动**真实**组件（§2）；禁止新增"复刻结构"式测试；GPU 相关用 `--test-threads=1`。
6. 闲置零重绘是底线：不许引入常驻 `request_repaint_after` 心跳。

## 参考（craft-launcher 侧索引）

- 仓库：`github.com/storytold/craft-launcher`（egui/eframe 0.36.2；其 `AGENTS.md` 是工程规则源头：Never crash / no unsafe / 颜色只用 token / 新 UI 从 widgets.rs 搭 / 动画尊重系统减少动态偏好）
- UI：`crates/ui-egui/src/{app,views,dialogs,theme,widgets,i18n}.rs`
- 协议：`crates/core`（`Command`/`Event` 枚举 + `Backend` trait）
- 测试：`crates/ui-egui/tests/ui.rs`（kittest + Fake + 快照）、`src/i18n/tests.rs`（目录完整性）、按语言独立测试二进制（german.rs / chinese.rs）
