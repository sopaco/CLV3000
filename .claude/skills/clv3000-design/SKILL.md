---
name: clv3000-design
description: Use when implementing or reviewing CLV3000 UI code (src/app.rs, widgets.rs, theme.rs, icons.rs). Encodes the design tokens extracted from the original design mockups and the egui-specific layout/rendering pitfalls this codebase has already hit once — read before writing new pages/components, not just after something looks wrong.
version: 0.1.0
---

# CLV3000 视觉设计规范

来源：用户提供的设计稿截图（仪表盘/闪电扫描/闪电扫描结果三张）+ 本项目实现过程中踩过的坑。
这份文档记录**已经验证过的结论**，不是猜测——凡是标了"⚠️ 已踩坑"的条目，都是真实在这个项目里出过的 bug。

## 设计 Token

### 配色（`src/theme.rs::colors`）

> 2026-10 视觉评审后调整过一轮：核心原则是**背景三层必须有肉眼可辨的台阶**
> （旧值三层亮度差 <3/255，整个窗口读作一整块黑）。区域交界仍只靠深浅区分、
> 不加分割线（坑 4 依旧成立）。

| 用途 | 颜色 | 说明 |
|---|---|---|
| 页面背景 | `BG_APP` `#0D1117` | 全局最底层背景（画布层） |
| 侧边栏背景 | `BG_SIDEBAR` `#05070A` | chrome 层，最深 |
| 标题栏背景 | `BG_TITLEBAR` `#07090D` | chrome 层，与侧栏同族 |
| 卡片/面板背景 | `BG_CARD` `#181D26` | 按钮、卡片、徽标的底色，与 BG_APP 台阶 ≥12/255 亮度差 |
| 描边/分割线 | `BORDER` `#2A313D` | 卡片描边，**不要**用来做大面积分割线（太显眼，见下面"面板分割线"坑） |
| 强调色（蓝） | `ACCENT_BLUE` `#3AA0E0` | 图标、进度、选中态 |
| 强调色背景 | `ACCENT_BLUE_BG` `#113247` | hover/active 底色 |
| 警告色（黄） | `AMBER` `#E6B43C` | 资源条偏高（`accent_for`），从硬编码收进 token |
| 安全/成功 | `GREEN` `#22C55E` | |
| 状态红（图形用） | `RED` `#EF4444` | 状态环/图标/描边等大图形，对比度按 3:1 图形标准 |
| 危险动作红（白字底） | `RED_ACTION` `#D92D20` / hover `#DE463B` | 白字实底按钮/徽标专用，对比度 ≥4.5:1（AA）；**不要**拿 RED 直接当白字底（只有 3.8:1） |
| 危险背景 | `RED_BG` `#2A1416` / `RED_BORDER` `#502024` | 威胁卡片 |
| 文字主色 | `TEXT_PRIMARY` `#F5F7FA` | |
| 文字次要 | `TEXT_SECONDARY` `#8A93A3` | |
| 文字弱化 | `TEXT_MUTED` `#7F8894` | 对 BG_CARD ≥4.5:1（旧值 #5C6472 只有 3.0:1，小字不达标） |

### 间距 / 圆角 / 字号

- **圆角刻度**（`src/theme.rs::radius`，2026-10 评审把十一种散落值收敛成三档，
  当天整体收紧一档，随后**再减半**——大色块上 12~16px 的圆角显得"圆钝"，
  8px 已经是常规偏方，收到 3~6px 才真正是"干练"：圆角只留一点暗示，不再是
  造型元素，靠层次和留白而不是圆润度表达结构）：
  - `SM` 3px：悬停底、图标底、toast、徽标、复选框、chip、计划编辑器小面板
  - `MD` 4px：卡片、按钮、侧栏格、tab、威胁卡
  - `LG` 6px：英雄卡、弹窗（窗口圆角同步 6）
  - egui 原生控件（ComboBox / selectable_label）的圆角单独设 2px：比自绘卡片更方，
    但不再是 egui 默认的 2~3px 尖角。
  - **刻度是像素值的单一事实来源**：`radius::SM_PX/MD_PX/LG_PX` 是 f32
    （`painter.rect_filled` 要 f32），`radius::SM/MD/LG` 由它派生；新增控件
    一律用 `radius::X`，**不要写裸数字**——之前 `toggle_checkbox`/`toggle_chip`
    各硬编码了一个 4.0，跟着刻度漂移了。
  - **不进刻度的两类**：数据药丸（`stat_pill`，999 = 无限圆，是"这是读数"的
    可视语言，不是控件造型）与细进度条（≈高/2）。
  - **所有可点的东西必须是同一档圆角**：按钮（primary/action/ghost/pill/
    threats_entry）、chip、徽标统一走 `MD`/`SM`；同一个弹窗里混用药丸形按钮
    和矩形按钮是"个人项目感"的主要来源（2026-10 用户反馈"太圆"后一并收掉）。
- 卡片描边：`1px BORDER`。
- 标准按钮内边距：水平 `14~16px`，垂直 `7~10px`。
- 图标尺寸经验值（2026-08 那轮"图标偏小"反馈后统一 +15% 得出）：
  - 侧边栏图标：40×40 容器，图形占 ~21px（`shrink(9.5)`）
  - 标题栏 App 图标：23px；标题栏最小化/关闭按钮图形：32×32 容器，图形 ~14px（`shrink(9.0)`）
  - 按钮内图标（`action_button`）：21px
  - 威胁卡片图标：39px
  - 大状态圆环里的图形：圆环直径 × 0.50
