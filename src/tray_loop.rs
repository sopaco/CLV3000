//! 纯托盘模式：不跑 eframe / OpenGL，只轮询托盘菜单与后台扫描。

use crate::app::{poll_tray_events, AppCore};
use crate::lifecycle::{Lifecycle, RunMode};
use crate::tray::Tray;
use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

/// 阻塞直到用户请求显示窗口或退出。`tray` 由 `main` 持有，跨 eframe 会话复用。
pub fn run(core: &Rc<RefCell<AppCore>>, lifecycle: &Rc<RefCell<Lifecycle>>, tray: &Tray) {
    while lifecycle.borrow().mode == RunMode::TrayOnly {
        {
            let mut core = core.borrow_mut();
            let mut lifecycle = lifecycle.borrow_mut();
            poll_tray_events(tray, &mut core, &mut lifecycle);
            core.poll_background();
        }
        std::thread::sleep(Duration::from_millis(200));
    }
}
