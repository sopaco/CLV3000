//! 托盘图标 + 右键菜单：显示主窗口 / 闪电扫描 / Optimize PC / 关于 / 退出。
//!
//! 集成方式说明：按 tray-icon 官方文档推荐，正规做法是用
//! `TrayIconEvent::set_event_handler` 配合 winit 的 `EventLoopProxy` 把事件转发进
//! 事件循环，这样点击托盘能立刻唤醒窗口。但 eframe 把 winit 的 `EventLoop` 封装在了
//! 内部，拿不到 `EventLoopProxy`。所以这里走 channel 路线：托盘/菜单事件都进
//! `TrayIconEvent::receiver()` / `MenuEvent::receiver()` 内置的 channel，由
//! `src/wakeup.rs` 的转发线程**阻塞**等待、事件到达即调 `egui::Context::request_repaint`
//! 唤醒 UI（egui-winit 内部有现成的 EventLoopProxy 通道），`App` 在帧里从转发队列
//! `try_recv` 消费。相比旧的 100ms 定时轮询，响应更快且闲置时事件循环真正睡死。

use muda::{Menu, MenuId, MenuItem, PredefinedMenuItem};
use tray_icon::{Icon, TrayIcon, TrayIconBuilder};

pub struct TrayMenuIds {
    pub show: MenuId,
    pub quick_scan: MenuId,
    pub optimize_pc: MenuId,
    pub about: MenuId,
    pub quit: MenuId,
}

pub struct Tray {
    /// 必须一直持有，Drop 之后托盘图标会立刻消失——字段本身不需要被读取。
    #[allow(dead_code)]
    pub icon: TrayIcon,
    pub ids: TrayMenuIds,
}

/// `icon_rgba`/`icon_w`/`icon_h`：调用者传进来的托盘图标（RGBA8），由 `icon_data`
/// 解码内嵌的正式美术图标得到，这里不再自己生成占位图标。
pub fn build(icon_rgba: Vec<u8>, icon_w: u32, icon_h: u32) -> anyhow::Result<Tray> {
    let show_item = MenuItem::new("Show Main Window", true, None);
    let quick_item = MenuItem::new("Quick Scan", true, None);
    let optimize_item = MenuItem::new("Optimize PC", true, None);
    let about_item = MenuItem::new("About", true, None);
    let quit_item = MenuItem::new("Quit", true, None);

    let ids = TrayMenuIds {
        show: show_item.id().clone(),
        quick_scan: quick_item.id().clone(),
        optimize_pc: optimize_item.id().clone(),
        about: about_item.id().clone(),
        quit: quit_item.id().clone(),
    };

    let menu = Menu::new();
    menu.append_items(&[
        &show_item,
        &quick_item,
        &optimize_item,
        &PredefinedMenuItem::separator(),
        &about_item,
        &quit_item,
    ])?;

    let icon = Icon::from_rgba(icon_rgba, icon_w, icon_h)?;

    let icon = TrayIconBuilder::new()
        .with_menu(Box::new(menu))
        .with_icon(icon)
        .with_tooltip("CLV3000 - Lightweight Security")
        .build()?;

    Ok(Tray { icon, ids })
}
