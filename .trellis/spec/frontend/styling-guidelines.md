# 样式规范

> Tailwind CSS + 原生 CSS 变量。双主题架构（亮色 Clean-Room Minimal Industrial + 暗色 Dark Industrial）。
> 通过 CSS 变量 + class 策略实现主题切换，组件内不写死颜色。

> ⚠️ **状态：立项约定（尚未经代码验证）**
> 首批页面落地后需回填真实样式示例与设计 token，并删除本提示。

---

## 设计基调

| | 亮色主题（默认） | 暗色主题 |
|---|---|---|
| 基调 | 高奢极简香槟瓷白（Clean-Room Minimal Industrial） | 深邃深空精密控制台（Dark Industrial） |
| 背景 | #FBFBFC + 校准网格 | #030408 + 校准网格 |
| 强调色 | #0052FF 纯净克莱因蓝 / 玫红强调 | #3B82F6 提亮蓝 / 靛青强调 |
| 材质 | 高透磨砂玻璃（白底 70% 透明） | 深色磨砂玻璃（深底 75% 透明） |
| 共用 | 微圆角 12-20px、Reticle 光标、Lucide 图标、Unbounded/Space Grotesk 展示、Space Mono 等宽数据 |

**严禁**：失控杂乱的霓虹乱闪、硬编码颜色值、Emoji 表情符号。

---

## 双主题 CSS 变量体系

所有颜色走 CSS 变量，组件内用 `var(--xxx)` 引用。主题切换通过 `<html class="dark">` 触发。

```css
/* src/styles/globals.css */

/* Tailwind CSS v4 CSS-First 主题扩展 */
@theme {
  --font-display: 'Unbounded', 'Space Grotesk', sans-serif;
  --font-tech: 'Space Grotesk', sans-serif;
  --font-sans: 'Inter', sans-serif;
  --font-data: 'Space Mono', monospace;
}

/* ── 亮色主题（默认） ── */
:root {
  /* 背景层 */
  --bg-primary: #FBFBFC;
  --bg-secondary: #F0F2F5;
  --bg-surface: rgba(255, 255, 255, 0.75);
  --bg-surface-solid: #FFFFFF;

  /* 文字 */
  --text-primary: #0B0F19;
  --text-secondary: #475569;
  --text-muted: #94A3B8;

  /* 强调色 */
  --accent: #0052FF;
  --accent-soft: rgba(0, 82, 255, 0.08);
  --accent-green: #10B981;
  --accent-amber: #D97706;
  --destructive: #DC2626;

  /* 边框 */
  --border: rgba(15, 23, 42, 0.08);
  --border-strong: rgba(15, 23, 42, 0.15);
  --ring: rgba(0, 82, 255, 0.4);

  /* 光效与阴影 */
  --glow: rgba(0, 82, 255, 0.06);
  --shadow-sm: 0 1px 2px rgba(0, 0, 0, 0.04);
  --shadow-md: 0 4px 16px rgba(0, 0, 0, 0.06);
  --shadow-lg: 0 8px 32px rgba(0, 0, 0, 0.08);
  --highlight-opacity: 1; /* 磨砂玻璃内层白色高光 */

  /* 圆角 */
  --radius-card: 16px;
  --radius-button: 12px;
  --radius-full: 9999px;

  /* 检测框与状态色（跨主题共用） */
  --detection-person: 142 71% 45%;
  --detection-vehicle: 217 91% 60%;
  --status-offline: 0 0% 45%;

  color-scheme: light dark;
}

/* ── 暗色主题 ── */
.dark {
  --bg-primary: #030408;
  --bg-secondary: #0B0F19;
  --bg-surface: rgba(8, 11, 18, 0.75);
  --bg-surface-solid: #11141D;

  --text-primary: #F8FAFC;
  --text-secondary: #94A3B8;
  --text-muted: #475569;

  --accent: #3B82F6;
  --accent-soft: rgba(59, 130, 246, 0.12);
  --accent-green: #34D399;
  --accent-amber: #F59E0B;
  --destructive: #EF4444;

  --border: rgba(255, 255, 255, 0.09);
  --border-strong: rgba(255, 255, 255, 0.18);
  --ring: rgba(59, 130, 246, 0.5);

  --glow: rgba(59, 130, 246, 0.08);
  --shadow-sm: 0 1px 2px rgba(0, 0, 0, 0.5);
  --shadow-md: 0 4px 16px rgba(0, 0, 0, 0.6);
  --shadow-lg: 0 8px 32px rgba(0, 0, 0, 0.7);
  --highlight-opacity: 0.08;
}
}
```

规则：