- **字号层级**（`theme.rs::apply`）：页面标题 20px（`bold_label`，每页一个，统一模式）、正文 14px、说明文字 `.small()` 12.5px。注意量宽必须用与渲染一致的字号（`stat_pill_width` 用 12.5 量 label）。**不要再把 Small 改成跟 Body 一样大**——2026-10 之前 Small=Body=14，`.small()` 无效，全页文字一样大。
- 侧边栏导航：`Panel::left` 的 frame 给 `inner_margin(symmetric(8, 0))`（导航行不贴窗口边缘）；宽形态每行 = 内容宽、高 40，用 `allocate_exact_size` + 自绘（背景圆角 MD、图标 20、文字 14、active 行 ACCENT_BLUE_BG + 左缘 2.5px 指示条）。**指示条要收进 12px 圆角的直边里**（x = left+0.5…left+3，y = top+11…bottom−11），画满整行高度会从圆角外侧支出来、看着像贴歪的蓝标签。纯图标窄栏必须带 `on_hover_text` tooltip + `widget_info`；文字用 `painter.text` 直画，不要 `ui.label`（accesskit 双节点，kittest 多匹配 panic）。行宽在循环外算一次传进 `labeled_nav_item`，不要每行现算 `available_width()`（坑 10 的横向版）。
- 复选框：用 `widgets::toggle_checkbox`（自绘 18px 圆角方框：未选=BORDER_STRONG 描边 / 选中=ACCENT_ACTION 实底+白色对勾 / 禁用=BORDER 灰态），不要用 egui 内置 `ui.checkbox`——本主题下它的方框与卡片同色，是隐形的（坑 17）。
- 小圆角标签（可选中）：`widgets::toggle_chip(ui, 可见文字, 无障碍名, 选中)`——用于"周内天"多选、"扫描类型"单选。可见文字用单字母时**无障碍名必须用全称**（两个 T、两个 S 会撞名 → kittest 多匹配 panic）。
- **信息卡**（Dashboard 三张）：`widgets::stat_card` 整张**自绘**（不套 egui 布局流）——内边距固定 20px、上下左右对称（2026-10 用户反馈：16 太贴边）。自绘的原因见坑 19/20：嵌套布局算不准行高与留白。
- Toast 浮层：锚在窗口右下角但要让开底部资源条（`widgets::RESOURCE_BAR_HEIGHT`），否则整个压在 CPU/Memory 仪表上。
- 圆点背景网格：间距 28px，半径 1px，颜色 `DOT_GRID`（premultiplied `(16,16,16,16)`，≈6% 不透明度的白）——含蓄的纹理感而不是视觉主体，但 8/255 时几乎不可见（实测亮度差仅 ~8），16 是校准后的下限。（premultiplied 语义见坑 3。）

### 组件形态

- **状态圆环**（仪表盘/扫描结果）：大圆 + 描边 + 内部填充 `BG_CARD` + 外圈柔和光晕（`widgets::paint_glow`）+ 居中图形（盾牌打勾 / 警告三角）。光晕是本项目里唯一近似"发光"效果的手段，本质是几层同心圆叠加、透明度从外到内递增，**必须**用 `from_rgba_unmultiplied`，且要在外面多留 `radius * 1.5` 左右的画布空间，不然会被裁掉。
- **进度圆环**（扫描中）：背景是一圈很淡的描边（`from_rgba_unmultiplied(255,255,255,18)`，不是纯色实心圈！），前景是从正上方顺时针延伸的实色弧。百分比未知时画一段跟随时间旋转的弧，制造"正在进行"的动感。
- **按钮/徽标**：`BG_CARD` 填充 + `BORDER` 描边 + 图标（可选）+ 文字，圆角走 `MD`（徽标/小 chip 走 `SM`）。"填充色"按钮（比如威胁卡片的"隔离"）用强调色/危险色整块填充、白字，"描边"按钮（比如"忽略"）用透明底+描边，两者高度必须一致。
- **威胁卡片**：`RED_BG` 底 + `RED_BORDER` 描边，左侧一个红色方块警告图标，中间病毒名+"高危"红色小徽标+文件路径，右侧隔离/忽略两个按钮。卡在 Frame 闭包开头 `ui.set_min_width(ui.available_width())` 吃满弹窗内容宽（所有卡等宽、按钮列对齐）；病毒名/路径用 `truncate_middle_to_px` 按像素截断，文本列预算 = 卡内宽 − 图标 − 按钮区（`pill_button_width` 量出），别按字符数猜。
- **卡片式设置项**（比如"包含可移动磁盘"开关）：不要让 checkbox 裸露在页面上，包一层 `BG_CARD` + `BORDER` 的卡片容器，宽度按内容**量出来**，不要拍脑袋写死（见坑 1）。隔离区/忽略列表这类行卡：Frame 闭包开头 `set_min_width(ui.available_width())` 吃满列宽，路径/病毒名按列宽像素截断（见坑 14）。

### 文案规则（2026-10 用户反馈后定的）

界面文案不是"把状态描述出来"就行，要按**新用户第一次看到这一页**的处境写：

