//! 仪表盘页面：页头 + 状态英雄卡（状态环 + 主操作）+ 三张信息卡。
//!
//! 2026-10 结构改版：原来只有"居中一坨"（环 + 标题 + 两个按钮），页面大半
//! 空白、和扫描页长得一样。现在仪表盘是"信息首页"：英雄卡给状态与主操作，
//! 信息卡行汇总上次扫描/病毒库状态，整页信息密度对标主流 AV 产品的首页。

use super::super::core::ScanPhase;
use super::super::{App, Page};
use crate::icons;
use crate::localtime::Timestamp;
use crate::theme::colors;
use crate::widgets::{self, action_button_named, primary_button_named};
use eframe::egui;
use egui::{Color32, Stroke, Vec2};

pub(in crate::app) fn dashboard_page(ui: &mut egui::Ui, ctx: &egui::Context, app: &mut App) {
    let today = Timestamp::now();

    // 信息卡要展示病毒库版本；None 时触发一次后台刷新（真实环境读 clamscan -V，
    // 只发一次线程；预览/测试里会预置 Some 跳过，见 ui_preview 公共底座）。
    if app.core.virus_db.db_version.is_none() {
        app.core.virus_db.refresh_db_version(ctx.clone());
    }

    // 窗口很窄时（MIN_INNER_SIZE 440）页头+英雄卡+三张卡的总高会超出内容区，
    // 包一层滚动兜底；正常尺寸下不出现滚动条。
    egui::ScrollArea::vertical().show(ui, |ui| {
        ui.add_space(20.0);
        widgets::page_header(
            ui,
            |p, r, s| icons::shield(p, r, s, None),
            "Dashboard",
            "Current protection status",
            |_| {},
        );
        ui.add_space(18.0);
        // 英雄卡/信息卡行统一：左右各留 PAGE_INSET 的整行容器（Frame 默认只包
        // 内容那么大，不撞满宽度的话卡片会收缩成内容宽，也吃不到页边距）。
        inset_row(ui, |ui| hero_card(ui, app));
        ui.add_space(14.0);
        inset_row(ui, |ui| stat_cards(ui, app, &today));
    });
}

/// 整行容器：左右各留 `PAGE_INSET`，内容（Frame 卡片等）撞满中间宽度。
fn inset_row(ui: &mut egui::Ui, add_contents: impl FnOnce(&mut egui::Ui)) {
    ui.horizontal(|ui| {
        ui.add_space(widgets::PAGE_INSET);
        ui.vertical(|ui| {
            ui.set_width((ui.available_width() - widgets::PAGE_INSET).max(200.0));
            add_contents(ui);
        });
    });
}

/// 英雄卡状态：首次使用 / 安全 / 有威胁 / 扫描进行中。共用一张卡的骨架，
/// 只换颜色、文案与按钮——用户扫一眼颜色就知道现在是什么局面。
enum HeroFlavor {
    /// 两次扫描都没有记录。首次使用不能声称 "Secure"（还没验证过），也不再用
    /// "Never / Not run yet" 反复强调空白，而是直接给出下一步。
    FirstRun,
    Secure,
    AtRisk,
    Running {
        /// 扫描进行中的是全盘扫描时，中心图形换主机图标（闪电扫描用闪电）。
        full: bool,
    },
}

