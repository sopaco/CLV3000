//! 纯托盘模式下泵送系统事件，让托盘/菜单点击能进入 channel。
//!
//! 不能在这里创建 winit `EventLoop`：主窗口的 eframe 会话也会创建 EventLoop，
//! 同一线程上重复创建会 `RecreationAttempt`。

use std::time::Duration;

/// 短暂泵送一次平台事件队列（阻塞最多 `timeout`）。
pub fn pump(timeout: Duration) {
    #[cfg(windows)]
    windows_pump();

    #[cfg(target_os = "macos")]
    macos_pump(timeout);

    #[cfg(all(not(windows), not(target_os = "macos")))]
    {
        let _ = timeout;
        std::thread::sleep(timeout);
    }
}

#[cfg(windows)]
fn windows_pump() {
    use windows::Win32::Foundation::HWND;
    use windows::Win32::UI::WindowsAndMessaging::{
        DispatchMessageW, PeekMessageW, TranslateMessage, MSG, PM_REMOVE,
    };

    // SAFETY: 标准 Win32 消息泵；`PeekMessageW` 无窗口句柄时处理当前线程队列。
    unsafe {
        let mut msg = MSG::default();
        while PeekMessageW(&mut msg, HWND::default(), 0, 0, PM_REMOVE).into() {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
}

#[cfg(target_os = "macos")]
fn macos_pump(timeout: Duration) {
    use core_foundation::runloop::{kCFRunLoopDefaultMode, CFRunLoop};

    let _ = CFRunLoop::run_in_mode(unsafe { kCFRunLoopDefaultMode }, timeout, true);
}
