// 发布版不弹黑色命令行窗口；调试时想看日志可以临时注释掉这一行，用 `cargo run` 直接跑。
#![windows_subsystem = "windows"]

mod app;
mod config;
mod fonts;
mod icon_data;
mod icons;
mod localtime;
mod paths;
mod scan;
mod single_instance;
mod sysmon;
mod theme;
mod tray;
mod widgets;

fn main() {
    if !single_instance::acquire() {
        // 已经有一个实例在跑了，直接退出。
        return;
    }

    let (icon_rgba, icon_w, icon_h) = icon_data::load_app_icon();

    let tray = match tray::build(icon_rgba.clone(), icon_w, icon_h) {
        Ok(t) => Some(t),
        Err(e) => {
            eprintln!("托盘图标初始化失败，将以无托盘模式运行：{e}");
            None
        }
    };

    let window_icon = egui::IconData {
        rgba: icon_rgba,
        width: icon_w,
        height: icon_h,
    };

    let native_options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("CLV3000")
            .with_inner_size([900.0, 600.0])
            .with_min_inner_size([760.0, 520.0])
            .with_decorations(false)
            .with_resizable(true)
            .with_icon(window_icon)
            // 我们自己画了标题栏/最小化/关闭按钮，这里把系统原生的那一套（尤其是
            // macOS 上即使 decorations(false) 也可能残留的一小条原生标题区/红绿灯
            // 按钮）也彻底关掉，避免和自绘的内容重叠、露出系统默认底色。
            .with_title_shown(false)
            .with_titlebar_shown(false)
            .with_titlebar_buttons_shown(false)
            .with_fullsize_content_view(true),
        ..Default::default()
    };

    let _ = eframe::run_native(
        "CLV3000",
        native_options,
        Box::new(|cc| Ok(Box::new(app::App::new(cc, tray)))),
    );
}
