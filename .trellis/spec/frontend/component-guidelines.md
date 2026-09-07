# 组件规范 (Component Guidelines)

> React 18/19 + TypeScript + shadcn/ui。
> 核心约束：**同时运行多路视频与高频事件流，不必要的重渲染与 DOM 样式变动是致命的性能瓶颈**。

---

## 1. 组件声明与接口规范

- **具名导出**：统一使用函数声明并具名导出（`export function MyComponent(...)`），**严禁 `export default`**；
- **禁止 `React.FC`**：直接对参数定义 `<ComponentName>Props` 类型，可选属性在参数解构时赋予默认值；
- **展示与容器分离**：`features/*/components/` 下的组件默认为纯展示组件，数据请求收敛于 `hooks/` 或页面容器；
- **容器与交互标记**：卡片面板统一样式类 `.frosted-glass`，可交互目标附加 `.reticle-target` 标记光标交互态。

```tsx
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
      className={cn("frosted-glass rounded-2xl p-3 text-left reticle-target", selected && "border-accent/40 bg-accent-soft")}
    >
      {/* 内容 */}
    </button>
  )
}
```

---

## 2. 边缘监控重渲染隔离铁律 (Argus Invariants)

多路视频（如 8 路）并发运行时，任何全局不当状态更新都会导致播放器撕裂或严重丢帧：

1. **细粒度 Zustand 订阅**：禁止无选择器订阅整个 store，必须提取细粒度属性（`useCameraStore(s => s.statuses[id])`），见 [state-management.md](./state-management.md)；
2. **高频检测框脱离 React 渲染树**：
   - **严禁将高频（15~30fps）检测框存入 React `useState` 驱动 DOM 重排**；
   - 必须在上层覆盖透明 HTML5 `<canvas>`，将检测框数据存入 `useRef`，通过 `requestAnimationFrame` 驱动 Canvas 2D 批量绘制与插值，彻底隔绝 React 响应式生命周期；
3. **视频播放器隔离保护**：`<LivePlayer />` 必须由 `React.memo` 保护并传入稳定 props，严禁因外层与事件无关的状态变动触发播放器重建；
4. **列表项强制 `React.memo`**：高频事件列表项必须用 `memo` 包装，传入的回调必须通过 `useCallback` 稳定化或直接在组件内自取。

---

## 3. 流媒体播放器架构 (`<LivePlayer />`)

- **MSE 硬件解码引擎 (`mpegts.js`)**：
  - 通过 `/api/v1/live/{cameraId}.flv?stream=main|sub&token={jwt}` 拉取 HTTP-FLV 流；
  - 完整支持 **Enhanced FLV (H.265 / HEVC FourCC `hvc1`)** 与标准 H.264，由浏览器 GPU 硬解播放 4K/1080P，避免 WebRTC 软解黑屏；
  - 开启低延迟追帧（`liveBufferLatencyChasing: true`，`enableWorker: true`），延迟控制在 100~200ms；
- **双码流分工**：Bento 辅轨固定拉取 `stream="sub"`（轻量子码流保持全天候低功耗），Hero 主屏默认 `stream="main"` 并支持一键切流；
- **严格生命周期清理**：组件卸载或切换摄像头时，必须执行 `player.destroy()` 释放 MSE SourceBuffer 与 Network Worker。

---

## 4. shadcn/ui 与 UI 基础设施

- **不修改生成源码**：通过 CLI 引入至 `src/components/ui/` 的组件不直接改动，如需定制通过二次封装包装；
- **国际化 (i18n)**：所有可见文本必须使用 `react-i18next` 的 `t()` 转译，禁止硬编码中文；容器必须支持弹性拉伸，容纳英文 2~3 倍字符膨胀；
- **语义化交互与 a11y**：可点击控件必须用 `<button>`（严禁 `<div onClick>`），图标按钮必须具备 `aria-label`；
- **键盘快捷键**：全局浮层、弹窗统一响应 `ESC` 栈式关闭并 `stopPropagation`；表单与确认弹窗响应 `Enter` 触发提交。

---

## 5. 禁止事项 (Iron Rules)

- ❌ 使用 `export default` 导出组件
- ❌ 使用 `React.FC` 定义组件
- ❌ 使用数组下标 `index` 作为动态列表的 `key`（必须使用实体稳定业务 ID）
- ❌ 将 15/30fps 高频推理检测框存入 React `useState` 驱动 DOM 节点更新（必须使用透明 Canvas + `useRef`）
- ❌ 在展示组件内部直接调用 `fetch` 或 `axios`（必须收敛在 hooks/API client）
- ❌ 订阅类 `useEffect` 忘记返回清理函数（引发内存与事件监听器泄漏）
- ❌ 在组件中直接编写硬编码十六进制颜色或硬编码 UI 文本
- ❌ 在生产界面中使用任何 Emoji 表情符号