- **空状态是引导，不是陈述。** 不要出现 `Never` / `Not run yet` / `No full scan
  performed yet` / `Run a scan to see results here` 这类反复强调"你什么都没做"的
  句子。同一件事（没扫过）在四个地方出现四次，用户读到的是一片空白加自责。
  正确写法是给下一步 + 预期：`Fast first check` / `About a minute`、
  `Start with a full scan` / `Build your protection baseline`。
- **同一信息只在一个地方出现。** 上次扫描的时间放信息卡，英雄卡就只说结论
  （"No threats in your last scan"）；页头副标题说"能解决什么问题"，页面里的
  清单说"具体扫什么"，两边不重复列表。
- **不要写实现里没有的东西。** 闪电扫描枚举的是"运行中进程及其加载的模块"
  （见 `scan/quick_scan.rs`），并不扫启动项、不扫内存——写 "startup items, and
  memory" 是虚假承诺。覆盖清单的每一行都要能在代码里找到对应。
- **数字加千分位**：`config::format_count`（文件数）、`clamav_info::format_build_number`
  （病毒库编号 28167 → 28,167）。信息卡内文宽只有 ~156px，说明行控制在 ~13 字
  以内，否则会被中间省略成半截话。
- **日期格式统一 `Mon D, YYYY`**（`Oct 28, 2026`），不是 `Oct, 28 2026`。

## 页面骨架（2026-10 五页统一改版定下的模式）

- **页头**：`widgets::page_header(icon, title, subtitle, right_side)`——38px 图标砖（ACCENT_BLUE_BG 底 + ACCENT_BLUE 描边图形）+ 20px 粗体标题 + 副标题；主操作按钮挂 `right_side`（比如扫描页的 Cancel、取消按钮用 ghost 形态）。五页共用，别自造页头。
- **主操作按钮**：`widgets::primary_button` —— ACCENT_ACTION 实底 + 白字（实测 5.40:1 AA），hover 换 ACCENT_ACTION_HOVER（4.69:1）。文字**必须 galley 直画**，不能用 `ui.label`——否则 accesskit 双节点，kittest `get_by_label` 多匹配 panic。页面上只放一个实底主按钮（视觉优先级），次要操作用 `ghost_button`/`action_button`。
- **仪表盘英雄卡**：`BG_CARD` + LG 圆角 + 1px BORDER。**三栏结构**：左 120px 状态环（Secure 绿/At Risk 红/Running 蓝）+ 中间标题/副标题 + 右侧动作列（Quick Scan 实底 + Full Scan 描边，竖排）。三栏都显式定宽，整行宽度恰好等于卡内宽——右缘跟下面三张信息卡严丝合缝对齐（2026-10 改版前是"环 + 文字 + 按钮"靠左一坨，右半张卡空着 200 多像素）。**纵向对齐要自己算**（坑 19）：环占固定高 200，文字块与动作列各自量出内容高度后补 `(200 − 内容高)/2` 的顶部留白，三者共享同一条中线。文字列宽度 = 卡内宽 − 环(200) − 间距 − 动作列 − 间距。副标题太长会折行，别让它太长。窄窗退化为竖排（环在上、文字与按钮在下）。首次使用（没有任何扫描记录）时：标题 `Start with a full scan`、副标题 `Build your protection baseline`、主行动是 Full Scan（先建立保护基线）——**不要**声称 "Secure"（还没验证过）。
- **信息卡行**：`widgets::stat_card`（整卡可点跳页，hover 只提描边不换底色，accessible 名 "Open {label}" 避免与侧栏撞名）。定高 104，三张等宽并排：行用 `ui.horizontal_top` + 逐卡 `allocate_ui_with_layout((card_w, 104), top_down(Min))`（坑 10/11 的正确示范）。**宽度预算要算上隐含的 `item_spacing.x`（坑 16）**。窄窗（< 560）纵向堆叠。**卡内四周内边距**：闭包内 `horizontal + add_space(PAD) + vertical(set_width(卡宽-2PAD))` 的 inset_row 式缩进——旧版只有顶部 PAD，图标/数值贴左边框 0px（2026-10 用户反馈“缺内边距”，修复后左缘 ≈20px）。
- **扫描页**（Quick/Full 共用 `scan_page`）：页头 + 居中主体。**空闲态版式（2026-10 重排）**：64px 描边图标（不是大圆环——120px 的环 + 一行字 + 一个按钮占满整屏，像 landing page 不像功能页）→ **覆盖清单卡**（`widgets::coverage_card`：三行"图标 + 说明"，14px 正文色，是页面的视觉重心）→ 一行"上次扫描结果 / 首次扫描耗时预期" → 主操作按钮。标题与副标题在页头，空闲态不再重复一遍。扫描中仍是进度环 + 数据胶囊 + 威胁入口。窄内容区由页面 ScrollArea 兜底。
- **覆盖清单**：`widgets::coverage_card(ui, rows)`，行内容 = 20px 品牌蓝描边图标 + 14px `TEXT_PRIMARY` 文字（原来这套信息被压成 12.5px 灰色小字，是全页信息量最大的部分却最不显眼）。文案必须与引擎实际行为一致（见"文案规则"）。
- **限宽居中卡**（病毒库页/扫描页覆盖清单）：`horizontal_top` 里手动 `add_space(left)` + `allocate_ui_with_layout((card_w, ⋯), ...)` + 卡内 Frame 闭包开头 `set_min_width(available)`；路径/版本行用 `truncate_middle_to_px` 像素截断（坑 14）。病毒库页英雄卡内不重复页面名——主标题让给 `Version 28,167`（千分位）+ 状态行 `Updated Oct 28, 2026 · Built-in database ready`；页面底部加**自动更新开关**（`widgets::toggle_setting_row`）+ 上次检查时间（`config::auto_update_definitions` / `config::last_update_check`，启动后自动检查一次，见 `VirusDbState::maybe_auto_update`）。
- **页边距**：`widgets::PAGE_INSET = 24`，内容区左右统一留边（`inset_row` 辅助容器）。

