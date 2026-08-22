//! 跨平台的"窗口置顶 / 激活策略"适配层：
//!
//! - **macOS**：控制 App 的「激活策略（activation policy）」，实现"关闭窗口后只留托盘、
//!   不再占用 Dock"的行为。进托盘态时切到 `Accessory`（无 Dock 图标），唤回时切回
//!   `Regular` 并 `orderFrontRegardless` 强制置顶。
//! - **Windows**：窗口已可见但被其它程序盖住时，通过托盘重新唤回需要主动调
//!   `SetForegroundWindow` 把窗口拉到前台（winit/eframe 的 wakeup 转发是异步的，
//!   错过了用户手势的 foreground 权限窗口，不主动调就只闪任务栏）。
//! - **其它平台**：no-op stub，调用方无需写 `#[cfg]` 分支。

// `set_foreground` 只在 Windows 侧的 `wakeup.rs` 里被调用（`#[cfg(target_os =
// "windows")]` 门控）；macOS 的唤回完全靠 `bring_to_front` + `activate_countdown`，
// 这里的 `set_foreground` 只是补齐跨平台统一接口的 no-op stub，在 macOS 构建下
// 永远不会被调用——跟下面 Windows/其它平台的 `imp` 模块一样需要 `allow(dead_code)`。
#[cfg(target_os = "macos")]
#[allow(dead_code)]
mod imp {
    use std::ptr::NonNull;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Once;

    use block2::RcBlock;
    use objc2::MainThreadMarker;
    use objc2_app_kit::{
        NSApplication, NSApplicationActivationPolicy, NSApplicationDidBecomeActiveNotification,
    };
    use objc2_foundation::{NSNotification, NSNotificationCenter};

    /// 当前实际生效的"是否 accessory"。用来避免每帧都去调一次 `setActivationPolicy`
    /// （那会引起不必要的状态切换 / 闪烁），只在状态真的要变时才调。
    static IS_ACCESSORY: AtomicBool = AtomicBool::new(false);
    /// 主窗口是否已缩到托盘（供 Dock 点击 / 切回 App 的通知回调判断）。
    static HIDDEN_TO_TRAY: AtomicBool = AtomicBool::new(false);
    static INSTALL_REOPEN_HANDLER: Once = Once::new();

    /// 隐藏到托盘时同步调用：立刻切 Accessory（去掉 Dock 图标）并标记托盘态。
    pub fn enter_tray_mode() {
        HIDDEN_TO_TRAY.store(true, Ordering::Relaxed);
        //  bust 缓存，确保 `set_accessory(true)` 真的会再打一次 AppKit。
        IS_ACCESSORY.store(false, Ordering::Relaxed);
        set_accessory(true);
    }

    /// 显示主窗口时同步调用：切回 Regular 并清掉托盘标记。
    pub fn leave_tray_mode() {
        HIDDEN_TO_TRAY.store(false, Ordering::Relaxed);
        IS_ACCESSORY.store(true, Ordering::Relaxed);
        set_accessory(false);
    }

