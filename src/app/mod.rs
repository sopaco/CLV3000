//! 主界面编排：`App` 实现 `eframe::App`，负责事件轮询、视口/生命周期对账、
//! 纹理资源管理，并把实际渲染分派给 `pages`（四个页面）与 `chrome`（标题栏/
//! 侧边栏/资源条）。业务状态（配置、扫描、病毒库）在 `core::AppCore`，
//! freshclam 子进程调用在 `freshclam`，跟渲染无关的小工具在 `util`。

mod chrome;
mod core;
mod freshclam;
mod pages;
mod util;

use crate::lifecycle::{Lifecycle, RunMode};
use crate::sysmon::{self, ResourceSample, SysMonHandle};
use crate::theme::{self, colors};
use crate::tray::Tray;
use crate::widgets::Toast;
use core::AppCore;
use eframe::egui;
use egui::ViewportCommand;
use std::time::Duration;
use tray_icon::TrayIconEvent;

/// 主窗口默认尺寸（与 `main.rs` 里 `with_inner_size` 一致）。
const MAIN_WINDOW_SIZE: [f32; 2] = [900.0, 600.0];
#[cfg(not(windows))]
const ABOUT_WINDOW_SIZE: [f32; 2] = [480.0, 472.0];
#[cfg(windows)]
const ABOUT_WINDOW_SIZE: [f32; 2] = [480.0, 428.0];

/// 从托盘唤回窗口后的「置顶倒计时」帧数。macOS 14+ 下单次 `activate()` 抢不到
/// 焦点，需要连续若干帧 `orderFrontRegardless` 才稳定。Windows
/// 下由 wakeup 线程在用户手势权限窗口内直接 `SetForegroundWindow`（主路径），
/// 这里只需 2 帧兜底：第 1 帧窗口可能还没真正可见（`Visible(true)` 是异步的），
/// 第 2 帧窗口已可见、`AttachThreadInput` 把它拉到前台。更多的帧会导致
/// `AttachThreadInput` 反复 attach/detach → 标题栏闪动 + 最终失焦。
///
/// 这是"最多重试几次"的上限，不是"每次都会跑满"——`bring_to_front()` 一旦发现
/// App/窗口状态已经符合预期就会提前收敛（见 `logic()` 里 `activate_countdown` 的
/// 使用处），这个上限只兜底真正需要多帧才能稳定下来的场景（托盘唤回、从最小化
/// 恢复——这两种场景系统确定不会自己处理好，必须我们主动纠正）。
#[cfg(target_os = "macos")]
const ACTIVATE_FRAMES: u8 = 12;
#[cfg(not(target_os = "macos"))]
const ACTIVATE_FRAMES: u8 = 2;

/// 「置顶倒计时」期间两次重试之间的间隔。托盘唤回/最小化恢复这两条路径用，
/// macOS 上单次 `activate()`/`orderFrontRegardless()` 不一定一次就稳，隔一小段
/// 时间再检查一次更保险。
const ACTIVATE_RETRY_INTERVAL_MS: u64 = 60;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Page {
    Dashboard,
    QuickScan,
    VirusDb,
    FullScan,
}