## egui 布局/渲染踩过的坑（写代码前先看这几条，能省很多返工）

### 坑 1：`Frame` / `ui.horizontal` 不参与外层 `Align::Center`

`egui::Frame::show(ui, ...)` 和 `ui.horizontal(...)` 在被外层 `vertical_centered`/`Align::Center` 布局调用时，会把自己的"期望尺寸"报成父容器**当前的全部可用宽度**（因为内容画完之前不知道真实大小，只能先占住最大空间）。结果就是：外层想居中一个"看起来应该是小按钮"的东西，实际拿到的是一个和容器一样宽的东西，居中直接失效——表现为按钮从最左边一路铺到很宽的位置，而不是一颗居中的小圆角方块。

**正确做法**：自己先用 `widgets::measure_text_width(ui, text, size)`（内部就是 `ui.ctx().fonts_mut(|f| f.layout_no_wrap(...))`）量出文字的真实尺寸，加上图标/内边距算出准确的 `desired_size`，再用 `ui.allocate_ui_with_layout(desired_size, layout, |ui| {...})` 分配——**不要**直接用 `Frame::show` 或裸的 `ui.horizontal` 去包一个需要被外层居中的组件。背景色用"占位 Shape + 事后回填"的技巧：

```rust
let bg_idx = ui.painter().add(egui::Shape::Noop);
let response = ui.allocate_ui_with_layout(desired_size, layout, |ui| { /* 内容 */ }).response;
let shape = egui::epaint::RectShape::new(response.rect, corner_radius, fill, stroke, egui::epaint::StrokeKind::Inside);
ui.painter().set(bg_idx, egui::Shape::Rect(shape));
```

参考实现：`app.rs` 的 `action_button`、`centered_card`；`widgets.rs` 的 `pill_button`、`stat_pill`/`centered_stat_pills`（一行多个数据药丸一起居中，宽度是每个量出来后加总的，不是猜的）。

**同一个坑的变体**：即使用了上面这套"量尺寸"手法，如果 `desired_size` 是**拍脑袋写死的固定值**（比如"这个设置卡片应该 420px 宽"），一旦实际内容（比如 checkbox 的文字标签）比这个固定值还宽，内容会溢出这个"看起来居中"的容器边界，视觉上还是会显得歪/不对称。**固定宽度只能给不会变的内容用，任何带文字、且文字会变化（扫描过程中的数字、用户输入……）的内容都要量出来，不要猜。**静态、不会变的文字（比如仪表盘那两个固定按钮"闪电扫描"/"全盘扫描"）拍一个校准过的偏移量问题不大，但能量出来就尽量量。

### 坑 2：`RichText::strong()` 只换颜色，不换字重

egui 的字体只加载了一个字重，没有 bold 变体文件；`.strong()` 的实现就是把颜色换成 `visuals.strong_text_color()`，**不会**让文字变粗。如果还手动加了 `.color(...)`，`.strong()` 的颜色效果也会被覆盖，等于完全没用。

**正确做法**：需要真正的粗体视觉效果时用 `widgets::bold_label(ui, text, size, color)`——把同一段文字横向错开 0.6px 描两遍，模拟笔画加粗（"伪粗体"）。不追求像素级精确，但比什么都不做强很多。

### 坑 3：`Color32::from_rgba_premultiplied` 的参数不是"你想要的颜色 + 透明度"

这个构造函数要求 RGB 分量**已经按 alpha 比例乘过**。想要"4% 不透明度的白"，正确写法是 `from_rgba_premultiplied(10, 10, 10, 10)`，**不是** `from_rgba_premultiplied(255, 255, 255, 10)`——后者等于告诉渲染器"这几乎是一块不透明的白"，实际效果比预期亮得多、显眼得多。本项目里这个错误在圆点背景和进度圆环的背景轨道上都出现过一次。

**正确做法**：除非明确知道要用 premultiplied 语义，一律用 `Color32::from_rgba_unmultiplied(r, g, b, a)`——直接给"看起来的"颜色和透明度，不用自己心算乘法。写 `const` 需要 premultiplied（`from_rgba_unmultiplied` 不是 `const fn`）时，手动把 RGB 设成和 alpha 一样的值（灰度色场景下 `r=g=b=alpha` 刚好是"白色×alpha"的 premultiplied 形式）。

### 坑 4：`egui::Panel`（Side/TopBottom 统一后的类型）默认画分割线

`Panel::top/bottom/left/right(id)` 默认 `show_separator_line(true)`，会在面板边缘画一条用 `visuals.widgets.noninteractive.bg_stroke` 着色的线——即使这个 stroke 颜色本身不是纯白，在深色主题下也会显得比设计稿里柔和的区域分界突兀。

**正确做法**：如果设计上想让区域之间只靠背景色深浅区分（本项目的设计稿是这样），每个 `Panel` 显式 `.show_separator_line(false)`。

### 坑 5：在 macOS 上 `with_decorations(false)` 不够彻底

