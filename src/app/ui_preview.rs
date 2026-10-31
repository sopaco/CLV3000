//! UI 视觉预览工具：用 egui_kittest 驱动**真实** `App`（`run_ui` 完整走
//! 标题栏/侧栏/资源条/中央面板），把各页面×各状态光栅化成 PNG 落到
//! `target/ui_preview/`，供设计走查与视觉评审——弥补"开发机 cargo run 看不了
//! 截图"的空档（见 .claude/skills/clv3000-design 末尾的检查清单）。
//!
//! 运行：`cargo test ui_preview -- --ignored`，然后看 `target/ui_preview/*.png`。
//!
//! 标记 `#[ignore]`：常规 `cargo test` 不跑（渲染 13 张 PNG 会让毫秒级的
//! 测试套件变慢），需要时显式运行。只写 `target/`（gitignored），无其它副作用；
//! 状态全部手工构造，不点任何会落盘/起子进程的路径（见 AGENTS.md 禁止事项）。

use super::Page;
use super::app_shell::App;
use crate::app::core::{ScanPageState, ScanPhase, SettingsTab};
use crate::config::{AppConfig, IgnoredEntry, QuarantineEntry, ScanRecord};
use crate::lifecycle::InitialMode;
use crate::localtime::Timestamp;
use crate::scan::{ScanKind, Threat};
use crate::sysmon::ResourceSample;
use egui_kittest::Harness;
use std::path::PathBuf;
use std::time::{Duration, Instant};

/// 与 `main.rs::build_viewport` 的默认窗口一致——预览即用户实际看到的比例。
const WINDOW_SIZE: egui::Vec2 = egui::Vec2::new(900.0, 600.0);

fn save(harness: &mut Harness<'_, Option<App>>, name: &str) -> image::RgbaImage {
    let image = harness.render().expect("CPU 光栅化渲染失败");
    let dir = std::path::Path::new("target/ui_preview");
    std::fs::create_dir_all(dir).expect("创建 target/ui_preview 失败");
    let path = dir.join(format!("{name}.png"));
    image.save(&path).expect("写 PNG 失败");
    println!("UI preview: {}", path.display());
    image
}

fn with_app<R>(harness: &mut Harness<'_, Option<App>>, f: impl FnOnce(&mut App) -> R) -> R {
    let app = harness
        .state_mut()
        .as_mut()
        .expect("App 在首帧 get_or_insert_with 时已创建");
    f(app)
}

fn reset_config(app: &mut App) {
    // 覆盖掉开发机真实 config.toml 的内容，保证预览确定性。
    app.core.config = AppConfig::default();
    app.core.config.last_quick_scan = None;
    app.core.config.last_full_scan = None;
}

fn past_timestamp(hour: u16, minute: u16) -> Timestamp {
    Timestamp {
        year: 2026,
        month: 10,
        day: 26,
        hour,
        minute,
    }
}

fn threats(count: usize) -> Vec<Threat> {
    (0..count)
        .map(|i| Threat {
            path: PathBuf::from(format!(
                r"C:\Users\saw\Downloads\Crack-Tool-v{i}\keygen.exe"
            )),
            virus_name: "Win.Trojan.Xpack-5".to_string(),
        })
        .collect()
}

