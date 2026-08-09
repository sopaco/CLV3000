# CLV3000

简约 Windows 手动杀毒工具，纯 Rust 实现，面向老旧机器（不做实时防护，只有手动的闪电扫描/全盘扫描）。

技术方案见 `/Users/bjsttlp485/.claude/plans/windows-1-dll-clamav-cli-2-exe-dll-clam-pure-pie.md`，本文档只讲"怎么把它跑起来"。

## 功能

- **闪电扫描**：枚举所有运行中进程加载的模块（含 DLL），去重后交给 ClamAV 扫描。
- **全盘扫描**：枚举本地固定磁盘上的可执行文件（`.exe .dll .sys .scr .com .cpl .ocx .drv`），交给 ClamAV 扫描。
- **病毒库**：查看内置病毒库状态，手动触发 `freshclam` 更新。
- **资源监控**：全局底部状态条，实时显示 CPU / 内存占用。
- **托盘图标**：双击打开主窗口，右键菜单（显示主窗口 / 闪电扫描 / 关于 / 退出）。关闭按钮默认最小化到托盘。

检出的威胁只报告，"隔离"按钮当前会提示"后续版本提供"（不做任何文件系统写操作）；"忽略"会把该文件+病毒名记入配置，之后的扫描不再重复提示。

## 目录结构

```
src/
  main.rs          程序入口：单实例锁、托盘初始化、窗口创建
  app.rs           四个页面 UI + 自绘标题栏 + 状态轮询
  theme.rs         配色 / 全局样式 / 圆点背景
  icons.rs         手绘矢量图标（不依赖图标字体）
  icon_data.rs     程序图标（RGBA 光栅化生成，占位美术）
  widgets.rs       圆环进度、状态胶囊、威胁卡片、Toast
  tray.rs          托盘图标 + 右键菜单
  sysmon.rs        资源监控后台线程
  single_instance.rs  单实例 Mutex
  config.rs        配置持久化（%APPDATA%\CLV3000\config.toml）
  localtime.rs     极简本地时间（只依赖 GetLocalTime，不引入 chrono）
  paths.rs         定位内置 clamav 目录 / 配置目录
  scan/
    mod.rs         共享事件类型
    engine.rs      clamscan.exe 子进程封装
    quick_scan.rs  闪电扫描：进程/模块枚举
    full_scan.rs   全盘扫描：磁盘枚举 + 目录遍历
```

## 依赖的第三方 ClamAV（必须手动补齐，不含在代码仓库里）

程序假设**可执行文件同目录**下有这样一份便携版 ClamAV for Windows：

```
<exe 所在目录>\clamav\
  clamscan.exe
  freshclam.exe
  *.dll                （libclamav 及其依赖）
  database\
    *.cvd / *.cld       （病毒库文件）
```

