# 样式规范 (Styling Guidelines)

> 基于 Tailwind CSS + CSS 原生变量的双主题架构（亮色 Minimal Industrial 默认 + 暗色 Dark Industrial）。
> 核心原则：**组件内严禁写死颜色，严格采用 CSS 变量与设计 Token 驱动**。

---

## 1. 设计基调与硬性视觉规范

| 维度 | 亮色主题 (默认) | 暗色主题 |
|------|---------------|---------|
| **视觉基调** | 高奢极简香槟瓷白（Clean-Room Minimal Industrial） | 深邃深空精密控制台（Dark Industrial） |
| **背景色** | `--bg-primary: #FBFBFC` + 发丝校准网格 | `--bg-primary: #030408` + 发丝校准网格 |
| **强调色** | `--accent: #0052FF`（纯净克莱因蓝） | `--accent: #3B82F6`（工业提亮蓝） |
| **物理材质** | 高透磨砂玻璃（白底 75% 磨砂，外边框+发光内阴影） | 深色磨砂玻璃（深底 75% 磨砂） |
| **通用规约** | 微圆角 16px~24px (`rounded-2xl`/`3xl`)，**全局严禁直角**，图标统一为 **Lucide** |

- **严禁 Emoji**：生产工业控制台与监控界面**全程严禁使用任何 Emoji 表情符号**，状态标识一律使用 Lucide 矢量图标或徽标 Badge。
- **色彩唯一信源**：组件中**严禁出现硬编码十六进制色值**（如 `#0052FF`、`#fff`）或直接使用 Tailwind 具名色（如 `bg-blue-500`），必须通过 `var(--xxx)` 或 Tailwind 语义类调用。

---

## 2. 核心 CSS 变量与双主题 Token

主题切换通过设置 `document.documentElement` 的 `<html class="dark">` 触发，样式通过 CSS 变量自动换肤，**不在组件内手写 `dark:` 变体覆盖颜色**：

```css
/* src/styles/globals.css */
:root {
  --bg-primary: #FBFBFC;
  --bg-secondary: #F0F2F5;
  --bg-surface: rgba(255, 255, 255, 0.75);
  --text-primary: #0B0F19;
  --text-secondary: #475569;
  --text-muted: #94A3B8;
  --accent: #0052FF;
  --accent-soft: rgba(0, 82, 255, 0.08);
  --accent-green: #10B981;
  --destructive: #DC2626;
  --border: rgba(15, 23, 42, 0.08);
  --border-strong: rgba(15, 23, 42, 0.15);
  --radius-card: 16px;
  --highlight-opacity: 1;
}

.dark {
  --bg-primary: #030408;
  --bg-secondary: #0B0F19;
  --bg-surface: rgba(8, 11, 18, 0.75);
  --text-primary: #F8FAFC;
  --text-secondary: #94A3B8;
  --text-muted: #475569;
  --accent: #3B82F6;
  --accent-soft: rgba(59, 130, 246, 0.12);
  --accent-green: #34D399;
  --destructive: #EF4444;
  --border: rgba(255, 255, 255, 0.09);
  --border-strong: rgba(255, 255, 255, 0.18);
  --highlight-opacity: 0.08;
}
```

---

## 3. 排印体系与等宽数据

| 字体族 | 字体 | 适用场景 | 核心约束 |
|-------|------|---------|---------|
| `font-display` | Unbounded | 仪表盘大标题、Hero 视觉呈现 | 紧凑字距 `tracking-[-0.03em]` |
| `font-tech` | Space Grotesk | 次级标题、状态指示、控件标签 | 工业几何感 |
| `font-sans` | Inter | 常规文本、表单输入、正文 | 极致清晰易读 |
| `font-data` | Space Mono | 遥测指标、HUD 坐标、FPS、延迟 | **必须启用 `tabular-nums` 等宽零抖动** |

```css
/* 等宽数据展示铁律：杜绝高频刷新时界面数字宽度抖动 */
.font-data {
  font-feature-settings: "tnum" 1, "zero" 1;
  font-variant-numeric: tabular-nums;
}
```

---

## 4. 物理材质与视频渲染层规范

- **磨砂玻璃材质 (`.frosted-glass`)**：
  ```css
  .frosted-glass {
    background: var(--bg-surface);
    backdrop-filter: blur(24px) saturate(160%);
    -webkit-backdrop-filter: blur(24px) saturate(160%);
    border: 1px solid var(--border);
    border-radius: var(--radius-card);
    box-shadow: inset 0 1px 1px rgba(255, 255, 255, var(--highlight-opacity)), var(--shadow-md);
  }
  ```
- **视频容器固定宽高比**：视频播放容器统一使用 `aspect-video`（16:9），严防媒体加载前后触发页面重排（CLS 布局抖动）。
- **实时检测框 Canvas 叠加层**：
  - 透明 Canvas 必须绝对定位贴合于视频容器，且必须声明 `pointer-events: none`，避免拦截画面交互点击；
  - **坐标归一化契约**：算法与后端输出的 `[x, y, w, h]` 严格归一化在 `[0.0, 1.0]`，Canvas 绘制时乘以渲染画布的实际像素尺寸，实现响应式自适应对齐。

---

## 5. 工具类组织与 `cn()` 规范

- 统一使用 `cn()`（`clsx` + `tailwind-merge`）组合样式；
- 组件暴露的 `className` prop 必须置于 `cn(...)` 参数列表末尾，确保调用方可覆盖基础样式；
- 建议顺序：`布局结构 (flex/grid) -> 盒模型 (p/m/w/h) -> 视觉修饰 (bg/border/rounded) -> 交互状态 (hover/focus)`。

---

## 6. 禁止事项 (Iron Rules)

- ❌ 在代码中包含任何 Emoji 表情符号
- ❌ 出现直角卡片（必须 `rounded-2xl` / 16px 以上）
- ❌ 硬编码色彩十六进制值（`#fff`, `#000`, `#1890ff`）或无语义的 Tailwind 具名色（`bg-red-500`）
- ❌ 使用 Tailwind `dark:` 变体处理色彩切换（统一由 CSS 变量自动适配）
- ❌ 动态数据与 FPS 渲染未采用 `.font-data`（未开启等宽字符导致布局高频横向抖动）
- ❌ 在 CSS 中随意滥用 `!important`
