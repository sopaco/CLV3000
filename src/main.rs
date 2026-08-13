// 发布版不弹黑色命令行窗口；调试时想看日志可以临时注释掉这一行，用 `cargo run` 直接跑。
#![windows_subsystem = "windows"]

mod about_dialog;
mod app;
mod autostart;
mod clamav_info;
mod config;
mod context_menu;
mod icon_data;
mod icons;
mod lifecycle;
mod localtime;
mod macos_reopen;
mod paths;
mod quarantine;
mod scan;
mod single_instance;
mod sysmon;
mod theme;
mod tray;
mod wakeup;
mod widgets;

use lifecycle::{parse_scan_path, parse_start_tray_only, InitialMode};

fn main() {
    // 右键菜单"用 CLV3000 扫描"/`--scan-path` 手动调试都可能在"已经有一个实例在跑"
    // 时启动第二个进程——这种情况不能像过去那样直接弹"已经在运行"就退出，得把
    // 扫描请求转发给正在跑的那个实例（见 `single_instance::forward_scan_request`）。
    // 所以要在 `acquire()` 判断之前先解析好这个参数。
    let scan_path_cli = parse_scan_path();

    if !single_instance::acquire() {
        if let Some(path) = &scan_path_cli {
            if !single_instance::forward_scan_request(path) {
                // 转发失败（极少数情况，比如具名事件/socket 都打不开）：退回旧行为，
                // 至少告诉用户"已经在运行"，而不是悄无声息什么都没发生。
                single_instance::notice_already_running();
            }
        } else {
            single_instance::notice_already_running();
        }
        return;
    }

    // 事件驱动的 UI 唤醒通道（托盘/菜单事件 → 转发线程 → request_repaint）。
    // 必须在托盘创建、eframe 会话启动之前初始化。
    wakeup::init();
    // 起扫描请求转发监听（见模块文档）。必须在这里、`eframe::run_native` 之前调用
    // ——Windows `--tray-only` 下进程可能长期停在下面的 `wait_in_tray` 里，不依赖
    // eframe 是否已经启动。
    single_instance::start_scan_request_listener();

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

    // `--tray-only` 启动时（Windows）在 eframe 之外空等托盘事件，完全不创建窗口——
    // 既不闪窗、也不占用 OpenGL 上下文内存，直到用户从托盘请求显示窗口/关于或退出。
    // 这是规避 eframe「首帧强制 set_visible(true)」闪窗的唯一办法（见
    // epi_integration.rs 的 post_rendering）：只要调了 `eframe::run_native`，窗口
    // 就一定会被创建并在首帧显示，`with_visible(false)` 拦不住。
    // macOS 的 tray-only 仍直接启动 eframe（隐藏），因为托盘事件投递依赖
    // NSApplication 事件循环，eframe 之外空等收不到托盘点击。
    let initial = match resolve_initial_mode(&tray, start_tray_only, scan_path_cli) {
        Some(i) => i,
        None => return, // tray-only 下用户从托盘退出
    };

    // `eframe::run_native` 只调用这一次，阻塞到用户真正退出（托盘菜单"退出"）
    // 才返回。窗口可见性、隐藏到托盘、关于覆盖层全部由 `App::logic`/`App::ui`
    // 内部按生命周期模式对账。
    //
    // About 也以隐藏姿态创建（with_visible(false)），由 `reconcile_lifecycle` 首帧
    // 按关于尺寸显示，避免先闪 900x600 主窗再缩到关于尺寸。
    let starts_hidden = matches!(initial, InitialMode::TrayOnly | InitialMode::About);
    let native_options = eframe::NativeOptions {
        viewport: build_viewport(window_icon).with_visible(!starts_hidden),
        // 隐藏启动时关闭居中，避免 eframe 初始化触发闪窗；可见启动正常居中。
        centered: !starts_hidden,
        ..Default::default()
    };

    let _ = eframe::run_native(
        "CLV3000",
        native_options,
        Box::new(move |cc| Ok(Box::new(app::App::new(cc, tray, initial)))),
    );
}