fn hero_card(ui: &mut egui::Ui, app: &mut App) {
    let quick_running = app.core.quick.is_running();
    let full_running = app.core.full.is_running();
    let running = quick_running || full_running;
    let first_run = app.core.config.last_quick_scan.is_none()
        && app.core.config.last_full_scan.is_none();

    let flavor = if running {
        HeroFlavor::Running { full: full_running }
    } else if last_scan_threats(app) > 0 {
        HeroFlavor::AtRisk
    } else if first_run {
        HeroFlavor::FirstRun
    } else {
        HeroFlavor::Secure
    };

    let (color, title, sub): (Color32, String, String) = match &flavor {
        HeroFlavor::Running { .. } => {
            let state = if quick_running {
                &app.core.quick
            } else {
                &app.core.full
            };
            let kind = if quick_running { "Quick" } else { "Full" };
            let progress = match &state.phase {
                ScanPhase::Enumerating { done, total, .. } => {
                    format!("{done} / {total} processes enumerated")
                }
                ScanPhase::Scanning { total, scanned, .. } => match total {
                    Some(t) => format!("{scanned} / {t} files scanned"),
                    None => format!("{scanned} files scanned"),
                },
                _ => "Preparing…".to_string(),
            };
            (
                colors::ACCENT_BLUE,
                format!("{kind} Scan in progress"),
                progress,
            )
        }
        HeroFlavor::AtRisk => (
            colors::RED,
            "System Status: At Risk".to_string(),
            "Threats were found in the last scan. Review them to restore protection.".to_string(),
        ),
        HeroFlavor::FirstRun => (
            colors::ACCENT_BLUE,
            "Start with a full scan".to_string(),
            "Build your protection baseline".to_string(),
        ),
        HeroFlavor::Secure => {
            // 副标题只给"结论"，不给"时间"——上次扫描的具体时间在下面 FULL SCAN
            // 信息卡里已经有一份，英雄卡再写一遍就是同一件事说两次。
            (colors::GREEN, "System Status: Secure".to_string(), "No threats in your last scan".to_string())
        }
    };

    egui::Frame::default()
        .fill(colors::BG_CARD)
        .stroke(Stroke::new(1.0, colors::BORDER))
        .corner_radius(crate::theme::radius::LG)
        .inner_margin(egui::Margin::symmetric(24, 22))
        .show(ui, |ui| {
            // Frame 只包内容那么大——主动撞满可用宽度，英雄卡才能铺满整行。
            ui.set_min_width(ui.available_width());
            let inner_w = ui.available_width();
            // 英雄卡原来只有"环 + 文字 + 按钮"靠左一坨，右半张卡空着（实测内容
            // 只占 635px 卡里的 390px，右侧留下 200 多像素的空白，看着像没排
            // 完）。改成"状态在左、动作在右"的三栏：状态环 | 标题/副标题 | 动作。
            // 三栏都显式定宽，整行宽度恰好等于卡内宽——右缘跟下面三张信息卡
            // 严丝合缝对齐，不会像之前那样把卡片撑出内容列。
            let spec = hero_actions_spec(running, first_run);
            let actions_w = hero_actions_width(ui, &spec);
            const RING_BOX: f32 = 200.0; // 120px 环 + 两侧各 40px 光晕留白
            const GAP: f32 = 10.0;
            // 横向布局会在两个子控件之间自动插入 item_spacing.x（主题里 10），
            // 宽度预算里必须算进去，否则整行会宽出 2×10 而把卡片撑破。
            let spacing_x = ui.spacing().item_spacing.x;
            let gap_extra = (GAP - spacing_x).max(0.0);
            let text_w = inner_w - RING_BOX - GAP - actions_w - GAP;
            if text_w >= 180.0 {
                // 纵向对齐：三栏（环 / 文字 / 动作）各自算好内容高度，再按同一个
                // 中线排布——之前用 `ui.horizontal` 的 cross-align 想让它们自动
                // 居中，但 egui 的横向布局 cross-align 只在"同一行的 cell 之间"
                // 生效，cell 内部的 `ui.vertical` 仍是从顶部开始排（实测标题顶
                // 端 y=175、环中心 y=269，差了 94px，看着全靠上）。这里改成显式
                // 计算：环占固定高 RING_BOX，文字块与动作列各自算出内容高度，
                // 上下各补 (RING_BOX - 内容高)/2 的空白，三者共享同一条中线。
                let text_h = widgets::measure_text_height(ui, &title, 20.0)
                    + 6.0
                    + widgets::measure_wrapped_text_height(ui, &sub, 14.0, text_w);
                let actions_h = hero_actions_height(ui, &spec);
                ui.horizontal_top(|ui| {
                    hero_ring(ui, &flavor, color);
                    ui.add_space(gap_extra);
                    ui.vertical(|ui| {
                        // 文字列定宽：不定的话长副标题会一路顶到右缘、把右侧
                        // 动作列压到文字底下（看着像两列叠在一起）。
                        ui.set_width(text_w);
                        ui.add_space(((RING_BOX - text_h) / 2.0).max(0.0));
                        hero_text(ui, &flavor, &title, &sub);
                    });
                    ui.add_space(gap_extra);
                    ui.vertical(|ui| {
                        ui.set_width(actions_w);
                        ui.add_space(((RING_BOX - actions_h) / 2.0).max(0.0));
                        hero_actions(ui, app, &spec);
                    });
                });
            } else {
                // 窄窗口：横排会挤爆文字块，退化为竖排居中。
                ui.vertical_centered(|ui| {
                    hero_ring(ui, &flavor, color);
                    hero_text(ui, &flavor, &title, &sub);
                    ui.add_space(16.0);
                    hero_actions(ui, app, &spec);
                });
            }
        });
}

