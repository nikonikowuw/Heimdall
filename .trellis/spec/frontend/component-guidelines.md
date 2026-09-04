# 组件规范

> React + TypeScript + shadcn/ui。核心约束：Argus 的界面上同时跑着多路视频和持续推送的事件流，**不必要的重渲染是真实的性能问题**。
> 双主题架构（亮色 Clean-Room / 暗色 Dark Industrial），颜色走 CSS 变量，组件内不写死颜色。

> ⚠️ **状态：立项约定（尚未经代码验证）**
> 首批组件落地后需回填真实组件示例，并删除本提示。

---

## 组件的基本形状

```tsx
// src/features/alarms/components/AlarmCard.tsx
interface AlarmCardProps {
  alarm: AlarmDto
  selected?: boolean
  onSelect?: (id: string) => void
}

export function AlarmCard({ alarm, selected = false, onSelect }: AlarmCardProps) {
  return (
    <button
      type="button"
      onClick={() => onSelect?.(alarm.id)}
      className={cn(
        "frosted-glass rounded-2xl p-3 text-left reticle-target",
        selected && "border-accent/40 bg-accent-soft"
      )}
    >
      {/* ... */}
    </button>
  )
}
```

约定：

- **函数声明，具名导出**。不用 `export default`（默认导出让重命名和检索变困难）。
- **props 接口叫 `<组件名>Props`**，写在组件正上方。
- **不用 `React.FC`**。它对泛型组件和 `children` 的处理都不好。
- 可选 props 在解构时给默认值，不在接口里写默认。
- **容器类组件使用 `frosted-glass` 类**，不手写 `backdrop-filter` / `box-shadow`。
- **可交互组件添加 `reticle-target` 类**，标记 Reticle 光标的交互态膨胀区域。

---

## 展示组件与容器组件分开

```tsx
// ❌ 组件自己拉数据 + 自己渲染：无法复用、难测试、每个使用处都触发一次请求
function EventList() {
  const [events, setEvents] = useState([])
  useEffect(() => { fetch("/api/v1/events").then(...) }, [])
  return <div>{events.map(...)}</div>
}

// ✅ 数据获取在容器/hook，渲染是纯函数
function EventListPage() {
  const { events, loading } = useEvents()          // 数据
  return <EventList events={events} loading={loading} />
}

function EventList({ events, loading }: EventListProps) {  // 纯展示
  ...
}
```

规则：**`features/*/components/` 下的组件默认是纯展示组件**，数据获取放在 feature 的 `hooks/` 或页面级组件里。

---

## 重渲染控制（Argus 特有）

实时预览页面上有 8 路视频 + 持续推送的事件流。一条事件进来导致 8 路播放器全部重渲染，界面就会卡。

**必须遵守**：

1. **订阅粒度要细**。从 Zustand 取状态时用选择器只取需要的字段，见 [state-management.md](./state-management.md)。

   ```tsx
   // ❌ 任何 store 变化都重渲染
   const store = useCameraStore()
   // ✅ 只有这一路的状态变化才重渲染
   const status = useCameraStore((s) => s.statuses[cameraId])
   ```

2. **列表项用 `memo`**。事件列表会持续追加，不 memo 就是每次追加全表重渲染。

   ```tsx
   export const EventCard = memo(function EventCard({ event, ... }: EventCardProps) { ... })
   ```

3. **传给 memo 组件的回调要稳定**。`onSelect={(id) => ...}` 内联箭头函数每次渲染都是新引用，直接让 `memo` 失效。用 `useCallback`，或者把回调放到 store 里让组件自己取。

4. **视频播放器组件必须隔离**。播放器挂载后不应因为父组件状态变化而重渲染 —— 重渲染可能导致播放中断。用 `memo` + 稳定 props，必要时用 `key` 明确控制何时重建。

5. **实时检测框与元数据叠加层采用 Canvas 2D 离屏渲染**。
   - 8 路视频在高帧率下每秒产生大量检测框，**严禁将高频 Bounding Box 存入 React `useState` 驱动 DOM 节点更新**（每秒数百上千次 DOM 样式变动会引发浏览器主线程严重卡顿）。
   - 在播放器上层覆盖透明 HTML5 `<canvas>`，数据存入 `useRef`，通过 `requestAnimationFrame` 读取与当前视频 PTS 匹配的检测框批量绘制，彻底脱离 React 组件树渲染循环。

