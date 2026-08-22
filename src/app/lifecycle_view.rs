//! 窗口生命周期：托盘事件、视口可见性对账、隐藏到托盘。

use super::app_shell::App;
use super::core::AppCore;
use super::Page;
use crate::lifecycle::{Lifecycle, RunMode};
use crate::tray::Tray;
use eframe::egui;
use egui::ViewportCommand;
use tray_icon::TrayIconEvent;

/// 主窗口默认尺寸（与 `main.rs` 里 `with_inner_size` 一致）。
pub(super) const MAIN_WINDOW_SIZE: [f32; 2] = [900.0, 600.0];
#[cfg(not(windows))]
pub(super) const ABOUT_WINDOW_SIZE: [f32; 2] = [480.0, 472.0];
#[cfg(windows)]
pub(super) const ABOUT_WINDOW_SIZE: [f32; 2] = [480.0, 428.0];

#[cfg(target_os = "macos")]
pub(super) const ACTIVATE_FRAMES: u8 = 12;
#[cfg(not(target_os = "macos"))]
pub(super) const ACTIVATE_FRAMES: u8 = 2;

pub(super) const ACTIVATE_RETRY_INTERVAL_MS: u64 = 60;

#[cfg(windows)]
fn trim_working_set() {
    use windows::Win32::System::Threading::{GetCurrentProcess, SetProcessWorkingSetSize};
    let _ = unsafe { SetProcessWorkingSetSize(GetCurrentProcess(), usize::MAX, usize::MAX) };
}

#[cfg(not(windows))]
fn trim_working_set() {}

fn poll_tray_events(tray: &Tray, core: &mut AppCore, lifecycle: &mut Lifecycle) -> bool {
    let mut focus_requested = false;
    while let Ok(event) = crate::wakeup::tray_events().lock().unwrap().try_recv() {
        if let TrayIconEvent::DoubleClick { .. } = event {
            lifecycle.mode = RunMode::ShowWindow;
            lifecycle.about_open = false;
            lifecycle.about_standalone = false;
            focus_requested = true;
        }
    }

    while let Ok(event) = crate::wakeup::menu_events().lock().unwrap().try_recv() {
        let id = event.id();
        if id == &tray.ids.show {
            lifecycle.mode = RunMode::ShowWindow;
            lifecycle.about_open = false;
            lifecycle.about_standalone = false;
            focus_requested = true;
        } else if id == &tray.ids.quick_scan {
            lifecycle.mode = RunMode::ShowWindow;
            lifecycle.about_open = false;
            lifecycle.about_standalone = false;
            core.page = Page::QuickScan;
            if !core.full.is_running() {
                core.quick.start(core.config.scan_removable_drives);
            }
            focus_requested = true;
        } else if id == &tray.ids.about {
            lifecycle.about_open = true;
            lifecycle.about_standalone = true;
            focus_requested = true;
        } else if id == &tray.ids.quit {
            lifecycle.mode = RunMode::Quit;
        }
    }
    focus_requested
}

impl App {
    pub(super) fn hide_to_tray(&mut self, ctx: &egui::Context) {
        self.release_ui_resources(ctx);
        self.lifecycle.mode = RunMode::TrayOnly;
        ctx.send_viewport_cmd(ViewportCommand::Visible(false));
        self.window_hidden = true;
        self.activate_countdown = 0;
        #[cfg(target_os = "macos")]
        crate::macos_reopen::enter_tray_mode();
        trim_working_set();
        // `reconcile_lifecycle` 在 `logic()` 里、`ui()` 之前跑——关窗发生在 `ui()`，
        // 若不主动要下一帧，隐藏后可能永远等不到 `set_accessory(true)`。
        ctx.request_repaint();
    }

    pub(super) fn poll_tray(&mut self) {
        let Some(tray) = self.tray.as_ref() else {
            return;
        };
        let tray_focus = poll_tray_events(tray, &mut self.core, &mut self.lifecycle);
        let next_mode = self.lifecycle.mode;

        if tray_focus {
            self.activate_countdown = self.activate_countdown.max(ACTIVATE_FRAMES);
        }

        if next_mode == RunMode::Quit {
            self.allow_exit = true;
        }
    }

