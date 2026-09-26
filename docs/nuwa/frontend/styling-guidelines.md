# 样式规范

主题与共享样式唯一来源：[globals.css](../../../web/src/styles/globals.css)。不在 spec 复制整套 token 值。

## 主题与材质

- 亮色为 Clean-Room Minimal Industrial，暗色为 Dark Industrial；保留 32px 校准网格与低噪工业界面。
- 色彩使用 CSS 变量或语义 Tailwind 类，不写硬编码色值、`bg-blue-500` 等具名色；不在组件用 `dark:` 重复配色。此约束由 ESLint `no-restricted-syntax` 强制（含模板字符串与条件类名），唯一豁免见下文「矢量/分类色板」。
  - **语义色**：`@theme` 已把状态色暴露为完整 utility 家族 —— `text-status-success`、`bg-status-warning/10`、`border-status-info/20`、`ring-status-danger/40`、`fill-`/`stroke-`/`from-`/`to-`/`divide-` 等，且**不透明度变体自动可用**（Tailwind v4 以 `color-mix` 实现）。取值是 `var(--status-*)` 的中转，所以主题与深底子树切换会自然跟随，**不要写 `dark:` 变体**。
  - **五个状态槽**：`success` / `danger` / `warning` / `info` / `neutral`。`neutral` 用于离线、未启用、未知等不表达成败的状态。每槽各有基色、`-soft`（低位底）、`-border`（描边）、`-rgb`（拼 rgba 发光）与 `-solid`（实底）档。
  - **不得新增与状态槽语义重叠的色 token**：历史上有 `--accent-green` / `--accent-amber`（表达「正常 / 需关注」，与 success / warning 完全重叠），已合并。它们亮色档为 `#10b981` / `#d97706`，对浅底仅 2.45:1 / 3.08:1 且未在深底子树覆写——重叠 token 会各自漂移出不同的取值与覆盖范围，合并后统一达到 5.30:1 / 4.86:1。需要新的视觉角色时新增独立状态槽，而不是复制现有槽的颜色。
    - **大面积填充不为「轻」另立一套色**：进度条、状态点、mini 柱状图这类纯装饰性填充同样使用状态槽基色，不因「浅色看起来更轻」而复制一份浅色档。合并后亮色主题的填充色会更深（`#10b981` → `#047857`），这是有意接受的取舍：同一语义在文字、图标、描边、填充上取值一致，比按面积分裂两套色更重要。若将来确有按面积区分的需求，应新增**语义明确**的槽位（如 `-fill`）并同时给出对比度依据，而不是复用旧值。
  - **危险色角色**：`--status-danger` 及其 `-soft` / `-border` / `-rgb` 变体用于主题感知的状态文字、图标、提示底和描边。`--status-danger-solid` 是两主题固定的深红，仅用于危险实底配白字（白字对比度 6.47:1）；不要将亮色档 `--status-danger` 用作白字实底。
  - **实底必须用 `-solid` 档**：主题感知的 `--status-*` 在暗色主题下是浅色档（如 `#34d399` 对白字仅 1.92:1）。凡是「实底 + 白字」（按钮、徽标、反转面），一律用 `bg-status-{槽}-solid`。`surface-inverse` / `on-inverse` 同理成对使用：`--surface-inverse` 在暗色主题下会反转为浅面，必须搭配 `text-[var(--on-inverse)]` 而非 `text-white`。
  - **例外：矢量/分类色板**。画布描边、ROI 抽屉、类别徽标等需要在同一容器内**并列区分**的多色场景（如 `ROI_COLOR_PALETTES`、`rulesStudioTypes` 的 role 色板）使用字面色。此类颜色不表达状态语义，套用语义 token 会破坏可区分性。状态色仍须走 token。该文件已在 ESLint 中单独豁免。
- **恒定深底子树的状态色**：底色不随主题变化的区域（视频 OSD、灯箱）加 `.on-dark-surface`（灯箱容器 `.modal-backdrop--immersive` / `--lightbox` 已内置），它在该子树内把**全部五个状态色与三个文本色**固定为浅色档。文本三档须保持逐级变暗的相对层级
（相邻档对比度差距 ≥1.5×；`--text-muted` 在视频底上不低于 4:1）——**不要直接照搬 `.dark` 的三档取值**，
`.dark` 的 `--text-muted` (`#475569`) 在近黑视频底上仅 2.55:1，会低于被它替换的原始硬编码。原因：亮色档 `--status-danger` `#b91c1c` 在 `--video-surface` 上仅 2.99:1，不加作用域会不可读。不要在深底上直接写 rose-* 等具名色绕过这一机制。
- 需要 rgba 三元组（发光阴影等）时用 `--*-rgb` token，如 `rgba(var(--status-danger-rgb), 0.6)`；不要写死 `rgba(244,63,94,…)`。
- **Canvas 2D / WebGL 不参与 CSS 级联**：`ctx.fillStyle`、`ctx.strokeStyle`、`ctx.shadowColor` 与 shader 字符串里的 `var()` 是非法颜色串，会被**静默忽略**（绘制回退到上一个颜色），因此这些位置必须写字面色值，并由 `themeTokens.test.ts` 守护。
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