---

## 视频播放器与流媒体渲染架构 (`<LivePlayer />`)

Argus 边缘监控台需要同时运行 1 大屏 (Hero Stage 72%) + 多小屏 (Bento Live Rail 28%)：

### 1. MSE 硬件解码引擎 (`mpegts.js`)
- 通过 `/api/v1/live/{cameraId}.flv?stream=main|sub&token={jwt}` 拉取 HTTP-FLV 流；
- 完整支持 **Enhanced FLV (H.265 / HEVC FourCC `hvc1`)** 与标准 H.264；
- 浏览器通过 GPU 硬件解码原生播放 4K/1080P H.265 画面，彻底消除 WebRTC 在主流浏览器（如 Chrome）下的 H.265 软解黑屏；
- 启用低延迟追帧（`liveBufferLatencyChasing: true`，`enableWorker: true`），延时控制在 100~200ms。

### 2. 双码流路由规则
- **Bento 辅助轨道**：固定使用 `stream="sub"`，拉取轻量子码流（720P/360P H.264），保持持续活跃低功耗监控；
- **Hero 主指挥舱**：默认拉取 `stream="main"`（支持 H.265 4K/1080P），支持主/子码流自由热切换与一键闭屏低功耗待机。

### 3. `<video>` 与 `mpegts.js` 播放器生命周期
- 必须在组件卸载或切换摄像头时调用 `player.destroy()` 释放 MSE SourceBuffer 与 Network Worker；
- 播放器加载后必须捕获 `player.play().catch(() => {})` 规避浏览器 Autoplay 策略阻塞。

**判断标准**：这个组件在事件流推送时会不会重渲染？如果它和事件无关却重渲染了，就是 bug。

---

## 列表渲染

- **`key` 用稳定业务 ID**，绝不用数组下标。事件列表是头部插入的，用下标做 key 会导致整表错位。
- 事件列表可能有上千条，**超过几百条要上虚拟滚动**（选型见待验证事项）。
- 列表容器和列表项分开：容器管数据和滚动，列表项是 memo 过的纯组件。

---

## shadcn/ui 使用约定

- **通过 `npx shadcn add <component>` 引入**，生成到 `src/components/ui/`。
- **不手改生成的文件**。需要项目定制就在外面包一层：

  ```tsx
  // src/components/DangerButton.tsx
  import { Button, type ButtonProps } from "@/components/ui/button"
  export function DangerButton(props: ButtonProps) {
    return <Button variant="destructive" {...props} />
  }
  ```

- 例外：调整 `ui/` 组件的 variant 定义（`cva` 配置）以匹配设计系统是允许的，但要在 PR 里说明，因为后续 `shadcn add` 更新会冲突。

---

## 磨砂玻璃组件模式

Argus 的核心容器组件（卡片、面板、导航栏）采用 Frosted Glassmorphism：

```tsx
// src/components/ui/frosted-card.tsx
interface FrostedCardProps {
  children: React.ReactNode
  className?: string
  /** 是否启用交互态光标膨胀效果 */
  interactive?: boolean
}

export function FrostedCard({ children, className, interactive = false }: FrostedCardProps) {
  return (
    <div className={cn(
      "frosted-glass rounded-2xl p-4",
      interactive && "reticle-target",
      className
    )}>
      {children}
    </div>
  )
}
```

规则：

- 所有 `frosted-glass` 类的组件必须接受 `className` prop 以支持外部覆盖。
- `reticle-target` 类用于标记可交互区域，Reticle 光标组件通过该类名检测 hover 态。
- frosted-glass 组件内部不再写 `border` / `shadow` / `backdrop-filter`，全部由 `.frosted-glass` 类承载。
- 暗色主题下 `--highlight-opacity: 0.05` 自动减弱内发光，无需额外处理。

---

## 主题切换

主题由根 `<html>` 元素的 `class="dark"` 控制。组件不直接处理主题逻辑。