    /// winit 0.30 不处理 Dock 图标点击；Accessory 未生效时 Dock 会留幽灵图标。
    /// 监听 `NSApplicationDidBecomeActive`，在托盘态被用户点 Dock / Cmd+Tab 切回时
    /// 走与 `--show` 相同的 `wakeup::push_show_request` 路径。
    pub fn install_reopen_handler() {
        INSTALL_REOPEN_HANDLER.call_once(|| {
            let Some(mtm) = MainThreadMarker::new() else {
                return;
            };
            let block = RcBlock::new(|_notification: NonNull<NSNotification>| {
                if HIDDEN_TO_TRAY.load(Ordering::Relaxed) {
                    crate::wakeup::push_show_request();
                }
            });
            let center = NSNotificationCenter::defaultCenter();
            // SAFETY: 通知名是 AppKit 常量；`block` 是 `'static` 的 RcBlock。
            let observer = unsafe {
                center.addObserverForName_object_queue_usingBlock(
                    Some(NSApplicationDidBecomeActiveNotification),
                    None,
                    None,
                    &block,
                )
            };
            // 观察者须活到进程结束；`NSNotificationCenter` 不会强引用 block 观察者。
            std::mem::forget(observer);
            let _ = mtm;
        });
    }

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
    ///
    /// 返回值给调用方（`app/mod.rs` 的 `activate_countdown` 循环）做"提前收敛"判断：
    /// `true` 表示**这一帧没做任何纠正动作**（App 已经 active、每个窗口都已经是
    /// key window）——调用方据此可以立刻把倒计时清零，不必再跑满 `ACTIVATE_FRAMES`
    /// 帧。`false` 表示这一帧确实调了 `activate()`/`orderFrontRegardless()` 中的至少
    /// 一个，状态还没稳定，下一帧要再确认一次才能信——不能仅凭"这一帧调过"就直接判定
    /// 成功，AppKit 这两个 API 都没有同步的成功回执，效果是否真的生效要等下一帧的
    /// `isActive()`/`isKeyWindow()` 读数才能确认。
    pub fn bring_to_front() -> bool {
        let Some(mtm) = MainThreadMarker::new() else {
            // 拿不到主线程标记（理论上不会发生，这里全部跑在 eframe 主线程），保守起见
            // 当作"还没收敛"，让调用方继续按原有节奏重试。
            return false;
        };
        let app = NSApplication::sharedApplication(mtm);
        let mut converged = true;
        // 只在真正不是前台 App 时才调 `activate()`：这个调用会让 WindowServer
        // 走一遍完整的应用切换流程，而 macOS 自己的"切换到本 App"动画（点 Dock /
        // Cmd+Tab 都会触发，不管走哪条路径）本身就要跑 ~300-400ms——`activate()`
        // 恰好和这段系统动画撞在同一帧，会在主线程上叠一次完整的激活流程，跟
        // WindowServer 的合成/动画抢主线程时间片，表现为"全盘扫描时从后台切到
        // 前台，进度环动画在切换那一下明显卡顿，但窗口一直在前台/后台稳定态时
        // 都很流畅"。已经 active 时跳过；调用方（`app/mod.rs` 的
        // `ACTIVATE_RETRY_INTERVAL_MS`）也把两次重试之间的间隔放宽了，减少
        // 我们自己的重绘请求跟这段系统动画抢主线程的机会。
        if !app.isActive() {
            app.activate();
            converged = false;
        }
        let windows = app.windows();
        let count = windows.count();
        for i in 0..count {
            let win = windows.objectAtIndex(i);
            // 已经是最前、拿到焦点的 key window 时跳过 `orderFrontRegardless()`：
            // 对一个已经最前的窗口重复做排序，偶尔会让 AppKit 投递虚假的
            // `mouseExited`/`mouseEntered`，导致 egui-winit 内部的光标图标缓存
            // 被清空，要等下一次真实鼠标移动才纠正回手型光标——表现为"刚打开
            // 窗口那一下,鼠标移进按钮不能很快变手型,但确实可以点"。同一个窗口
            // 一旦已经是 key window 就不再多余地重排它。
            if !win.isKeyWindow() {
                win.orderFrontRegardless();
                converged = false;
            }
        }
        converged
    }

    /// wakeup 线程调用：macOS 的 NSApplication/NSWindow API 必须在主线程调用，
    /// wakeup 线程不是主线程，所以这里 no-op。macOS 的唤回完全由主线程的
    /// `bring_to_front` + `activate_countdown` 机制处理。
    pub fn set_foreground() {}

    /// 扫描期间持有的"别把我 App Nap 了"令牌。
    ///
    /// macOS 会对窗口被遮挡/App 非前台活跃的进程做 App Nap 节流（降低 CPU/GPU
    /// 调度优先级、暂停 display link 等省电手段）。扫描本身是用户主动发起、需要
    /// 持续绘制进度动画的前台工作，被节流之后，一旦窗口重新变成前台活跃，系统要
    /// 把这些调度状态恢复回正常档位——这个"唤醒"过程实测量级在几百毫秒，且完全
    /// 发生在系统调度层面，跟 `bring_to_front`/`activate_countdown` 那套窗口置顶
    /// 逻辑无关（那套逻辑的单次调用正常应在几毫秒以内）。这正是"全盘扫描时从后台
    /// 切回前台，进度环卡一下"的更可能来源：持续调过 `activate()`/
    /// `orderFrontRegardless()` 的时序、重试间隔都没能消除这一下卡顿，说明卡的
    /// 不是这几个调用本身。
    ///
    /// `NSProcessInfo::beginActivityWithOptions(_:reason:)` + `NSActivityUserInitiated`
    /// 是 Apple 官方文档给"用户发起、不允许因省电被节流"这类工作推荐的标准做法：
    /// 持有期间系统不会把本进程判定为可以 App Nap 的候选，`endActivity:`（这里
    /// 由 `Drop` 触发）之后节流策略才重新生效。
    pub struct ScanActivity(objc2::rc::Retained<objc2::runtime::ProtocolObject<dyn objc2::runtime::NSObjectProtocol>>);

