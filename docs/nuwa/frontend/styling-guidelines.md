# 样式规范

主题与共享样式唯一来源：[globals.css](../../../web/src/styles/globals.css)。不在 spec 复制整套 token 值。

## 主题与材质

- 亮色为 Clean-Room Minimal Industrial，暗色为 Dark Industrial；保留 32px 校准网格与低噪工业界面。
- 色彩使用 CSS 变量或语义 Tailwind 类，不写硬编码色值、`bg-blue-500` 等具名色；不在组件用 `dark:` 重复配色。
  - **危险色角色**：`--status-danger` 及其 `-soft` / `-border` / `-rgb` 变体用于主题感知的状态文字、图标、提示底和描边。`--status-danger-solid` 是两主题固定的深红，仅用于危险实底配白字（白字对比度 6.47:1）；不要将亮色档 `--status-danger` 用作白字实底。
  - **例外：矢量/分类色板**。画布描边、ROI 抽屉、类别徽标等需要在同一容器内**并列区分**的多色场景（如 `ROI_PALETTES`、`rulesStudioTypes` 的 role 色板）使用字面色。此类颜色不表达状态语义，套用语义 token 会破坏可区分性。状态色仍须走 token。
- **恒定深底子树的状态色**：底色不随主题变化的区域（视频 OSD、灯箱）加 `.on-dark-surface`（灯箱容器 `.modal-backdrop--immersive` / `--lightbox` 已内置），它在该子树内把状态色固定为暗色档。原因：亮色档 `--status-danger` `#b91c1c` 在 `--video-surface` 上仅 2.99:1，不加作用域会不可读。不要在深底上直接写 rose-* 等具名色绕过这一机制。
- 需要 rgba 三元组（发光阴影等）时用 `--*-rgb` token，如 `rgba(var(--status-danger-rgb), 0.6)`；不要写死 `rgba(244,63,94,…)`。
- `.dark` 根类切换变量，初始主题及持久化以 [use-theme.ts](../../../web/src/hooks/use-theme.ts) 为准（当前默认 dark）。
- 面板复用 `.frosted-glass` 等共享材质，卡片使用 16～24px 圆角，避免直角；不随意使用 `!important`。
- 需要悬停/聚焦反馈的可点击面板用 `.frosted-glass-interactive`：它与 `.frosted-glass` 材质参数一致，但声明在 `@layer components`，因此 `hover:border-*`、`hover:shadow-*` 等 utilities 能正常覆盖。`.frosted-glass` 是 unlayered 规则，会压过 utilities 的 border/background/box-shadow，在这些属性上属于静默失效。
- 生产图标统一 Lucide，零 Emoji；避免霓虹闪烁，图标沿用统一 stroke 风格。
- **高频页面避免无意义的无限装饰动画**：实时监控、多路视频、高 DPR Canvas 或大量 `backdrop-filter` 区域内，默认不用 `animate-pulse`、`animate-ping`、`animate-bounce` 等持续动画表达在线状态；静态颜色、边框与 `shadow` 已能传达状态。单次入场动画不受此禁令。
  - **风险机制**：动画若位于 `backdrop-filter` 子树内，或模糊面覆盖在逐帧变化的视频/Canvas 之上，浏览器可能需要重复更新 backdrop 表面并重新采样。具体代价取决于浏览器、合成层与背景内容，应在目标环境实测；不要据此断言所有动画都会令整页重绘。
  - **实测记录**（1920×1080 @DPR=2，Apple M4，Chrome 153，使用本地 mock 数据）：告警中心基线 49.8%，移除毛玻璃子树内的无限动画后 0.3%～0.4%；实时大屏基线约 10.4%，停止 5 个状态点动画后约 1.1%。这些数值用于说明当前实现的风险，不构成其他设备的性能保证。
  - 视频/Canvas OSD 若背景已是高不透明度纯色，优先移除看不出效果的 `backdrop-filter`；加载骨架屏保留必要的加载动画，但不叠加无视觉收益的 `frosted-glass`。
  - 检测持续动画：`document.getAnimations().filter(a => a.effect?.getTiming().iterations === Infinity).length`。在持续渲染页面中逐项核对结果；非零项必须有明确的用户语义和实测预算。
- `cn()` 组合类名，调用方 `className` 放末尾；工具类由格式化工具排序。

## 表单弹窗

- 创建/编辑表单统一使用 [ModalFormHeader](../../../web/src/components/ui/ModalFormHeader.tsx) 与 `.modal-surface--form`。表单面板使用实体 `--bg-surface-solid`；遮罩可保留单层轻模糊，不叠加 `.modal-surface--glass`，标题、滚动内容、底部操作分别使用 `.modal-form-header`、`.modal-form-content`、`.modal-form-footer`。
- 文本输入、选择框和文本域复用 `.modal-form-field`；标签使用 `.modal-form-label`，操作按钮使用 `.modal-form-button` 的次要/主要变体。业务分组和选择项可保留各自布局，不重复定义弹窗外壳的尺寸、滚动与页脚间距。
- 表单说明、徽标、关闭控件和字段占位文字使用 `--text-secondary`；`--text-muted` 只用于非关键元数据。主操作按钮使用 `--accent` 背景和 `--bg-surface-solid` 前景，让亮暗主题分别采用高对比文字色；避免在调用处强制 `text-white`。
- 验证键盘焦点、字段错误、内容溢出及亮暗主题；窄视口下标题和说明允许换行，不能遮挡关闭按钮。

## 排印

| token          | 字体与用途                                                |
| -------------- | --------------------------------------------------------- |
| `font-display` | Unbounded，展示标题                                       |
| `font-tech`    | Space Grotesk，技术标题/控件                              |
| `font-sans`    | Inter，正文/表单                                          |
| `font-data`    | Space Mono，FPS/延迟/置信度；配 `tabular-nums` 防宽度抖动 |

字体自托管，使用 `font-display: swap`。文本通过 [i18n](./directory-structure.md#国际化)，布局容纳英文扩展，表单和关键错误不因截断丢失含义。

## 视频与叠加

- 视频容器预留 `aspect-video`（16:9），加载前后不跳动。
- 展示型透明 Canvas 绝对定位贴合视频内容区，设置 `pointer-events: none`；规则编辑层单独处理交互。
- 归一化坐标绘制时乘实际内容区尺寸，考虑留黑边与缩放，不能使用固定屏幕像素定位。
- **离屏 Canvas 的 rAF 循环不得无条件整屏重绘**：每次 `clearRect(0, 0, canvas.width, canvas.height)` 都按物理像素计费（1080p @DPR=2 约 510 万像素）。若没有检测轨迹且上一帧已清空，跳过 `clearRect` 与绘制，仅续约 rAF；如上一帧画过内容，仍须清空一次以擦除旧图形。判定依据使用本帧实际轨迹和上帧绘制状态。
- 规则编辑在动态子码流上完成；数据与播放帧按时间戳对齐，详见 [组件规范](./component-guidelines.md)。
- 自定义光标的交互目标沿用 `.reticle-target`，保留原生键盘焦点和可访问性。

验证双主题、32px 网格、三语溢出、等宽数字、视频多路负载和主题切换无残留硬编码颜色。
