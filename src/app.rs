//! 主界面：仪表盘 / 闪电扫描 / 病毒库 / 全盘扫描 四个页面 + 自绘标题栏 + 全局底部资源条。

use crate::config::{AppConfig, ScanRecord};
use crate::localtime::Timestamp;
use crate::paths;
use crate::scan::{self, CancelFlag, ScanEvent, ScanKind, Threat};
use crate::sysmon::{self, ResourceSample, SysMonHandle};
use crate::theme::{self, colors};
use crate::tray::Tray;
use crate::widgets::{self, ThreatAction, Toast};
use eframe::egui;
use egui::{Color32, Stroke, Vec2, ViewportCommand};
use muda::MenuEvent;
use std::path::PathBuf;
use std::sync::atomic::Ordering;
use std::sync::mpsc::Receiver;
use std::time::Duration;
use tray_icon::TrayIconEvent;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Page {
    Dashboard,
    QuickScan,
    VirusDb,
    FullScan,
}

enum ScanPhase {
    Idle,
    /// 仅闪电扫描会经历这个阶段：进程/模块枚举中。
    Enumerating {
        done: usize,
        total: usize,
        files_found: usize,
    },
    Scanning {
        /// 全盘扫描不知道总数，只能显示"已扫描 N 个"；闪电扫描枚举完就知道总数了。
        total: Option<usize>,
        scanned: usize,
        current_path: String,
    },
    Done {
        scanned: usize,
        elapsed: Duration,
        cancelled: bool,
    },
}

struct ScanPageState {
    kind: ScanKind,
    phase: ScanPhase,
    cancel: Option<CancelFlag>,
    rx: Option<Receiver<ScanEvent>>,
    threats: Vec<Threat>,
    last_error: Option<String>,
}

impl ScanPageState {
    fn new(kind: ScanKind) -> Self {
        Self {
            kind,
            phase: ScanPhase::Idle,
            cancel: None,
            rx: None,
            threats: Vec::new(),
            last_error: None,
        }
    }

    fn is_running(&self) -> bool {
        matches!(
            self.phase,
            ScanPhase::Enumerating { .. } | ScanPhase::Scanning { .. }
        )
    }

    fn start(&mut self, scan_removable: bool) {
        if self.is_running() {
            return;
        }
        self.threats.clear();
        self.last_error = None;
        let cancel = scan::new_cancel_flag();
        let (tx, rx) = std::sync::mpsc::channel();
        self.cancel = Some(cancel.clone());
        self.rx = Some(rx);
        self.phase = match self.kind {
            ScanKind::Quick => ScanPhase::Enumerating {
                done: 0,
                total: 0,
                files_found: 0,
            },
            ScanKind::Full => ScanPhase::Scanning {
                total: None,
                scanned: 0,
                current_path: String::new(),
            },
        };
        match self.kind {
            ScanKind::Quick => {
                std::thread::spawn(move || scan::quick_scan::run(tx, cancel));
            }
            ScanKind::Full => {
                std::thread::spawn(move || scan::full_scan::run(tx, cancel, scan_removable));
            }
        }
    }

    fn request_cancel(&self) {
        if let Some(c) = &self.cancel {
            c.store(true, Ordering::SeqCst);
        }
    }

    /// 返回 `Some((scanned, elapsed, cancelled))` 当这一批事件里出现了 `Finished`。
    fn poll(&mut self, config: &AppConfig) -> Option<(usize, Duration, bool)> {
        let mut finished = None;
        let Some(rx) = &self.rx else { return None };
        while let Ok(event) = rx.try_recv() {
            match event {
                ScanEvent::Enumerating {
                    processes_done,
                    processes_total,
                    files_found,
                } => {
                    self.phase = ScanPhase::Enumerating {
                        done: processes_done,
                        total: processes_total,
                        files_found,
                    };
                }
                ScanEvent::FileScanned { path, infected } => {
                    let total_hint = match &self.phase {
                        ScanPhase::Enumerating { files_found, .. } => Some(*files_found),
                        ScanPhase::Scanning { total, .. } => *total,
                        _ => None,
                    };
                    let scanned_before = match &self.phase {
                        ScanPhase::Scanning { scanned, .. } => *scanned,
                        _ => 0,
                    };
                    if let Some(name) = infected
                        && !config.is_ignored(&path, &name)
                    {
                        self.threats.push(Threat {
                            path: PathBuf::from(&path),
                            virus_name: name,
                        });
                    }
                    self.phase = ScanPhase::Scanning {
                        total: total_hint,
                        scanned: scanned_before + 1,
                        current_path: path,
                    };
                }
                ScanEvent::Finished {
                    scanned,
                    elapsed,
                    cancelled,
                } => {
                    self.phase = ScanPhase::Done {
                        scanned,
                        elapsed,
                        cancelled,
                    };
                    finished = Some((scanned, elapsed, cancelled));
                }
                ScanEvent::Error(e) => {
                    self.last_error = Some(e);
                }
            }
        }
        finished
    }
}

struct VirusDbState {
    updating: bool,
    rx: Option<Receiver<Result<(), String>>>,
}

impl VirusDbState {
    fn new() -> Self {
        Self {
            updating: false,
            rx: None,
        }
    }

