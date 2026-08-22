//! `App` 结构体、资源管理与 `eframe::App` 实现。

use super::core::AppCore;
use super::lifecycle_view::ACTIVATE_RETRY_INTERVAL_MS;
#[cfg(target_os = "macos")]
use super::lifecycle_view::ACTIVATE_FRAMES;
use super::Page;
use crate::lifecycle::{InitialMode, Lifecycle, RunMode};
use crate::sysmon::{self, ResourceSample, SysMonHandle};
use crate::theme::{self, colors};
use crate::tray::Tray;
use crate::widgets::Toast;
use eframe::egui;
use egui::ViewportCommand;
use std::time::Duration;

pub struct App {
    pub(super) core: AppCore,
    pub(super) lifecycle: Lifecycle,
    pub(super) tray: Option<Tray>,
    pub(super) sysmon: Option<SysMonHandle>,
    pub(super) last_sample: ResourceSample,
    pub(super) toasts: Vec<Toast>,
    pub(super) allow_exit: bool,
    pub(super) app_icon_texture: Option<egui::TextureHandle>,
    #[cfg(not(windows))]
    pub(super) titlebar_icon_texture: Option<egui::TextureHandle>,
    pub(super) dotted_bg_texture: Option<egui::TextureHandle>,
    pub(super) window_hidden: bool,
    pub(super) activate_countdown: u8,
    pub(super) size_intent: u8,
    #[cfg(target_os = "macos")]
    pub(super) macos_was_miniaturized: bool,
    #[cfg(target_os = "macos")]
    pub(super) scan_activity: Option<crate::macos_reopen::ScanActivity>,
}

fn load_texture(
    ctx: &egui::Context,
    name: &str,
    (rgba, w, h): (Vec<u8>, u32, u32),
) -> egui::TextureHandle {
    let color_image = egui::ColorImage::from_rgba_unmultiplied([w as usize, h as usize], &rgba);
    ctx.load_texture(name, color_image, egui::TextureOptions::LINEAR)
}

impl App {
    pub fn new(
        cc: &eframe::CreationContext<'_>,
        tray: Option<Tray>,
        initial: InitialMode,
    ) -> Self {
        theme::apply(&cc.egui_ctx);
        crate::wakeup::register_ctx(&cc.egui_ctx);
        #[cfg(target_os = "macos")]
        crate::macos_reopen::install_reopen_handler();

        let start_tray_only = matches!(initial, InitialMode::TrayOnly | InitialMode::About);
        let mut lifecycle = Lifecycle::new(start_tray_only);
        let mut core = AppCore::new();

        match &initial {
            InitialMode::ShowWindow | InitialMode::TrayOnly => {}
            InitialMode::QuickScan => {
                core.page = Page::QuickScan;
                if !core.full.is_running() {
                    core.quick.start(core.config.scan_removable_drives);
                }
            }
            InitialMode::About => {
                lifecycle.about_open = true;
                lifecycle.about_standalone = true;
            }
            InitialMode::ScanPath(path) => {
                core.page = Page::FullScan;
                core.full.start_path(path.clone());
            }
        }

        let window_hidden = matches!(initial, InitialMode::TrayOnly | InitialMode::About);
        if matches!(initial, InitialMode::TrayOnly) {
            cc.egui_ctx.send_viewport_cmd(ViewportCommand::Visible(false));
            #[cfg(target_os = "macos")]
            crate::macos_reopen::enter_tray_mode();
        }

        Self {
            core,
            lifecycle,
            tray,
            sysmon: None,
            last_sample: ResourceSample::default(),
            toasts: Vec::new(),
            allow_exit: false,
            app_icon_texture: None,
            #[cfg(not(windows))]
            titlebar_icon_texture: None,
            dotted_bg_texture: None,
            window_hidden,
            activate_countdown: 0,
            size_intent: 0,
            #[cfg(target_os = "macos")]
            macos_was_miniaturized: false,
            #[cfg(target_os = "macos")]
            scan_activity: None,
        }
    }

    pub(super) fn toast(&mut self, text: impl Into<String>) {
        self.toasts.push(Toast::new(text));
    }

    pub(super) fn navigate(&mut self, page: Page) {
        self.core.page = page;
    }

    pub(super) fn ensure_ui_resources(&mut self, ctx: &egui::Context) {
        if self.sysmon.is_none() {
            self.sysmon = Some(sysmon::spawn(ctx.clone()));
        }
        if self.app_icon_texture.is_none() {
            const LOGO_DISPLAY_PT: f32 = 90.0;
            self.app_icon_texture = Some(load_texture(
                ctx,
                "app_icon",
                crate::icon_data::load_app_icon_for_display(
                    LOGO_DISPLAY_PT,
                    ctx.pixels_per_point(),
                ),
            ));
        }
        #[cfg(not(windows))]
        if self.titlebar_icon_texture.is_none() {
            self.titlebar_icon_texture = Some(load_texture(
                ctx,
                "titlebar_icon",
                crate::icon_data::load_tray_icon(64),
            ));
        }
        if self.dotted_bg_texture.is_none() {
            self.dotted_bg_texture = Some(ctx.load_texture(
                "dotted_bg_tile",
                theme::dotted_tile_image(),
                egui::TextureOptions::LINEAR_REPEAT,
            ));
        }
    }