/// 托盘/菜单事件：窗口与纯托盘循环共用。
/// 事件来自 `wakeup` 模块的转发队列——转发线程阻塞在 tray-icon/muda 的全局
/// channel 上，事件到达时已经替我们 `request_repaint` 唤醒了 UI，这里只管排空。
/// 排空托盘事件队列。返回 `true` 表示用户通过托盘请求把窗口带到前台（含窗口已可见但被
/// 其它 App 遮挡、仅需置顶的情况）。
fn poll_tray_events(tray: &Tray, core: &mut AppCore, lifecycle: &mut Lifecycle) -> bool {
    let mut focus_requested = false;
    // 锁只在这一小段排空循环里持有，发送端（wakeup 转发线程）基本碰不到竞争。
    while let Ok(event) = crate::wakeup::tray_events().lock().unwrap().try_recv() {
        if let TrayIconEvent::DoubleClick { .. } = event {
            // 双击托盘：显示主窗口（同时清掉可能打开的关于层）。
            lifecycle.mode = RunMode::ShowWindow;
            lifecycle.about_open = false;
            lifecycle.about_standalone = false;
            focus_requested = true;
        }
    }

    while let Ok(event) = crate::wakeup::menu_events().lock().unwrap().try_recv() {
        let id = event.id();
        if id == &tray.ids.show {
            // 显示主窗口：清掉关于层，否则关于独占窗口会挡住主界面。
            lifecycle.mode = RunMode::ShowWindow;
            lifecycle.about_open = false;
            lifecycle.about_standalone = false;
            focus_requested = true;
        } else if id == &tray.ids.quick_scan {
            lifecycle.mode = RunMode::ShowWindow;
            lifecycle.about_open = false;
            lifecycle.about_standalone = false;
            core.page = Page::QuickScan;
            // 全盘扫描已经在跑时不要抢着启动闪电扫描（见 `AppCore::any_scan_running`
            // 的注释）——只切到闪电扫描页让用户看到当前状态，不触发新扫描。
            if !core.full.is_running() {
                core.quick.start(core.config.scan_removable_drives);
            }
            focus_requested = true;
        } else if id == &tray.ids.about {
            // 来自托盘的关于：只占整个窗口画关于页，不画主界面（about_standalone）。
            lifecycle.about_open = true;
            lifecycle.about_standalone = true;
            focus_requested = true;
        } else if id == &tray.ids.quit {
            lifecycle.mode = RunMode::Quit;
        }
    }
    focus_requested
}

pub struct App {
    core: AppCore,
    lifecycle: Lifecycle,
    tray: Option<Tray>,
    sysmon: Option<SysMonHandle>,
    last_sample: ResourceSample,
    toasts: Vec<Toast>,
    allow_exit: bool,
    /// 完整品牌图标的纹理（`icon_app.png`），"关于"区块用它显示 logo。
    app_icon_texture: Option<egui::TextureHandle>,
    /// 简化版图标的纹理（`icon_tray.png`），自绘标题栏左上角那个小图标用它
    /// （Windows 用系统标题栏，不加载）。
    #[cfg(not(windows))]
    titlebar_icon_texture: Option<egui::TextureHandle>,
    /// 点阵背景瓦片纹理（`theme::dotted_tile_image`），平铺整块背景用。跟其它
    /// 纹理一样按会话生命周期缓存，避免每帧重新生成（见 `theme::paint_dotted_background`
    /// 顶部注释——这原本是每帧几百个矢量圆重新 tessellate，现在只是一次纹理生成）。
    dotted_bg_texture: Option<egui::TextureHandle>,
    /// 主视口当前是否处于「隐藏到托盘」状态。eframe 会话全程存活（不再关闭重建），
    /// 关闭窗口改成 `Visible(false)` 隐藏视口，靠这个标志位在 `logic` 里把生命周期
    /// 模式（ShowWindow / TrayOnly）对齐到真实的视口可见性。
    window_hidden: bool,
    /// 从托盘唤回窗口后的「置顶倒计时」：macOS 14+ 下单次 `activate()` 抢不到焦点、
    /// 窗口不会自动浮到最前，需要在接下来若干帧里反复 `orderFrontRegardless()` 才能稳
    /// 定把窗口提到最前（见 `macos_reopen::bring_to_front`）。每帧递减，归零后停止。
    activate_countdown: u8,
    /// 当前窗口尺寸的「意图」：0 = 主窗口尺寸，1 = 关于独占窗口尺寸。只在意图变化
    /// 时才发 `InnerSize` 指令，避免每帧重置、干扰用户对主窗口的手动缩放。
    size_intent: u8,
    /// macOS：上一帧 `NSWindow::isMiniaturized` 是否为 true。用于区分「正在最小化」
    /// （egui 已标记 minimized 但原生窗口尚未 miniaturize）与「从 Dock 恢复」
    /// （原生已 deminiaturize 但 egui 仍标记 minimized）。
    #[cfg(target_os = "macos")]
    macos_was_miniaturized: bool,
    /// macOS：扫描运行期间持有的"别把我 App Nap 了"令牌，见
    /// `macos_reopen::ScanActivity`。`None` 表示当前没有扫描在跑（或不是
    /// macOS）；扫描开始时 `Some`，扫描结束时置回 `None`（`Drop` 触发
    /// `endActivity:`）。
    #[cfg(target_os = "macos")]
    scan_activity: Option<crate::macos_reopen::ScanActivity>,
}