## 工具栏输入控件

高度、边框、圆角与聚焦环在同一工具栏内必须一致，否则一行控件会出现多种视觉语言。全仓由两套共享类族与对应组件承担，业务代码不自行拼样式：

| 用途 | 类族 | 组件 |
| --- | --- | --- |
| 关键字检索 | `.search-field`（`--compact` / `--form`） | [SearchInput](../../../web/src/components/ui/SearchInput.tsx) |
| 枚举选择 | `.select-field`（`--compact`） | [SelectField](../../../web/src/components/ui/SelectField.tsx) |

- 尺寸基线：默认 `2.25rem`（36px）与 `.page-action-btn` 对齐；`--compact` 为 `2rem`（32px），用于日志类紧凑工具栏；`--form` 为 `2.5rem`，用于弹窗表单。
- **可访问名称必填**：两个组件的 `label` / `aria-label` 都是必填属性，把「漏写导致屏幕阅读器只读“组合框”」从运行期缺陷变成编译期错误。
- **焦点可见**：聚焦态由类族内的 `:focus-visible` 提供（`.search-field` 用 `:has()` 放大边框，`.select-field` 用 `outline` + `--ring`）。**不要用裸 `outline-none` 屏蔽焦点** —— Tailwind v4 下它只产出 `outline-style: none`，若不补 `focus:border-*` / `focus:ring-*`，键盘用户将完全看不到焦点位置。
- 选择框重置原生 `appearance` 后必须渲染 `.select-field__caret`，否则不同平台的原生箭头会与图标体系冲突。
- **强调态**：筛选维度已收敛当前视图（取值不等于「全部」）时传 `emphasis`，由 `.select-field--emphasis` 统一着色，不在调用点写条件模板串。
- 原生下拉面板的 `option` 配色不受页面层叠影响，由 `.select-field option` 统一指定，避免暗色主题下亮出系统白底。

## 表单弹窗

- 创建/编辑表单统一使用 [ModalFormHeader](../../../web/src/components/ui/ModalFormHeader.tsx) 与 `.modal-surface--form`。表单面板使用实体 `--bg-surface-solid`；遮罩可保留单层轻模糊，不叠加 `.modal-surface--glass`，标题、滚动内容、底部操作分别使用 `.modal-form-header`、`.modal-form-content`、`.modal-form-footer`。
- 文本输入、选择框和文本域复用 `.modal-form-field`；标签使用 `.modal-form-label`，操作按钮使用 `.modal-form-button` 的次要/主要变体。业务分组和选择项可保留各自布局，不重复定义弹窗外壳的尺寸、滚动与页脚间距。
- 表单说明、徽标、关闭控件和字段占位文字使用 `--text-secondary`；`--text-muted` 只用于非关键元数据。主操作按钮使用 `--accent` 背景与高对比白色文字（`.modal-form-button--primary` 已内置固定双主题高对比前景）；操作按钮统一复用 `.modal-form-button` 的主要/次要/危险（`--danger`）变体。
- 验证键盘焦点、字段错误、内容溢出及亮暗主题；窄视口下标题和说明允许换行，不能遮挡关闭按钮。

## 页面工具栏与操作按钮

- 页面头部和工具栏操作统一复用全局 `.page-action-btn` 体系，高度固定为 `2.25rem`（36px），圆角 `0.75rem`（12px），内边距 `0 0.875rem`（图标按钮为等宽高 `.page-action-btn--icon`）。
- **主要操作按钮**使用 `.page-action-btn--primary`：`--accent` 背景搭配固定高对比白色前景 `#ffffff` 与 `var(--shadow-xs)`。
- **状态报告徽章与业务入口**：柔和状态使用 `.page-action-btn--soft-info`、`.page-action-btn--soft-success` 或 `.page-action-btn--soft-warning`，用于轻量报告入口或运行中反馈。
- **分区规范**：只读状态指示（如算法就绪、离线告警）必须与可交互操作按钮建立清晰的视觉分区（状态区使用高度 `2rem`（32px）的小胶囊，与右侧 `36px` 高度的操作按钮群通过竖线分隔），杜绝“状态胶囊伪装为按钮”或“按钮层级混杂”。

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

色彩契约由 [themeTokens.test.ts](../../../web/src/styles/themeTokens.test.ts) 守护：token 完整性、深底子树覆盖、
禁止复活已合并的重叠 token、`-solid` 与白字的搭配、canvas 禁用 `var()`、具名色仅限豁免色板；
并由 ESLint `no-restricted-syntax`（具名色、模板字符串、条件类名）把关。
新增状态语义时应扩充 `@theme` 槽位而非新增具名色，也不得复制现有槽的取值另立 token。