#[test]
#[ignore = "视觉预览工具：cargo test ui_preview -- --ignored，产物在 target/ui_preview/"]
fn render_ui_preview() {
    // App 延迟到首帧再建：App::new 需要属于 harness 的 egui::Context。
    // get_or_insert_with 保证整个 harness 生命周期只建一次（sysmon 线程、
    // 纹理、wakeup 注册都只发生一次）。
    let mut harness = Harness::builder()
        .with_size(WINDOW_SIZE)
        .with_max_steps(10)
        .renderer(software_renderer::SoftwareRenderer::default())
        .build_ui_state(
            |ui, app| {
                let app =
                    app.get_or_insert_with(|| App::new(ui.ctx(), None, InitialMode::ShowWindow));
                app.run_ui(ui);
            },
            None,
        );

    // 用 run_steps 而不是 run：扫描页/进度环会持续 request_repaint（动画），
    // run() 会在 max_steps 处 panic；固定步数跑足布局收敛（垂直居中高度、
    // 弹窗锚定均在前几帧收敛）即可。
    let settle = |harness: &mut Harness<'_, Option<App>>| harness.run_steps(8);

    // ── 公共底座：确定性配置 + 非零资源条（否则 CPU/内存条恒为空）。
    // db_version 预置 Some：仪表盘信息卡会懒加载版本号，真实路径是 clamscan -V
    // 子进程——AGENTS.md 禁止 harness 起子进程，预置跳过（见 dashboard_page）。
    with_app(&mut harness, |app| {
        reset_config(app);
        // 预置**生产形态**的版本串：`clamscan -V` 解析出来是
        // `28167 (Sat Oct 28 09:32:57 2026)`（clamav_info::parse_database_from_clamscan_v），
        // 不是 clamscan 的原始输出行——之前预置原始行，预览里量到的截断效果
        // 跟真实运行不一致。编号刻意选 5 位（28167），用来验证千分位显示。
        app.core.virus_db.db_version = Some("28167 (Sat Oct 28 09:32:57 2026)".to_string());
        app.last_sample = ResourceSample {
            cpu_percent: 23.0,
            mem_used_bytes: 10_314_886_144,
            mem_total_bytes: 17_179_869_184,
        };
    });
    settle(&mut harness);

    // 0. 仪表盘 · 首次使用（没有任何扫描记录）——空状态是引导，不是"你什么都没做"
    let img = save(&mut harness, "00-dashboard-first-run");
    analysis::ascii_preview("00-dashboard-first-run", &img);
    analysis::probe_chrome(&img);

    // 1. 仪表盘 · 安全态（预置两条扫描记录，信息卡有内容可展示）
    with_app(&mut harness, |app| {
        app.core.config.last_quick_scan = Some(ScanRecord {
            time: past_timestamp(9, 12),
            threats_found: 0,
            scanned_count: 342,
        });
        app.core.config.last_full_scan = Some(ScanRecord {
            time: past_timestamp(8, 30),
            threats_found: 0,
            scanned_count: 128_450,
        });
    });
    settle(&mut harness);
    let img = save(&mut harness, "01-dashboard-secure");
    analysis::ascii_preview("01-dashboard-secure", &img);
    analysis::probe_chrome(&img);

    // 2. 仪表盘 · 风险态（上次全盘扫出过威胁）
    with_app(&mut harness, |app| {
        app.core.config.last_full_scan = Some(ScanRecord {
            time: past_timestamp(9, 12),
            threats_found: 3,
            scanned_count: 128_450,
        });
    });
    settle(&mut harness);
    let img = save(&mut harness, "02-dashboard-at-risk");
    analysis::ascii_preview("02-dashboard-at-risk", &img);

    // 3. 闪电扫描 · 空闲（简报卡预置上次记录，让卡片有内容可看）
    with_app(&mut harness, |app| {
        app.core.page = Page::QuickScan;
        reset_config(app);
        app.core.config.last_quick_scan = Some(ScanRecord {
            time: past_timestamp(9, 12),
            threats_found: 0,
            scanned_count: 342,
        });
    });
    settle(&mut harness);
    let img = save(&mut harness, "03-quick-idle");
    analysis::ascii_preview("03-quick-idle", &img);

    // 4. 闪电扫描 · 枚举进程
    with_app(&mut harness, |app| {
        app.core.quick.phase = ScanPhase::Enumerating {
            done: 120,
            total: 342,
            files_found: 89,
        };
    });
    settle(&mut harness);
    let img = save(&mut harness, "04-quick-enumerating");
    analysis::ascii_preview("04-quick-enumerating", &img);

    // 5. 闪电扫描 · 扫描中（已知总数 → 百分比进度 + 当前路径 + 已扫出 1 个威胁）
    with_app(&mut harness, |app| {
        app.core.quick.phase = ScanPhase::Scanning {
            total: Some(342),
            scanned: 128,
            current_path: r"C:\Users\saw\AppData\Local\Temp\setup_helper.dll".to_string(),
        };
        app.core.quick.started_at = Some(Instant::now() - Duration::from_secs(47));
        app.core.quick.threats = threats(1);
    });
    settle(&mut harness);
    let img = save(&mut harness, "05-quick-scanning");
    analysis::ascii_preview("05-quick-scanning", &img);

    // 6. 闪电扫描 · 完成（带威胁 → 入口按钮）
    with_app(&mut harness, |app| {
        app.core.quick.phase = ScanPhase::Done {
            scanned: 342,
            elapsed: Duration::from_secs(63),
            cancelled: false,
        };
        app.core.quick.started_at = None;
        app.core.quick.threats = threats(2);
        app.core.quick.threats_modal_open = false;
    });
    settle(&mut harness);
    let img = save(&mut harness, "06-quick-done-threats");
    analysis::ascii_preview("06-quick-done-threats", &img);

    // 7. 威胁列表弹窗（同一个 Done 状态，弹窗打开）
    with_app(&mut harness, |app| {
        app.core.quick.threats_modal_open = true;
    });
    settle(&mut harness);
    let img = save(&mut harness, "07-quick-threats-modal");
    analysis::ascii_preview("07-quick-threats-modal", &img);
    analysis::probe_modal_center(&img);

    // 8. 全盘扫描 · 空闲（同样预置上次记录，展示简报卡的有数据形态）
    with_app(&mut harness, |app| {
        app.core.quick = ScanPageState::new(ScanKind::Quick);
        app.core.page = Page::FullScan;
        app.core.config.last_full_scan = Some(ScanRecord {
            time: past_timestamp(8, 30),
            threats_found: 2,
            scanned_count: 128_450,
        });
    });
    settle(&mut harness);
    let img = save(&mut harness, "08-full-idle");
    analysis::ascii_preview("08-full-idle", &img);

    // 9. 全盘扫描 · 遍历磁盘准备阶段（未知总量 → 计数 + 旋转弧）
    with_app(&mut harness, |app| {
        app.core.full.phase = ScanPhase::Scanning {
            total: None,
            scanned: 0,
            current_path: String::new(),
        };
        app.core.full.started_at = Some(Instant::now() - Duration::from_secs(12));
        app.core.full.walk_files_found = 12_345;
    });
    settle(&mut harness);
    let img = save(&mut harness, "09-full-walking");
    analysis::ascii_preview("09-full-walking", &img);

    // 10. 病毒库页（版本号已在公共底座预置，避免触发真实 clamscan -V 子进程）
    with_app(&mut harness, |app| {
        app.core.full = ScanPageState::new(ScanKind::Full);
        app.core.page = Page::VirusDb;
    });
    settle(&mut harness);
    let img = save(&mut harness, "10-virus-db");
    analysis::ascii_preview("10-virus-db", &img);

    // 11. 设置页 · 隔离区/忽略列表（各造两条，看卡片样式）
    with_app(&mut harness, |app| {
        app.core.page = Page::Settings;
        app.core.settings.tab = SettingsTab::QuarantineIgnore;
        app.core.config.quarantined = vec![
            QuarantineEntry {
                original_path: r"C:\Users\saw\Downloads\crack.exe".to_string(),
                virus_name: "Win.Trojan.Xpack-5".to_string(),
                quarantined_at: past_timestamp(9, 12),
                stored_name: "q_0001.bin".to_string(),
            },
            QuarantineEntry {
                original_path: r"C:\Users\saw\AppData\Roaming\svchost32.exe".to_string(),
                virus_name: "Trojan.Agent.Generic.4d".to_string(),
                quarantined_at: past_timestamp(10, 3),
                stored_name: "q_0002.bin".to_string(),
            },
        ];
        app.core.config.ignored = vec![IgnoredEntry {
            path: r"C:\Tools\eicar.com".to_string(),
            virus_name: "Eicar-Test-Signature".to_string(),
        }];
    });
    settle(&mut harness);
    let img = save(&mut harness, "11-settings-quarantine");
    analysis::ascii_preview("11-settings-quarantine", &img);
    analysis::contrast_report();

    // 12. 设置页 · General（打开定时扫描的计划编辑器）
    with_app(&mut harness, |app| {
        app.core.settings.tab = SettingsTab::General;
        app.core.config.auto_scan_on_boot = true;
        app.core.config.auto_scan_after_update = true;
        app.core.config.scheduled_scan = Some(crate::config::ScheduleConfig {
            days_mask: 0b0100_1010,
            hour: 9,
            minute: 30,
            scan_full: false,
        });
    });
    settle(&mut harness);
    let img = save(&mut harness, "12-settings-general");
    analysis::ascii_preview("12-settings-general", &img);

    // 13. Toast 浮层（右下角）
    with_app(&mut harness, |app| {
        app.core.page = Page::Dashboard;
        // 状态 03 的 reset_config 清掉了扫描记录，这里补回来让卡片有内容。
        app.core.config.last_quick_scan = Some(ScanRecord {
            time: past_timestamp(9, 12),
            threats_found: 0,
            scanned_count: 342,
        });
        app.toasts.push(crate::widgets::Toast::new(
            "Finish the current scan before starting another",
        ));
    });
    settle(&mut harness);
    save(&mut harness, "13-dashboard-toast");

    // 14. 仪表盘 · 扫描进行中（英雄卡切到进度态 + View Progress 按钮）
    with_app(&mut harness, |app| {
        app.toasts.clear();
        app.core.quick.phase = ScanPhase::Scanning {
            total: Some(342),
            scanned: 128,
            current_path: r"C:\Users\saw\AppData\Local\Temp\setup_helper.dll".to_string(),
        };
        app.core.quick.started_at = Some(Instant::now() - Duration::from_secs(47));
    });
    settle(&mut harness);
    let img = save(&mut harness, "14-dashboard-scanning");
    analysis::ascii_preview("14-dashboard-scanning", &img);

    // 15. 闪电扫描 · 空闲（首次使用：无任何记录）——空状态是引导
    with_app(&mut harness, |app| {
        app.core.quick = ScanPageState::new(ScanKind::Quick);
        app.core.page = Page::QuickScan;
        reset_config(app);
    });
    settle(&mut harness);
    let img = save(&mut harness, "15-quick-idle-first-run");
    analysis::ascii_preview("15-quick-idle-first-run", &img);

    // 16. 全盘扫描 · 空闲（首次使用）：覆盖清单与耗时预期
    with_app(&mut harness, |app| {
        app.core.page = Page::FullScan;
        reset_config(app);
    });
    settle(&mut harness);
    let img = save(&mut harness, "16-full-idle-first-run");
    analysis::ascii_preview("16-full-idle-first-run", &img);
}