```ts
// src/hooks/use-theme.ts
export function useTheme() {
  const theme = useUiStore((s) => s.theme)  // 'light' | 'dark'
  const setTheme = useUiStore((s) => s.setTheme)

  useEffect(() => {
    document.documentElement.classList.toggle('dark', theme === 'dark')
  }, [theme])

  return { theme, setTheme, toggleTheme: () => setTheme(theme === 'light' ? 'dark' : 'light') }
}
```

规则：

- 全局主题状态放 Zustand `uiStore`（`theme: 'light' | 'dark'`）。
- `useTheme` hook 提供 `theme`、`setTheme`、`toggleTheme`。
- 主题偏好持久化到 `localStorage`（key: `argus-theme`）。
- 首次访问跟随系统 `prefers-color-scheme`。
- **组件内禁止**：用 `dark:` Tailwind 变体写颜色（走 CSS 变量）；用 JS 读 `theme` 值来条件渲染不同颜色（走 CSS 变量）。

---

## WebGL 背景与组件层级

Three.js 拓扑背景固定在最底层，所有 UI 内容在其上层渲染。

| 层级 | z-index | 内容 |
|------|---------|------|
| Three.js Canvas | z-0 | 拓扑背景，`pointer-events: none`，`gl={{ alpha: true }}` |
| 校准网格 | z-10 | CSS 生成的 32px 发丝线网格，`pointer-events: none` |
| 主内容区 | z-20 | 所有页面内容 |
| Reticle 光标 | z-50 | `pointer-events: none`，跟随鼠标 |

规则：

- Three.js Canvas 必须设置 `pointer-events: none`，确保不阻挡用户交互。
- Three.js Canvas 必须设置 `gl={{ alpha: true }}`，允许 CSS 背景色透出。
- 亮色/暗色主题切换时，Three.js Canvas 的线框和粒子颜色联动变化（通过 CSS 变量或 uniform 传递）。

---

## i18n 文本处理

所有 UI 文本必须包裹在 i18n 函数中，禁止硬编码字符串：

```tsx
// ❌ 硬编码
<h1>实时预览</h1>

// ✅ i18n
import { useTranslation } from 'react-i18next'
const { t } = useTranslation()
<h1>{t('live.title')}</h1>
```

规则：

- 英文可能比中文长 2-3 倍，容器必须支持弹性布局（`flex` / `min-w-0`）。
- 翻译键用点分隔领域前缀：`camera.status.online`、`alarm.type.motion`。
- 不允许在 `t()` 调用外拼接字符串（会破坏翻译上下文）。

---

## 状态放哪

| 状态 | 放哪 |
|------|------|
| 只有这个组件用（展开/折叠、输入框） | `useState` |
| 父子几层之间共享 | props 传递 |
| 跨 feature 共享（当前选中的摄像头、连接状态） | Zustand store |
| 服务端数据（事件列表、摄像头配置） | 数据获取 hook，见 [state-management.md](./state-management.md) |
| 能从其它状态算出来 | **不存**，直接算或 `useMemo` |

**规则**：能用 `useState` 解决的不要上 store。全局 store 里每多一个字段，就多一处潜在的重渲染源。

---

## 副作用

- `useEffect` 只用于**真正的副作用**：订阅、定时器、命令式 DOM 操作。
- **不要用 `useEffect` 做派生状态**。`useEffect(() => setB(f(a)), [a])` 应该直接写成 `const b = f(a)`。
- 订阅类 effect **必须返回清理函数**。WebSocket、事件监听、`requestAnimationFrame` 忘记清理会在页面切换时泄漏。
- 依赖数组要完整。用 ESLint 的 `exhaustive-deps` 规则，不要靠注释关掉它。

---

## 可访问性底线

- 可点击元素用 `<button>`，不用 `<div onClick>`。
- 图标按钮必须有 `aria-label`。
- 表单控件必须关联 `<label>`。
- 焦点样式不允许被 `outline: none` 干掉（Tailwind 用 `focus-visible:` 变体）。

shadcn/ui 基于 Radix，大部分交互组件的 a11y 已经处理好了 —— **这也是优先用 shadcn 组件而非自己造的理由**。

---