    fn start_update(&mut self) {
        if self.updating || !paths::freshclam_available() {
            return;
        }
        let (tx, rx) = std::sync::mpsc::channel();
        self.rx = Some(rx);
        self.updating = true;
        std::thread::spawn(move || {
            let result = run_freshclam();
            let _ = tx.send(result);
        });
    }

    /// 返回本次轮询里出现的结果（如果有）。
    fn poll(&mut self) -> Option<Result<(), String>> {
        let mut result = None;
        if let Some(rx) = &self.rx
            && let Ok(r) = rx.try_recv()
        {
            self.updating = false;
            result = Some(r);
        }
        result
    }
}

#[cfg(windows)]
fn run_freshclam() -> Result<(), String> {
    use std::os::windows::process::CommandExt;
    use std::process::{Command, Stdio};

    let mut cmd = Command::new(paths::freshclam_path());
    cmd.arg(format!(
        "--datadir={}",
        paths::clamav_database_dir().display()
    ))
    .stdout(Stdio::null())
    .stderr(Stdio::null())
    .creation_flags(0x0800_0000);

    match cmd.status() {
        Ok(status) if status.success() => Ok(()),
        Ok(status) => Err(format!("更新病毒库失败，退出码 {status}")),
        Err(e) => Err(format!("无法启动 freshclam：{e}")),
    }
}

/// 开发预览用：不真的联网更新，睡一下模拟"正在更新"的等待感，然后报成功。
#[cfg(not(windows))]
fn run_freshclam() -> Result<(), String> {
    std::thread::sleep(Duration::from_millis(1200));
    Ok(())
}

pub struct App {
    page: Page,
    config: AppConfig,
    quick: ScanPageState,
    full: ScanPageState,
    virus_db: VirusDbState,
    sysmon: SysMonHandle,
    last_sample: ResourceSample,
    toasts: Vec<Toast>,
    about_open: bool,
    allow_exit: bool,
    tray: Option<Tray>,
    /// 完整品牌图标的纹理（`icon_app.png`），"关于"区块用它显示 logo。
    app_icon_texture: egui::TextureHandle,
    /// 简化版图标的纹理（`icon_tray.png`），自绘标题栏左上角那个小图标用它，
    /// 跟窗口图标/托盘图标保持视觉一致。
    titlebar_icon_texture: egui::TextureHandle,
}

/// 把 `icon_data::load_*_icon` 解出来的 `(rgba, w, h)` 传进 egui 的纹理系统，
/// 拿到一个能在 `egui::Image` 里直接用的 `TextureHandle`。
fn load_texture(
    ctx: &egui::Context,
    name: &str,
    (rgba, w, h): (Vec<u8>, u32, u32),
) -> egui::TextureHandle {
    let color_image = egui::ColorImage::from_rgba_unmultiplied([w as usize, h as usize], &rgba);
    ctx.load_texture(name, color_image, egui::TextureOptions::LINEAR)
}

impl App {
    pub fn new(_cc: &eframe::CreationContext<'_>, tray: Option<Tray>) -> Self {
        theme::apply(&_cc.egui_ctx);
        crate::fonts::install_cjk_font(&_cc.egui_ctx);

        let app_icon_texture = load_texture(&_cc.egui_ctx, "app_icon", crate::icon_data::load_app_icon(160));
        let titlebar_icon_texture =
            load_texture(&_cc.egui_ctx, "titlebar_icon", crate::icon_data::load_tray_icon(64));

        Self {
            page: Page::Dashboard,
            config: AppConfig::load(),
            quick: ScanPageState::new(ScanKind::Quick),
            full: ScanPageState::new(ScanKind::Full),
            virus_db: VirusDbState::new(),
            sysmon: sysmon::spawn(),
            last_sample: ResourceSample::default(),
            toasts: Vec::new(),
            about_open: false,
            allow_exit: false,
            tray,
            app_icon_texture,
            titlebar_icon_texture,
        }
    }

    fn toast(&mut self, text: impl Into<String>) {
        self.toasts.push(Toast::new(text));
    }

    fn navigate(&mut self, ctx: &egui::Context, page: Page) {
        self.page = page;
        ctx.send_viewport_cmd(ViewportCommand::Visible(true));
        ctx.send_viewport_cmd(ViewportCommand::Focus);
    }

    fn poll_tray(&mut self, ctx: &egui::Context) {
        let Some(tray) = &self.tray else { return };

        while let Ok(event) = TrayIconEvent::receiver().try_recv() {
            if let TrayIconEvent::DoubleClick { .. } = event {
                ctx.send_viewport_cmd(ViewportCommand::Visible(true));
                ctx.send_viewport_cmd(ViewportCommand::Minimized(false));
                ctx.send_viewport_cmd(ViewportCommand::Focus);
            }
        }

        while let Ok(event) = MenuEvent::receiver().try_recv() {
            let id = event.id();
            if id == &tray.ids.show {
                ctx.send_viewport_cmd(ViewportCommand::Visible(true));
                ctx.send_viewport_cmd(ViewportCommand::Minimized(false));
                ctx.send_viewport_cmd(ViewportCommand::Focus);
            } else if id == &tray.ids.quick_scan {
                ctx.send_viewport_cmd(ViewportCommand::Visible(true));
                ctx.send_viewport_cmd(ViewportCommand::Minimized(false));
                ctx.send_viewport_cmd(ViewportCommand::Focus);
                self.page = Page::QuickScan;
                self.quick.start(self.config.scan_removable_drives);
            } else if id == &tray.ids.about {
                ctx.send_viewport_cmd(ViewportCommand::Visible(true));
                ctx.send_viewport_cmd(ViewportCommand::Focus);
                self.about_open = true;
            } else if id == &tray.ids.quit {
                self.allow_exit = true;
                ctx.send_viewport_cmd(ViewportCommand::Close);
            }
        }
    }