/// 纯 CPU 软件光栅化 `TestRenderer`：把 egui tessellate 出的三角形网格逐像素
/// 画进 RGBA 缓冲。egui_kittest 0.36 只带 wgpu 一种渲染器（重依赖、要 GPU），
/// 而 `snapshot` feature 只是对比机制不含光栅化——这里按 AGENTS.md / Cargo.toml
/// 注释里“CPU 光栅化、不需要 GPU”的既定意图补上这个缺口，零新增依赖。
///
/// 精度取舍（对视觉预览够用，非像素级回归基准）：
/// - 三角形按像素中心单采样；egui tessellator 自带羽化边（边缘渐变三角形），
///   抗锯齿效果由几何提供，单采样不引入硬边。
/// - 纹理双线性采样；wrap 模式按 `TextureOptions::wrap_mode` 区分 Repeat/Clamp。
/// - `Primitive::Callback` 跳过（本项目无自定义 GPU 绘制回调）。
mod software_renderer {
    use egui_kittest::TestRenderer;
    use std::collections::HashMap;

    /// 软件光栅化用的纹理存储：handle_delta 累积的完整图像 + 采样参数。
    #[derive(Default)]
    pub(crate) struct SoftwareRenderer {
        textures: HashMap<egui::TextureId, (egui::ColorImage, egui::TextureOptions)>,
    }

