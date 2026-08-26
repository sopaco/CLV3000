//! Windows 无边框 + 自绘标题栏时的客户区对齐。
//!
//! winit 在 `decorations(false)` 下仍给顶层窗口保留 `WS_CAPTION | WS_BORDER` 样式（见
//! winit `WindowFlags::to_window_styles`），靠拦截 `WM_NCCALCSIZE` 把客户区扩成整窗。
//! 但窗口首次显示时，DWM 会按「有标题栏」的布局把客户区表面合成错位：内容整体偏移
//! 一个边框+标题栏的距离——顶部多出背景条、右侧被裁掉（实测偏移 9/38px @125% DPI）。
//!
//! 修复（实验验证）分三层：
//!
//! 1. **禁用 DWM 非客户区渲染**（`DWMWA_NCRENDERING_POLICY = DWMNCRP_DISABLED`，
//!    首次 `logic()`、窗口显示前设置一次）：eframe 首帧渲染完才 `set_visible(true)`，
//!    而 winit 的 `set_visible` 会把 `WS_CAPTION` 写回样式，显示瞬间 DWM 会按
//!    「有标题栏」绘制一帧系统标题栏（和自绘标题栏叠出「两个标题栏」的闪现）。
//!    这是 DWM 层属性，winit 重写样式也不会碰，显示时就彻底不画系统标题栏。
//! 2. **decap**（窗口可见后）：去掉 `WS_CAPTION | WS_BORDER` + `SWP_FRAMECHANGED`，
//!    DWM 立即按无边框布局重新合成，修复客户区表面的错位。
//! 3. **caption guard**（HWND 就绪后装一次窗口子类）：失焦再回到前台时，系统会发
//!    `WM_NCACTIVATE`。winit 0.30 的窗口过程把这条消息原样交给 `DefWindowProcW`，
//!    默认处理会按「有标题栏」重绘非客户区——自绘标题栏位置闪出一帧系统标题栏，
//!    下一帧客户区覆盖后又消失。官方约定：`lParam == -1` 时 `DefWindowProc` **更新
//!    激活状态但不重绘非客户区**。子类把 `lParam` 改成 -1 再交给 winit，焦点事件
//!    照常，只挡住这一笔系统标题栏绘制。主题引擎的 `WM_NCUAHDRAWCAPTION` /
//!    `WM_NCUAHDRAWFRAME` 同样会单独画 caption，直接吞掉。
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

/// 窗口子类 ID（`SetWindowSubclass` 用），"CLV3"。
#[cfg(windows)]
const CAPTION_GUARD_SUBCLASS_ID: usize = 0x434C_5633;

/// 未公开的主题消息：激活时单独画系统标题栏 / 边框（Vista+ uxtheme）。
#[cfg(windows)]
const WM_NCUAHDRAWCAPTION: u32 = 0x00AE;
#[cfg(windows)]
const WM_NCUAHDRAWFRAME: u32 = 0x00AF;

#[cfg(windows)]
use std::sync::atomic::{AtomicU8, Ordering};

#[cfg(windows)]
static CHROME_TUNE_ATTEMPTS: AtomicU8 = AtomicU8::new(0);

/// 调谐状态：`countdown` 到 0 执行 decap；其余两个标志是一次性安装结果。
#[cfg(windows)]
#[derive(Default)]
pub struct ChromeTune {
    countdown: u8,
    ncr_disabled: bool,
    caption_guard: bool,
}

#[cfg(windows)]
impl ChromeTune {
    pub fn scheduled() -> Self {
        Self {
            countdown: CHROME_TUNE_DELAY_FRAMES,
            ncr_disabled: false,
            caption_guard: false,
        }
    }
}

/// 安排一次新的调谐（窗口即将显示/尺寸变更后调用）。
///
/// 只重置 decap 倒计时和 DWM 属性（隐藏后再显示时 DWM 可能丢了 NCR 策略）。
/// 窗口子类挂在 HWND 上，整个窗口生命周期有效，不能跟着清掉。
#[cfg(windows)]
pub fn schedule_chrome_tune(state: &mut ChromeTune) {
    let caption_guard = state.caption_guard;
    *state = ChromeTune::scheduled();
    state.caption_guard = caption_guard;
    CHROME_TUNE_ATTEMPTS.store(0, Ordering::Relaxed);
}