    pub(super) fn poll_scan_requests(&mut self) {
        let mut got_request = false;
        while let Ok(path) = crate::wakeup::scan_requests().lock().unwrap().try_recv() {
            got_request = true;
            self.begin_path_scan(path);
        }
        if got_request {
            self.request_show_window();
        }
    }

    pub(super) fn poll_show_requests(&mut self) {
        let mut got_request = false;
        while crate::wakeup::show_requests()
            .lock()
            .unwrap()
            .try_recv()
            .is_ok()
        {
            got_request = true;
        }
        if got_request {
            self.request_show_window();
        }
    }

    fn request_show_window(&mut self) {
        self.lifecycle.mode = RunMode::ShowWindow;
        self.lifecycle.about_open = false;
        self.lifecycle.about_standalone = false;
        #[cfg(target_os = "macos")]
        crate::macos_reopen::leave_tray_mode();
        self.activate_countdown = self.activate_countdown.max(ACTIVATE_FRAMES);
    }

    fn begin_path_scan(&mut self, path: std::path::PathBuf) {
        if self.core.any_scan_running() {
            self.toast("Finish the current scan before starting another");
            return;
        }
        self.core.page = Page::FullScan;
        self.core.full.start_path(path);
    }

    pub(super) fn reconcile_lifecycle(&mut self, ctx: &egui::Context) {
        let mode = self.lifecycle.mode;
        let about_open = self.lifecycle.about_open;
        let about_standalone = self.lifecycle.about_standalone;
        if mode == RunMode::Quit {
            self.allow_exit = true;
            ctx.send_viewport_cmd(ViewportCommand::Close);
            return;
        }
        let desired_visible = mode == RunMode::ShowWindow || about_open;
        if desired_visible {
            #[cfg(target_os = "macos")]
            crate::macos_reopen::leave_tray_mode();
            let just_shown = self.window_hidden;
            if just_shown {
                ctx.send_viewport_cmd(ViewportCommand::Visible(true));
                self.window_hidden = false;
                self.activate_countdown = ACTIVATE_FRAMES;
            }
            let intent = if about_open && about_standalone { 1 } else { 0 };
            let intent_changed = intent != self.size_intent;
            if intent_changed {
                let size = if intent == 1 {
                    ABOUT_WINDOW_SIZE
                } else {
                    MAIN_WINDOW_SIZE
                };
                ctx.send_viewport_cmd(ViewportCommand::InnerSize(size.into()));
                self.size_intent = intent;
            }
            if just_shown || intent_changed {
                let size = if intent == 1 {
                    ABOUT_WINDOW_SIZE
                } else {
                    MAIN_WINDOW_SIZE
                };
                if let Some(monitor) = ctx.input(|i| i.viewport().monitor_size) {
                    let origin = ((monitor - egui::Vec2::from(size)) / 2.0).max(egui::Vec2::ZERO);
                    ctx.send_viewport_cmd(ViewportCommand::OuterPosition(origin.to_pos2()));
                }
            }
        } else if !self.window_hidden {
            ctx.send_viewport_cmd(ViewportCommand::Visible(false));
            self.window_hidden = true;
            self.activate_countdown = 0;
            #[cfg(target_os = "macos")]
            crate::macos_reopen::enter_tray_mode();
        } else {
            let still_visible = ctx.input(|i| i.viewport().visible().unwrap_or(false));
            if still_visible {
                ctx.send_viewport_cmd(ViewportCommand::Visible(false));
            }
            #[cfg(target_os = "macos")]
            crate::macos_reopen::enter_tray_mode();
        }
    }

    #[cfg(target_os = "macos")]
    pub(super) fn sync_macos_minimized_viewport(&mut self, ctx: &egui::Context) -> bool {
        if self.window_hidden {
            self.macos_was_miniaturized = false;
            return false;
        }
        let now_miniaturized = crate::macos_reopen::is_miniaturized();
        let stale_minimized = ctx.input(|i| i.viewport().minimized == Some(true));
        let restored_from_dock =
            stale_minimized && !now_miniaturized && self.macos_was_miniaturized;
        self.macos_was_miniaturized = now_miniaturized;
        if restored_from_dock {
            ctx.send_viewport_cmd(ViewportCommand::Minimized(false));
            ctx.request_repaint();
            return true;
        }
        false
    }
}