自绘标题栏时，光设 `with_decorations(false)` 在 macOS 上可能还留一条原生标题区/红绿灯按钮的残余，会露出系统默认底色（一条白边）。要配合 `.with_title_shown(false)`、`.with_titlebar_shown(false)`、`.with_titlebar_buttons_shown(false)`、`.with_fullsize_content_view(true)` 一起用。

### 坑 6：想要精确对齐的按钮，别用 `ui.horizontal` 的光标累加去猜位置

自绘标题栏的最小化/关闭按钮最早是用"剩余宽度减一个估算值"算拖拽区宽度，估算有偏差，按钮位置就跟着飘。**正确做法**：先拿 `ui.max_rect()`，直接算出每个按钮的精确 `Rect`（比如"贴着右边缘留 8px"），用 `ui.interact(rect, id, Sense::click())` 在这个固定矩形上做点击检测，不走光标布局。

### 坑 7：`ui.interact()` 手搓的按钮不会自动显示手型光标

`egui::Button` 之类的标准控件会看 `Visuals::interact_cursor` 这个全局样式自动换手型光标（本项目在 `theme::apply` 里设成了 `Some(CursorIcon::PointingHand)`），但我们自己拿 `ui.interact(rect, id, Sense::click())` 或 `ui.allocate_painter(size, Sense::click())` 手搓的按钮（`action_button`/`pill_button`/侧边栏图标/标题栏按钮）**不会**自动读这个全局设置——每一处都要显式 `.on_hover_cursor(egui::CursorIcon::PointingHand)`。新增自定义可点击元素时记得补这一句。

### 坑 8：`FontTweak.y_offset_factor` 只挪墨迹，不挪布局包围盒——会破坏图标/文字对齐

之前为了让中文和拉丁字体混排时基线好看一点，给 CJK fallback 字体加了 `y_offset_factor`。结果这个偏移只影响**画笔实际落墨的位置**，`egui::Align::Center` 用来计算居中的是**文字的布局包围盒**（`FontTweak` 文档原话："this is only a visual effect and does not affect the text layout"）——两者不是一回事。于是图标按包围盒正确居中了，文字的包围盒也居中了，但文字的墨迹在盒子里悄悄往下偏了几像素，肉眼看就是"图标和文字没对齐"。**结论：不要用 `FontTweak` 的偏移量去做"跟别的控件对齐"这件事，它解决不了这个问题，反而会制造新的错位。** 如果确实有 CJK/拉丁混排的基线问题要处理，去找视觉上更协调的字号/字体，而不是事后拿 offset 硬掰。

### 坑 9：`ui.allocate_painter()` 返回的画笔会把裁剪框收紧到那一小块矩形

手动微调文字/图形的落墨位置时（比如坑 8 那种"往左下角挪几像素"的补偿），如果用
`ui.allocate_painter(size, sense)` 拿画笔，它返回的 `Painter` 裁剪框是
`ui.clip_rect().intersect(刚分配的这个小矩形)`——比 `ui.painter()`（裁剪框是整个
当前 `ui`，通常是整个面板那么大）小得多。这时候如果画的位置比分配的矩形还往外
挪（比如 `rect.min + Vec2::new(-2.5, 0.0)`），挪出去的那部分笔墨会直接被裁掉，
表现就是"文字缺了一块/被挡住一角"。

**正确做法**：需要手动挪位置画东西时，用 `ui.allocate_exact_size(size, sense)`
只占用布局空间（不产生特殊裁剪），画的时候用 `ui.painter()`（外层裁剪框足够宽松）。
`widgets::bold_label_nudged` 一直是这么写的，`action_button` 里最早误用了
`allocate_painter`，加上偏移后就复现了这个坑。

### 坑 10：`set_min_size`/`set_min_width`/`set_min_height` 是"从 cursor 再申请一块"的语义，放在内容之后会把 ui 撑高/撑宽一倍多

egui 0.36 的 `Placer::set_min_height` 实现是 `next_widget_space_ignore_wrap_justify(vec2(0.0, height))` 再 `expand_to_include_y`——即从**cursor 当前位置**（内容已经流过的地方）再往下申请 `height`，然后把 min_rect 扩到包含它。如果放在闭包**末尾**，min_rect = 内容高度 + spacing + height（实测：stat_card 内容 82px + 9 + 104 = 195，卡片阶梯错位的根源）；放在**开头**才是想要的"保底尺寸"。

**横向布局里更狠**：`set_min_width` 在 `left_to_right` 布局里会算出一个"从 cursor 起宽 W 的矩形"再 `expand_to_include_x`，把这一行的 min_rect 撑成"内容宽 + W"，而**父级的可用宽是按子控件的 min_rect 算的**——于是下一行再取 `available_width()` 就更宽，形成级联。侧栏导航行栽在这上面：行宽 194 → 310 → 427 → 531 → 672，交互矩形一路伸进主内容区（页面上任意空白处点击都会误触导航），而画出来的底色被面板裁到只剩一条，看着"像没事"。**症状是"命中区比画出来的区域大一大截"，排查时先量交互矩形，不要只看像素。**