#[cfg(windows)]
pub fn poll_chrome_tune(state: &mut ChromeTune, _ctx: &egui::Context, frame: &eframe::Frame) {
    let Some(hwnd) = hwnd_from_frame(frame) else {
        // HWND 还没就绪时下帧重试（与 decap 共用重试计数）。
        if !state.ncr_disabled || !state.caption_guard || state.countdown > 0 {
            bump_attempts(state);
            if state.countdown == 0 {
                state.countdown = 1;
            }
        }
        return;
    };

    // 第 0 步（一次性，窗口显示前）：禁止 DWM 画非客户区，消除显示瞬间的系统标题栏闪现。
    if !state.ncr_disabled {
        state.ncr_disabled = disable_nc_rendering(hwnd);
    }

    // 挡住失焦/回前台时 `DefWindowProc(WM_NCACTIVATE)` 画系统标题栏。
    if !state.caption_guard {
        state.caption_guard = install_caption_guard(hwnd);
    }

    if state.countdown == 0 {
        return;
    }
    state.countdown -= 1;
    if state.countdown != 0 {
        return;
    }

    if decap_caption_styles(hwnd) {
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

#[cfg(windows)]
fn hwnd_from_frame(handle: &eframe::Frame) -> Option<windows::Win32::Foundation::HWND> {
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    use windows::Win32::Foundation::HWND;

    let win_handle = handle.window_handle().ok()?;
    let RawWindowHandle::Win32(win32) = win_handle.as_raw() else {
        return None;
    };
    Some(HWND(win32.hwnd.get() as _))
}

/// 禁用 DWM 非客户区渲染（`DWMNCRP_DISABLED`）。
///
/// winit 的 `set_visible(true)` 会把 `WS_CAPTION` 写回样式，窗口显示瞬间 DWM 会
/// 画一帧系统标题栏（与自绘标题栏叠出「两个标题栏」）。该 DWM 属性不受 winit
/// 重写样式影响，设置一次即可让整个会话期都不画系统标题栏。无边框窗口本来也
/// 不需要 DWM 的边框/标题栏渲染（阴影由 winit 的 `MARKER_UNDECORATED_SHADOW` 处理）。
#[cfg(windows)]
fn disable_nc_rendering(hwnd: windows::Win32::Foundation::HWND) -> bool {
    use windows::Win32::Graphics::Dwm::{
        DwmSetWindowAttribute, DWMNCRP_DISABLED, DWMWA_NCRENDERING_POLICY,
    };

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

/// 挂窗口子类，挡住激活时系统标题栏的那一帧绘制。
///
/// winit 的 `WM_NCACTIVATE` 分支会 `DefWindowProcW(..., 原始 lParam)`。官方文档：
/// `lParam == -1` 时默认过程只更新激活状态、不重绘非客户区。子类改写后再
/// `DefSubclassProc`，winit 的焦点跟踪仍然收到这条消息。
#[cfg(windows)]
fn install_caption_guard(hwnd: windows::Win32::Foundation::HWND) -> bool {
    use windows::Win32::UI::Shell::SetWindowSubclass;

    unsafe { SetWindowSubclass(hwnd, Some(caption_guard_proc), CAPTION_GUARD_SUBCLASS_ID, 0) }
        .as_bool()
}

#[cfg(windows)]
unsafe extern "system" fn caption_guard_proc(
    hwnd: windows::Win32::Foundation::HWND,
    msg: u32,
    wparam: windows::Win32::Foundation::WPARAM,
    lparam: windows::Win32::Foundation::LPARAM,
    _id: usize,
    _data: usize,
) -> windows::Win32::Foundation::LRESULT {
    use windows::Win32::Foundation::{LPARAM, LRESULT};
    use windows::Win32::UI::Shell::{DefSubclassProc, RemoveWindowSubclass};
    use windows::Win32::UI::WindowsAndMessaging::{WM_NCACTIVATE, WM_NCDESTROY};

    match msg {
        WM_NCACTIVATE => {
            // 让 winit 继续处理焦点；只禁止 DefWindowProc 重绘系统标题栏。
            unsafe { DefSubclassProc(hwnd, msg, wparam, LPARAM(-1)) }
        }
        WM_NCUAHDRAWCAPTION | WM_NCUAHDRAWFRAME => LRESULT(0),
        WM_NCDESTROY => {
            let _ = unsafe {
                RemoveWindowSubclass(hwnd, Some(caption_guard_proc), CAPTION_GUARD_SUBCLASS_ID)
            };
            unsafe { DefSubclassProc(hwnd, msg, wparam, lparam) }
        }
        _ => unsafe { DefSubclassProc(hwnd, msg, wparam, lparam) },
    }
}

/// 去掉 `WS_CAPTION | WS_BORDER` 并刷新窗框，修复 DWM 按旧帧布局错位合成客户区的问题。
/// 窗口不可见时返回 `false` 交由上层重试。
#[cfg(windows)]
fn decap_caption_styles(hwnd: windows::Win32::Foundation::HWND) -> bool {
    use windows::Win32::UI::WindowsAndMessaging::{
        GetWindowLongW, IsWindowVisible, SetWindowLongW, SetWindowPos, GWL_STYLE,
        SWP_FRAMECHANGED, SWP_NOMOVE, SWP_NOSIZE, SWP_NOZORDER, WS_BORDER, WS_CAPTION,
    };

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
