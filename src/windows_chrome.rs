//! Windows 无边框 + 自绘标题栏时的客户区对齐。
//!
//! winit 在 `decorations(false)` 下仍给顶层窗口保留 `WS_CAPTION | WS_BORDER` 样式（见
//! winit `WindowFlags::to_window_styles`），靠拦截 `WM_NCCALCSIZE` 把客户区扩成整窗。
//! 但窗口首次显示时，DWM 会按「有标题栏」的布局把客户区表面合成错位：内容整体偏移
//! 一个边框+标题栏的距离——顶部多出背景条、右侧被裁掉（实测偏移 9/38px @125% DPI）。
//!
//! 修复（实验验证）分两层：
//!
//! 1. **禁用 DWM 非客户区渲染**（`DWMWA_NCRENDERING_POLICY = DWMNCRP_DISABLED`，
//!    首次 `logic()`、窗口显示前设置一次）：eframe 首帧渲染完才 `set_visible(true)`，
//!    而 winit 的 `set_visible` 会把 `WS_CAPTION` 写回样式，显示瞬间 DWM 会按
//!    「有标题栏」绘制一帧系统标题栏（和自绘标题栏叠出「两个标题栏」的闪现）。
//!    这是 DWM 层属性，winit 重写样式也不会碰，显示时就彻底不画系统标题栏。
//! 2. **decap**（窗口可见后）：去掉 `WS_CAPTION | WS_BORDER` + `SWP_FRAMECHANGED`，
//!    DWM 立即按无边框布局重新合成，修复客户区表面的错位。
//!
//! 注意 decap 后**不要**把样式写回（recap）——写回瞬间又会闪一帧系统标题栏。
//! 不写回没有副作用：winit 只在 `Visible` 等 flag 变化时才重写样式，且那种重写
//! 伴随的 `SWP_FRAMECHANGED` 不会重新触发错位（错位只发生在窗口首次显示时的
//! DWM 帧布局缓存上）；万一哪个场景又错位了，下一次窗口显示会重新调度 decap 修复。
//!
//! 视口命令（`Visible(true)` 等）在当帧 `logic()` 发出、下帧初才被 winit 应用，
//! 所以 decap 延迟一帧执行；窗口未真正可见时推迟重试。

/// 视口命令发出后，等多少帧再执行 decap（1 = 下一帧 `logic()` 开头执行）。
#[cfg(windows)]
pub const CHROME_TUNE_DELAY_FRAMES: u8 = 1;

/// decap 失败时的最大重试次数（窗口尚未显示/HWND 未就绪时首几帧可能失败）。
#[cfg(windows)]
const CHROME_TUNE_MAX_ATTEMPTS: u8 = 5;

#[cfg(windows)]
use std::sync::atomic::{AtomicU8, Ordering};

#[cfg(windows)]
static CHROME_TUNE_ATTEMPTS: AtomicU8 = AtomicU8::new(0);

/// 调谐状态：`countdown` 到 0 执行 decap；`ncr_disabled` 记录 DWM 非客户区渲染是否已禁用。
#[cfg(windows)]
#[derive(Default)]
pub struct ChromeTune {
    countdown: u8,
    ncr_disabled: bool,
}

#[cfg(windows)]
impl ChromeTune {
    pub fn scheduled() -> Self {
        Self {
            countdown: CHROME_TUNE_DELAY_FRAMES,
            ncr_disabled: false,
        }
    }
}

/// 安排一次新的调谐（窗口即将显示/尺寸变更后调用）。
#[cfg(windows)]
pub fn schedule_chrome_tune(state: &mut ChromeTune) {
    *state = ChromeTune::scheduled();
    CHROME_TUNE_ATTEMPTS.store(0, Ordering::Relaxed);
}

#[cfg(windows)]
pub fn poll_chrome_tune(state: &mut ChromeTune, _ctx: &egui::Context, frame: &eframe::Frame) {
    // 第 0 步（一次性，窗口显示前）：禁止 DWM 画非客户区，消除显示瞬间的系统标题栏闪现。
    if !state.ncr_disabled {
        if disable_nc_rendering(frame) {
            state.ncr_disabled = true;
        } else {
            // HWND 还没就绪时下帧重试（与 decap 共用重试计数）。
            bump_attempts(state);
            if state.countdown == 0 {
                state.countdown = 1;
            }
        }
    }

    if state.countdown == 0 {
        return;
    }
    state.countdown -= 1;
    if state.countdown != 0 {
        return;
    }

    if decap_caption_styles(frame) {
        CHROME_TUNE_ATTEMPTS.store(0, Ordering::Relaxed);
        return;
    }
    bump_attempts(state);
}