    fn poll_scans(&mut self) {
        if let Some((scanned, elapsed, cancelled)) = self.quick.poll(&self.config) {
            if !cancelled {
                self.config.last_quick_scan = Some(ScanRecord {
                    time: Timestamp::now(),
                    threats_found: self.quick.threats.len(),
                    scanned_count: scanned,
                });
                self.config.save();
            }
            let _ = elapsed;
        }
        if let Some((scanned, elapsed, cancelled)) = self.full.poll(&self.config) {
            if !cancelled {
                self.config.last_full_scan = Some(ScanRecord {
                    time: Timestamp::now(),
                    threats_found: self.full.threats.len(),
                    scanned_count: scanned,
                });
                self.config.save();
            }
            let _ = elapsed;
        }
        if let Some(result) = self.virus_db.poll() {
            match result {
                Ok(()) => self.toast("病毒库更新完成"),
                Err(e) => self.toast(format!("病毒库更新失败：{e}")),
            }
        }
    }
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        // egui 0.36 把 SidePanel/TopBottomPanel/CentralPanel 都统一成了直接消费传入
        // `ui` 剩余空间的容器，不再像旧版本那样单独走 `ctx.show(...)`。这里先拿一份
        // `Context` 的 clone（内部是 Arc，克隆很便宜），后面凡是需要 `&Context` 的地方
        // （viewport 命令、input 查询）都用它，`ui` 则专门用来摆放各个面板。
        let ctx = ui.ctx().clone();

        // 关闭按钮默认改成"最小化到托盘"，只有托盘菜单里的"退出"才真正关程序。
        if ctx.input(|i| i.viewport().close_requested()) && !self.allow_exit {
            ctx.send_viewport_cmd(ViewportCommand::CancelClose);
            ctx.send_viewport_cmd(ViewportCommand::Visible(false));
        }

        self.poll_tray(&ctx);
        self.poll_scans();
        if let Ok(sample) = self.sysmon.rx.try_recv() {
            self.last_sample = sample;
        }
        self.toasts.retain(|t| !t.expired());

        // 保持轮询节奏，这样即使窗口被隐藏到托盘，托盘点击也能在不太离谱的延迟内被处理到。
        ctx.request_repaint_after(Duration::from_millis(250));

        title_bar(ui, &ctx, &self.titlebar_icon_texture);

        egui::Panel::bottom("resource_bar")
            .exact_size(50.0)
            .resizable(false)
            // Panel 默认会在边缘画一条分割线，用的是主题里偏亮的 noninteractive
            // stroke，在深色底上显得很突兀（白边）——设计稿里几个区域之间基本靠
            // 背景色深浅本身区分，没有这种硬分割线，所以统一关掉。
            .show_separator_line(false)
            .frame(
                egui::Frame::default()
                    .fill(colors::BG_APP)
                    .inner_margin(egui::Margin::symmetric(20, 10)),
            )
            .show(ui, |ui| resource_bar(ui, self.last_sample));

        egui::Panel::left("sidebar")
            .exact_size(64.0)
            .resizable(false)
            .show_separator_line(false)
            .frame(egui::Frame::default().fill(colors::BG_SIDEBAR))
            .show(ui, |ui| sidebar(ui, &ctx, self));

        egui::CentralPanel::default()
            .frame(egui::Frame::default().fill(colors::BG_APP))
            .show(ui, |ui| {
                theme::paint_dotted_background(ui.painter(), ui.max_rect());
                match self.page {
                    Page::Dashboard => dashboard_page(ui, &ctx, self),
                    Page::QuickScan => quick_scan_page(ui, self),
                    Page::VirusDb => virus_db_page(ui, self),
                    Page::FullScan => full_scan_page(ui, self),
                }
            });

        if self.about_open {
            about_window(&ctx, self);
        }

        widgets::show_toasts(&ctx, &self.toasts);
    }
}

const TITLE_BAR_HEIGHT: f32 = 44.0;

