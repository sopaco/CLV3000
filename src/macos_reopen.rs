//! macOS 专属：控制 App 的「激活策略（activation policy）」，实现"关闭窗口后只留托盘、
//! 不再占用 Dock"的行为。
//!
//! 背景：本项目把"关闭窗口"实现成 `ViewportCommand::Visible(false)`（窗口藏起来、
//! eframe 会话不销毁）。但这样 App 仍是一个 Regular（有 Dock 图标、有菜单栏）的 App，
//! 用户点 Dock 图标或 Cmd+Tab 仍会试图唤回窗口——而 winit 0.30 不处理 macOS 的 dock
//! 重新打开事件，导致窗口藏起来后点 Dock 唤不回来。
//!
//! 做法：进托盘态（窗口隐藏）时把激活策略切到 `Accessory`（菜单栏小工具模式，无 Dock
//! 图标、无前台菜单），这样 App 在托盘态下**根本不在 Dock 上**，既不占 Dock、点 Dock
//! 也自然无从谈起；需要时（托盘点"显示主窗口"）再切回 `Regular` 并显示窗口。这一招
//! 同时满足了用户的需求——"关闭窗口后只留托盘即可，不必再占 Dock"。

#[cfg(target_os = "macos")]
mod imp {
    use std::sync::atomic::{AtomicBool, Ordering};

    use objc2::MainThreadMarker;
    use objc2_app_kit::{NSApplication, NSApplicationActivationPolicy};

    /// 当前实际生效的"是否 accessory"。用来避免每帧都去调一次 `setActivationPolicy`
    /// （那会引起不必要的状态切换 / 闪烁），只在状态真的要变时才调。
    static IS_ACCESSORY: AtomicBool = AtomicBool::new(false);

    /// 设置 App 的激活策略：
    /// - `accessory = true`：切到 `Accessory`（无 Dock 图标、无前台菜单），用于托盘态；
    /// - `accessory = false`：切回 `Regular`（正常 App，有 Dock 图标），用于有窗口时。
    ///
    /// 必须在主线程（eframe 的 ui/logic 跑在主线程）调用。重复设置同一状态是 no-op。
    pub fn set_accessory(accessory: bool) {
        if IS_ACCESSORY.load(Ordering::Relaxed) == accessory {
            return;
        }
        if let Some(mtm) = MainThreadMarker::new() {
            let app = NSApplication::sharedApplication(mtm);
            let policy = if accessory {
                NSApplicationActivationPolicy::Accessory
            } else {
                NSApplicationActivationPolicy::Regular
            };
            let _ = app.setActivationPolicy(policy);
            IS_ACCESSORY.store(accessory, Ordering::Relaxed);
        }
    }

    /// 把窗口真正唤到最前（从托盘唤回主窗口 / 关于窗口时调用）。
    ///
    /// 仅靠 `NSApplication::activate()` 在 macOS 14+ 已经不够：苹果在 14 把
    /// `activateIgnoringOtherApps:` 标为 deprecated 且**完全不再生效**，普通 `activate()`
    /// 既抢不到键盘焦点、也不会把窗口自动浮到最前——这正是之前"Dock 图标出现了、窗口
    /// 却要手动点一下才出来"的根因。
    ///
    /// 真正的可靠路径是 `NSWindow::orderFrontRegardless()`：它**无视 App 当前是否激活、
    /// 是否处于 Accessory 模式**，直接把窗口设为可见并提到所有窗口最前。这里两者都做：
    /// `activate()` 让 App 成为前台 App（键盘焦点交给系统），`orderFrontRegardless()`
    /// 兜底强制窗口可见且置顶。
    pub fn bring_to_front() {
        if let Some(mtm) = MainThreadMarker::new() {
            let app = NSApplication::sharedApplication(mtm);
            app.activate();
            let windows = app.windows();
            let count = windows.count();
            for i in 0..count {
                let win = windows.objectAtIndex(i);
                win.orderFrontRegardless();
            }
        }
    }
}

/// 非 macOS 平台：提供同名空实现，调用方无需写 `#[cfg]` 分支。
#[cfg(not(target_os = "macos"))]
#[allow(dead_code)]
mod imp {
    pub fn set_accessory(_accessory: bool) {}
    pub fn bring_to_front() {}
}

pub use imp::*;