/// 把 `icon_data::load_*_icon` 解出来的 `(rgba, w, h)` 传进 egui 的纹理系统，
/// 拿到一个能在 `egui::Image` 里直接用的 `TextureHandle`。
fn load_texture(
    ctx: &egui::Context,
    name: &str,
    (rgba, w, h): (Vec<u8>, u32, u32),
) -> egui::TextureHandle {
    let color_image = egui::ColorImage::from_rgba_unmultiplied([w as usize, h as usize], &rgba);
    ctx.load_texture(name, color_image, egui::TextureOptions::LINEAR)
}

impl App {
    /// `tray`：main.rs 里已经建好的托盘（可能因为初始化失败而是 `None`，此时
    /// 无托盘运行）。`start_tray_only`：`--tray-only` 命令行参数，决定初始
    /// 生命周期模式与主窗口是否一开始就隐藏。
    pub fn new(
        cc: &eframe::CreationContext<'_>,
        tray: Option<Tray>,
        start_tray_only: bool,
    ) -> Self {
        theme::apply(&cc.egui_ctx);
        // 注册当前会话的 egui Context，让 wakeup 转发线程 / sysmon 采样线程能在
        // 有事件（托盘点击、菜单、资源采样）时主动唤醒 UI——替代旧的定时心跳。
        crate::wakeup::register_ctx(&cc.egui_ctx);

        let lifecycle = Lifecycle::new(start_tray_only);
        let window_hidden = lifecycle.mode == RunMode::TrayOnly;

        Self {
            core: AppCore::new(),
            lifecycle,
            tray,
            sysmon: None,
            last_sample: ResourceSample::default(),
            toasts: Vec::new(),
            allow_exit: false,
            app_icon_texture: None,
            #[cfg(not(windows))]
            titlebar_icon_texture: None,
            dotted_bg_texture: None,
            window_hidden,
            activate_countdown: 0,
            size_intent: 0,
            #[cfg(target_os = "macos")]
            macos_was_miniaturized: false,
            #[cfg(target_os = "macos")]
            scan_activity: None,
        }
    }

    fn toast(&mut self, text: impl Into<String>) {
        self.toasts.push(Toast::new(text));
    }

    fn navigate(&mut self, page: Page) {
        self.core.page = page;
    }

    fn ensure_ui_resources(&mut self, ctx: &egui::Context) {
        if self.sysmon.is_none() {
            self.sysmon = Some(sysmon::spawn(ctx.clone()));
        }
        if self.app_icon_texture.is_none() {
            const LOGO_DISPLAY_PT: f32 = 90.0;
            self.app_icon_texture = Some(load_texture(
                ctx,
                "app_icon",
                crate::icon_data::load_app_icon_for_display(
                    LOGO_DISPLAY_PT,
                    ctx.pixels_per_point(),
                ),
            ));
        }
        #[cfg(not(windows))]
        if self.titlebar_icon_texture.is_none() {
            self.titlebar_icon_texture = Some(load_texture(
                ctx,
                "titlebar_icon",
                crate::icon_data::load_tray_icon(64),
            ));
        }
        if self.dotted_bg_texture.is_none() {
            self.dotted_bg_texture = Some(ctx.load_texture(
                "dotted_bg_tile",
                theme::dotted_tile_image(),
                egui::TextureOptions::LINEAR_REPEAT,
            ));
        }
    }