fn title_bar(ui: &mut egui::Ui, ctx: &egui::Context, icon_texture: &egui::TextureHandle) {
    egui::Panel::top("title_bar")
        .exact_size(TITLE_BAR_HEIGHT)
        .resizable(false)
        .show_separator_line(false)
        .frame(egui::Frame::default().fill(colors::BG_TITLEBAR))
        .show(ui, |ui| {
            // 按钮位置直接按 `full_rect` 算好精确坐标，不走 `ui.horizontal` 的光标累加——
            // 光标累加容易因为间距估算偏差导致最后一个按钮离边缘忽远忽近，看起来"没对齐"。
            let full_rect = ui.max_rect();
            let btn_size = 32.0;
            let btn_gap = 4.0;
            let edge_margin = 8.0;

            let close_rect = egui::Rect::from_center_size(
                egui::pos2(
                    full_rect.right() - edge_margin - btn_size / 2.0,
                    full_rect.center().y,
                ),
                Vec2::splat(btn_size),
            );
            let min_rect = egui::Rect::from_center_size(
                egui::pos2(
                    close_rect.left() - btn_gap - btn_size / 2.0,
                    full_rect.center().y,
                ),
                Vec2::splat(btn_size),
            );

            if title_bar_button(ui, close_rect, "close", |painter, rect| {
                icons::close(painter, rect, Stroke::new(1.4, colors::TEXT_SECONDARY));
            }) {
                ctx.send_viewport_cmd(ViewportCommand::Visible(false));
            }
            if title_bar_button(ui, min_rect, "minimize", |painter, rect| {
                icons::minimize(painter, rect, Stroke::new(1.4, colors::TEXT_SECONDARY));
            }) {
                ctx.send_viewport_cmd(ViewportCommand::Minimized(true));
            }

            // 左边图标 + 标题文字，走正常的光标布局就行，反正不需要跟右边对齐。
            // 图标用真实的简化版美术图标（icon_tray.png），跟窗口图标/托盘图标
            // 保持一致，不再用矢量画的盾牌。
            ui.horizontal_centered(|ui| {
                ui.add_space(14.0);
                ui.add(
                    egui::Image::new((icon_texture.id(), icon_texture.size_vec2()))
                        .fit_to_exact_size(Vec2::splat(22.0)),
                );
                ui.add_space(8.0);
                widgets::bold_label(ui, "CLV3000", 15.0, colors::TEXT_PRIMARY);
            });

            // 标题文字和右侧按钮之间的整段空白区域用来拖动窗口。
            let drag_rect = egui::Rect::from_min_max(
                egui::pos2(full_rect.left() + 140.0, full_rect.top()),
                egui::pos2(min_rect.left() - btn_gap, full_rect.bottom()),
            );
            if drag_rect.width() > 0.0 {
                let drag_resp = ui.interact(
                    drag_rect,
                    ui.id().with("titlebar_drag"),
                    egui::Sense::drag(),
                );
                if drag_resp.drag_started() {
                    ctx.send_viewport_cmd(ViewportCommand::StartDrag);
                }
            }
        });
}

fn title_bar_button(
    ui: &mut egui::Ui,
    rect: egui::Rect,
    name: &str,
    draw: impl FnOnce(&egui::Painter, egui::Rect),
) -> bool {
    let response = ui
        .interact(
            rect,
            ui.id().with(("titlebar_btn", name)),
            egui::Sense::click(),
        )
        .on_hover_cursor(egui::CursorIcon::PointingHand);
    let painter = ui.painter();
    if response.hovered() {
        painter.rect_filled(rect, 6.0, colors::ACCENT_BLUE_BG);
    }
    let glyph_rect = rect.shrink(9.0);
    draw(painter, glyph_rect);
    response.clicked()
}

use crate::icons;

struct SidebarItem {
    page: Page,
    draw: fn(&egui::Painter, egui::Rect, Stroke),
}

fn sidebar(ui: &mut egui::Ui, ctx: &egui::Context, app: &mut App) {
    ui.add_space(18.0);
    let items = [
        SidebarItem {
            page: Page::Dashboard,
            draw: |p, r, s| icons::shield(p, r, s, None),
        },
        SidebarItem {
            page: Page::QuickScan,
            draw: |p, r, s| icons::bolt(p, r, s.color),
        },
        SidebarItem {
            page: Page::FullScan,
            draw: |p, r, s| icons::hamburger(p, r, s),
        },
        SidebarItem {
            page: Page::VirusDb,
            draw: |p, r, s| icons::database(p, r, s),
        },
    ];

    for item in items {
        let active = app.page == item.page;
        ui.vertical_centered(|ui| {
            let size = Vec2::splat(40.0);
            let (response, painter) = ui.allocate_painter(size, egui::Sense::click());
            let response = response.on_hover_cursor(egui::CursorIcon::PointingHand);
            if active {
                painter.rect_filled(response.rect, 10.0, colors::ACCENT_BLUE_BG);
            } else if response.hovered() {
                painter.rect_filled(response.rect, 10.0, colors::BG_CARD);
            }
            let color = if active {
                colors::ACCENT_BLUE
            } else {
                colors::TEXT_SECONDARY
            };
            let glyph_rect = response.rect.shrink(9.5);
            (item.draw)(&painter, glyph_rect, Stroke::new(1.6, color));
            if response.clicked() {
                app.navigate(ctx, item.page);
            }
        });
        ui.add_space(14.0);
    }
}

fn resource_bar(ui: &mut egui::Ui, sample: ResourceSample) {
    ui.horizontal(|ui| {
        ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
            ui.add_space(ui.available_width() / 2.0 - 160.0);
            resource_meter(ui, "CPU", sample.cpu_percent);
            ui.add_space(24.0);
            resource_meter(ui, "内存", sample.mem_percent());
        });
    });
}

