# 自定义 Hook 规范

> hook 是前端逻辑复用的主要手段。**组件里出现复杂逻辑就该抽 hook。**

> ⚠️ **状态：立项约定（尚未经代码验证）**
> 首批 hook 落地后需回填真实示例，并删除本提示。

---

## 什么时候抽 hook

| 信号 | 动作 |
|------|------|
| 组件里有超过一个 `useEffect` | 按职责各抽一个 hook |
| 同样的 `useState` + `useEffect` 组合出现第二次 | 抽成共享 hook |
| 组件顶部有十几行 hook 调用才到 JSX | 抽 hook 把逻辑收拢 |
| 逻辑需要单独测试 | 抽 hook（hook 比组件好测） |

**不要为了抽而抽**：只被一个组件用、只有三行的逻辑留在组件里更好读。

---

## 命名与放置

| 规则 | 说明 |
|------|------|
| `use` 前缀 | 强制。ESLint 的 hook 规则依赖这个前缀 |
| 名字表达"提供什么" | `useEventStream`、`useCameraStatus`，不是 `useEventLogic` |
| 只被一个 feature 用 | `src/features/<name>/hooks/` |
| 跨 feature 复用 | `src/hooks/` |
| 一个文件一个 hook | 文件名 = hook 名 |

**避免**：`useData`、`useHelper`、`useUtils` 这类无信息量的名字。

---

## 返回值形状

```ts
// 返回多个相关值 → 对象（调用方可按需解构、顺序无关）
export function useEvents(query: ListEventsQuery) {
  return { data, loading, error, refetch }
}

// 返回 [值, 设值函数] 这种明确成对的 → 元组（对齐 useState 直觉）
export function useToggle(initial = false): [boolean, () => void] {
  ...
}
```

规则：

- **数据获取类 hook 统一返回 `{ data, loading, error, refetch }`**，见 [state-management.md](./state-management.md)。形状一致才能在换实现时集中改动。
- **返回的函数必须用 `useCallback` 包**，否则调用方无法把它安全地放进依赖数组或传给 `memo` 组件。
- **返回的对象/数组如果会进依赖数组，要用 `useMemo` 稳定引用**。

---

## 清理是必须的

Heimdall 前端有大量长连接和定时器（WebSocket、播放器、状态轮询）。忘记清理会在页面切换时泄漏，长时间运行的看板会越来越卡。

```ts
export function useEventStream(cameraId: string | null) {
  useEffect(() => {
    if (!cameraId) return

    const ctrl = new AbortController()
    const unsubscribe = eventBus.subscribe(cameraId, handler)

    return () => {
      unsubscribe()
      ctrl.abort()
    }
  }, [cameraId])
}
```

**硬性规则**：任何建立订阅、定时器、连接、`requestAnimationFrame` 的 hook **必须返回清理函数**。这在 code review 里是必查项。

---

## 依赖数组

- 依赖数组必须完整，开启 ESLint `react-hooks/exhaustive-deps`。
- **不允许用 `// eslint-disable-next-line` 关掉它**。想关说明依赖设计有问题：要么用 `useCallback` 稳定函数，要么用 ref 存不该触发重跑的值。
- 对象/数组作为依赖时注意引用稳定性。传进来的 props 如果是内联对象，effect 会每次都重跑。

---

## hook 里不做的事

- ❌ **不直接渲染**。hook 返回数据和函数，JSX 是组件的事。
- ❌ **不写死 URL**。请求走 `src/lib/api/`。
- ❌ **不在 hook 里做条件调用**。`if (x) useEffect(...)` 违反 hook 规则。条件写在 effect 内部。
- ❌ **不吞错误**。catch 之后要把错误暴露到返回值里，别只 `console.error`。

---

## 常见 Heimdall hook 清单（预期）

落地时按需实现，命名先约定好，避免同一件事出现两个名字：

| hook | 职责 |
|------|------|
| `useEvents(query)` | 分页拉取历史事件 |
| `useEventStream(cameraId?)` | 订阅 WebSocket 实时事件 |
| `useCameras()` | 摄像头列表与状态 |
| `useConnectionStatus()` | WebSocket 连接状态 |
| `useVideoPlayer(ref, source)` | 播放器实例生命周期 |
| `useTheme()` | 主题切换（light/dark），读写 localStorage，跟随系统偏好 |
| `useLocale()` | 语言切换（zh-CN/zh-TW/en），读写 localStorage，触发 i18n 模块按需加载 |
| `useHotkeys(keys, handler)` | 全局/局部键盘快捷键注册，ESC 关闭、Enter 确认等 |

---

## 禁止事项

- ❌ 订阅/定时器类 hook 不写清理函数
- ❌ 关闭 `exhaustive-deps` 检查
- ❌ 返回未 `useCallback` 包裹的函数
- ❌ hook 里返回 JSX
- ❌ 条件式调用 hook
- ❌ 无信息量的 hook 命名

---

## 待验证事项

- [ ] `useVideoPlayer` 的实现取决于流方案选型，见 [../backend/api-guidelines.md](../backend/api-guidelines.md)
- [ ] 是否需要一个统一的 `useApi` 基础 hook 承载重试与错误处理
