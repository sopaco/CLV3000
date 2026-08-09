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

    // 窗口图标/托盘图标用简化版（icon_tray.png）：细节少，缩到任务栏/托盘那种
    // 小尺寸也认得清楚；带文字的完整版（icon_app.png）留给"关于"页那种画得大的地方。
    // 窗口图标可以大一点（任务栏/切换窗口时看得清），托盘图标按系统惯例给小图。
    let (win_icon_rgba, win_icon_w, win_icon_h) = icon_data::load_tray_icon(128);
    let (tray_icon_rgba, tray_icon_w, tray_icon_h) = icon_data::load_tray_icon(32);

    let tray = match tray::build(tray_icon_rgba, tray_icon_w, tray_icon_h) {
        Ok(t) => Some(t),
        Err(e) => {
            eprintln!("托盘图标初始化失败，将以无托盘模式运行：{e}");
            None
        }
    };

    let window_icon = egui::IconData {
        rgba: win_icon_rgba,
        width: win_icon_w,
        height: win_icon_h,
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