fn resource_meter(ui: &mut egui::Ui, label: &str, percent: f32) {
    ui.label(egui::RichText::new(label).color(colors::TEXT_SECONDARY));
    let (response, painter) = ui.allocate_painter(Vec2::new(100.0, 8.0), egui::Sense::hover());
    let rect = response.rect;
    painter.rect_filled(rect, 4.0, colors::BG_CARD);
    let fraction = (percent / 100.0).clamp(0.0, 1.0);
    if fraction > 0.0 {
        let filled =
            egui::Rect::from_min_size(rect.min, Vec2::new(rect.width() * fraction, rect.height()));
        painter.rect_filled(filled, 4.0, theme::accent_for(percent));
    }
    ui.label(egui::RichText::new(format!("{percent:.0}%")).color(colors::TEXT_PRIMARY));
}

fn dashboard_page(ui: &mut egui::Ui, ctx: &egui::Context, app: &mut App) {
    let today = Timestamp::now();
    let has_threats = app
        .config
        .last_full_scan
        .as_ref()
        .map(|r| r.threats_found > 0)
        .unwrap_or(false)
        || app
            .config
            .last_quick_scan
            .as_ref()
            .map(|r| r.threats_found > 0)
            .unwrap_or(false);

    ui.vertical_centered(|ui| {
        ui.add_space(60.0);
        let (color, title) = if has_threats {
            (colors::RED, "系统状态：存在风险")
        } else {
            (colors::GREEN, "系统状态：安全")
        };

        // 画布比圆环本身大一圈，专门留给外面那层光晕，不然会被裁掉。
        const DIAMETER: f32 = 180.0;
        const GLOW_MARGIN: f32 = 60.0;
        let (response, painter) = ui.allocate_painter(
            Vec2::splat(DIAMETER + GLOW_MARGIN * 2.0),
            egui::Sense::hover(),
        );
        let center = response.rect.center();
        let radius = DIAMETER / 2.0 - 4.0;
        widgets::paint_glow(&painter, center, radius, color);
        painter.circle_filled(center, radius, colors::BG_CARD);
        painter.circle_stroke(center, radius, Stroke::new(3.0, color));
        let glyph_rect = egui::Rect::from_center_size(center, Vec2::splat(DIAMETER * 0.50));
        if has_threats {
            icons::warning_triangle(&painter, glyph_rect, Stroke::new(2.4, color), None);
        } else {
            icons::shield_check(&painter, glyph_rect, Stroke::new(2.4, color));
        }

        ui.add_space(20.0);
        widgets::bold_label(ui, title, 20.0, colors::TEXT_PRIMARY);
        ui.add_space(6.0);
        let sub = match &app.config.last_full_scan {
            Some(r) if r.threats_found == 0 => {
                format!(
                    "上次全盘扫描 · {} · 未发现威胁",
                    r.time.display_relative_to(&today)
                )
            }
            Some(r) => format!(
                "上次全盘扫描 · {} · 发现 {} 个威胁",
                r.time.display_relative_to(&today),
                r.threats_found
            ),
            None => "尚未进行过全盘扫描".to_string(),
        };
        ui.label(egui::RichText::new(sub).color(colors::TEXT_SECONDARY));

        ui.add_space(28.0);
        ui.horizontal(|ui| {
            ui.add_space(ui.available_width() / 2.0 - 160.0);
            if action_button(ui, "闪电扫描", |p, r, s| icons::bolt(p, r, s.color)) {
                app.navigate(ctx, Page::QuickScan);
                app.quick.start(app.config.scan_removable_drives);
            }
            ui.add_space(12.0);
            if action_button(ui, "全盘扫描", icons::database) {
                app.navigate(ctx, Page::FullScan);
                app.full.start(app.config.scan_removable_drives);
            }
        });
    });
}