#[cfg(windows)]
fn bump_attempts(state: &mut ChromeTune) {
    let attempts = CHROME_TUNE_ATTEMPTS.fetch_add(1, Ordering::Relaxed) + 1;
    if attempts < CHROME_TUNE_MAX_ATTEMPTS {
        state.countdown = CHROME_TUNE_DELAY_FRAMES;
    } else {
        CHROME_TUNE_ATTEMPTS.store(0, Ordering::Relaxed);
        eprintln!(
            "windows_chrome: chrome tune failed after {} attempts, giving up",
            CHROME_TUNE_MAX_ATTEMPTS
        );
    }
}

/// 禁用 DWM 非客户区渲染（`DWMNCRP_DISABLED`）。
///
/// winit 的 `set_visible(true)` 会把 `WS_CAPTION` 写回样式，窗口显示瞬间 DWM 会
/// 画一帧系统标题栏（与自绘标题栏叠出「两个标题栏」）。该 DWM 属性不受 winit
/// 重写样式影响，设置一次即可让整个会话期都不画系统标题栏。无边框窗口本来也
/// 不需要 DWM 的边框/标题栏渲染（阴影由 winit 的 `MARKER_UNDECORATED_SHADOW` 处理）。
#[cfg(windows)]
fn disable_nc_rendering(handle: &eframe::Frame) -> bool {
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    use windows::Win32::Foundation::HWND;
    use windows::Win32::Graphics::Dwm::{
        DwmSetWindowAttribute, DWMNCRP_DISABLED, DWMWA_NCRENDERING_POLICY,
    };

    let Ok(win_handle) = handle.window_handle() else {
        return false;
    };
    let RawWindowHandle::Win32(win32) = win_handle.as_raw() else {
        return false;
    };
    let hwnd = HWND(win32.hwnd.get() as _);

    let policy = DWMNCRP_DISABLED;
    unsafe {
        DwmSetWindowAttribute(
            hwnd,
            DWMWA_NCRENDERING_POLICY,
            &policy as *const _ as *const core::ffi::c_void,
            std::mem::size_of_val(&policy) as u32,
        )
        .is_ok()
    }
}

/// 去掉 `WS_CAPTION | WS_BORDER` 并刷新窗框，修复 DWM 按旧帧布局错位合成客户区的问题。
/// 窗口不可见或 HWND 未就绪时返回 `false` 交由上层重试。
#[cfg(windows)]
fn decap_caption_styles(handle: &eframe::Frame) -> bool {
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    use windows::Win32::Foundation::HWND;
    use windows::Win32::UI::WindowsAndMessaging::{
        GetWindowLongW, IsWindowVisible, SetWindowLongW, SetWindowPos, GWL_STYLE,
        SWP_FRAMECHANGED, SWP_NOMOVE, SWP_NOSIZE, SWP_NOZORDER, WS_BORDER, WS_CAPTION,
    };

    let Ok(win_handle) = handle.window_handle() else {
        return false;
    };
    let RawWindowHandle::Win32(win32) = win_handle.as_raw() else {
        return false;
    };
    let hwnd = HWND(win32.hwnd.get() as _);

    // eframe 创建窗口时是隐藏的，首帧渲染完才 `set_visible(true)`；错位的 DWM 帧布局
    // 正是在「显示」那一刻生成的，必须等窗口真正可见后再动样式。
    if !unsafe { IsWindowVisible(hwnd) }.as_bool() {
        return false;
    }

    unsafe {
        let style = GetWindowLongW(hwnd, GWL_STYLE);
        let frame_bits = (WS_CAPTION | WS_BORDER).0 as i32;
        if style & frame_bits == 0 {
            return true; // 已经是无边框样式（winit 没写过或已被处理过）
        }
        SetWindowLongW(hwnd, GWL_STYLE, style & !frame_bits);
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
#[derive(Default)]
pub struct ChromeTune;

#[cfg(not(windows))]
#[allow(dead_code)]
pub fn schedule_chrome_tune(_state: &mut ChromeTune) {}

#[cfg(not(windows))]
#[allow(dead_code)]
pub fn poll_chrome_tune(_state: &mut ChromeTune, _ctx: &egui::Context, _frame: &eframe::Frame) {}