/// 英雄卡的标题 + 副标题。
fn hero_text(ui: &mut egui::Ui, flavor: &HeroFlavor, title: &str, sub: &str) {
    let color = match flavor {
        HeroFlavor::FirstRun => colors::ACCENT_BLUE,
        HeroFlavor::Secure => colors::GREEN,
        HeroFlavor::AtRisk => colors::RED,
        HeroFlavor::Running { .. } => colors::ACCENT_BLUE,
    };
    widgets::bold_label(ui, title, 20.0, color);
    ui.add_space(6.0);
    ui.label(egui::RichText::new(sub).color(colors::TEXT_SECONDARY));
}

/// 英雄卡的动作列（竖排两颗按钮；扫描中只有一颗"View Progress"）。
fn hero_actions(ui: &mut egui::Ui, app: &mut App, spec: &HeroActions) {
    let (primary_label, primary_page) = spec.primary;
    // 无障碍名跟可见文字区分开：跟侧栏同名导航项撞名会让 kittest
    // `get_by_label` 多匹配 panic；"Start …" 仍包含可见文字（WCAG 2.5.3）。
    let primary_accessible = format!("Start {primary_label}");
    if spec.primary_is_secondary {
        // 扫描进行中：唯一可点的是"去看进度"（次要操作，描边形态）——真正的
        // 取消按钮在扫描页页头，这里不重复。
        let target = if app.core.quick.is_running() {
            Page::QuickScan
        } else {
            Page::FullScan
        };
        if action_button_named(
            ui,
            primary_label,
            &primary_accessible,
            |p, r, s| icons::shield(p, r, s, None),
        ) {
            app.navigate(target);
        }
        return;
    }
    let primary_icon: fn(&egui::Painter, egui::Rect, Stroke) = match primary_page {
        Page::FullScan => icons::computer,
        _ => |p, r, s| icons::bolt(p, r, s.color),
    };
    if primary_button_named(ui, primary_label, &primary_accessible, primary_icon) {
        start_from_dashboard(app, primary_page);
    }
    if let Some((secondary_label, secondary_page)) = spec.secondary {
        ui.add_space(12.0);
        let secondary_accessible = format!("Start {secondary_label}");
        let secondary_icon: fn(&egui::Painter, egui::Rect, Stroke) = match secondary_page {
            Page::FullScan => icons::computer,
            _ => |p, r, s| icons::bolt(p, r, s.color),
        };
        if action_button_named(ui, secondary_label, &secondary_accessible, secondary_icon) {
            start_from_dashboard(app, secondary_page);
        }
    }
}

/// 从 Dashboard 启动扫描：先跳页，再检查是否已有扫描在跑。
fn start_from_dashboard(app: &mut App, page: Page) {
    app.navigate(page);
    if app.core.any_scan_running() {
        app.toast("Finish the current scan before starting another");
        return;
    }
    let removable = app.core.config.scan_removable_drives;
    match page {
        Page::QuickScan => app.core.quick.start(removable),
        Page::FullScan => app.core.full.start(removable),
        _ => unreachable!("只有扫描页能从这里启动"),
    }
}