/// 一个"图标 + 文字"的胶囊按钮。
///
/// 不能直接用 `Frame::show(...)`——`Frame`/`ui.horizontal` 默认会把自己的
/// "期望尺寸"报成父容器当前的全部可用宽度（因为它们要等内容画完才知道真实大小，
/// 只能先占住最大空间），这样一来外层 `vertical_centered` 之类的居中布局，
/// 拿到的就是一个"和容器一样宽"的东西，居中也就没有意义——表现出来就是按钮
/// 从容器最左边一路铺到很宽的位置，而不是一颗居中的小胶囊。
///
/// 解决办法是自己先量出图标+文字真正需要的尺寸，把这个"小尺寸"传给
/// `allocate_ui_with_layout`，父容器的 `Align::Center` 才有东西可对齐。
fn action_button(
    ui: &mut egui::Ui,
    label: &str,
    draw: impl FnOnce(&egui::Painter, egui::Rect, Stroke),
) -> bool {
    const ICON_SIZE: f32 = 21.0; // 原本 18，整体图标 +15% 的一部分
    const ICON_GAP: f32 = 10.0;
    const H_PAD: f32 = 16.0;
    const V_PAD: f32 = 10.0;

    let font_id = egui::FontId::proportional(14.0);
    let galley = ui
        .ctx()
        .fonts_mut(|f| f.layout_no_wrap(label.to_owned(), font_id, colors::TEXT_PRIMARY));
    let text_size = galley.size();

    let desired = Vec2::new(
        H_PAD * 2.0 + ICON_SIZE + ICON_GAP + text_size.x,
        V_PAD * 2.0 + ICON_SIZE.max(text_size.y),
    );

    let bg_shape_idx = ui.painter().add(egui::Shape::Noop);
    let response = ui
        .allocate_ui_with_layout(
            desired,
            egui::Layout::left_to_right(egui::Align::Center),
            |ui| {
                ui.add_space(H_PAD);
                let (icon_resp, painter) =
                    ui.allocate_painter(Vec2::splat(ICON_SIZE), egui::Sense::hover());
                draw(
                    &painter,
                    icon_resp.rect,
                    Stroke::new(1.6, colors::ACCENT_BLUE),
                );
                ui.add_space(ICON_GAP);
                ui.label(egui::RichText::new(label).color(colors::TEXT_PRIMARY));
                ui.add_space(H_PAD);
            },
        )
        .response;

    let bg_rect = response.rect;
    let interact = ui
        .interact(bg_rect, response.id.with("btn"), egui::Sense::click())
        .on_hover_cursor(egui::CursorIcon::PointingHand);
    let fill = if interact.hovered() {
        colors::ACCENT_BLUE_BG
    } else {
        colors::BG_CARD
    };
    let shape = egui::epaint::RectShape::new(
        bg_rect,
        egui::CornerRadius::same(12),
        fill,
        Stroke::new(1.0, colors::BORDER),
        egui::epaint::StrokeKind::Inside,
    );
    ui.painter().set(bg_shape_idx, egui::Shape::Rect(shape));

    interact.clicked()
}

fn quick_scan_page(ui: &mut egui::Ui, app: &mut App) {
    scan_page(
        ui,
        &mut app.quick,
        &mut app.config,
        &mut app.toasts,
        "闪电扫描",
        colors::ACCENT_BLUE,
        |p, r, s| icons::bolt(p, r, s.color),
        true,
    );
}

fn full_scan_page(ui: &mut egui::Ui, app: &mut App) {
    // 按需求去掉了"包含可移动磁盘"设置项，全盘扫描默认只扫固定磁盘
    // （`config.scan_removable_drives` 字段还留着，默认 false，只是不再从 UI 暴露）。
    scan_page(
        ui,
        &mut app.full,
        &mut app.config,
        &mut app.toasts,
        "全盘扫描",
        colors::ACCENT_BLUE,
        icons::hamburger,
        true,
    );
}

/// 一个内容"量多少占多少"的居中卡片：宽高固定，父容器的居中布局才有东西可对齐
/// （原理和 `action_button` 里说的一样，`Frame` 自己没法参与外层的 `Align::Center`）。
fn centered_card(
    ui: &mut egui::Ui,
    width: f32,
    height: f32,
    add_contents: impl FnOnce(&mut egui::Ui),
) {
    let bg_idx = ui.painter().add(egui::Shape::Noop);
    let response = ui
        .allocate_ui_with_layout(
            Vec2::new(width, height),
            egui::Layout::left_to_right(egui::Align::Center),
            |ui| {
                ui.add_space(16.0);
                add_contents(ui);
            },
        )
        .response;
    let shape = egui::epaint::RectShape::new(
        response.rect,
        egui::CornerRadius::same(10),
        colors::BG_CARD,
        Stroke::new(1.0, colors::BORDER),
        egui::epaint::StrokeKind::Inside,
    );
    ui.painter().set(bg_idx, egui::Shape::Rect(shape));
}