    impl TestRenderer for SoftwareRenderer {
        fn handle_delta(&mut self, delta: &mut egui::TexturesDelta) {
            for (id, deltas) in &delta.set {
                for d in deltas {
                    let egui::ImageData::Color(img) = &d.image;
                    match (self.textures.get_mut(id), d.pos) {
                        (Some((tex, _)), Some([x, y])) => {
                            // 字体图集增量扩展：把 patch 拷进已分配的纹理区域。
                            for row in 0..img.size[1] {
                                let src = row * img.size[0];
                                let dst = (y + row) * tex.size[0] + x;
                                tex.pixels[dst..dst + img.size[0]]
                                    .copy_from_slice(&img.pixels[src..src + img.size[0]]);
                            }
                        }
                        _ => {
                            self.textures.insert(*id, ((**img).clone(), d.options));
                        }
                    }
                }
            }
            for id in &delta.free {
                self.textures.remove(id);
            }
            // epaint 的 TexturesDelta 在 Drop 时 debug_assert 必须已被消费
            // （防止集成端丢了纹理更新）——消费完后清空标记已处理。
            delta.clear();
        }

        fn render(
            &mut self,
            ctx: &egui::Context,
            output: &egui::FullOutput,
        ) -> Result<image::RgbaImage, String> {
            let ppp = output.pixels_per_point;
            let screen_px = ctx.input(|i| i.viewport_rect()).size() * ppp;
            let (w, h) = (screen_px.x as usize, screen_px.y as usize);
            if w == 0 || h == 0 {
                return Err("空画布".to_string());
            }

            // premultiplied RGBA 浮点帧缓冲，最后统一转 straight-alpha u8。
            let mut buf = vec![[0.0f32; 4]; w * h];

            for clipped in ctx.tessellate(output.shapes.clone(), ppp) {
                let egui::epaint::Primitive::Mesh(mesh) = clipped.primitive else {
                    continue; // PaintCallback：本项目没有自定义 GPU 绘制
                };
                let clip_px = clipped.clip_rect * ppp;
                let x_min = (clip_px.left().max(0.0).floor() as i64).max(0);
                let x_max = (clip_px.right().min(w as f32).ceil() as i64).min(w as i64);
                let y_min = (clip_px.top().max(0.0).floor() as i64).max(0);
                let y_max = (clip_px.bottom().min(h as f32).ceil() as i64).min(h as i64);
                if x_min >= x_max || y_min >= y_max {
                    continue;
                }

                let tex = self.textures.get(&mesh.texture_id);
                let (chunks, _rest) = mesh.indices.as_chunks::<3>();
                for tri in chunks {
                    let [a, b, c] = [
                        mesh.vertices[tri[0] as usize],
                        mesh.vertices[tri[1] as usize],
                        mesh.vertices[tri[2] as usize],
                    ];
                    rasterize_triangle(&mut buf, w, h, &[a, b, c], tex, ppp);
                }
            }

            let mut pixels = Vec::with_capacity(w * h * 4);
            for px in buf {
                let a = px[3];
                let rgb = if a > 0.0 {
                    // premultiplied → straight alpha
                    [px[0] / a, px[1] / a, px[2] / a]
                } else {
                    [0.0; 3]
                };
                for c in rgb {
                    pixels.push((c * 255.0).round().clamp(0.0, 255.0) as u8);
                }
                pixels.push((a * 255.0).round().clamp(0.0, 255.0) as u8);
            }
            Ok(image::RgbaImage::from_raw(w as u32, h as u32, pixels)
                .expect("缓冲长度与画布尺寸一致"))
        }
    }

    /// 画一个三角形（像素中心采样 + 重心插值 + 纹理调制 + premultiplied src-over）。
    /// 顶点坐标是逻辑点，乘 `ppp` 转像素；egui 不保证绕向，按面积符号自适应。
    fn rasterize_triangle(
        buf: &mut [[f32; 4]],
        w: usize,
        h: usize,
        [v0, v1, v2]: &[egui::epaint::Vertex; 3],
        tex: Option<&(egui::ColorImage, egui::TextureOptions)>,
        ppp: f32,
    ) {
        let p = |v: egui::epaint::Vertex| egui::pos2(v.pos.x * ppp, v.pos.y * ppp);
        let (p0, p1, p2) = (p(*v0), p(*v1), p(*v2));

        let area = (p1.x - p0.x) * (p2.y - p0.y) - (p2.x - p0.x) * (p1.y - p0.y);
        if area.abs() < f32::EPSILON {
            return;
        }
        let inv_area = 1.0 / area;

        let min_x = p0.x.min(p1.x).min(p2.x).floor().max(0.0) as usize;
        let max_x = p0.x.max(p1.x).max(p2.x).ceil().min(w as f32) as usize;
        let min_y = p0.y.min(p1.y).min(p2.y).floor().max(0.0) as usize;
        let max_y = p0.y.max(p1.y).max(p2.y).ceil().min(h as f32) as usize;

        let edge = |a: egui::Pos2, b: egui::Pos2, q: egui::Pos2| {
            (b.x - a.x) * (q.y - a.y) - (b.y - a.y) * (q.x - a.x)
        };

        for y in min_y..max_y {
            for x in min_x..max_x {
                let q = egui::pos2(x as f32 + 0.5, y as f32 + 0.5);
                let (w0, w1, w2) = (edge(p1, p2, q), edge(p2, p0, q), edge(p0, p1, q));
                // 三个边函数与面积同号 → 在三角形内（兼容两种绕向）。
                let same_sign =
                    (w0 >= 0.0 && w1 >= 0.0 && w2 >= 0.0) || (w0 <= 0.0 && w1 <= 0.0 && w2 <= 0.0);
                if !same_sign {
                    continue;
                }
                let (b0, b1, b2) = (w0 * inv_area, w1 * inv_area, w2 * inv_area);

                let uv = egui::pos2(
                    v0.uv.x * b0 + v1.uv.x * b1 + v2.uv.x * b2,
                    v0.uv.y * b0 + v1.uv.y * b1 + v2.uv.y * b2,
                );
                let texel = sample_texture(tex, uv);

                // 纹理（premultiplied）× 顶点色（premultiplied），分量相乘。
                let vc = |c: egui::Color32| {
                    [
                        c.r() as f32 / 255.0,
                        c.g() as f32 / 255.0,
                        c.b() as f32 / 255.0,
                        c.a() as f32 / 255.0,
                    ]
                };
                let c0 = vc(v0.color);
                let c1 = vc(v1.color);
                let c2 = vc(v2.color);
                let mut src = [0.0f32; 4];
                for i in 0..4 {
                    let vc_interp = c0[i] * b0 + c1[i] * b1 + c2[i] * b2;
                    src[i] = texel[i] * vc_interp;
                }

                // premultiplied src-over
                let dst = &mut buf[y * w + x];
                let inv_a = 1.0 - src[3];
                for i in 0..4 {
                    dst[i] = src[i] + dst[i] * inv_a;
                }
            }
        }
    }