    /// 释放 GPU 纹理与资源监控，为纯托盘模式腾出内存。
    fn release_ui_resources(&mut self, ctx: &egui::Context) {
        self.sysmon.take();
        // 纹理句柄 drop 即可；随后 eframe 会话结束会释放 OpenGL 资源。
        let _ = ctx;
        self.app_icon_texture.take();
        #[cfg(not(windows))]
        self.titlebar_icon_texture.take();
        self.dotted_bg_texture.take();
        self.toasts.clear();
        self.last_sample = ResourceSample::default();
    }

    fn hide_to_tray(&mut self, ctx: &egui::Context) {
        // 释放 GPU 纹理与资源监控，但**不关闭** eframe 会话——eframe 事件循环（以及
        // 其背后的 AppKit / winit 消息泵）必须一直存活，托盘图标的菜单点击才能被
        // 系统正常投递。真正"关闭窗口"改成把视口藏起来（`Visible(false)`）。
        self.release_ui_resources(ctx);
        self.lifecycle.mode = RunMode::TrayOnly;
        // 立即发 `Visible(false)` 并置 `window_hidden`，而不是等下一帧 reconcile——
        // `Visible` 是异步指令（winit 在本帧结束才真正 `orderOut`），越早发越早生效。
        // 若拖到下一帧 reconcile 发，则下一帧 `ui()` 会在仍可见的窗口上因 `window_hidden`
        // 早退、画一帧纯背景色，用户就看到"关闭时闪一下"。立即发则 winit 在本帧结束就
        // 藏窗口，下一帧 `ui()` 跑在已隐藏窗口上，无可见闪烁。同时清零 `activate_countdown`，
        // 避免隐藏后倒计数继续 `orderFrontRegardless` 把窗口又拉可见（见 clv3000-platform-pitfalls）。
        // Accessory 策略由 reconcile 的"已隐藏"分支下一帧补上（窗口已藏，1 帧延迟无碍）。
        ctx.send_viewport_cmd(ViewportCommand::Visible(false));
        self.window_hidden = true;
        self.activate_countdown = 0;
    }

    fn poll_tray(&mut self) {
        let Some(tray) = self.tray.as_ref() else {
            return;
        };
        let tray_focus = poll_tray_events(tray, &mut self.core, &mut self.lifecycle);
        let next_mode = self.lifecycle.mode;

        if tray_focus {
            self.activate_countdown = self.activate_countdown.max(ACTIVATE_FRAMES);
        }

        if next_mode == RunMode::Quit {
            self.allow_exit = true;
        }
    }