**正确做法**：
- `allocate_ui_with_layout(desired, layout, |ui| { ui.set_min_size(desired); /* 内容 */ })`——`set_min_*` 放闭包**开头**，cursor 还在顶部时申请，min_rect 就是 `[top, top+desired]`，后续内容只在里面流，不会叠加。
- 反过来，`set_max_width` 不受 cursor 影响（它只改 max_rect），可以随时调。
- **要"整行等宽 + 命中区精确"的行（侧栏导航项这种）根本别用 `allocate_ui_with_layout`**：自己算精确矩形 + `ui.allocate_exact_size(size, Sense::click())`，背景/图标/文字全部自绘（`painter.text` 直画，文字不要再用 `ui.label`，否则 accesskit 多一个同名文本节点）。行宽在循环外算一次传进去，不要每行现算 `available_width()`。
- 验证手段：`ui_preview` 的探针 8（三卡几何 y∈[401,509] 同顶同底 104 高）就是这次的回归探针。

### 坑 11：`ui.horizontal` 的 cross-align 是 Center——嵌套 allocate 的 cell 会在"行高"里垂直居中，卡片一高就阶梯错位

`ui.horizontal` = `Layout::left_to_right(Align::Center)`，它的 `next_frame_ignore_wrap` 会把 cell 的 frame 撑到当前剩余高度、再把 child 垂直居中。行里前面的卡一旦变高（比如踩了坑 10），后面的卡就会在披撑大的行里居中、顶不再对齐——三张卡顶呈 394/436/458 阶梯状。

**正确做法**：等高卡片行用 `ui.horizontal_top`（cross-align = Min，顶对齐），配合外层 `allocate_ui_with_layout((card_w, H), top_down(Min))` 逐卡定宽（不要用 `ui.columns`，它首列前也吞一个 item_spacing，左右边距不对称）。

### 坑 12：`Frame::show` 闭包里的 `ui.available_width()` 在零高 ui 里仍可用，但零高 + 内容溢出时 label 不一定 wrap

`vertically_centered` 两帧布局里，Frame 的父 ui 高度可能为 0（`max_rect` 高 0）。这时 `ui.label` 的换行判定用的可用宽度不可靠——遇到过一行 label 705px 宽直接把 Frame 撑到右缘 1025（窗口 900）的实例。

**正确做法**：路径/版本号这类"可能很长"的单行信息，用 `widgets::truncate_middle_to_px(ui, text, font_size, max_px)` 按像素截断（中间省略，见坑 14）——既不换行也不硬裁切；卡的限宽不要靠 Frame 闭包里的 `set_min_width`（见坑 10），由外层 cell `allocate_ui_with_layout((w, h), ...)` + 闭包开头 `set_min_width(w)` 完成，Frame 闭包开头再 `ui.set_min_width(ui.available_width())` 让卡皮填满 cell（同英雄卡模式）。

### 坑 13：软件渲染器（ui_preview 的 CPU 光栅化）不遵守 painter 的 clip_rect

超出面板/ScrollArea 视口的形状在预览 PNG 里依然可见（GPU 渲染路径会裁掉）。这会造成"幽灵内容"：一次布局 bug 里，溢出到底部资源条面板后面的卡片残影在预览里直接把资源条仪表盖住了，探针误以为资源条没渲染。**预览里看到"不该出现的内容叠在别的区域上"时，先怀疑溢出 + 无裁剪渲染，再怀疑面板叠序**；反之真实窗口也可能与预览不同（残影不可见）。定位手段：直接对像素取色（`(x, y) → rgb`）对照色板。

### 坑 14：单行长文本截断必须按像素，不能按字符数；`Label::truncate()` 是硬裁切也不够

字符数拍脑袋在不同字号/字体下宽度天差地别：stat_card 的 value 用 16px 伪粗体时 26 个字符 ≈234px，而卡内可用只有 170px——字符数版把卡撑爆、文字叠到卡边；反过来给窄处定了小字符数，宽屏又浪费一截。`Label::truncate()` 能防止撑爆，但它是**右端硬裁切**（尾部直接切掉不留省略号），路径这种"开头结尾都有信息"的文本可读性差。

**正确做法**：一律用 `widgets::truncate_middle_to_px(ui, text, font_size, max_px)`（中间省略，二分收敛到刚好塞下，省略号计入宽度）：
- 卡内定宽字段：`text_w = (卡内可用宽).max(40)`，value/detail 分别用 16/12.5px 截断后再交给 `bold_label`/`ui.label`（`bold_label` 是 galley 直画，不 wrap 不截断，长文本必须外面先截好）。
- 带后缀的拼接行（如 `{path} — loading engine…`）：先量后缀宽，剩余额度才留给路径。
- 模态弹窗里的路径：给 `egui::Window` 设 `min_width` 后按 `ui.available_width()` 截，窗口不会被长路径撑宽。
- 丈量自绘按钮占宽用配套辅助（如 `pill_button_width`），别再手抄一份 magic number。

### 坑 15：`ScrollArea` 内 `available_height()` 是无穷大，垂直居中助手会直接吃成 inf

把 `widgets::vertically_centered`（或任何「按可用高度算 top_pad」的逻辑）放进自由高度的 `ScrollArea` 里，`top_pad = (∞ − 内容高)/2 = ∞`，布局直接坏。但又需要 ScrollArea 兜底长内容（纯居中页面一旦内容超出可视高度就是硬裁切，扫描页 Idle 加了简报卡后实测溢出 48px、卡片画到状态栏上）。

