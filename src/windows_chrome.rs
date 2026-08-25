//! Windows 无边框 + 自绘标题栏时的客户区对齐。
//!
//! winit 在 `decorations(false)` 下通过 `WM_NCCALCSIZE` 管理非客户区，**不要**在这里
//! 手改 `GWL_STYLE`（会与 winit 在 `Visible` / 尺寸变更时写回的样式打架）。我们只补
//! DWM 边距清零 + `SWP_FRAMECHANGED`，并在每次视口变更后延迟一帧重跑——`Visible(true)`
//! 等命令在本帧 `logic()` 发出、下帧初才被 winit 应用。

/// 视口命令发出后，等多少帧再调 DWM（1 = 下一帧 `logic()` 开头执行）。
#[cfg(windows)]
pub const CHROME_TUNE_DELAY_FRAMES: u8 = 1;

/// DWM 调谐失败时的最大重试次数（HWND 未就绪时首帧可能失败）。
#[cfg(windows)]
const CHROME_TUNE_MAX_ATTEMPTS: u8 = 3;

#[cfg(windows)]
use std::sync::atomic::{AtomicU8, Ordering};

#[cfg(windows)]
static CHROME_TUNE_ATTEMPTS: AtomicU8 = AtomicU8::new(0);

#[cfg(windows)]
pub fn schedule_chrome_tune(countdown: &mut u8) {
    *countdown = CHROME_TUNE_DELAY_FRAMES;
    CHROME_TUNE_ATTEMPTS.store(0, Ordering::Relaxed);
}

#[cfg(windows)]
pub fn poll_chrome_tune(countdown: &mut u8, frame: &eframe::Frame) {
    if *countdown == 0 {
        return;
    }
    *countdown -= 1;
    if *countdown != 0 {
        return;
    }
    if tune_borderless_client_area(frame) {
        CHROME_TUNE_ATTEMPTS.store(0, Ordering::Relaxed);
        return;
    }
    let attempts = CHROME_TUNE_ATTEMPTS.fetch_add(1, Ordering::Relaxed) + 1;
    if attempts < CHROME_TUNE_MAX_ATTEMPTS {
        *countdown = CHROME_TUNE_DELAY_FRAMES;
    } else {
        CHROME_TUNE_ATTEMPTS.store(0, Ordering::Relaxed);
        eprintln!(
            "windows_chrome: DWM tune failed after {} attempts, giving up",
            CHROME_TUNE_MAX_ATTEMPTS
        );
    }
}

/// 清零 DWM 非客户区边距并刷新窗框，避免顶部「幽灵标题栏」导致点击飘逸。
#[cfg(windows)]
fn tune_borderless_client_area(handle: &eframe::Frame) -> bool {
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    use windows::Win32::Foundation::HWND;
    use windows::Win32::Graphics::Dwm::DwmExtendFrameIntoClientArea;
    use windows::Win32::UI::Controls::MARGINS;
    use windows::Win32::UI::WindowsAndMessaging::{
        SetWindowPos, SWP_FRAMECHANGED, SWP_NOMOVE, SWP_NOSIZE, SWP_NOZORDER,
    };

    let Ok(win_handle) = handle.window_handle() else {
        eprintln!("windows_chrome: failed to get window handle");
        return false;
    };
    let RawWindowHandle::Win32(win32) = win_handle.as_raw() else {
        eprintln!("windows_chrome: unexpected non-Win32 window handle");
        return false;
    };
    let hwnd = HWND(win32.hwnd.get() as _);

    unsafe {
        let margins = MARGINS {
            cxLeftWidth: 0,
            cxRightWidth: 0,
            cyTopHeight: 0,
            cyBottomHeight: 0,
        };
        if let Err(e) = DwmExtendFrameIntoClientArea(hwnd, &margins) {
            eprintln!("windows_chrome: DwmExtendFrameIntoClientArea failed: {e}");
            return false;
        }

        if let Err(e) = SetWindowPos(
            hwnd,
            None,
            0,
            0,
            0,
            0,
            SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER | SWP_FRAMECHANGED,
        ) {
            eprintln!("windows_chrome: SetWindowPos(SWP_FRAMECHANGED) failed: {e}");
            return false;
        }
    }

    true
}

#[cfg(not(windows))]
#[allow(dead_code)]
pub fn schedule_chrome_tune(_countdown: &mut u8) {}

#[cfg(not(windows))]
#[allow(dead_code)]
pub fn poll_chrome_tune(_countdown: &mut u8, _frame: &eframe::Frame) {}