    /// 把生命周期模式对齐到真实的视口可见性 + macOS 激活策略。eframe 会话全程存活，
    /// 所以这里只发 `Visible` / 激活策略指令，绝不 `Close`（除非用户真的点了退出）。
    ///
    /// 可见条件：`ShowWindow` 模式，或「关于」打开（无论是覆盖在主窗上、还是独占窗口）。
    /// 「关于」独占窗口时主视口必须可见——这正是来自托盘的关于会把窗口带出来的原因；
    /// 关闭关于后若来源是托盘、`mode` 仍是 `TrayOnly`，下一帧这里就会把视口重新藏起来，
    /// 不会残留主窗口。
    ///
    /// macOS 激活策略（见 src/macos_reopen.rs）：
    /// - 有窗口时 → `Regular`：正常 App，带 Dock 图标与前台菜单；
    /// - 隐藏到托盘时 → `Accessory`：菜单栏小工具模式，无 Dock 图标。这样托盘态下 App
    ///   根本不在 Dock 上，用户想要"关闭窗口后只留托盘、不必再占 Dock"的需求直接满足，
    ///   也彻底绕开了"winit 不处理 Dock 重新打开事件 → 点 Dock 唤不回窗口"的坑。
    fn reconcile_lifecycle(&mut self, ctx: &egui::Context) {
        let mode = self.lifecycle.mode;
        let about_open = self.lifecycle.about_open;
        let about_standalone = self.lifecycle.about_standalone;
        if mode == RunMode::Quit {
            self.allow_exit = true;
            ctx.send_viewport_cmd(ViewportCommand::Close);
            return;
        }
        let desired_visible = mode == RunMode::ShowWindow || about_open;
        if desired_visible {
            // 有窗口：必须是 Regular（Dock 图标 + 前台菜单），并确保窗口可见。
            #[cfg(target_os = "macos")]
            crate::macos_reopen::set_accessory(false);
            // 记下"本帧是否刚从隐藏变为可见"——这是"窗口被打开"的判定，用于统一居中。
            let just_shown = self.window_hidden;
            if just_shown {
                ctx.send_viewport_cmd(ViewportCommand::Visible(true));
                self.window_hidden = false;
                // 从托盘（隐藏）唤回：接下来若干帧反复把窗口提到最前
                // （Accessory→Regular 切换 + macOS 14 激活策略变化，单次 activate 不够）。
                self.activate_countdown = ACTIVATE_FRAMES;
            }
            // 关于独占窗口用较小的尺寸，主窗口用默认尺寸；只在「尺寸意图」变化时发
            // 指令，避免每帧都重置、干扰用户对主窗口的手动缩放。
            let intent = if about_open && about_standalone { 1 } else { 0 };
            let intent_changed = intent != self.size_intent;
            if intent_changed {
                let size = if intent == 1 {
                    ABOUT_WINDOW_SIZE
                } else {
                    MAIN_WINDOW_SIZE
                };
                ctx.send_viewport_cmd(ViewportCommand::InnerSize(size.into()));
                self.size_intent = intent;
            }
            // 统一居中：窗口刚从隐藏显示（"打开"），或尺寸意图变化（主窗↔关于窗切换）
            // 时，都把窗口挪到所在显示器正中央。两类触发都是边沿条件，用户拖动后停留
            // 在同尺寸窗口/同一可见周期内不会被反复拽回中心。
            if just_shown || intent_changed {
                let size = if intent == 1 {
                    ABOUT_WINDOW_SIZE
                } else {
                    MAIN_WINDOW_SIZE
                };
                // 无边框窗口 OuterPosition 即内容区左上角；取所在显示器的尺寸算居中。
                // 首次显示时若 winit 还没给出 monitor（--tray-only 启动后第一次唤回），
                // 这里 `None` 跳过，由 `NativeOptions::centered` 兜底。
                if let Some(monitor) = ctx.input(|i| i.viewport().monitor_size) {
                    let origin = ((monitor - egui::Vec2::from(size)) / 2.0).max(egui::Vec2::ZERO);
                    ctx.send_viewport_cmd(ViewportCommand::OuterPosition(origin.to_pos2()));
                }
            }
        } else if !self.window_hidden {
            // 进托盘态：先把视口藏起来，再切到 Accessory（离开 Dock）。
            ctx.send_viewport_cmd(ViewportCommand::Visible(false));
            self.window_hidden = true;
            self.activate_countdown = 0;
            #[cfg(target_os = "macos")]
            crate::macos_reopen::set_accessory(true);
        } else {
            // 已隐藏：确保每个隐藏周期都落到 Accessory（例如 `--tray-only` 启动即隐藏）。
            #[cfg(target_os = "macos")]
            crate::macos_reopen::set_accessory(true);
        }
    }

    fn poll_background(&mut self, ctx: &egui::Context) {
        let toasts = self.core.poll_background(ctx);
        for msg in toasts {
            self.toast(msg);
        }
    }

    /// macOS：把 egui 里陈旧的 `minimized` 标记与 `NSWindow` 真实状态对齐。
    #[cfg(target_os = "macos")]
    fn sync_macos_minimized_viewport(&mut self, ctx: &egui::Context) -> bool {
        if self.window_hidden {
            self.macos_was_miniaturized = false;
            return false;
        }
        let now_miniaturized = crate::macos_reopen::is_miniaturized();
        let stale_minimized = ctx.input(|i| i.viewport().minimized == Some(true));
        let restored_from_dock =
            stale_minimized && !now_miniaturized && self.macos_was_miniaturized;
        self.macos_was_miniaturized = now_miniaturized;
        if restored_from_dock {
            ctx.send_viewport_cmd(ViewportCommand::Minimized(false));
            ctx.request_repaint();
            return true;
        }
        false
    }
}

