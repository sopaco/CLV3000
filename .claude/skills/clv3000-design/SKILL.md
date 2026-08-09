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

| 用途 | 颜色 | 说明 |
|---|---|---|
| 页面背景 | `BG_APP` `#0A0D12` | 全局最底层背景 |
| 侧边栏背景 | `BG_SIDEBAR` `#06080B` | 比 BG_APP 更深 |
| 标题栏背景 | `BG_TITLEBAR` `#080A0E` | 介于两者之间 |
| 卡片/面板背景 | `BG_CARD` `#12161D` | 按钮、卡片、胶囊的底色 |
| 描边/分割线 | `BORDER` `#222832` | 卡片描边，**不要**用来做大面积分割线（太显眼，见下面"面板分割线"坑） |
| 强调色（蓝） | `ACCENT_BLUE` `#3AA0E0` | 图标、进度、选中态 |
| 强调色背景 | `ACCENT_BLUE_BG` `#113247` | hover/active 底色 |
| 安全/成功 | `GREEN` `#22C55E` | |
| 危险/威胁 | `RED` `#EF4444` | |
| 危险背景 | `RED_BG` `#2A1416` / `RED_BORDER` `#502024` | 威胁卡片 |
| 文字主色 | `TEXT_PRIMARY` `#F5F7FA` | |
| 文字次要 | `TEXT_SECONDARY` `#8A93A3` | |
| 文字弱化 | `TEXT_MUTED` `#5C6472` | |

### 间距 / 圆角 / 字号

- 卡片圆角：`10~12px`；胶囊按钮圆角：`255`（`CornerRadius::same` 的最大值，等价于"完全圆"，具体多圆由高度决定）。
- 卡片描边：`1px BORDER`。
- 标准按钮内边距：水平 `14~16px`，垂直 `7~10px`。
- 图标尺寸经验值（2026-08 那轮"图标偏小"反馈后统一 +15% 得出）：
  - 侧边栏图标：40×40 容器，图形占 ~21px（`shrink(9.5)`）
  - 标题栏 App 图标：23px；标题栏最小化/关闭按钮图形：32×32 容器，图形 ~14px（`shrink(9.0)`）
  - 按钮内图标（`action_button`）：21px
  - 威胁卡片图标：39px
  - 大状态圆环里的图形：圆环直径 × 0.50
- 字号：标题 18~20px，正文 14px（默认），说明文字 `small()`（约 12~13px）。
- 圆点背景网格：间距 28px，半径 1px，颜色 `DOT_GRID`（premultiplied `(8,8,8,8)`，即约 3% 不透明度的白）——**故意做得很淡**，是纹理感而不是视觉主体，如果看起来"抢眼"基本可以确定是颜色写错了（见下方坑 3）。

### 组件形态

- **状态圆环**（仪表盘/扫描结果）：大圆 + 描边 + 内部填充 `BG_CARD` + 外圈柔和光晕（`widgets::paint_glow`）+ 居中图形（盾牌打勾 / 警告三角）。光晕是本项目里唯一近似"发光"效果的手段，本质是几层同心圆叠加、透明度从外到内递增，**必须**用 `from_rgba_unmultiplied`，且要在外面多留 `radius * 1.5` 左右的画布空间，不然会被裁掉。
- **进度圆环**（扫描中）：背景是一圈很淡的描边（`from_rgba_unmultiplied(255,255,255,18)`，不是纯色实心圈！），前景是从正上方顺时针延伸的实色弧。百分比未知时画一段跟随时间旋转的弧，制造"正在进行"的动感。
- **胶囊按钮/徽标**：`BG_CARD` 填充 + `BORDER` 描边 + 图标（可选）+ 文字，垫圆角。"填充色"按钮（比如威胁卡片的"隔离"）用强调色/危险色整块填充、白字，"描边"按钮（比如"忽略"）用透明底+描边，两者高度必须一致。
- **威胁卡片**：`RED_BG` 底 + `RED_BORDER` 描边，左侧一个红色方块警告图标，中间病毒名+"高危"红色小徽标+文件路径，右侧隔离/忽略两个胶囊按钮。
- **卡片式设置项**（比如"包含可移动磁盘"开关）：不要让 checkbox 裸露在页面上，包一层 `BG_CARD` + `BORDER` 的卡片容器，宽度按内容**量出来**，不要拍脑袋写死（见坑 1）。