    pub(super) fn release_ui_resources(&mut self, ctx: &egui::Context) {
        self.sysmon.take();
        ctx.memory_mut(|m| {
            m.data.clear();
            m.caches = Default::default();
            m.reset_areas();
            m.to_global.clear();
        });
        self.app_icon_texture.take();
        #[cfg(not(windows))]
        self.titlebar_icon_texture.take();
        self.dotted_bg_texture.take();
        self.toasts.clear();
        self.last_sample = ResourceSample::default();
    }

    pub(super) fn poll_background(&mut self, ctx: &egui::Context) {
        let toasts = self.core.poll_background(ctx);
        for msg in toasts {
            self.toast(msg);
        }
    }
}

impl Drop for App {
    fn drop(&mut self) {
        crate::wakeup::unregister_ctx();
    }
}

impl eframe::App for App {
    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        colors::BG_APP.to_normalized_gamma_f32()
    }

    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.poll_tray();
        self.poll_show_requests();
        self.poll_scan_requests();
        self.reconcile_lifecycle(ctx);

        #[cfg(target_os = "macos")]
        if self.sync_macos_minimized_viewport(ctx) {
            self.activate_countdown = self.activate_countdown.max(ACTIVATE_FRAMES);
        }

        if self.activate_countdown > 0 {
            if !self.window_hidden {
                if crate::macos_reopen::bring_to_front() {
                    self.activate_countdown = 0;
                } else {
                    self.activate_countdown -= 1;
                }
            } else {
                self.activate_countdown = 0;
            }
        }

        self.poll_background(ctx);

        if let Some(sysmon) = &self.sysmon
            && let Ok(sample) = sysmon.rx.try_recv()
        {
            self.last_sample = sample;
        }

        let scanning = self.core.quick.is_running() || self.core.full.is_running();
        #[cfg(target_os = "macos")]
        {
            if scanning && self.scan_activity.is_none() {
                self.scan_activity = Some(crate::macos_reopen::ScanActivity::begin(
                    "CLV3000 正在扫描，需要持续绘制进度动画",
                ));
            } else if !scanning && self.scan_activity.is_some() {
                self.scan_activity = None;
            }
        }

        let visible = ctx.input(|i| i.viewport().visible().unwrap_or(true));
        if self.activate_countdown > 0 {
            ctx.request_repaint_after(Duration::from_millis(ACTIVATE_RETRY_INTERVAL_MS));
        } else if scanning {
            ctx.request_repaint_after(Duration::from_millis(if visible { 250 } else { 500 }));
        }
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();

        if ctx.input(|i| i.viewport().close_requested()) {
            let is_quit = self.lifecycle.mode == RunMode::Quit || self.allow_exit;
            if is_quit {
            } else if self.lifecycle.about_open {
                ctx.send_viewport_cmd(ViewportCommand::CancelClose);
                let mode = self.lifecycle.mode;
                self.lifecycle.about_open = false;
                self.lifecycle.about_standalone = false;
                if mode == RunMode::TrayOnly {
                    self.hide_to_tray(&ctx);
                }
                return;
            } else {
                ctx.send_viewport_cmd(ViewportCommand::CancelClose);
                self.hide_to_tray(&ctx);
                return;
            }
        }

        let about_open = self.lifecycle.about_open;
        let about_standalone = self.lifecycle.about_standalone;

        if self.window_hidden && !about_open {
            return;
        }

        if about_open && about_standalone {
            crate::about_dialog::paint_about_fullscreen(ui);
            if crate::about_dialog::take_closed() {
                let mode = self.lifecycle.mode;
                self.lifecycle.about_open = false;
                self.lifecycle.about_standalone = false;
                if mode == RunMode::TrayOnly {
                    self.hide_to_tray(&ctx);
                }
            }
            return;
        }

        self.ensure_ui_resources(&ctx);
        self.toasts.retain(|t| !t.expired());

        #[cfg(not(windows))]
        if let Some(tex) = self.titlebar_icon_texture.clone() {
            super::chrome::title_bar(ui, &ctx, &tex, self);
        }

        egui::Panel::bottom("resource_bar")
            .exact_size(50.0)
            .resizable(false)
            .show_separator_line(false)
            .frame(
                egui::Frame::default()
                    .fill(colors::BG_APP)
                    .inner_margin(egui::Margin::symmetric(20, 10)),
            )
            .show(ui, |ui| super::chrome::resource_bar(ui, self.last_sample));

        egui::Panel::left("sidebar")
            .exact_size(64.0)
            .resizable(false)
            .show_separator_line(false)
            .frame(egui::Frame::default().fill(colors::BG_SIDEBAR))
            .show(ui, |ui| super::chrome::sidebar(ui, &ctx, self));

        let dotted_bg_texture = self.dotted_bg_texture.clone();
        egui::CentralPanel::default()
            .frame(egui::Frame::default().fill(colors::BG_APP))
            .show(ui, |ui| {
                theme::paint_dotted_background(
                    ui.painter(),
                    ui.max_rect(),
                    dotted_bg_texture.as_ref(),
                );
                let page = self.core.page;
                match page {
                    Page::Dashboard => super::pages::dashboard_page(ui, &ctx, self),
                    Page::QuickScan => super::pages::quick_scan_page(ui, self),
                    Page::VirusDb => super::pages::virus_db_page(ui, self),
                    Page::FullScan => super::pages::full_scan_page(ui, self),
                    Page::Settings => super::settings::settings_page(ui, self),
                }
            });

        crate::widgets::show_toasts(&ctx, &self.toasts);

        if about_open && !about_standalone {
            crate::about_dialog::paint_about_modal(&ctx);
        }
    }
}
