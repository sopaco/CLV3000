//! 事件驱动的 UI 唤醒：让 eframe/winit 事件循环在闲置时真正睡死，而不是靠
//! `request_repaint_after` 定时心跳硬撑（那是老机器上常驻 CPU 占用的主要来源）。
//!
//! 背景：tray-icon / muda 把托盘图标事件和菜单点击事件投递到进程内全局 channel
//! （`TrayIconEvent::receiver()` / `MenuEvent::receiver()`）。之前 UI 线程每 100ms
//! 轮询一次这两个 channel——窗口隐藏时也必须保持这个心跳，否则托盘点击没人处理，
//! 于是事件循环永远睡不着。
//!
//! 这里的做法：起两条专用的转发线程，**阻塞**在全局 channel 的 `recv()` 上（线程
//! 本身零 CPU），事件一到就转进本模块的私有 channel，并通过当前注册的
//! `egui::Context` 调 `request_repaint()`——egui-winit 内部会把这个请求经
//! winit 的 EventLoopProxy 投递成 user event，立刻唤醒事件循环跑一帧。UI 帧里再从
//! 私有 channel `try_recv` 取事件处理。从此闲置时（无论窗口可见还是纯托盘）都不需要
//! 任何定时重绘。
//!
//! 用全局静态而不是 `App` 字段的原因：托盘/菜单的全局 channel 是进程级单例，转发
//! 线程在 `main` 里只起一次；而 eframe 会话理论上可能重建（见 `main.rs` 的 loop），
//! 事件不能因为会话切换而丢失，所以事件队列和"当前 Context"槽位都放进程级静态里。

use egui::Context;
use muda::MenuEvent;
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock, mpsc};
use tray_icon::TrayIconEvent;

/// 当前活跃的 `egui::Context`（每帧结束后的重绘请求都发给它）。
/// 会话建立时注册、销毁时清空；未注册时事件照样进队列，等下一会话的帧来消费。
static REPAINT_CTX: Mutex<Option<Context>> = Mutex::new(None);

// std mpsc 的 Receiver 不是 Sync，套一层 Mutex 才能放静态里；接收端只有 UI 线程
// 一个消费者，锁永远无竞争。
static TRAY_EVENTS: OnceLock<(mpsc::Sender<TrayIconEvent>, Mutex<mpsc::Receiver<TrayIconEvent>>)> =
    OnceLock::new();
static MENU_EVENTS: OnceLock<(mpsc::Sender<MenuEvent>, Mutex<mpsc::Receiver<MenuEvent>>)> =
    OnceLock::new();
/// 右键菜单"用 CLV3000 扫描"/`--scan-path` 冷启动转发过来的扫描请求（见
/// `single_instance::forward_scan_request`）。跟托盘/菜单事件同一套"全局队列 +
/// ping()"机制：`main::wait_in_tray`（Windows tray-only、eframe 还没启动）和
/// `App::logic`（eframe 已启动）都从这里排空，不用关心当前是不是已经进了 eframe。
static SCAN_REQUESTS: OnceLock<(mpsc::Sender<PathBuf>, Mutex<mpsc::Receiver<PathBuf>>)> =
    OnceLock::new();

/// 起两条转发线程（幂等，重复调用直接返回）。必须在 `main` 开头、托盘创建之前调用。
pub fn init() {
    if TRAY_EVENTS.get().is_some() {
        return;
    }
    let (tray_tx, tray_rx) = mpsc::channel();
    let _ = TRAY_EVENTS.set((tray_tx, Mutex::new(tray_rx)));
    let (menu_tx, menu_rx) = mpsc::channel();
    let _ = MENU_EVENTS.set((menu_tx, Mutex::new(menu_rx)));
    let (scan_tx, scan_rx) = mpsc::channel();
    let _ = SCAN_REQUESTS.set((scan_tx, Mutex::new(scan_rx)));

    // 托盘图标事件（双击等）。
    std::thread::spawn(|| {
        let src = TrayIconEvent::receiver();
        let dst = &TRAY_EVENTS.get().expect("init set TRAY_EVENTS").0;
        while let Ok(event) = src.recv() {
            // 只在双击时抢前台——双击是显示主窗口，不会弹出菜单。
            // 单击（尤其是右键）可能触发系统弹出上下文菜单，此时调
            // SetForegroundWindow 把主窗口拉到前台会让菜单失去前台
            // 焦点而自动关闭（Windows 弹出菜单在前台丢失时自动 dismiss）。
            #[cfg(target_os = "windows")]
            if matches!(event, TrayIconEvent::DoubleClick { .. }) {
                crate::macos_reopen::set_foreground();
            }
            if dst.send(event).is_err() {
                break;
            }
            ping();
        }
    });

    // 托盘菜单点击事件。
    std::thread::spawn(|| {
        let src = MenuEvent::receiver();
        let dst = &MENU_EVENTS.get().expect("init set MENU_EVENTS").0;
        while let Ok(event) = src.recv() {
            if dst.send(event).is_err() {
                break;
            }
            // 菜单项点击时菜单已经关闭（TrackPopupMenu 是同步的，用户选择后
            // 函数返回、菜单 dismiss，然后才发送命令事件），此时
            // SetForegroundWindow 不会关掉任何菜单，安全。
            #[cfg(target_os = "windows")]
            crate::macos_reopen::set_foreground();
            ping();
        }
    });
}

/// 唤醒 UI 跑一帧（若当前有注册中的 egui Context）。
/// 没有注册时静默丢弃唤醒——事件已在队列里，下个会话的第一帧会消费。
/// tray-only 启动模式（eframe 未启动）下 Context 槽位为空，ping 是 no-op，
/// 主线程在 `main::wait_in_tray` 里跑 Win32 消息循环自己排空 channel。
fn ping() {
    if let Some(ctx) = &*REPAINT_CTX.lock().unwrap() {
        ctx.request_repaint();
    }
}

/// 由 `App::new` 调用：注册当前 eframe 会话的 Context，之后事件才能唤醒它。
pub fn register_ctx(ctx: &Context) {
    *REPAINT_CTX.lock().unwrap() = Some(ctx.clone());
}

/// 由 `App::drop` 调用：会话结束，清掉 Context 槽位（会话按顺序重建，无并发竞争）。
pub fn unregister_ctx() {
    *REPAINT_CTX.lock().unwrap() = None;
}

/// UI 帧里轮询托盘图标事件（替代直接读 `TrayIconEvent::receiver()`）。
pub fn tray_events() -> &'static Mutex<mpsc::Receiver<TrayIconEvent>> {
    &TRAY_EVENTS.get().expect("wakeup::init not called").1
}

/// UI 帧里轮询托盘菜单事件（替代直接读 `MenuEvent::receiver()`）。
pub fn menu_events() -> &'static Mutex<mpsc::Receiver<MenuEvent>> {
    &MENU_EVENTS.get().expect("wakeup::init not called").1
}

/// 由 `single_instance` 的转发监听线程调用：把收到的扫描请求推进队列并唤醒 UI
/// （若当前有 eframe 会话在跑；没有的话事件留在队列里，`wait_in_tray` 会自己排空）。
pub fn push_scan_request(path: PathBuf) {
    if let Some((tx, _)) = SCAN_REQUESTS.get() {
        let _ = tx.send(path);
        ping();
    }
}

/// UI 帧 / `wait_in_tray` 里轮询转发过来的扫描请求。
pub fn scan_requests() -> &'static Mutex<mpsc::Receiver<PathBuf>> {
    &SCAN_REQUESTS.get().expect("wakeup::init not called").1
}