- **新增颜色先加变量再用**。组件里出现 `#hex`、`rgb()`、`bg-blue-500` 就等于绕过了主题系统。
- 主题切换通过 `useTheme` hook 设置 `<html>` 的 `class="dark"`，**不在组件里写 `dark:` 变体处理颜色**（布局差异可以用 `dark:`）。
- 检测类别颜色（人/车/宠物）走变量 —— 它们同时被 CSS 和 Canvas 绘制用到，变量是唯一真相源。
- 暗色主题的强调色比亮色主题提亮一档（`#0052FF` → `#3B82F6`），确保对比度达标。

---

## 排印系统

四套字体各司其职，在 Tailwind CSS v4 中通过 CSS `@theme` 扩展配置：

| 字体族 | 字体 | 用途 | 关键特性 |
|--------|------|------|---------|
| `font-display` | Unbounded 700/800 | 先锋艺术展示、大标题 | 视觉张力强，letter-spacing: -0.03em |
| `font-tech` | Space Grotesk 500/600 | 科技感次级标题、标签 | 几何感强，硬朗工业风 |
| `font-sans` | Inter 400/500/600 | 正文、系统 UI 控件 | 高度清晰易读 |
| `font-data` | Space Mono 400/700 | 遥测指标、HUD 坐标、FPS | 等宽数字零抖动 |

**字号两极张力**：

- 巨型标题：72px+（hero 区域），font-display，letter-spacing: -0.04em
- 工程元数据标签：9px~11px，全大写，tracking-[0.2em]，font-data

**等宽数字**：`.font-data` 必须开启 `tnum` + `zero` 特性，确保毫秒延迟、置信度、FPS 等刷新数据零抖动：

```css
.font-data {
  font-feature-settings: "tnum" 1, "zero" 1;
  font-variant-numeric: tabular-nums;
}
```

---

## 物理材质（Frosted Glassmorphism）

全局卡片与浮动面板采用磨砂玻璃材质，**禁止直角设计**：

```css
.frosted-glass {
  background: var(--bg-surface);
  backdrop-filter: blur(24px) saturate(160%);
  -webkit-backdrop-filter: blur(24px) saturate(160%);
  border: 1px solid var(--border);
  border-radius: var(--radius-card);
  box-shadow:
    inset 0 1px 1px rgba(255, 255, 255, var(--highlight-opacity)),
    var(--shadow-md);
  transition: all 0.3s cubic-bezier(0.16, 1, 0.3, 1);
}
```

**双层光学边缘高光**：外层超细边框叠加内部发光内阴影（`inset 0 1.5px 2px rgba(255,255,255,1)`），营造光学镀膜镜片悬浮感。亮色主题下内发光明显（`--highlight-opacity: 1`），暗色主题下极度微弱（`--highlight-opacity: 0.05`）。

**圆角规则**：严格 12px-20px（`rounded-2xl` / `rounded-3xl`），全局禁止直角。

---

## 背景层：校准网格与环境光晕

body 背景叠加两层装饰，统一用 CSS 变量自动适配双主题：

```css
body {
  background-color: var(--bg-primary);
  background-image:
    /* 校准网格（Calibration Grid）：32px 间距的极细发丝线 */
    linear-gradient(to right, var(--border) 1px, transparent 1px),
    linear-gradient(to bottom, var(--border) 1px, transparent 1px);
  background-size: 32px 32px;
}
```

**克莱因蓝环境光晕**：极低透明度的 `radial-gradient`，从屏幕中心扩散，叠加在校准网格之上。由全局 CSS 或 `TopologyCanvas` 组件实现。

---

## 自定义光标（Reticle Cursor）

接管系统光标，拟态为光学检测 ROI 边界框：

- **默认态**：48×48px，`rounded-2xl`，1px 克莱因蓝边框（50% opacity），左上角显示微米坐标（font-data, 8px）
- **交互态**（hover 可交互元素时）：膨胀至 64×64px，`rounded-3xl`，边框加深至 80% opacity，填充 `var(--accent-soft)`
- **实现**：React 组件 + `requestAnimationFrame` + lerp 阻尼跟随，`pointer-events: none`
- body 设置 `cursor: none` 全局禁用默认光标
- **触屏设备降级**：触摸时不显示 Reticle，恢复系统光标

详见组件规范中的 `ReticleCursor` 组件定义。

---

## 图标系统

- 统一使用 **Lucide** 图标（`lucide-react`），stroke-width 设为 **1.5px** 或 **1.75px**
- **界面全程严禁使用任何 Emoji 表情符号**

---

## 三者的分工

| 用什么 | 场景 |
|--------|------|
| **Tailwind 工具类** | 间距、字号、圆角、flex/grid 基础布局 |
| **CSS 变量** | 所有颜色、阴影、边框（跨主题共用） |
| **原生 CSS** | 容器查询、复杂选择器、动画关键帧、`:has()`、磨砂玻璃材质 |