    /// 双线性采样。Repeat wrap（点阵背景平铺）按 uv 取模，ClampToEdge 夹边界。
    /// 没有纹理时返回不透明白（等价 epaint 的 WHITE_UV 约定路径）。
    fn sample_texture(
        tex: Option<&(egui::ColorImage, egui::TextureOptions)>,
        uv: egui::Pos2,
    ) -> [f32; 4] {
        let Some((img, options)) = tex else {
            return [1.0, 1.0, 1.0, 1.0];
        };
        let (tw, th) = (img.size[0] as f32, img.size[1] as f32);
        let wrap = options.wrap_mode == egui::TextureWrapMode::Repeat;
        let (fx, fy) = if wrap {
            let x = uv.x.rem_euclid(1.0) * tw - 0.5;
            let y = uv.y.rem_euclid(1.0) * th - 0.5;
            (x.rem_euclid(tw), y.rem_euclid(th))
        } else {
            (
                (uv.x * tw - 0.5).clamp(0.0, tw - 1.0),
                (uv.y * th - 0.5).clamp(0.0, th - 1.0),
            )
        };

        let x0 = fx.floor() as usize;
        let y0 = fy.floor() as usize;
        let x1 = if wrap {
            (x0 + 1) % img.size[0]
        } else {
            (x0 + 1).min(img.size[0] - 1)
        };
        let y1 = if wrap {
            (y0 + 1) % img.size[1]
        } else {
            (y0 + 1).min(img.size[1] - 1)
        };
        let (tx, ty) = (fx - x0 as f32, fy - y0 as f32);

        let px = |x: usize, y: usize| {
            let c = img.pixels[y * img.size[0] + x];
            [
                c.r() as f32 / 255.0,
                c.g() as f32 / 255.0,
                c.b() as f32 / 255.0,
                c.a() as f32 / 255.0,
            ]
        };
        let c00 = px(x0, y0);
        let c10 = px(x1, y0);
        let c01 = px(x0, y1);
        let c11 = px(x1, y1);

        let mut out = [0.0f32; 4];
        for i in 0..4 {
            let top = c00[i] * (1.0 - tx) + c10[i] * tx;
            let bottom = c01[i] * (1.0 - tx) + c11[i] * tx;
            out[i] = top * (1.0 - ty) + bottom * ty;
        }
        out
    }
}

/// 客观量测输出：ASCII 构图、像素探针、WCAG 对比度。模型/人都能读，
/// 用来支撑视觉评审的结论（而不是拍脑袋说“丑/不丑”）。
mod analysis {
    use crate::theme::colors;
    use egui::Color32;

    const BG: [f32; 3] = [13.0, 17.0, 23.0]; // colors::BG_APP