// 参数是有点多，但拆成一个配置 struct 目前收益不大（调用点就 2 个，字段名本身
// 已经很直白），先用 allow 压掉这条 lint。
#[allow(clippy::too_many_arguments)]
fn scan_page(
    ui: &mut egui::Ui,
    state: &mut ScanPageState,
    config: &mut AppConfig,
    toasts: &mut Vec<Toast>,
    title: &str,
    ring_color: Color32,
    icon: fn(&egui::Painter, egui::Rect, Stroke),
    show_start_button_when_idle: bool,
) {
    ui.vertical_centered(|ui| {
        ui.add_space(24.0);
        match &state.phase {
            ScanPhase::Idle => {
                // 仪表盘页那种"大圆环 + 图标"的视觉语言在这里也来一份，跟概览页
                // 呼应，不然闪电扫描/全盘扫描的待机画面只有两行字，太空。这里没有
                // 状态色（还没扫描，谈不上安全/危险），就用页面自己的强调色。
                let (deco_resp, painter) =
                    ui.allocate_painter(Vec2::splat(120.0), egui::Sense::hover());
                let deco_center = deco_resp.rect.center();
                let deco_radius = 56.0;
                painter.circle_filled(deco_center, deco_radius, colors::BG_CARD);
                painter.circle_stroke(deco_center, deco_radius, Stroke::new(2.0, ring_color));
                let deco_glyph = egui::Rect::from_center_size(deco_center, Vec2::splat(52.0));
                icon(&painter, deco_glyph, Stroke::new(2.0, ring_color));

                ui.add_space(14.0);
                ui.label(egui::RichText::new(format!("准备{title}")).color(colors::TEXT_SECONDARY));
                ui.add_space(16.0);
                if show_start_button_when_idle
                    && action_button(ui, &format!("开始{title}"), icon)
                {
                    state.start(config.scan_removable_drives);
                }
            }
            ScanPhase::Enumerating {
                done,
                total,
                files_found,
            } => {
                widgets::progress_ring(
                    ui,
                    220.0,
                    None,
                    ring_color,
                    "枚举中",
                    &format!("{done}/{total} 进程"),
                );
                ui.add_space(16.0);
                widgets::centered_stat_pills(
                    ui,
                    &[
                        (format!("{done} / {total}"), "进程"),
                        (files_found.to_string(), "文件"),
                    ],
                );
                ui.add_space(10.0);
                if ui.link("取消扫描").clicked() {
                    state.request_cancel();
                }
            }
            ScanPhase::Scanning {
                total,
                scanned,
                current_path,
            } => {
                let percent = total.map(|t| {
                    if t == 0 {
                        1.0
                    } else {
                        *scanned as f32 / t as f32
                    }
                });
                let title_text = percent
                    .map(|p| format!("{:.0}%", p * 100.0))
                    .unwrap_or_else(|| format!("{scanned}"));
                widgets::bold_label(ui, &format!("正在{title}"), 14.0, colors::TEXT_PRIMARY);
                ui.add_space(6.0);
                widgets::progress_ring(ui, 220.0, percent, ring_color, &title_text, "");
                ui.add_space(4.0);
                ui.label(
                    egui::RichText::new(truncate(current_path, 60))
                        .color(colors::TEXT_SECONDARY)
                        .small(),
                );
                ui.add_space(16.0);
                let first_pill = match total {
                    Some(t) => (format!("{scanned} / {t}"), "文件"),
                    None => (scanned.to_string(), "已扫描"),
                };
                widgets::centered_stat_pills(
                    ui,
                    &[first_pill, (state.threats.len().to_string(), "威胁")],
                );
                ui.add_space(10.0);
                if ui.link("取消扫描").clicked() {
                    state.request_cancel();
                }
            }
            ScanPhase::Done {
                scanned,
                elapsed,
                cancelled,
            } => {
                let has_threats = !state.threats.is_empty();
                let color = if has_threats {
                    colors::RED
                } else {
                    colors::GREEN
                };
                const DIAMETER: f32 = 140.0;
                const GLOW_MARGIN: f32 = 50.0;
                let (response, painter) = ui.allocate_painter(
                    Vec2::splat(DIAMETER + GLOW_MARGIN * 2.0),
                    egui::Sense::hover(),
                );
                let center = response.rect.center();
                let radius = DIAMETER / 2.0 - 4.0;
                widgets::paint_glow(&painter, center, radius, color);
                painter.circle_filled(center, radius, colors::BG_CARD);
                painter.circle_stroke(center, radius, Stroke::new(3.0, color));
                let glyph_rect = egui::Rect::from_center_size(center, Vec2::splat(DIAMETER * 0.50));
                if has_threats {
                    icons::warning_triangle(&painter, glyph_rect, Stroke::new(2.2, color), None);
                } else {
                    icons::shield_check(&painter, glyph_rect, Stroke::new(2.2, color));
                }
                ui.add_space(14.0);
                let heading = if *cancelled {
                    "扫描已取消".to_string()
                } else if has_threats {
                    format!("发现 {} 个威胁", state.threats.len())
                } else {
                    "未发现威胁".to_string()
                };
                widgets::bold_label(ui, &heading, 18.0, colors::TEXT_PRIMARY);
                ui.label(
                    egui::RichText::new(format!(
                        "{title} · 用时 {} · 已扫描 {scanned} 个文件",
                        format_duration(*elapsed)
                    ))
                    .color(colors::TEXT_SECONDARY)
                    .small(),
                );
                ui.add_space(16.0);
                if action_button(ui, &format!("重新{title}"), icon) {
                    state.start(config.scan_removable_drives);
                }
            }
        }

        if let Some(err) = &state.last_error {
            ui.add_space(10.0);
            ui.label(egui::RichText::new(err).color(colors::RED));
        }

        ui.add_space(20.0);
        let mut ignore_target: Option<usize> = None;
        for (i, threat) in state.threats.iter().enumerate() {
            ui.horizontal(|ui| {
                ui.add_space((ui.available_width() - 700.0).max(0.0) / 2.0);
                ui.vertical(|ui| {
                    ui.set_width(700.0_f32.min(ui.available_width()));
                    let path_str = threat.path.display().to_string();
                    let action = widgets::threat_card(ui, &threat.virus_name, &path_str);
                    match action {
                        ThreatAction::Ignore => ignore_target = Some(i),
                        ThreatAction::Quarantine => {
                            toasts.push(Toast::new("隔离功能将在后续版本中提供"));
                        }
                        ThreatAction::None => {}
                    }
                    ui.add_space(8.0);
                });
            });
        }
        if let Some(i) = ignore_target {
            let t = state.threats.remove(i);
            config.add_ignored(t.path.display().to_string(), t.virus_name);
        }
    });
}

