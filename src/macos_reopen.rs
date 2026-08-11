//! 跨平台的"窗口置顶 / 激活策略"适配层：
//!
//! - **macOS**：控制 App 的「激活策略（activation policy）」，实现"关闭窗口后只留托盘、
//!   不再占用 Dock"的行为。进托盘态时切到 `Accessory`（无 Dock 图标），唤回时切回
//!   `Regular` 并 `orderFrontRegardless` 强制置顶。
//! - **Windows**：窗口已可见但被其它程序盖住时，通过托盘重新唤回需要主动调
//!   `SetForegroundWindow` 把窗口拉到前台（winit/eframe 的 wakeup 转发是异步的，
//!   错过了用户手势的 foreground 权限窗口，不主动调就只闪任务栏）。
//! - **其它平台**：no-op stub，调用方无需写 `#[cfg]` 分支。

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

    /// 主窗口是否处于 macOS 原生「最小化到 Dock」状态（`NSWindow::isMiniaturized`）。
    ///
    /// egui-winit 在 macOS 上**运行时**不会刷新 `ViewportInfo::minimized`（避免
    /// `window.is_minimized()` 死锁，见 egui #3494），但 `Minimized(true)` 命令会
    /// 把 `minimized` 置为 `Some(true)`。用户从 Dock 点回窗口时，原生窗口已恢复、
    /// 而 egui 仍认为 minimized → `visible()` 为 false → `ui()` 整帧跳过，界面卡死。
    /// 用本函数读真实窗口状态，与 egui 缓存对比后补发 `Minimized(false)` 即可。
    pub fn is_miniaturized() -> bool {
        if let Some(mtm) = MainThreadMarker::new() {
            let app = NSApplication::sharedApplication(mtm);
            let windows = app.windows();
            let count = windows.count();
            for i in 0..count {
                if windows.objectAtIndex(i).isMiniaturized() {
                    return true;
                }
            }
        }
        false
    }

    /// 当前 App 是否为前台活跃应用（`NSApplication::isActive`）。
    pub fn is_app_active() -> bool {
        if let Some(mtm) = MainThreadMarker::new() {
            let app = NSApplication::sharedApplication(mtm);
            return app.isActive();
        }
        false
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

    /// wakeup 线程调用：macOS 的 NSApplication/NSWindow API 必须在主线程调用，
    /// wakeup 线程不是主线程，所以这里 no-op。macOS 的唤回完全由主线程的
    /// `bring_to_front` + `activate_countdown` 机制处理。
    pub fn set_foreground() {}
}

/// Windows：把本进程的可见顶层窗口拉到前台。
///
/// 用户切到别的程序后通过托盘重新唤回窗口时，Windows 不会自动把已有窗口提到前台
/// （eframe 的 wakeup 转发是异步的，错过了用户手势赋予的 foreground 权限窗口），
/// 需要主动 `SetForegroundWindow`。但 Windows 的前台锁定机制会让后台进程的
/// `SetForegroundWindow` 只闪任务栏、不真正置顶——用 `AttachThreadInput` 把当前
/// 线程的输入队列临时与前台线程 attach 在一起，骗过这个限制。
#[cfg(target_os = "windows")]
#[allow(dead_code)]
mod imp {
    use windows::core::BOOL;
    use windows::Win32::Foundation::{HWND, LPARAM};
    use windows::Win32::System::Threading::{
        AttachThreadInput, GetCurrentProcessId, GetCurrentThreadId,
    };
    use windows::Win32::UI::WindowsAndMessaging::{
        BringWindowToTop, EnumWindows, GetForegroundWindow, GetWindowThreadProcessId,
        SetForegroundWindow,
    };

    pub fn set_accessory(_accessory: bool) {}
    pub fn is_miniaturized() -> bool {
        false
    }
    pub fn is_app_active() -> bool {
        true
    }