## 键盘快捷键

ESC 和 Enter 是全局高频快捷键，所有交互组件必须正确处理。

### 全局快捷键表

| 按键 | 行为 | 适用场景 |
|------|------|---------|
| `ESC` | 关闭当前最顶层浮层 / 取消操作 / 退出全屏 | Modal、Dialog、Popover、Dropdown、全屏预览、编辑态 |
| `Enter` | 确认当前操作 / 提交表单 / 激活聚焦元素 | 表单提交、确认弹窗、按钮聚焦后触发 |
| `Tab` | 焦点在浮层内循环（Trap Focus） | Modal、Dialog 打开时 |

### ESC 规则

- **栈式管理**：多个浮层叠加时，ESC 只关闭最顶层（Modal 上有 Popover → 关 Popover，再按 ESC 关 Modal）。
- **不冒泡**：浮层内的 ESC 处理必须 `stopPropagation`，避免触发父组件的关闭逻辑。
- **回调可阻止**：ESC 可以被拦截（例如编辑未保存时弹"是否放弃"确认），但必须给用户明确反馈。
- **退出全屏**：全屏视频/地图预览按 ESC 退出，不依赖 UI 按钮。

### Enter 规则

- **表单提交**：聚焦在输入框内按 Enter 提交表单（`<form onSubmit>`），不依赖按钮点击。
- **按钮聚焦**：Button 获得焦点后按 Enter 触发 `onClick`（浏览器默认行为，无需额外处理）。
- **确认弹窗**：ESC 取消、Enter 确认，符合用户直觉。
- **搜索/过滤**：输入框内按 Enter 立即执行搜索，不延迟。

### 实现方式

```tsx
// ✅ 使用 useHotkeys hook 管理快捷键
import { useHotkeys } from '@/hooks/use-hotkeys'

function ImageViewer() {
  const [fullscreen, setFullscreen] = useState(false)

  useHotkeys('Escape', () => {
    if (fullscreen) setFullscreen(false)
    // 否则交给上层浮层处理
  }, { enabled: fullscreen })

  useHotkeys('Enter', () => {
    // 激活当前选中的摄像头
    activateSelected()
  })

  // ...
}
```

```ts
// src/hooks/use-hotkeys.ts
// 基于 addEventListener('keydown', ...) 实现
// 支持全局/局部模式、组合键（Ctrl/Cmd+S）、enabled 开关
// 不引入第三方库（如 hotkeys-js），保持轻量
```

规则：

- **用 `useHotkeys` hook**，不在组件里直接 `addEventListener('keydown')`（容易忘记清理）。
- **局部快捷键**：组件销毁时自动注销，不在全局留下残留监听。
- **组合键**：Ctrl/Cmd + S 保存等组合键通过 hook 参数声明，不手动判断 `event.metaKey`。
- **避免冲突**：不要覆盖浏览器原生快捷键（Ctrl+C/V/X/A 等），不要覆盖系统级快捷键。

---

## 禁止事项

- ❌ `export default` 组件
- ❌ `React.FC`
- ❌ 数组下标做 `key`
- ❌ 组件内直接 `fetch`
- ❌ 手改 `components/ui/` 生成物
- ❌ 内联箭头函数传给 `memo` 组件
- ❌ 用 `useEffect` 同步派生状态
- ❌ 订阅类 effect 不写清理函数
- ❌ `<div onClick>` 当按钮用
- ❌ 组件内硬编码暗色背景色（`bg-slate-900` 等，走 CSS 变量）
- ❌ 组件内硬编码 UI 文本字符串（必须走 i18n）
- ❌ Emoji 表情符号（走 Lucide 图标）

---

## 待验证事项

- [ ] 虚拟滚动选型（`@tanstack/virtual` vs 自己实现）
- [ ] 视频播放器实现方式（WebCodecs Canvas / MSE 播放器封装）
- [ ] Canvas 2D 叠加层与视频画面的 DPI 缩放适配（`devicePixelRatio`）
- [ ] 多路预览的布局方案（CSS Grid 自适应 vs 固定宫格）
- [ ] Three.js Canvas 颜色与 CSS 变量的联动方式（React context vs uniform 注入）