    fn luminance(c: [f32; 3]) -> f32 {
        0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2]
    }

    /// 相对亮度（WCAG）：先把 8bit 分量线性化再加权。这里输入的是 sRGB 0-255。
    fn rel_lum(c: [f32; 3]) -> f32 {
        let f = |v: f32| {
            let v = v / 255.0;
            if v <= 0.03928 {
                v / 12.92
            } else {
                ((v + 0.055) / 1.055).powf(2.4)
            }
        };
        0.2126 * f(c[0]) + 0.7152 * f(c[1]) + 0.0722 * f(c[2])
    }

    fn contrast(a: [f32; 3], b: [f32; 3]) -> f32 {
        let (la, lb) = (rel_lum(a), rel_lum(b));
        let (hi, lo) = if la > lb { (la, lb) } else { (lb, la) };
        (hi + 0.05) / (lo + 0.05)
    }

    fn rgb(c: Color32) -> [f32; 3] {
        [c.r() as f32, c.g() as f32, c.b() as f32]
    }

    /// 常用前景/背景对的 WCAG 对比度（正文 AA 要求 4.5:1，大字/相体 3:1）。
    pub(crate) fn contrast_report() {
        println!("\n── WCAG 对比度 ──");
        let pairs = [
            (
                "TEXT_PRIMARY / BG_APP",
                rgb(colors::TEXT_PRIMARY),
                rgb(colors::BG_APP),
            ),
            (
                "TEXT_SECONDARY / BG_APP",
                rgb(colors::TEXT_SECONDARY),
                rgb(colors::BG_APP),
            ),
            (
                "TEXT_SECONDARY / BG_CARD",
                rgb(colors::TEXT_SECONDARY),
                rgb(colors::BG_CARD),
            ),
            (
                "TEXT_MUTED / BG_CARD",
                rgb(colors::TEXT_MUTED),
                rgb(colors::BG_CARD),
            ),
            (
                "ACCENT_BLUE / BG_CARD",
                rgb(colors::ACCENT_BLUE),
                rgb(colors::BG_CARD),
            ),
            ("RED / BG_CARD", rgb(colors::RED), rgb(colors::BG_CARD)),
            ("RED / RED_BG", rgb(colors::RED), rgb(colors::RED_BG)),
            (
                "WHITE / RED_ACTION(徽标/危险按钮)",
                [255.0, 255.0, 255.0],
                rgb(colors::RED_ACTION),
            ),
            (
                "WHITE / ACCENT_ACTION(主按钮)",
                [255.0, 255.0, 255.0],
                rgb(colors::ACCENT_ACTION),
            ),
            (
                "WHITE / ACCENT_ACTION_HOVER",
                [255.0, 255.0, 255.0],
                rgb(colors::ACCENT_ACTION_HOVER),
            ),
            (
                "ACCENT_BLUE / ACCENT_BLUE_BG(页头图标砖)",
                rgb(colors::ACCENT_BLUE),
                rgb(colors::ACCENT_BLUE_BG),
            ),
            (
                "GREEN / BG_CARD(英雄卡安全标题)",
                rgb(colors::GREEN),
                rgb(colors::BG_CARD),
            ),
            (
                "TEXT_SECONDARY / BG_SIDEBAR(非激活导航项)",
                rgb(colors::TEXT_SECONDARY),
                rgb(colors::BG_SIDEBAR),
            ),
        ];
        for (name, a, b) in pairs {
            println!("  {name:<40} {:>5.2}:1", contrast(a, b));
        }
    }

    fn px(img: &image::RgbaImage, x: u32, y: u32) -> [f32; 3] {
        let p = img.get_pixel(x, y);
        [p[0] as f32, p[1] as f32, p[2] as f32]
    }

    fn avg(img: &image::RgbaImage, x0: u32, y0: u32, x1: u32, y1: u32) -> [f32; 3] {
        let mut acc = [0.0f32; 3];
        let n = ((x1 - x0) * (y1 - y0)).max(1) as f32;
        for y in y0..y1 {
            for x in x0..x1 {
                let p = px(img, x, y);
                acc[0] += p[0];
                acc[1] += p[1];
                acc[2] += p[2];
            }
        }
        [acc[0] / n, acc[1] / n, acc[2] / n]
    }

    /// ASCII 构图图：每格取平均色，彩色元素用字母（B=蓝 R=红 G=绿 Y=黄），
    /// 灰阶用亮度梯度。能看出布局、对齐、留白与色彩分布。
    pub(crate) fn ascii_preview(name: &str, img: &image::RgbaImage) {
        const COLS: u32 = 100;
        const ROWS: u32 = 33;
        let (w, h) = (img.width(), img.height());
        println!(
            "\n── {name} 构图（{w}x{h}，每格 {}/{:.1}px）──",
            w / COLS,
            h as f32 / ROWS as f32
        );
        for row in 0..ROWS {
            let mut line = String::with_capacity(COLS as usize);
            for col in 0..COLS {
                let x0 = col * w / COLS;
                let x1 = ((col + 1) * w / COLS).max(x0 + 1);
                let y0 = row * h / ROWS;
                let y1 = ((row + 1) * h / ROWS).max(y0 + 1);
                let c = avg(img, x0, y0, x1.min(w), y1.min(h));
                let (r, g, b) = (c[0], c[1], c[2]);
                let ch = if b - r.max(g) > 25.0 && b > 60.0 {
                    'B'
                } else if r - g.max(b) > 35.0 && r > 90.0 {
                    'R'
                } else if g - r.max(b) > 25.0 && g > 90.0 {
                    'G'
                } else if r > 150.0 && g > 110.0 && b < 90.0 {
                    'Y'
                } else {
                    let l = luminance(c);
                    const RAMP: [char; 10] = [' ', '.', ':', '-', '=', '+', '*', '#', '%', '@'];
                    let idx = ((l - 14.0) / 26.0).clamp(0.0, 9.0) as usize;
                    RAMP[idx]
                };
                line.push(ch);
            }
            println!("|{}|", line);
        }
    }

    /// 窗口框架探针：面板分界、侧栏导航行、资源条居中、点阵可见度、
    /// 仪表盘英雄卡/信息卡行的几何。
    pub(crate) fn probe_chrome(img: &image::RgbaImage) {
        println!("\n── 窗口框架探针 ──");
        // 1. 标题栏/内容分界（y=43..46）：有没有分割线（不同于两側底色的行）
        for y in 42..47u32 {
            let c = avg(img, 300, y, 800, y + 1);
            println!(
                "  标题栏边界 y={y}: rgb({:.0},{:.0},{:.0})",
                c[0], c[1], c[2]
            );
        }
        // 2. 侧栏（宽 200 带文字）：首项（Dashboard，active）行底色应接近
        //    ACCENT_BLUE_BG；文字像素亮度应明显高于底色。取样区域避开图标与
        //    文字（图标从 x≈28 起、文字从 x≈60 起），只看底色。
        let active = avg(img, 20, 66, 26, 100);
        let sidebar_bg = avg(img, 8, 320, 190, 420);
        let mut label_max = 0.0f32;
        for y in 62u32..94 {
            for x in 60u32..180 {
                label_max = label_max.max(luminance(px(img, x, y)));
            }
        }
        println!(
            "  侧栏激活行 avg rgb({:.0},{:.0},{:.0})（ACCENT_BLUE_BG=17,50,71）文字峰值亮度 {label_max:.0}  侧栏底 avg rgb({:.0},{:.0},{:.0})（BG_SIDEBAR=5,7,10）",
            active[0], active[1], active[2], sidebar_bg[0], sidebar_bg[1], sidebar_bg[2]
        );
        // 侧栏激活行实测：ACCENT_BLUE_BG 填充的包围盒应约在 y 64..103、
        //    x 16..200（面板内容区 x 16..200 = 侧栏 200 宽减去 8px 内边距）。
        let bg_blue = |c: [f32; 3]| {
            (c[0] - 17.0).abs() < 12.0 && (c[1] - 50.0).abs() < 14.0 && (c[2] - 71.0).abs() < 16.0
        };
        let (mut s_min_y, mut s_max_y) = (u32::MAX, 0u32);
        let (mut s_min_x, mut s_max_x) = (u32::MAX, 0u32);
        for y in 44u32..300 {
            for x in 0u32..208 {
                if bg_blue(px(img, x, y)) {
                    s_min_y = s_min_y.min(y);
                    s_max_y = s_max_y.max(y);
                    s_min_x = s_min_x.min(x);
                    s_max_x = s_max_x.max(x);
                }
            }
        }
        if s_min_y <= s_max_y {
            println!(
                "  侧栏激活行填充 x∈[{s_min_x},{s_max_x}] y∈[{s_min_y},{s_max_y}]（预期 x≈16..200、y≈64..103）"
            );
        }
        for (x, y) in [
            (10, 78),
            (100, 78),
            (180, 78),
            (10, 120),
            (100, 126),
            (10, 78),
        ] {
            let c = px(img, x, y);
            println!(
                "  侧栏采样 ({x},{y}) rgb({:.0},{:.0},{:.0})",
                c[0], c[1], c[2]
            );
        }
        // 3. 资源条：扫描 y=556..598 找非背景像素的水平范围；只统计“实心”内容
        //    （与背景差 >60），把边缘抗锯齿点单独列出来诊断。
        let mut solid = (u32::MAX, 0u32);
        let mut outliers: Vec<(u32, u32, [f32; 3])> = Vec::new();
        for y in 556u32..598 {
            for x in 1u32..img.width() - 1 {
                let c = px(img, x, y);
                let d = (c[0] - BG[0]).abs() + (c[1] - BG[1]).abs() + (c[2] - BG[2]).abs();
                if d > 60.0 {
                    solid.0 = solid.0.min(x);
                    solid.1 = solid.1.max(x);
                } else if d > 30.0 && (x < 100 || x > img.width() - 100) {
                    let same_as_row = |dx: i32| {
                        let xx = (x as i32 + dx).clamp(0, img.width() as i32 - 1) as u32;
                        let c2 = px(img, xx, y);
                        (c2[0] - c[0]).abs() + (c2[1] - c[1]).abs() + (c2[2] - c[2]).abs() < 20.0
                    };
                    // 只记与横向邻居不同的孤立点（边缘/抗锯齿），避免把文字边缘全记上
                    if !same_as_row(-2) && !same_as_row(2) && outliers.len() < 8 {
                        outliers.push((x, y, c));
                    }
                }
            }
        }
        if solid.0 <= solid.1 {
            let mid = (solid.0 + solid.1) as f32 / 2.0;
            println!(
                "  资源条实心内容 x∈[{},{}] 中点 {mid:.0}（窗口中点 450，偏移 {:+.0}px，量宽居中应 ±10px 内）",
                solid.0,
                solid.1,
                mid - 450.0
            );
        }
        for (x, y, c) in outliers {
            println!(
                "  资源条边缘点 ({x},{y}) rgb({:.0},{:.0},{:.0})",
                c[0], c[1], c[2]
            );
        }
        // 4. 点阵可见度：页头与英雄卡之间的净空带（y≈120..132），最大亮度与
        //    背景差。（信息卡行下方只剩 y543..549 几像素，不够采样。）
        let mut max_l = 0.0f32;
        for y in 120u32..132 {
            for x in 220u32..880 {
                max_l = max_l.max(luminance(px(img, x, y)));
            }
        }
        println!(
            "  点阵区域最大亮度 {max_l:.1}（背景 ≈16，差 {:.1}）",
            max_l - luminance(BG)
        );
        // 5. 卡片列居中：BG_CARD 色块（英雄卡+三张信息卡）的联合包围盒中点
        //    vs 内容区中心（内容区 x 208..892，中心 = 550）。
        //    注：egui 0.36 的 Panel 自带 8px margin，中央面板内容不在 x 200..900。
        let card_mask = |c: [f32; 3]| {
            let d = (c[0] - 24.0).abs() + (c[1] - 29.0).abs() + (c[2] - 38.0).abs();
            d < 24.0
        };
        let (mut min_x, mut max_x, mut min_y, mut max_y) = (u32::MAX, 0u32, u32::MAX, 0u32);
        for y in 44u32..550 {
            for x in 205u32..img.width() {
                if card_mask(px(img, x, y)) {
                    min_x = min_x.min(x);
                    max_x = max_x.max(x);
                    min_y = min_y.min(y);
                    max_y = max_y.max(y);
                }
            }
        }
        if min_x <= max_x {
            let cx = (min_x + max_x) as f32 / 2.0;
            println!(
                "  卡片区联合包围盒 x∈[{min_x},{max_x}] y∈[{min_y},{max_y}] 中心 x={cx:.0}（内容区中心 550，偏移 {:+.0}px，≈+9 来自右侧描边/抗锯齿）",
                cx - 550.0
            );
        }
        // 6. 英雄卡状态环（绿色，限卡内左半 x<450，排除右侧绿色标题文字）：
        //    环心应在卡内左列（环盒 248..448，环心 ≈348）。
        let (mut r_min_x, mut r_max_x, mut r_min_y, mut r_max_y) = (u32::MAX, 0u32, u32::MAX, 0u32);
        for y in 44u32..550 {
            for x in 205u32..450 {
                let c = px(img, x, y);
                if c[1] - c[0].max(c[2]) > 40.0 && c[1] > 90.0 {
                    r_min_x = r_min_x.min(x);
                    r_max_x = r_max_x.max(x);
                    r_min_y = r_min_y.min(y);
                    r_max_y = r_max_y.max(y);
                }
            }
        }
        if r_min_x <= r_max_x {
            let cx = (r_min_x + r_max_x) as f32 / 2.0;
            let cy = (r_min_y + r_max_y) as f32 / 2.0;
            println!(
                "  英雄卡状态环 x∈[{r_min_x},{r_max_x}] y∈[{r_min_y},{r_max_y}] 中心 ({cx:.0},{cy:.0})（环盒中心 ≈356，英雄卡 ≈y150..387）"
            );
        }
        // 7. 信息卡行：卡内底部留白带水平扫描 BG_CARD 区段，应为 3 段。
        //    实测卡行 y417..520（104 高），PAD 20 → 底部留白带 ≈503..521，取 y=510。
        let mut runs = Vec::new();
        let mut run_start: Option<u32> = None;
        for x in 205u32..img.width() {
            if card_mask(px(img, x, 510)) {
                if run_start.is_none() {
                    run_start = Some(x);
                }
            } else if let Some(s) = run_start.take() {
                runs.push((s, x - 1));
            }
        }
        if let Some(s) = run_start.take() {
            runs.push((s, img.width() - 1));
        }
        let widths: Vec<String> = runs.iter().map(|(a, b)| format!("{a}..{b}")).collect();
        println!(
            "  信息卡行 y=510 共 {} 段：{}（应为 3 段，每段 ≈200px）",
            runs.len(),
            widths.join(" ")
        );
        // 8. 三卡几何：每张卡的 BG_CARD 填充 y 范围。扫描窗限制在中央面板内
        //    （y ≤ 542）——底部资源条面板里仪表 track 也是 BG_CARD 色，不限制
        //    会混进来误报；扫描窗限制在 y 440..530（避开英雄卡底 ≈387 与卡行底
        //    之下的点阵）。三张卡应顶对齐且等高（116px）。
        for (name, x0, x1) in [
            ("卡1", 230u32, 437u32),
            ("卡2", 456, 663),
            ("卡3", 682, 899),
        ] {
            let (mut lo, mut hi) = (u32::MAX, 0u32);
            for y in 410u32..535 {
                for x in x0..x1 {
                    if card_mask(px(img, x, y)) {
                        lo = lo.min(y);
                        hi = hi.max(y);
                    }
                }
            }
            println!("  信息{name} BG_CARD y∈[{lo},{hi}]（三卡应同顶同底 ≈417..520，高 104）");
        }
        // 9. 内容列右缘回归：英雄卡与三张信息卡的右缘必须落在同一条线上
        //    （x≈867 = 内容列右缘）。曾经三张信息卡排到 887——宽度预算漏了
        //    隐含的 item_spacing.x（design skill 坑 16），卡片被窗口裁掉。
        let mut right = 0u32;
        for y in 44u32..545 {
            for x in 205u32..img.width() {
                if card_mask(px(img, x, y)) {
                    right = right.max(x);
                }
            }
        }
        println!(
            "  卡片列最右像素 x={right}（应 ≈867，超出说明宽度预算漏算 item_spacing）"
        );
    }

    /// 弹窗居中探针：找红色像素的包围盒，对比屏幕中心。
    pub(crate) fn probe_modal_center(img: &image::RgbaImage) {
        let (mut min_x, mut min_y, mut max_x, mut max_y) = (u32::MAX, u32::MAX, 0u32, 0u32);
        for y in 0..img.height() {
            for x in 0..img.width() {
                let c = px(img, x, y);
                if c[0] - c[1].max(c[2]) > 35.0 && c[0] > 90.0 {
                    min_x = min_x.min(x);
                    max_x = max_x.max(x);
                    min_y = min_y.min(y);
                    max_y = max_y.max(y);
                }
            }
        }
        let (cx, cy) = ((min_x + max_x) as f32 / 2.0, (min_y + max_y) as f32 / 2.0);
        println!(
            "\n── 弹窗探针：红色包围盒 x∈[{min_x},{max_x}] y∈[{min_y},{max_y}]，中心 ({cx:.0},{cy:.0}) vs 屏幕中心 (450,300)，偏移 ({:+.0},{:+.0}) ──",
            cx - 450.0,
            cy - 300.0
        );
    }
}