获取方式：从 [ClamAV 官网](https://www.clamav.net/downloads) 下载 Windows 版安装包，装到一台机器上后，把上述几类文件拷出来即可（也可以用官方提供的免安装/portable 构建）。首次运行前建议手动跑一次
`freshclam.exe --datadir=clamav\database` 把病毒库拉到最新，程序内"病毒库"页的"手动更新病毒库"按钮之后也能触发同样的操作。

没有这份目录时，程序不会崩溃，会在扫描页面提示"找不到扫描引擎"。

## 构建

### 在 Windows 上（推荐，最终一定要在真机验证）

```bash
cargo build --release
```

产物在 `target\release\clv3000.exe`。把它和上面那份 `clamav\` 目录放在同一个文件夹里再运行。

### 从 macOS/Linux 交叉编译成 Windows 版

需要本机装好 `mingw-w64`（macOS: `brew install mingw-w64`），仓库自带的 `.cargo/config.toml` 已经配好了对应 target 的 linker，显式指定 target 即可：

```bash
cargo build --release --target x86_64-pc-windows-gnu
```

产物在 `target/x86_64-pc-windows-gnu/release/clv3000.exe`。这条路径能编译、能链接，但**交叉编译出来的是 Windows 可执行文件，在 macOS/Linux 上运行不了**，托盘、进程枚举、单实例锁这些 Win32 相关行为最终必须在真实 Windows 上跑一遍才算数——见下面"验证清单"。

## macOS / Linux 开发预览（mock 模式）

想在非 Windows 的开发机上直接看 UI 和交互效果，不用交叉编译、不用等 Windows 机器，直接跑本机原生版本就行：

```bash
cargo run
```

不用加任何 `--target`（`.cargo/config.toml` 已经不强制默认 target 了，会自动编译成当前机器的原生程序）。这条路径下 `windows` crate 整个不会被拉进依赖图（在 `Cargo.toml` 里被限定成只在 `cfg(windows)` 时才依赖），所有 Win32 专属逻辑都换成了同接口的 mock 实现：

| 模块 | Windows 上是什么 | 非 Windows 上 mock 成什么 |
|---|---|---|
| 闪电扫描 | `Toolhelp32` 枚举真实进程/DLL | 假装有 342 个进程，模块列表程序生成，数量级和真机接近 |
| 全盘扫描 | 枚举真实磁盘 + 遍历文件 | 生成约 3000 个假路径喂给流程，不碰真实文件系统 |
| 扫描引擎 | 调 `clamscan.exe` 子进程 | 按小延迟消费路径、模拟 OK/FOUND，两次运行交替出"未发现威胁"/"发现威胁"，方便同时看到两种结果页样式 |
| 单实例锁 | 具名 Mutex | 直接放行，不限制多开 |
| 本地时间 | `GetLocalTime` | 用 `SystemTime` 自己算日期（没查时区表，是 UTC，不影响"今天/X月X日"这种文案） |
| 病毒库状态 | 真的检查 `clamscan.exe`/`freshclam.exe` 是否存在 | 一律报"就绪"；点"手动更新"会睡 1.2 秒模拟联网后返回成功 |
| 配置持久化、托盘、窗口 UI | —— | **是真的**，跟 Windows 上一样走 `%APPDATA%`/`~/Library/...`、tray-icon、eframe，不是 mock |

想看"发现威胁"的红色结果页和威胁卡片交互（隔离按钮的提示、忽略按钮的效果），多点几次"重新扫描"就行——mock 引擎每跑一次结果就翻一面。

这条路径只用来看 UI/走交互流程，报出来的进程数、文件路径、扫描结果都是假的，不代表任何真实安全状态。

## 验证清单（对应技术方案第 7 节，务必在 Windows 上过一遍）

1. 把 `clamscan.exe`/`freshclam.exe`/DLL/`database\` 放进 `clamav\` 子目录，双击 `clv3000.exe`。
2. 用 [EICAR 测试文件](https://en.wikipedia.org/wiki/EICAR_test_file)（标准杀毒软件自测文件，内容公开、非真实病毒）放到桌面，跑一次全盘扫描，确认能检出并在 UI 里展示。
3. 闪电扫描：确认能看到进程数/文件数统计，扫描完成后能看到用时和已扫描文件数。
4. 病毒库页：点"手动更新病毒库"，确认能联网更新（需要 `freshclam.exe` 存在）。
5. 托盘：双击打开主窗口；右键菜单四项都能正常工作；点窗口关闭按钮应该只是隐藏到托盘（任务管理器里进程还在）；托盘"退出"才真正结束进程。
6. 多开一次 `clv3000.exe`，确认第二个实例直接退出（单实例锁生效）。
7. 任务管理器交叉核对底部状态条的 CPU/内存数值是否合理。
8. 扫描过程中点"取消扫描"，确认 `clamscan.exe` 子进程被结束、UI 状态正常复位。

## 已知的简化/占位项

- **程序图标**是用 `icon_data.rs` 里的一段代码按坐标光栅化生成的盾牌轮廓，不是正式美术资源；有正式图标后直接在 `main.rs`/`tray.rs` 里换成从文件加载即可。
- **"隔离"按钮**只弹提示，不做任何文件操作（详见技术方案的产品决策）。
- **托盘事件轮询**：受 eframe 封装限制，托盘/菜单点击不是靠系统消息立即唤醒界面，而是每帧（含窗口隐藏时，通过 `request_repaint_after(250ms)` 维持轮询）去查一次事件队列，最坏延迟在几百毫秒量级，实测如果觉得响应变慢，可以调小 `app.rs` 里那个 250ms。
- **病毒库更新**目前是纯手动触发，没有自动定时更新。