    /// 从 wakeup 转发线程调用：用户刚点了托盘，此刻仍在系统赋予的 foreground 权限
    /// 窗口内，直接 `SetForegroundWindow` 即可，不需要 `AttachThreadInput`（后者
    /// 只对调用线程本身的输入队列生效，wakeup 线程不是窗口所属线程，用了也不对）。
    /// 若窗口当前是隐藏的（`Visible(false)`），`SetForegroundWindow` 会失败——
    /// 没关系，UI 线程的 `bring_to_front` 兜底会处理。
    pub fn set_foreground() {
        unsafe {
            let pid = GetCurrentProcessId();
            let _ = EnumWindows(Some(foreground_proc), LPARAM(pid as isize));
        }
    }

    /// 从 UI 线程（`logic`）调用：作为 `set_foreground` 的兜底。窗口已由
    /// `Visible(true)` 显示但可能仍不在前台时，用 `AttachThreadInput` 绕过
    /// 前台锁定把窗口拉到最前。
    pub fn bring_to_front() {
        unsafe {
            let pid = GetCurrentProcessId();
            let _ = EnumWindows(Some(raise_proc), LPARAM(pid as isize));
        }
    }

    /// `set_foreground` 的枚举回调：找到本进程第一个顶层窗口就调
    /// `SetForegroundWindow` 并停止枚举。不做 `IsWindowVisible` 检查——
    /// 窗口可能刚被 `Visible(true)` 还未真正可见，`SetForegroundWindow`
    /// 对不可见窗口只是 no-op（返回 false），无害。
    unsafe extern "system" fn foreground_proc(hwnd: HWND, lparam: LPARAM) -> BOOL {
        unsafe {
            let target_pid = lparam.0 as u32;
            let mut window_pid: u32 = 0;
            let _tid = GetWindowThreadProcessId(hwnd, Some(&mut window_pid));
            if window_pid != target_pid {
                return BOOL(1); // 不是本进程的窗口 → 继续枚举
            }
            let _ = SetForegroundWindow(hwnd);
            BOOL(0) // 已找到 → 停止枚举
        }
    }

    /// `bring_to_front` 的枚举回调：找到本进程第一个顶层窗口就 `raise_window` 并停止。
    unsafe extern "system" fn raise_proc(hwnd: HWND, lparam: LPARAM) -> BOOL {
        unsafe {
            let target_pid = lparam.0 as u32;
            let mut window_pid: u32 = 0;
            let _tid = GetWindowThreadProcessId(hwnd, Some(&mut window_pid));
            if window_pid != target_pid {
                return BOOL(1); // 不是本进程的窗口 → 继续枚举
            }
            raise_window(hwnd);
            BOOL(0) // 已找到 → 停止枚举
        }
    }

    /// 单个窗口的置顶逻辑：AttachThreadInput → BringWindowToTop → SetForegroundWindow。
    /// 不调 `ShowWindow(SW_RESTORE)`——窗口可见性由 winit/egui 管理（`Visible(true)`），
    /// 自行 `ShowWindow` 会让 winit 的窗口状态缓存失真。
    unsafe fn raise_window(hwnd: HWND) {
        unsafe {
            let foreground = GetForegroundWindow();
            if foreground == hwnd {
                return; // 已是前台窗口
            }
            // AttachThreadInput 把当前线程的输入队列临时与前台线程的 attach 在一起，
            // 绕过"只有前台进程才能 SetForegroundWindow"的限制。
            let fg_tid = if foreground.is_invalid() {
                0
            } else {
                GetWindowThreadProcessId(foreground, None)
            };
            let our_tid = GetCurrentThreadId();
            let attached = fg_tid != 0 && fg_tid != our_tid;
            if attached {
                let _ = AttachThreadInput(our_tid, fg_tid, true);
            }
            let _ = BringWindowToTop(hwnd);
            let _ = SetForegroundWindow(hwnd);
            if attached {
                let _ = AttachThreadInput(our_tid, fg_tid, false);
            }
        }
    }
}

/// 其它平台（Linux 等）：no-op stub，调用方无需写 `#[cfg]` 分支。
#[cfg(not(any(target_os = "macos", target_os = "windows")))]
#[allow(dead_code)]
mod imp {
    pub fn set_accessory(_accessory: bool) {}
    pub fn is_miniaturized() -> bool {
        false
    }
    pub fn is_app_active() -> bool {
        true
    }
    pub fn set_foreground() {}
    pub fn bring_to_front() {}
}

#[allow(unused_imports)]
pub use imp::*;
