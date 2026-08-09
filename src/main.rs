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
mod paths;
mod scan;
mod single_instance;
mod sysmon;
mod theme;
mod tray;
mod tray_loop;
mod widgets;

use app::AppCore;
use lifecycle::{parse_start_tray_only, Lifecycle, RunMode};
use std::cell::RefCell;
use std::rc::Rc;

fn main() {
    if !single_instance::acquire() {
        return;
    }

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
            RunMode::TrayOnly => {
                if tray_slot.borrow().is_some() {
                    tray_loop::run(&core, &lifecycle, tray_slot.borrow().as_ref().unwrap());
                } else {
                    break;
                }
            }
            RunMode::AboutOnly => {
                about_dialog::show_standalone();
                let mut lc = lifecycle.borrow_mut();
                let resume = lc
                    .resume_after_about
                    .take()
                    .unwrap_or(RunMode::TrayOnly);
                lc.mode = resume;
            }
            RunMode::ShowWindow => {
                let native_options = eframe::NativeOptions {
                    viewport: build_viewport(egui::IconData {
                        rgba: window_icon.rgba.clone(),
                        width: window_icon.width,
                        height: window_icon.height,
                    }),
                    ..Default::default()
                };

                let core = Rc::clone(&core);
                let lifecycle = Rc::clone(&lifecycle);
                let tray_slot = Rc::clone(&tray_slot);

                let _ = eframe::run_native(
                    "CLV3000",
                    native_options,
                    Box::new(move |cc| {
                        Ok(Box::new(app::App::new(
                            cc,
                            core,
                            lifecycle,
                            tray_slot,
                        )))
                    }),
                );
            }
        }
    }
}

/// 按平台配置窗口装饰：Windows 用系统标题栏（避免无边框时客户区顶部"幽灵标题栏"
/// 导致鼠标坐标与 egui 绘制差一个标题栏高度）；macOS 开发预览继续自绘标题栏。
fn build_viewport(window_icon: egui::IconData) -> egui::ViewportBuilder {
    let mut builder = egui::ViewportBuilder::default()
        .with_title("CLV3000")
        .with_inner_size([900.0, 600.0])
        .with_min_inner_size([760.0, 520.0])
        .with_resizable(true)
        .with_icon(window_icon);

    #[cfg(windows)]
    {
        builder = builder.with_decorations(true);
    }

    #[cfg(not(windows))]
    {
        builder = builder
            .with_decorations(false)
            .with_title_shown(false)
            .with_titlebar_shown(false)
            .with_titlebar_buttons_shown(false)
            .with_fullsize_content_view(true);
    }

    builder
}