**默认用 Tailwind**。写原生 CSS 前先确认 Tailwind 真的做不到 —— 大多数"必须写 CSS"的场景其实有工具类。

---

## 何时该用原生 CSS

Argus 的界面确实会碰到 Tailwind 的表达边界。这些场景直接写 CSS，不要硬凑工具类：

**① 容器查询** —— 视频宫格里的单元格要根据自身宽度调整布局，而不是视口宽度：

```css
.camera-tile {
  container-type: inline-size;
}
@container (min-width: 24rem) {
  .camera-tile__overlay { display: flex; }
}
```

**② `:has()` 关系选择器** —— 父元素根据子元素状态变化：

```css
.event-row:has(.event-row__badge--alert) {
  background: color-mix(in oklab, var(--destructive) 8%, transparent);
}
```

**③ 复杂动画** —— 多关键帧、多元素编排的动画写 `@keyframes`，不要用一长串 Tailwind animate 工具类拼。

**④ `subgrid`** —— 时间线/表格需要跨行对齐时，`subgrid` 是唯一干净的解法。

**⑤ `color-mix()` / 相对颜色** —— 基于设计 token 派生颜色，避免在 config 里手写一堆近似色。

---

## className 组织

用 `cn()`（`clsx` + `tailwind-merge`）：

```tsx
<div
  className={cn(
    "frosted-glass rounded-2xl p-4",        // 基础材质
    isActive && "border-accent/40",           // 条件
    className,                                // 外部覆盖
  )}
/>
```

约定：

- **工具类按「布局 → 盒模型 → 视觉 → 交互态」大致排序**。不强制，但同一个项目里保持一致能大幅提升可读性。
- **条件类单独一行**，不要塞进模板字符串里拼。
- **组件接受 `className` prop 并放在 `cn()` 最后**，让调用方能覆盖。`tailwind-merge` 会正确处理冲突。
- 超过 15 个工具类的元素 → 考虑是不是该拆组件，或者该用一个语义化 CSS 类。

---

## 原生 CSS 的放置与作用域

- feature 专属样式放该 feature 目录下的 `.css` 文件，在组件里 `import "./grid.css"`。
- **类名加 feature 前缀**（`camera-tile`、`event-row`），BEM 风格的 `__element` / `--modifier`。Vite 不做 CSS Modules 的话就靠命名约定隔离。
- 全局样式只放 `src/styles/globals.css`：Tailwind 指令、CSS 变量、`@layer base` 里的元素级重置、`.frosted-glass` 类。
- **不要用 `!important`**。需要它说明选择器优先级设计错了，或者该用 `cn()` 的合并能力。

---

## 视频区域的特殊约束

- 视频容器用 `aspect-video`（16:9）固定比例，**避免加载时的布局抖动**。
- 宫格布局用 CSS Grid + `auto-fit` / `minmax()`，让路数变化时自动重排：

  ```css
  .camera-grid {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(20rem, 1fr));
    gap: 0.5rem;
  }
  ```

- **检测框 Canvas 叠加层**：采用与视频容器同尺寸的透明 Canvas，绝对定位贴合并设置 `pointer-events: none`，避免阻挡用户交互。
- **坐标归一化绘制**：模型产出的坐标在后处理后归一化到 `[0.0, 1.0]`，Canvas 在绘制时乘以当前画面的真实像素宽高，确保在窗口缩放与响应式宫格变化时框位严格精准。

---

## 禁止事项

- ❌ 暗黑黑客风、赛博朋克霓虹发光效果（暗色主题也是工业精密风）
- ❌ Emoji 表情符号
- ❌ 直角卡片（必须 `rounded-2xl` 以上）
- ❌ 硬编码色值（`#hex`、`rgb()`、`bg-blue-500`）
- ❌ 内联 `style={{...}}` 写静态样式（动态计算值除外，如检测框位置）
- ❌ `!important`
- ❌ 用 Tailwind `dark:` 变体处理颜色（走 CSS 变量自动适配）
- ❌ 无前缀的全局 CSS 类名
- ❌ 视频容器不设固定宽高比
- ❌ 系统默认光标可见（由 Reticle 接管）
- ❌ 未经 `.font-data` 类修饰的等宽数据显示（必须开启 tnum + zero 特性）

---

## 待验证事项

- [ ] Three.js 拓扑背景在低端设备（RK3568 集成 GPU）上的 GPU 占用与降级策略
- [ ] Reticle 光标在触屏设备上的降级方案（触摸时不显示光标？）
- [ ] Space Mono / Unbounded 字体文件体积（自托管 vs Google Fonts CDN 的取舍）
- [ ] `backdrop-filter` 在 Safari 15 以下的兼容性 polyfill
- [ ] 亮色/暗色主题切换时 Three.js Canvas 配色的联动策略