impl Drop for App {
    fn drop(&mut self) {
        crate::wakeup::unregister_ctx();
    }
}

impl eframe::App for App {
    /// eframe 默认 `clear_color` 是近黑、半透明的 `(12,12,12,180)`（为"窗口阴影在
    /// 浅色系统主题下不显得怪"设计）
    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        colors::BG_APP.to_normalized_gamma_f32()
    }

    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.poll_tray();

        self.reconcile_lifecycle(ctx);

        #[cfg(target_os = "macos")]
        if self.sync_macos_minimized_viewport(ctx) {
            // Dock 点回最小化窗口：与托盘唤回类似，连续几帧置顶更稳。
            self.activate_countdown = self.activate_countdown.max(ACTIVATE_FRAMES);
        }

        // 从托盘唤回后的几帧内，反复把窗口提到最前（见 macos_reopen::bring_to_front）。
        if self.activate_countdown > 0 {
            if !self.window_hidden {
                if crate::macos_reopen::bring_to_front() {
                    self.activate_countdown = 0;
                } else {
                    self.activate_countdown -= 1;
                }
            } else {
                self.activate_countdown = 0;
            }
        }

        self.poll_background(ctx);

        if let Some(sysmon) = &self.sysmon
            && let Ok(sample) = sysmon.rx.try_recv()
        {
            self.last_sample = sample;
        }

        let scanning = self.core.quick.is_running() || self.core.full.is_running();
        #[cfg(target_os = "macos")]
        {
            if scanning && self.scan_activity.is_none() {
                self.scan_activity = Some(crate::macos_reopen::ScanActivity::begin(
                    "CLV3000 正在扫描，需要持续绘制进度动画",
                ));
            } else if !scanning && self.scan_activity.is_some() {
                self.scan_activity = None;
            }
        }

        // 重绘策略：尽量事件驱动，绝不让事件循环空转——这是老机器上常驻 CPU 的关键。
        // - 正在「置顶唤回」：`ACTIVATE_RETRY_INTERVAL_MS` 间隔快速收敛（多数场景
        //   1 帧内就靠 `bring_to_front()` 的提前收敛结束，只有少数场景才会跑满
        //   `ACTIVATE_FRAMES`）。
        // - 有扫描在跑：扫描页自己按 ~30fps 刷新（见 scan_page / progress_ring）；
        //   这里只留一个低频兜底（可见 250ms / 托盘 500ms），保证用户停在其它页面、
        //   或窗口隐藏时，扫描事件仍能被排空、结果被记录。
        // - 其余（闲置，无论窗口可见还是纯托盘）：**不安排任何定时重绘**。托盘/菜单
        //   点击由 wakeup 转发线程唤醒，底部资源条由 sysmon 采样线程按 1Hz 唤醒，
        //   Toast 有自己的短时定时器，键盘/鼠标输入本身就会触发重绘。
        let visible = ctx.input(|i| i.viewport().visible().unwrap_or(true));
        if self.activate_countdown > 0 {
            ctx.request_repaint_after(Duration::from_millis(ACTIVATE_RETRY_INTERVAL_MS));
        } else if scanning {
            ctx.request_repaint_after(Duration::from_millis(if visible { 250 } else { 500 }));
        }
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();

        if ctx.input(|i| i.viewport().close_requested()) {
            let is_quit = self.lifecycle.mode == RunMode::Quit || self.allow_exit;
            if is_quit {
                // 真正退出：不 CancelClose，让窗口关闭、eframe 会话结束、main 函数返回。
            } else if self.lifecycle.about_open {
                ctx.send_viewport_cmd(ViewportCommand::CancelClose);
                let mode = self.lifecycle.mode;
                self.lifecycle.about_open = false;
                self.lifecycle.about_standalone = false;
                // 来自托盘的关于（TrayOnly）连窗口一起藏（hide_to_tray 立即发 Visible(false)），
                // 否则要等下一帧 reconcile 才藏，中间一帧 ui() 会画纯背景色 → 闪一下。
                if mode == RunMode::TrayOnly {
                    self.hide_to_tray(&ctx);
                }
                return;
            } else {
                ctx.send_viewport_cmd(ViewportCommand::CancelClose);
                self.hide_to_tray(&ctx);
                return;
            }
        }

        let about_open = self.lifecycle.about_open;
        let about_standalone = self.lifecycle.about_standalone;

        // 隐藏到托盘（且没开关于）时整窗不绘制——既无可见内容，也避免出现"关掉关于
        // 窗却留下主窗口"的错觉。
        if self.window_hidden && !about_open {
            return;
        }

        // 来自托盘的关于：独占整个窗口画关于页，不画主界面——背后是深色主题底，
        // 看起来就是一张独立的关于窗口。关闭后由 reconcile 自动缩回托盘，不会残留主窗口。
        if about_open && about_standalone {
            crate::about_dialog::paint_about_fullscreen(ui);
            if crate::about_dialog::take_closed() {
                let mode = self.lifecycle.mode;
                self.lifecycle.about_open = false;
                self.lifecycle.about_standalone = false;
                // 来自托盘的关于（mode=TrayOnly）：连窗口一起藏；主窗可见时
                // （mode=ShowWindow）只关关于层，主窗下一帧自然接管。
                if mode == RunMode::TrayOnly {
                    self.hide_to_tray(&ctx);
                }
            }
            return;
        }

        self.ensure_ui_resources(&ctx);
        self.toasts.retain(|t| !t.expired());

        #[cfg(not(windows))]
        if let Some(tex) = self.titlebar_icon_texture.clone() {
            chrome::title_bar(ui, &ctx, &tex, self);
        }

        egui::Panel::bottom("resource_bar")
            .exact_size(50.0)
            .resizable(false)
            // Panel 默认会在边缘画一条分割线，用的是主题里偏亮的 noninteractive
            // stroke，在深色底上显得很突兀（白边）——设计稿里几个区域之间基本靠
            // 背景色深浅本身区分，没有这种硬分割线，所以统一关掉。
            .show_separator_line(false)
            .frame(
                egui::Frame::default()
                    .fill(colors::BG_APP)
                    .inner_margin(egui::Margin::symmetric(20, 10)),
            )
            .show(ui, |ui| chrome::resource_bar(ui, self.last_sample));

        egui::Panel::left("sidebar")
            .exact_size(64.0)
            .resizable(false)
            .show_separator_line(false)
            .frame(egui::Frame::default().fill(colors::BG_SIDEBAR))
            .show(ui, |ui| chrome::sidebar(ui, &ctx, self));

        let dotted_bg_texture = self.dotted_bg_texture.clone();
        egui::CentralPanel::default()
            .frame(egui::Frame::default().fill(colors::BG_APP))
            .show(ui, |ui| {
                theme::paint_dotted_background(
                    ui.painter(),
                    ui.max_rect(),
                    dotted_bg_texture.as_ref(),
                );
                let page = self.core.page;
                match page {
                    Page::Dashboard => pages::dashboard_page(ui, &ctx, self),
                    Page::QuickScan => pages::quick_scan_page(ui, self),
                    Page::VirusDb => pages::virus_db_page(ui, self),
                    Page::FullScan => pages::full_scan_page(ui, self),
                }
            });

        crate::widgets::show_toasts(&ctx, &self.toasts);

        // 主窗内打开的关于（当前无入口，预留）：覆盖在主界面之上的居中模态。
        if about_open && !about_standalone {
            crate::about_dialog::paint_about_modal(&ctx);
        }
    }
}