**正确做法**（见 `scan_page`）：在 ScrollArea **外面**先拿 `ui.available_height()`（面板可视高度，有限），手算 `top_pad = ((avail_h − 上帧内容高) * 0.5).max(0.0)`，在 ScrollArea 里 `add_space(top_pad)` 后再 `vertical_centered`；内容高度用上一帧的测量值（`ui.min_rect().height()`，两帧收敛，和 `vertically_centered` 同语义）。短内容时同样居中，内容变高/窗口变矮时 top_pad 归零、自然滚动。

另注：UI 测试/预览的软件渲染器不遵守 clip（坑 13），ScrollArea 溢出的内容会画在下层面板上——预览里看到“卡片压在状态栏上”就是溢出的直接证据，不是叠序错了。

### 坑 16：横向布局会在两个子控件之间**自动插入** `item_spacing.x`，宽度预算漏掉它就会整行超宽

`Layout::advance_after_rects` 在左到右布局里，每放一个子控件就把 cursor 推到 `widget.right() + item_spacing.x`。所以一行 n 个子控件的实际总宽 = Σ(子控件宽) + (n−1)×`item_spacing.x`，**你额外写的 `ui.add_space(GAP)` 是叠加在隐含间距之上的**，不是替代。

漏算的后果是整行比内容列宽出 2×`item_spacing`（本项目 `item_spacing.x = 10`）：仪表盘三张信息卡右缘 x=887（内容列右缘 x=867，多出 20px）、设置页双栏右缘 x=877（多出 10px），卡片被窗口右缘裁掉。

**正确做法**：
- 预算时把隐含间距算进去：`card_w = (avail − 2×GAP) / 3`，其中 `GAP` 应当是**最终视觉间距**；循环里 `ui.add_space((GAP − ui.spacing().item_spacing.x).max(0.0))` 补足到 GAP（隐含的那份已经在里面了）。
- 或直接别写 `add_space`，接受 `item_spacing.x` 当视觉间距。
- 验证手段：`ui_preview` 探针——扫所有 PNG 的最右 BG_CARD 像素，超过内容列右缘就是漏算（现在应恒为 867）。

### 坑 20：自绘卡片的"占位 ShapeIdx + 回填"顺序不能反，否则卡皮盖住文字

自绘卡片的做法是：先 `ui.painter().add(egui::Shape::Noop)` 占一个坑位（保证它在内容
**之前**入队），画完内容再用 `ui.painter().set(idx, shape)` 回填。

直接 `ui.painter().add(shape)` 会把形状排在**内容之后**——卡片是实心的 BG_CARD，
于是整张卡只剩底色，文字一行都不剩（2026-10 实测：信息卡看起来"画好了"，放大
才发现里面是空的）。Frame 自带的 fill/stroke 也有同样的坑：`Frame::show` 的形状是
在内容之后画的，**不要**再叠一层自己的填充。

### 坑 19：`ui.horizontal` 的 cross-align 不会让**嵌套的 `ui.vertical`** 居中

横向布局里，`cross_align` 只在"同一行的 cell 之间"生效：cell 的 frame 会被垂直居中，
但 cell 内部的 `ui.vertical`（scope）拿到的是**整行高度**的 max_rect，内容从顶部
开始排。结果就是"环在中间、文字和按钮全靠上"。

实测（2026-10 英雄卡）：环中心 y=269，标题顶端 y=175——差 94px，看着像没排版。

**正确做法**：显式算内容高度，再自己补顶部留白，让三栏共享同一条中线：

```rust
let text_h = widgets::measure_text_height(ui, &title, 20.0)
    + 6.0 + widgets::measure_wrapped_text_height(ui, &sub, 14.0, text_w);
ui.vertical(|ui| {
    ui.set_width(text_w);
    ui.add_space(((RING_BOX - text_h) / 2.0).max(0.0));   // 顶部留白 = 居中
    hero_text(ui, &title, &sub);
});
```

配套的量高辅助（`widgets.rs`）：`measure_text_height`（单行）、`measure_wrapped_text_height`
（按给定宽度换行后）。量高必须用与渲染一致的字号，且**标题是伪粗体**（`bold_label`
多描 0.6px 宽，不影响行高）。

**延伸**：需要"严格等距 + 严格对称留白"的卡片（信息卡），别用布局流拼——
横向行在竖向容器里会被 cross-align 拉满可用高度，行的位置和高度都算不准
（信息卡实测：上留白 39px / 下留白 15px）。改成**整张卡自绘**：自己量行高、自己
定位每个元素（`allocate_exact_size` 定位卡片矩形 → `painter.text` / `painter.galley`
画文字），几何就完全可控了。

### 坑 17：本主题下 egui 内置 `Checkbox` 的方框是"隐形的"

`ui.checkbox` 的方框底色取 `visuals.bg_fill`、描边取 `visuals.bg_stroke`；本主题把 `widgets.noninteractive.bg_fill` 与 `inactive.bg_fill` 都设成了 `BG_CARD`，而设置项的卡片底色**也是** `BG_CARD`——方框与卡片同色同描边，画面上只剩一个悬空的"✓"（2026-10 预览 3× 放大实测）。

**正确做法**：用 `widgets::toggle_checkbox`（自绘 18px 圆角方框：未选=BORDER_STRONG 描边、选中=ACCENT_ACTION 实底 + 白色对勾、禁用=BORDER 灰态），并补 `WidgetInfo::labeled(WidgetType::Checkbox, checked, label)` 进 accesskit 树。摆放时用 `horizontal_top`（cross-align Min）让方框顶对齐标题首行——`horizontal` 会把它按整个两行文字块垂直居中，看着像错行。

