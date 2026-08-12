// 发布版不弹黑色命令行窗口；调试时想看日志可以临时注释掉这一行，用 `cargo run` 直接跑。
#![windows_subsystem = "windows"]

mod about_dialog;
mod app;
mod clamav_info;
mod config;
mod icon_data;
mod icons;
mod lifecycle;
mod localtime;
mod macos_reopen;
mod paths;
mod scan;
mod single_instance;
mod sysmon;
mod theme;
mod tray;
mod wakeup;
mod widgets;

use lifecycle::parse_start_tray_only;

fn main() {
    if !single_instance::acquire() {
        single_instance::notice_already_running();
        return;
    }

    // 事件驱动的 UI 唤醒通道（托盘/菜单事件 → 转发线程 → request_repaint）。
    // 必须在托盘创建、eframe 会话启动之前初始化。
    wakeup::init();

    let start_tray_only = parse_start_tray_only();

    let (win_icon_rgba, win_icon_w, win_icon_h) = icon_data::load_tray_icon(128);
    let (tray_icon_rgba, tray_icon_w, tray_icon_h) = icon_data::load_tray_icon(32);

    let tray = match tray::build(tray_icon_rgba, tray_icon_w, tray_icon_h) {
        Ok(t) => Some(t),
        Err(e) => {
            eprintln!("Failed to initialize tray icon; running without system tray: {e}");
            None
        }
    };

    let window_icon = egui::IconData {
        rgba: win_icon_rgba,
        width: win_icon_w,
        height: win_icon_h,
    };

    // `eframe::run_native` 只调用这一次，阻塞到用户真正退出（托盘菜单"退出"）
    // 才返回——不像早期版本那样在一个 `loop` 里按 `RunMode` 反复销毁/重建
    // eframe 会话（那样做在 macOS 上会让托盘事件投递失效）。窗口可见性、
    // 隐藏到托盘、关于覆盖层全部由 `App::logic`/`App::ui` 内部按生命周期模式
    // 对账，这里不需要关心。`AppCore`/`Lifecycle` 也因此不再需要
    // `Rc<RefCell<_>>` 跨会话共享，直接在 `App::new` 内部构造、由 `App` 全程
    // own。
    let native_options = eframe::NativeOptions {
        viewport: build_viewport(window_icon)
            // `--tray-only` 启动时不闪一下主窗口：初始就隐藏，交由 `App::logic`
            // 维持托盘态。
            .with_visible(!start_tray_only),
        centered: true,
        ..Default::default()
    };

    let _ = eframe::run_native(
        "CLV3000",
        native_options,
        Box::new(move |cc| Ok(Box::new(app::App::new(cc, tray, start_tray_only)))),
    );
}

/// 按平台配置窗口装饰：Windows 用系统标题栏（避免无边框时客户区顶部"幽灵标题栏"
/// 导致鼠标坐标与 egui 绘制差一个标题栏高度）；macOS 继续自绘标题栏。
fn build_viewport(window_icon: egui::IconData) -> egui::ViewportBuilder {
    // 最小值要 ≤ 关于独占窗口尺寸（ABOUT_WINDOW_SIZE，见 app/mod.rs），否则 winit
    // 会把关于窗口夹到这个最小值、缩不小、背后仍留大片黑底。高度故意跟
    // ABOUT_WINDOW_SIZE 的高度保持相等（不只是"≤"）：这样主窗口被手动缩到最小时，
    // 也不会比关于页需要的高度更矮——两处高度改动要一起改。
    //
    // 按平台区分：ABOUT_WINDOW_SIZE 在 macOS/Linux 和 Windows 上高度不一样
    // （Windows 用系统标题栏、不用像 macOS/Linux 那样在 InnerSize 里额外留自绘
    // 标题栏的高度，见 app/mod.rs 里 `ABOUT_WINDOW_SIZE` 的注释），这里的最小高度
    // 必须跟对应平台那份保持一致，不能两个平台共用同一个数字。
    #[cfg(not(windows))]
    const MIN_INNER_SIZE: [f32; 2] = [440.0, 472.0];
    #[cfg(windows)]
    const MIN_INNER_SIZE: [f32; 2] = [440.0, 428.0];

    let mut builder = egui::ViewportBuilder::default()
        .with_title("CLV3000")
        .with_inner_size([900.0, 600.0])
        .with_min_inner_size(MIN_INNER_SIZE)
        .with_resizable(true)
        .with_icon(window_icon);

    #[cfg(windows)]
    {
        builder = builder.with_decorations(true);
    }

    #[cfg(not(windows))]
    {
        // 无边框窗口：没有原生标题栏，标题栏由 egui 自绘（见 app/chrome.rs 的
        // `title_bar` 和 about_dialog.rs 的 `paint_about_fullscreen`）。这里**不要**加
        // `with_fullsize_content_view(true)`——它只在"有原生标题栏、内容延伸到标题栏
        // 之下"时有意义；无边框窗口加了它反而会让 winit 在 macOS 上把窗口框体与内容
        // 区错开一个标题栏高度，egui 画不到的那条缝隙就显示成"底部一块黑色区域"。
        builder = builder
            .with_decorations(false)
            .with_title_shown(false)
            .with_titlebar_shown(false)
            .with_titlebar_buttons_shown(false);
    }

    builder
}