## egui 布局/渲染踩过的坑（写代码前先看这几条，能省很多返工）

### 坑 1：`Frame` / `ui.horizontal` 不参与外层 `Align::Center`

`egui::Frame::show(ui, ...)` 和 `ui.horizontal(...)` 在被外层 `vertical_centered`/`Align::Center` 布局调用时，会把自己的"期望尺寸"报成父容器**当前的全部可用宽度**（因为内容画完之前不知道真实大小，只能先占住最大空间）。结果就是：外层想居中一个"看起来应该是小按钮"的东西，实际拿到的是一个和容器一样宽的东西，居中直接失效——表现为按钮从最左边一路铺到很宽的位置，而不是一颗居中的小胶囊。

**正确做法**：自己先用 `widgets::measure_text_width(ui, text, size)`（内部就是 `ui.ctx().fonts_mut(|f| f.layout_no_wrap(...))`）量出文字的真实尺寸，加上图标/内边距算出准确的 `desired_size`，再用 `ui.allocate_ui_with_layout(desired_size, layout, |ui| {...})` 分配——**不要**直接用 `Frame::show` 或裸的 `ui.horizontal` 去包一个需要被外层居中的组件。背景色用"占位 Shape + 事后回填"的技巧：

```rust
let bg_idx = ui.painter().add(egui::Shape::Noop);
let response = ui.allocate_ui_with_layout(desired_size, layout, |ui| { /* 内容 */ }).response;
let shape = egui::epaint::RectShape::new(response.rect, corner_radius, fill, stroke, egui::epaint::StrokeKind::Inside);
ui.painter().set(bg_idx, egui::Shape::Rect(shape));
```

参考实现：`app.rs` 的 `action_button`、`centered_card`；`widgets.rs` 的 `pill_button`、`stat_pill`/`centered_stat_pills`（一行多个胶囊一起居中，宽度是每个胶囊量出来后加总的，不是猜的）。

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

## 图标绘制约定（`src/icons.rs`）

- 所有图标是手绘矢量（`egui::Shape` 基础图元），不依赖图标字体——保证跨平台渲染一致。
- 图形要在归一化的 `[0,1]×[0,1]` 单位正方形里保持**上下/左右对称**或至少视觉重心居中，不然即使外层布局对齐正确，图标本身"偏"也会显得没对齐（`icons::database` 曾经上边距 0.18/下边距 0.10 不对称，改成两边都是 0.14）。
- 新增图标前先检查现有的（`shield`/`shield_check`/`warning_triangle`/`bolt`/`database`/`hamburger`/`minimize`/`close`），风格要统一（描边为主，线宽 1.4~2.4px 之间视图标大小调整）。

## 做视觉改动时的检查清单

1. 新按钮/卡片是否需要被外层居中？→ 用"量尺寸 + `allocate_ui_with_layout`"，不要用裸 `Frame`/`ui.horizontal`。
2. 用到"淡淡的一层颜色"（光晕、背景纹理、半透明描边）？→ 检查是 `unmultiplied` 还是 `premultiplied`，别选错构造函数。
3. 要粗体？→ `bold_label`，不是 `.strong()`。
4. 新加了 `Panel`？→ 记得 `.show_separator_line(false)`，除非确实想要那条线。
5. 图标画完之后，自己在 `[0,1]` 单位格里目测一下是否对称。
6. 改完之后跑一遍 `cargo check`（原生 target）——这个环境是 macOS，`cargo run` 能直接起真实窗口做交互预览（见 README「Mock Mode」），但看不了截图，视觉细节最终要请用户在他们机器上确认。
