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

use app::AppCore;
use lifecycle::{Lifecycle, RunMode, parse_start_tray_only};
use std::cell::RefCell;
use std::rc::Rc;

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

    let core = Rc::new(RefCell::new(AppCore::new()));
    let lifecycle = Rc::new(RefCell::new(Lifecycle::new(start_tray_only)));
    let tray_slot = Rc::new(RefCell::new(tray));

    loop {
        // 必须先拷贝 mode 再 match：`match lifecycle.borrow().mode` 会让不可变借用
        // 贯穿整个 match 分支（含阻塞的 `run_native`），App 里再 `borrow_mut` 会 panic。
        let mode = lifecycle.borrow().mode;
        match mode {
            RunMode::Quit => break,
            // ShowWindow / TrayOnly 都跑同一个 eframe 会话：窗口可见性、隐藏到托盘、
            // 关于覆盖层全部由 `App::logic` 内部根据模式对账，不再在这里反复
            // 销毁/重建 eframe（macOS 上重建会导致托盘事件投递失效）。
            RunMode::ShowWindow | RunMode::TrayOnly => {
                let native_options = eframe::NativeOptions {
                    viewport: build_viewport(egui::IconData {
                        rgba: window_icon.rgba.clone(),
                        width: window_icon.width,
                        height: window_icon.height,
                    })
                    // `--tray-only` 启动时不闪一下主窗口：初始就隐藏，交由 `logic`
                    // 维持托盘态。
                    .with_visible(!start_tray_only),
                    centered: true,
                    ..Default::default()
                };

                let core = Rc::clone(&core);
                let lifecycle = Rc::clone(&lifecycle);
                let tray_slot = Rc::clone(&tray_slot);

                let _ = eframe::run_native(
                    "CLV3000",
                    native_options,
                    Box::new(move |cc| Ok(Box::new(app::App::new(cc, core, lifecycle, tray_slot)))),
                );
            }
        }
    }
}

/// 按平台配置窗口装饰：Windows 用系统标题栏（避免无边框时客户区顶部"幽灵标题栏"
/// 导致鼠标坐标与 egui 绘制差一个标题栏高度）；macOS 继续自绘标题栏。
fn build_viewport(window_icon: egui::IconData) -> egui::ViewportBuilder {
    let mut builder = egui::ViewportBuilder::default()
        .with_title("CLV3000")
        .with_inner_size([900.0, 600.0])
        // 最小值要 ≤ 关于独占窗口尺寸（ABOUT_WINDOW_SIZE = [480,460]，见 app.rs），
        // 否则 winit 会把关于窗口夹到这个最小值、缩不小、背后仍留大片黑底。
        .with_min_inner_size([440.0, 460.0])
        .with_resizable(true)
        .with_icon(window_icon);

    #[cfg(windows)]
    {
        builder = builder.with_decorations(true);
    }

    #[cfg(not(windows))]
    {
        // 无边框窗口：没有原生标题栏，标题栏由 egui 自绘（见 app.rs 的 `title_bar`
        // 和 about_dialog.rs 的 `paint_about_fullscreen`）。这里**不要**加
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