### 坑 18：自绘按钮里放 `ui.label` 会多出一道 item_spacing，按钮实际比量出来的宽 10px

`action_button` 原来在 `allocate_ui_with_layout` 里放 `ui.label`：横向布局会在图标与文字之间自动插入 `item_spacing.x`，而 `desired` 只算了显式的 `ICON_GAP`，于是**实际内容比 desired 宽 10px**，右侧内边距被挤掉一半（右缘对不齐），而且 accesskit 树里多出一个同名的"文本节点"（与按钮节点撞名，kittest `get_by_label` 多匹配 panic）。

**正确做法**：文字用 galley 直画（`painter.text` / `TextShape`），图标与文字的位置自己算——跟 `primary_button`/`pill_button` 同一套。量宽辅助也只量**本体**（`widgets::primary_button_body_width` / `widgets::action_button_width`，都不含外侧 `item_spacing`）；`primary_button_width` 那种"含两道间距"的版本只适合摆一整行按钮时用，拿它居中单颗按钮会偏 10px。

## 自动扫描（2026-10 新增）

三种触发共用一套"到点了就发起一次扫描"的逻辑，判定在 `AppCore::poll_background`
（每帧调用，内部有节流）里：

| 功能 | 配置项 | 行为 |
|---|---|---|
| 开机扫描 | `config.auto_scan_on_boot` | 启动后延迟 5 秒（等引擎冷启动）跑一次**闪电扫描** |
| 定时扫描 | `config.scheduled_scan: Option<ScheduleConfig>` | `days_mask`（bit0=周一…bit6=周日）+ `hour`/`minute`；到点跑用户选的**闪电/全盘** |
| 更新后扫描 | `config.auto_scan_after_update` | 病毒库**更新成功**后跑一次闪电扫描 |

- 定时判定：`Timestamp::weekday()`（0=周一）+ 计划时刻，允许 **5 分钟宽限窗口**
  （判定被节流错开时的补扫；窗口太大会让"下午开机"补跑早上的档）。
- 同一档计划当天只触发一次（`AutoScanState::last_fired`）。
- 三个开关默认全关（"用户不在场时发生的事"必须用户明确开启）；病毒库**自动更新**
  默认开（过期病毒库是安全风险）。
- 设置页 General tab 顶部三行 + 计划编辑器（周内天 chip + 时刻下拉 + 闪电/全盘 chip）。

## 图标绘制约定（`src/icons.rs`）

- 所有图标是手绘矢量（`egui::Shape` 基础图元），不依赖图标字体——保证跨平台渲染一致。
- 图形要在归一化的 `[0,1]×[0,1]` 单位正方形里保持**上下/左右对称**或至少视觉重心居中，不然即使外层布局对齐正确，图标本身"偏"也会显得没对齐（`icons::database` 曾经上边距 0.18/下边距 0.10 不对称，改成两边都是 0.14）。
- 新增图标前先检查现有的（`shield`/`shield_check`/`warning_triangle`/`bolt`/`database`/`hamburger`/`minimize`/`close`/`bolt_outlined`/`process_window`/`library_chip`/`disk`），风格要统一（描边为主，线宽 1.4~2.4px 之间视图标大小调整）。

## 做视觉改动时的检查清单

1. 新按钮/卡片是否需要被外层居中？→ 用"量尺寸 + `allocate_ui_with_layout`"，不要用裸 `Frame`/`ui.horizontal`。
2. 用到"淡淡的一层颜色"（光晕、背景纹理、半透明描边）？→ 检查是 `unmultiplied` 还是 `premultiplied`，别选错构造函数。
3. 要粗体？→ `bold_label`，不是 `.strong()`。
4. 新加了 `Panel`？→ 记得 `.show_separator_line(false)`，除非确实想要那条线。
5. 图标画完之后，自己在 `[0,1]` 单位格里目测一下是否对称。
6. 有单行长文本（路径/版本号/病毒名）？→ `truncate_middle_to_px` 按像素截断，不是字符数、不是 `Label::truncate()`（坑 14）。
7. 一行里排了多个子控件？→ 宽度预算要算上隐含的 `item_spacing.x`（坑 16）；量按钮宽度用"本体宽"辅助（不含外侧间距），居中单颗按钮别用含间距的版本（坑 18）。
8. 自绘可交互组件？→ 补 `widget_info`；文字用 galley 直画，别用 `ui.label`（accesskit 双节点 → kittest 多匹配 panic）；可见文字与无障碍名撞名时用 `primary_button_named` / `action_button_named` 给一个更具体的无障碍名（含可见文字即可）。
9. 改完之后跑 `cargo check`（原生 target），再跑 `cargo test ui_preview -- --ignored --nocapture`（见 `src/app/ui_preview.rs`）——它驱动真实 `App` 把 14 个状态渲染成 `target/ui_preview/*.png`，并在 stdout 打印 ASCII 构图、像素探针（居中偏移、点阵可见度 Δ、卡片列右缘）和 WCAG 对比度表。探针数字可做回归（例如资源条居中偏移曾从 +44px 修到 +4px；卡片列右缘应恒为 867）；PNG 本身要肉眼验收——Agent 看不了图，视觉终审请用户打开 `target/ui_preview/` 确认，或 `cargo run` 真机看（macOS 可直接起窗口，见 README「Mock Mode」）。