/// 决定 eframe 启动时的初始模式。
/// - 带 `--scan-path`（右键菜单"用 CLV3000 扫描"/手动调试）：不管 `--tray-only`，
///   直接进 `ScanPath` 模式显示窗口看扫描结果——用户显式要看一次扫描，不该被
///   silently 吞进托盘。
/// - 否则 `--tray-only` 启动时：
///   - Windows：在 eframe 之外空等托盘事件（`wait_in_tray`），直到用户请求显示
///     窗口/关于/退出，**或者**收到一个转发过来的扫描请求。返回 `None` 表示
///     用户从托盘退出，main 直接 return。
///   - macOS / 其它：仍启动 eframe（隐藏），因为托盘事件投递依赖 NSApplication
///     事件循环，eframe 之外空等收不到托盘点击。
fn resolve_initial_mode(
    tray: &Option<tray::Tray>,
    start_tray_only: bool,
    scan_path: Option<std::path::PathBuf>,
) -> Option<InitialMode> {
    if let Some(path) = scan_path {
        return Some(InitialMode::ScanPath(path));
    }
    if !start_tray_only {
        return Some(InitialMode::ShowWindow);
    }
    #[cfg(windows)]
    {
        if let Some(t) = tray.as_ref() {
            return wait_in_tray(t);
        }
        // 托盘初始化失败：没有托盘就无法从后台唤回，直接显示主窗口兜底。
        return Some(InitialMode::ShowWindow);
    }
    #[cfg(not(windows))]
    {
        let _ = tray;
        Some(InitialMode::TrayOnly)
    }
}

/// Windows tray-only 启动：在 eframe 之外跑 Win32 消息循环，不创建任何窗口。
/// 返回 `Some(mode)` 表示用户请求显示窗口（或关于/闪电扫描），`None` 表示退出。
///
/// **必须跑消息循环**：tray-icon/muda 在主线程创建消息窗口（`CreateWindowExW`），
/// 其 wndproc（`tray_proc`）只有 `GetMessage`/`PeekMessage`+`DispatchMessage` 时
/// 才会被调用来处理托盘点击和菜单命令。condvar 阻塞不行—— wndproc 不跑，
/// 托盘点不出菜单、事件不进 channel。
#[cfg(windows)]
fn wait_in_tray(tray: &tray::Tray) -> Option<InitialMode> {
    use windows::Win32::UI::WindowsAndMessaging::{
        DispatchMessageW, MSG, MsgWaitForMultipleObjectsEx, MWMO_INPUTAVAILABLE,
        MSG_WAIT_FOR_MULTIPLE_OBJECTS_EX_FLAGS, PeekMessageW, PM_REMOVE, QS_ALLINPUT,
        TranslateMessage, WM_QUIT,
    };
    use tray_icon::TrayIconEvent;

    let mut msg = MSG::default();
    loop {
        // 1. 排空转发到我们 channel 的托盘图标事件（双击 → 显示主窗口）。
        while let Ok(event) = crate::wakeup::tray_events().lock().unwrap().try_recv() {
            if matches!(event, TrayIconEvent::DoubleClick { .. }) {
                return Some(InitialMode::ShowWindow);
            }
        }
        // 排空菜单事件。
        while let Ok(event) = crate::wakeup::menu_events().lock().unwrap().try_recv() {
            let id = event.id();
            if id == &tray.ids.show {
                return Some(InitialMode::ShowWindow);
            } else if id == &tray.ids.quick_scan {
                return Some(InitialMode::QuickScan);
            } else if id == &tray.ids.about {
                return Some(InitialMode::About);
            } else if id == &tray.ids.quit {
                return None;
            }
        }
        // 排空转发过来的扫描请求（右键菜单"用 CLV3000 扫描"，已经有实例在跑时走
        // 这条路径——这个进程正停在 tray-only 的空等循环里，eframe 还没启动）。
        if let Ok(path) = crate::wakeup::scan_requests().lock().unwrap().try_recv() {
            return Some(InitialMode::ScanPath(path));
        }

        // 2. PeekMessage 非阻塞排空消息队列，DispatchMessage 让 tray-icon/muda 的
        //    wndproc 处理托盘点击和菜单命令（wndproc 发事件到内部 channel，
        //    转发线程再转发到我们的 channel，下一轮循环顶部排空）。
        unsafe {
            while PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE).as_bool() {
                if msg.message == WM_QUIT {
                    return None;
                }
                let _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        }

        // 3. 阻塞等待新消息到达（0 CPU）。30ms 超时兜底转发线程与主线程的竞态：
        //    dispatch 后转发线程可能还没把事件转发到我们的 channel，本轮排空落空，
        //    30ms 后超时醒来再排空一次。托盘点击延迟 30ms 无感。
        let _ = unsafe {
            MsgWaitForMultipleObjectsEx(
                None,
                30,
                QS_ALLINPUT,
                MSG_WAIT_FOR_MULTIPLE_OBJECTS_EX_FLAGS(MWMO_INPUTAVAILABLE.0),
            )
        };
    }
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