    impl ScanActivity {
        /// 扫描开始时调用一次，返回值要一直存活到扫描结束（存进调用方的字段里，
        /// 扫描结束时 drop 掉即可，见 `app/mod.rs` 的用法）。
        pub fn begin(reason: &str) -> Self {
            let info = objc2_foundation::NSProcessInfo::processInfo();
            let reason = objc2_foundation::NSString::from_str(reason);
            let token = info.beginActivityWithOptions_reason(
                objc2_foundation::NSActivityOptions::UserInitiated,
                &reason,
            );
            Self(token)
        }
    }

    impl Drop for ScanActivity {
        fn drop(&mut self) {
            // `endActivity:` 的 unsafe 要求只是"`activity` 必须是正确类型"——这里
            // 的 `self.0` 就是 `begin` 时 `beginActivityWithOptions_reason` 亲手
            // 返回的那个 token，类型上不可能对不上。
            let info = objc2_foundation::NSProcessInfo::processInfo();
            unsafe { info.endActivity(&self.0) };
        }
    }
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
    pub fn enter_tray_mode() {}
    pub fn leave_tray_mode() {}
    pub fn install_reopen_handler() {}
    pub fn is_miniaturized() -> bool {
        false
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
    ///
    /// 返回值同 macOS 侧：`true` 表示这一帧发现窗口本来就已经是前台窗口，没做任何
    /// 纠正动作，调用方可以据此提前把 `activate_countdown` 清零；`false` 表示刚
    /// 调了 `SetForegroundWindow`，状态还没确认稳定，下一帧要再检查一次。
    pub fn bring_to_front() -> bool {
        unsafe {
            // `lparam` 这个字长的参数只够传一个东西：原来传 pid 用来在回调里过滤窗口，
            // 现在改成传"结果 bool"的指针，pid 过滤挪到 `raise_proc` 内部自己现查
            // `GetCurrentProcessId()`（这本身就是个廉价调用，不必从外面传进来）。
            let mut converged = false;
            let _ = EnumWindows(
                Some(raise_proc),
                LPARAM(&mut converged as *mut bool as isize),
            );
            converged
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

    /// `bring_to_front` 的枚举回调：找到本进程第一个顶层窗口就 `raise_window` 并停止，
    /// 把"是否已经是前台窗口、无需纠正"的结果写回 `lparam` 指向的 `bool`。
    unsafe extern "system" fn raise_proc(hwnd: HWND, lparam: LPARAM) -> BOOL {
        unsafe {
            let target_pid = GetCurrentProcessId();
            let mut window_pid: u32 = 0;
            let _tid = GetWindowThreadProcessId(hwnd, Some(&mut window_pid));
            if window_pid != target_pid {
                return BOOL(1); // 不是本进程的窗口 → 继续枚举
            }
            let converged = &mut *(lparam.0 as *mut bool);
            *converged = raise_window(hwnd);
            BOOL(0) // 已找到 → 停止枚举
        }
    }

    /// 单个窗口的置顶逻辑：AttachThreadInput → BringWindowToTop → SetForegroundWindow。
    /// 不调 `ShowWindow(SW_RESTORE)`——窗口可见性由 winit/egui 管理（`Visible(true)`），
    /// 自行 `ShowWindow` 会让 winit 的窗口状态缓存失真。
    ///
    /// 返回 `true` 表示窗口在调用前就已经是前台窗口（无需任何动作）；返回 `false`
    /// 表示刚做了 `SetForegroundWindow` 纠正，是否真的生效要等下一帧再确认——
    /// `SetForegroundWindow` 的返回值本身不够可靠，不能拿它当"已收敛"的证据。
    unsafe fn raise_window(hwnd: HWND) -> bool {
        unsafe {
            let foreground = GetForegroundWindow();
            if foreground == hwnd {
                return true; // 已是前台窗口，这一帧不需要任何动作
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
            false
        }
    }
}

/// 其它平台（Linux 等）：no-op stub，调用方无需写 `#[cfg]` 分支。
#[cfg(not(any(target_os = "macos", target_os = "windows")))]
#[allow(dead_code)]
mod imp {
    pub fn set_accessory(_accessory: bool) {}
    pub fn enter_tray_mode() {}
    pub fn leave_tray_mode() {}
    pub fn install_reopen_handler() {}
    pub fn is_miniaturized() -> bool {
        false
    }
    pub fn set_foreground() {}
    // no-op：本来就没有需要纠正的窗口置顶状态，直接算"已收敛"。
    pub fn bring_to_front() -> bool {
        true
    }
}

#[allow(unused_imports)]
pub use imp::*;
