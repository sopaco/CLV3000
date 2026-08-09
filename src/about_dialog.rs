//! 轻量「关于」对话框：托盘菜单触发，单独跑一个小的 eframe 会话（用完即释放 OpenGL）。

use crate::clamav_info::ClamAvInfo;
use crate::icon_data;
use crate::theme::{self, colors};
use crate::widgets;
use eframe::egui;
use egui::{TextureHandle, Vec2, ViewportCommand};

const LOGO_TEX_ID: &str = "clv3000_about_logo";

/// 阻塞直到用户关闭关于窗。
pub fn show_standalone() {
    let info = ClamAvInfo::gather();
    let icon = icon_data::load_app_icon(64);
    let window_icon = egui::IconData {
        rgba: icon.0.clone(),
        width: icon.1,
        height: icon.2,
    };

    let native_options = eframe::NativeOptions {
        viewport: about_viewport_builder(window_icon),
        ..Default::default()
    };

    let _ = eframe::run_native(
        "About CLV3000",
        native_options,
        Box::new(move |cc| Ok(Box::new(AboutDialog::new(&cc.egui_ctx, info)))),
    );
}

fn about_viewport_builder(window_icon: egui::IconData) -> egui::ViewportBuilder {
    let size = Vec2::new(400.0, 340.0);
    let mut builder = egui::ViewportBuilder::default()
        .with_title("About CLV3000")
        .with_inner_size(size)
        .with_min_inner_size(size)
        .with_max_inner_size(size)
        .with_resizable(false)
        .with_icon(window_icon);

    #[cfg(windows)]
    {
        builder = builder.with_decorations(true);
    }

    #[cfg(not(windows))]
    {
        builder = builder
            .with_decorations(false)
            .with_title_shown(false)
            .with_titlebar_shown(false)
            .with_titlebar_buttons_shown(false)
            .with_fullsize_content_view(true);
    }

    builder
}

struct AboutDialog {
    info: ClamAvInfo,
    logo: TextureHandle,
}

impl AboutDialog {
    fn new(ctx: &egui::Context, info: ClamAvInfo) -> Self {
        theme::apply(ctx);
        Self {
            info,
            logo: load_logo_texture(ctx),
        }
    }
}

impl eframe::App for AboutDialog {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let mut close = false;

        #[cfg(not(windows))]
        paint_frameless_chrome(ui, &mut close);

        egui::CentralPanel::default()
            .frame(egui::Frame::default().fill(colors::BG_APP))
            .show(ui, |ui| {
                paint_about_body(ui, &self.logo, &self.info);
                ui.add_space(12.0);
                ui.vertical_centered(|ui| {
                    if ok_button(ui).clicked() {
                        close = true;
                    }
                });
            });

        if close {
            ui.ctx().send_viewport_cmd(ViewportCommand::Close);
        }
    }
}

fn load_logo_texture(ctx: &egui::Context) -> TextureHandle {
    let (rgba, w, h) = icon_data::load_app_icon(120);
    let image = egui::ColorImage::from_rgba_unmultiplied([w as usize, h as usize], &rgba);
    ctx.load_texture(LOGO_TEX_ID, image, egui::TextureOptions::LINEAR)
}

fn paint_about_body(ui: &mut egui::Ui, logo: &TextureHandle, info: &ClamAvInfo) {
    ui.vertical_centered(|ui| {
        ui.add_space(8.0);
        ui.add(
            egui::Image::new((logo.id(), logo.size_vec2()))
                .fit_to_exact_size(Vec2::splat(88.0))
                .corner_radius(14.0),
        );
        ui.add_space(10.0);
        widgets::bold_label(ui, "CLV3000", 20.0, colors::TEXT_PRIMARY);
        ui.label(
            egui::RichText::new(format!("Version {}", env!("CARGO_PKG_VERSION")))
                .color(colors::TEXT_SECONDARY)
                .small(),
        );
        ui.add_space(14.0);
        info_row(ui, "ClamAV Engine", &info.engine);
        ui.add_space(6.0);
        info_row(ui, "Virus Database", &info.database);
        ui.add_space(6.0);
        ui.label(
            egui::RichText::new("Fast, reliable virus protection for even older PCs.")
                .color(colors::TEXT_MUTED)
                .small(),
        );
    });
}

fn info_row(ui: &mut egui::Ui, label: &str, value: &str) {
    ui.horizontal(|ui| {
        ui.add_space((ui.available_width() - 320.0).max(0.0) / 2.0);
        ui.vertical(|ui| {
            ui.set_width(320.0);
            ui.label(egui::RichText::new(label).color(colors::TEXT_MUTED).small());
            ui.label(egui::RichText::new(value).color(colors::TEXT_PRIMARY));
        });
    });
}

fn ok_button(ui: &mut egui::Ui) -> egui::Response {
    ui.add(
        egui::Button::new(egui::RichText::new("OK").color(colors::TEXT_PRIMARY))
            .fill(colors::ACCENT_BLUE_BG)
            .stroke(egui::Stroke::new(1.0, colors::BORDER))
            .min_size(Vec2::new(120.0, 32.0)),
    )
}

#[cfg(not(windows))]
fn paint_frameless_chrome(ui: &mut egui::Ui, close_requested: &mut bool) {
    egui::Panel::top("about_title_bar")
        .exact_size(36.0)
        .show_separator_line(false)
        .frame(egui::Frame::default().fill(colors::BG_TITLEBAR))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.add_space(12.0);
                widgets::bold_label(ui, "About CLV3000", 14.0, colors::TEXT_PRIMARY);
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.button("✕").clicked() {
                        *close_requested = true;
                    }
                });
            });
        });
}