/// 英雄卡右侧的动作列规格：按钮文字 + 目标页面 + 是否主操作。
/// 宽度/高度/绘制三处共用同一份规格，避免"量宽用一套、画出来是另一套"。
struct HeroActions {
    primary: (&'static str, Page),
    secondary: Option<(&'static str, Page)>,
    /// 扫描进行中时只有一个"View Progress"按钮，它是次要操作（描边），不是主操作。
    primary_is_secondary: bool,
}

fn hero_actions_spec(running: bool, first_run: bool) -> HeroActions {
    if running {
        return HeroActions {
            primary: ("View Progress", Page::QuickScan),
            secondary: None,
            primary_is_secondary: true,
        };
    }
    // 首次使用：主行动是 Full Scan（先建立保护基线）；之后主行动回到 Quick Scan。
    let (primary, secondary) = if first_run {
        (("Full Scan", Page::FullScan), ("Quick Scan", Page::QuickScan))
    } else {
        (("Quick Scan", Page::QuickScan), ("Full Scan", Page::FullScan))
    };
    HeroActions {
        primary,
        secondary: Some(secondary),
        primary_is_secondary: false,
    }
}

/// 动作列的宽度（最宽的那颗按钮）。按钮本体宽度要自己量——不能拿"含外侧
/// item_spacing"的估算值，否则列会比实际宽 20px。
fn hero_actions_width(ui: &egui::Ui, spec: &HeroActions) -> f32 {
    let primary = if spec.primary_is_secondary {
        widgets::action_button_width(ui, spec.primary.0)
    } else {
        widgets::primary_button_body_width(ui, spec.primary.0)
    };
    match spec.secondary {
        Some((label, _)) => primary.max(widgets::action_button_width(ui, label)),
        None => primary,
    }
}

/// 动作列的总高度（纵向居中预算用）。
fn hero_actions_height(ui: &egui::Ui, spec: &HeroActions) -> f32 {
    let primary = if spec.primary_is_secondary {
        widgets::action_button_height(ui, spec.primary.0)
    } else {
        widgets::primary_button_body_height(ui, spec.primary.0)
    };
    match spec.secondary {
        Some((label, _)) => primary + 12.0 + widgets::action_button_height(ui, label),
        None => primary,
    }
}

/// 英雄卡左侧的状态环：120px 圆 + 光晕 + 中心图形。安全=盾牌打勾、有威胁=
/// 警告三角、扫描中=对应扫描类型的图标（闪电/主机），跟标题文字同色。
fn hero_ring(ui: &mut egui::Ui, flavor: &HeroFlavor, color: Color32) {
    const DIAMETER: f32 = 120.0;
    const GLOW_MARGIN: f32 = 40.0;
    let (response, painter) = ui.allocate_painter(
        Vec2::splat(DIAMETER + GLOW_MARGIN * 2.0),
        egui::Sense::hover(),
    );
    let center = response.rect.center();
    let radius = DIAMETER / 2.0 - 4.0;
    widgets::paint_glow(&painter, center, radius, color);
    painter.circle_filled(center, radius, colors::BG_CARD);
    painter.circle_stroke(center, radius, Stroke::new(3.0, color));
    let glyph_rect = egui::Rect::from_center_size(center, Vec2::splat(DIAMETER * 0.46));
    match flavor {
        HeroFlavor::FirstRun => icons::shield(&painter, glyph_rect, Stroke::new(2.4, color), None),
        HeroFlavor::Secure => icons::status_glyph_secure(&painter, glyph_rect, color),
        HeroFlavor::AtRisk => icons::status_glyph_at_risk(&painter, glyph_rect, color),
        HeroFlavor::Running { full } => {
            if *full {
                icons::computer(&painter, glyph_rect, Stroke::new(2.4, color));
            } else {
                icons::bolt(&painter, glyph_rect, color);
            }
        }
    }
}

/// 上次扫描（快/全取更糟的那个）共发现多少威胁——决定英雄卡是 Secure
/// 还是 At Risk。
fn last_scan_threats(app: &App) -> usize {
    [
        app.core
            .config
            .last_quick_scan
            .as_ref()
            .map(|r| r.threats_found),
        app.core
            .config
            .last_full_scan
            .as_ref()
            .map(|r| r.threats_found),
    ]
    .into_iter()
    .flatten()
    .max()
    .unwrap_or(0)
}

/// 一张信息卡的展示数据。先把数据从 `app` 里读出来（全 owned），渲染闭包里
/// 不再借用 app，点击后的 navigate 攒到渲染结束后统一处理——借用检查器友好。
struct CardSpec {
    icon: fn(&egui::Painter, egui::Rect, Stroke),
    label: &'static str,
    value: String,
    detail: String,
    value_color: Color32,
    detail_color: Color32,
    page: Page,
}

/// 三张信息卡：上次闪电扫描 / 上次全盘扫描 / 病毒库。整卡可点跳转对应页面；
/// 宽窗口三张并排，窄窗口纵向堆叠（ScrollArea 兜底滚动）。
fn stat_cards(ui: &mut egui::Ui, app: &mut App, today: &Timestamp) {
    let scan_spec = |icon: fn(&egui::Painter, egui::Rect, Stroke),
                     label: &'static str,
                     page: Page,
                     record: Option<&crate::config::ScanRecord>| {
        let (value, detail) = match record {
            Some(r) => (
                r.time.display_relative_to(today),
                format!(
                    "{} files · {} threat{}",
                    crate::config::format_count(r.scanned_count),
                    r.threats_found,
                    if r.threats_found == 1 { "" } else { "s" }
                ),
            ),
            // 空状态给"这项扫描能做什么 + 预计多久"，不再写 Never / Run a scan
            // to see results here——新用户需要的是下一步，而不是被提醒什么都没做。
            // 说明行控制在 ~13 字以内：信息卡内文宽只有 ~156px，写长了会被
            // 中间省略成 "Running processe…t a minute" 这种半截话。
            None => match page {
                Page::QuickScan => (
                    "Fast first check".to_string(),
                    "About a minute".to_string(),
                ),
                Page::FullScan => (
                    "Build your baseline".to_string(),
                    "About 5–15 min".to_string(),
                ),
                _ => unreachable!("只有两种扫描信息卡"),
            },
        };
        CardSpec {
            icon,
            label,
            value,
            detail,
            value_color: colors::TEXT_PRIMARY,
            detail_color: match record.map(|r| r.threats_found) {
                Some(n) if n > 0 => colors::RED,
                _ => colors::TEXT_MUTED,
            },
            page,
        }
    };

    let db_version = app.core.virus_db.db_version.clone();
    let engine_ok = app.core.virus_db.engine_probe().available;
    // 版本串的生产形态是 `28088 (Sat Oct 25 09:32:57 2026)`
    // （clamav_info::parse_database_from_clamscan_v）。整个塞进信息卡的 16px 主值位
    // 必然被中间省略成 `2808…2026` 这种没用的碎片——拆成"库编号当主值、日期当
    // 说明行"，两个字段都完整可读。
    let (db_value, db_date) = match &db_version {
        Some(v) => {
            let (build, date) = crate::clamav_info::split_version(v);
            (crate::clamav_info::format_build_number(&build), date)
        }
        None => ("Checking…".to_string(), None),
    };
    let db_spec = CardSpec {
        icon: icons::database,
        label: "VIRUS DATABASE",
        value: db_value,
        detail: match (engine_ok, db_date) {
            (false, _) => "Scan engine not found".to_string(),
            (true, Some(date)) => date,
            (true, None) => "Built-in engine ready".to_string(),
        },
        value_color: colors::TEXT_PRIMARY,
        detail_color: if engine_ok {
            colors::TEXT_MUTED
        } else {
            colors::RED
        },
        page: Page::VirusDb,
    };

    let specs = vec![
        scan_spec(
            |p, r, s| icons::bolt(p, r, s.color),
            "QUICK SCAN",
            Page::QuickScan,
            app.core.config.last_quick_scan.as_ref(),
        ),
        scan_spec(
            icons::computer,
            "FULL SCAN",
            Page::FullScan,
            app.core.config.last_full_scan.as_ref(),
        ),
        db_spec,
    ];

    let mut goto: Option<Page> = None;
    if ui.available_width() >= 560.0 {
        // 三张卡等宽并排。两个关键点（都是 2026-10 预览探针踩过的坑）：
        // 1. 不用 ui.columns（它在首列前也吞一个 item_spacing，三张卡的左右边距
        //    会不对称）；
        // 2. 用 horizontal_top 而不是 horizontal——后者的 cross-align 是 Center，
        //    卡片一旦变高，后续卡会在被撑大的行里垂直居中，顶就不再对齐。
        //
        // 宽度预算还要算上 egui 在横向布局的两个子控件之间自动插入的
        // item_spacing.x（主题里 10）：只算自己加的 GAP 的话，一行三张卡实际
        // 比内容列宽出 2×10=20px，右缘被窗口裁掉（实测卡右缘 x=887，内容列
        // 右缘 x=867）。
        const GAP: f32 = 12.0;
        let avail = ui.available_width();
        let spacing_x = ui.spacing().item_spacing.x;
        let card_w = ((avail - GAP * 2.0) / 3.0).max(120.0);
        ui.horizontal_top(|ui| {
            for (i, spec) in specs.into_iter().enumerate() {
                if i > 0 {
                    // 隐含的 item_spacing 已经在两卡之间，这里只补足到 GAP。
                    ui.add_space((GAP - spacing_x).max(0.0));
                }
                // 卡自己分配高度（内容决定），外层 cell 只负责给宽度。
                if widgets::stat_card(
                    ui,
                    card_w,
                    spec.icon,
                    spec.label,
                    &spec.value,
                    &spec.detail,
                    spec.value_color,
                    spec.detail_color,
                ) {
                    goto = Some(spec.page);
                }
            }
        });
    } else {
        ui.vertical(|ui| {
            for spec in specs {
                if widgets::stat_card(
                    ui,
                    ui.available_width(),
                    spec.icon,
                    spec.label,
                    &spec.value,
                    &spec.detail,
                    spec.value_color,
                    spec.detail_color,
                ) {
                    goto = Some(spec.page);
                }
                ui.add_space(8.0);
            }
        });
    }

    if let Some(page) = goto {
        app.navigate(page);
    }
}

#[cfg(test)]
mod tests {
    //! egui_kittest 驱动**真实**仪表盘的 UI 测试（见 AGENTS.md「UI 代码验证流程」）。
    //! 注意：App::new 会读真实 config.toml（只读），这里覆写成默认值保证确定性；
    //! db_version 预置 Some 跳过 clamscan -V 子进程（AGENTS.md 禁止 harness 起子进程）。

    use super::*;
    use crate::lifecycle::InitialMode;
    use egui_kittest::Harness;
    use egui_kittest::kittest::Queryable;

    /// 信息卡整卡可点、且无障碍名不与侧栏同名项撞车（"Open …" 句式）。
    #[test]
    fn dashboard_stat_cards_navigate_on_click() {
        let mut harness = Harness::builder()
            .with_size(egui::Vec2::new(900.0, 600.0))
            .with_max_steps(10)
            .build_ui_state(
                |ui, app| {
                    let app = app.get_or_insert_with(|| {
                        let mut app = App::new(ui.ctx(), None, InitialMode::ShowWindow);
                        app.core.config = crate::config::AppConfig::default();
                        app.core.virus_db.db_version = Some("ClamAV 1.4.4/28088".to_string());
                        app
                    });
                    let ctx = ui.ctx().clone();
                    dashboard_page(ui, &ctx, app);
                },
                None,
            );
        harness.run();

        harness.get_by_label("Open QUICK SCAN").click();
        harness.run();
        let app = harness.state().as_ref().expect("App 已在首帧创建");
        assert!(
            matches!(app.core.page, Page::QuickScan),
            "点击信息卡应跳转对应页面"
        );
    }
}