fn virus_db_page(ui: &mut egui::Ui, app: &mut App) {
    ui.add_space(28.0);
    ui.columns(2, |columns| {
        virus_db_status_column(&mut columns[0], app);
        virus_db_about_column(&mut columns[1], app);
    });
}

/// 左栏：病毒库状态 + 手动更新交互。
fn virus_db_status_column(ui: &mut egui::Ui, app: &mut App) {
    ui.vertical_centered(|ui| {
        let (response, painter) = ui.allocate_painter(Vec2::splat(96.0), egui::Sense::hover());
        icons::database(
            &painter,
            response.rect.shrink(6.0),
            Stroke::new(2.0, colors::ACCENT_BLUE),
        );
        ui.add_space(14.0);
        widgets::bold_label(ui, "病毒库", 18.0, colors::TEXT_PRIMARY);
        ui.add_space(14.0);

        let available = paths::clamscan_available();
        let status = if available {
            "内置病毒库已就绪".to_string()
        } else {
            format!("未找到扫描引擎：{}", paths::clamav_dir().display())
        };
        let path_text = format!("路径：{}", paths::clamav_database_dir().display());

        // 状态信息包一层卡片，别让文字裸露在圆点背景上——单独两行文字堆在那没有
        // 视觉容器，看起来像页面没做完，包一层就有"这是一块信息"的感觉了。
        let status_galley = ui.ctx().fonts_mut(|f| {
            f.layout_no_wrap(
                status.clone(),
                egui::FontId::proportional(14.0),
                colors::TEXT_SECONDARY,
            )
        });
        let path_galley = ui.ctx().fonts_mut(|f| {
            f.layout_no_wrap(
                path_text.clone(),
                egui::FontId::proportional(12.0),
                colors::TEXT_MUTED,
            )
        });
        // 卡片最大宽度不能超过这一栏的可用宽度，不然会溢出到右栏那边去。
        let card_width = (status_galley.size().x.max(path_galley.size().x) + 40.0)
            .min(ui.available_width() - 8.0);
        centered_card(ui, card_width, 58.0, |ui| {
            ui.vertical(|ui| {
                ui.label(egui::RichText::new(&status).color(colors::TEXT_SECONDARY));
                ui.label(
                    egui::RichText::new(&path_text)
                        .color(colors::TEXT_MUTED)
                        .small(),
                );
            });
        });

        ui.add_space(16.0);
        let label = if app.virus_db.updating {
            "正在更新…"
        } else {
            "手动更新病毒库"
        };
        if action_button(ui, label, icons::database) && !app.virus_db.updating {
            app.virus_db.start_update();
            app.toast("开始更新病毒库…");
        }
    });
}

/// 右栏：关于（真实品牌图标 + 名称 + 版本 + 简介）。
fn virus_db_about_column(ui: &mut egui::Ui, app: &App) {
    ui.vertical_centered(|ui| {
        ui.add_space(20.0);
        // 用真实美术图标（贴图），跟左栏功能性 UI 的矢量图标风格特意区分开——
        // 这里是想展示"这是什么产品"，用实际品牌图标比扁平线框图标更合适。
        let logo_size = 72.0;
        ui.add(
            egui::Image::new((app.app_icon_texture.id(), app.app_icon_texture.size_vec2()))
                .fit_to_exact_size(Vec2::splat(logo_size))
                .corner_radius(16.0),
        );
        ui.add_space(12.0);
        widgets::bold_label(ui, "CLV3000", 17.0, colors::TEXT_PRIMARY);
        ui.add_space(4.0);
        ui.label(
            egui::RichText::new(format!("版本 {}", env!("CARGO_PKG_VERSION")))
                .color(colors::TEXT_SECONDARY)
                .small(),
        );
        ui.add_space(10.0);
        ui.label(
            egui::RichText::new("极速、可靠、高效的病毒防护程序，适合各类老旧系统和电脑")
                .color(colors::TEXT_MUTED)
                .small(),
        );
    });
}

fn about_window(ctx: &egui::Context, app: &mut App) {
    let mut open = app.about_open;
    egui::Window::new("关于 CLV3000")
        .collapsible(false)
        .resizable(false)
        .open(&mut open)
        .show(ctx, |ui| {
            widgets::bold_label(ui, "CLV3000", 18.0, colors::TEXT_PRIMARY);
            ui.label(format!("版本 {}", env!("CARGO_PKG_VERSION")));
            ui.add_space(8.0);
            ui.label(
                egui::RichText::new("极速、可靠、高效的病毒防护程序，适合各类老旧系统和电脑。")
                    .color(colors::TEXT_SECONDARY)
                    .small(),
            );
        });
    app.about_open = open;
}

fn truncate(s: &str, max_chars: usize) -> String {
    if s.chars().count() <= max_chars {
        s.to_string()
    } else {
        let head: String = s.chars().take(max_chars).collect();
        format!("{head}…")
    }
}

fn format_duration(d: Duration) -> String {
    let secs = d.as_secs();
    let m = secs / 60;
    let s = secs % 60;
    if m > 0 {
        format!("{m} 分 {s} 秒")
    } else {
        format!("{s} 秒")
    }
}
