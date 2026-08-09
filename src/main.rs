// 发布版不弹黑色命令行窗口；调试时想看日志可以临时注释掉这一行，用 `cargo run` 直接跑。
#![windows_subsystem = "windows"]

mod app;
mod config;
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

    // 窗口图标/托盘图标用简化版（icon_tray.png）：细节少，缩到任务栏/托盘那种
    // 小尺寸也认得清楚；带文字的完整版（icon_app.png）留给"关于"页那种画得大的地方。
    // 窗口图标可以大一点（任务栏/切换窗口时看得清），托盘图标按系统惯例给小图。
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

    let native_options = eframe::NativeOptions {
        viewport: build_viewport(window_icon),
        ..Default::default()
    };

    let _ = eframe::run_native(
        "CLV3000",
        native_options,
        Box::new(|cc| Ok(Box::new(app::App::new(cc, tray)))),
    );
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
        // 自绘标题栏/最小化/关闭按钮：把系统原生那一套（尤其是 macOS 上即使
        // decorations(false) 也可能残留的一小条原生标题区/红绿灯按钮）彻底关掉，
        // 避免和自绘内容重叠、露出系统默认底色。
        builder = builder
            .with_decorations(false)
            .with_title_shown(false)
            .with_titlebar_shown(false)
            .with_titlebar_buttons_shown(false)
            .with_fullsize_content_view(true);
    }

    builder
}
