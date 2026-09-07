# 状态管理

> Zustand 管**客户端状态**。服务端数据是另一类东西，不要混在一起。

> ⚠️ **状态：立项约定（尚未经代码验证）**
> 服务端状态方案尚未选定，见文末待验证事项。首批 store 落地后需回填真实示例并删除本提示。

---

## 两类状态，两套处理

| | 客户端状态 | 服务端状态 |
|---|---|---|
| 例子 | 当前选中的摄像头、布局宫格数、筛选条件、侧边栏展开 | 事件列表、摄像头配置、录像索引 |
| 特点 | 只存在于本次会话，刷新即丢 | 有远端真相，需要缓存、失效、重取、加载态 |
| 用什么 | **Zustand** | **数据获取 hook**（见下） |

**最常见的错误是把服务端数据塞进 Zustand store**，然后手写一堆 `loading` / `error` / `refetch` 字段。那是在重新实现一个缓存库，且大概率实现得不对。

---

## Zustand store 约定

```ts
// src/stores/uiStore.ts
interface UiState {
  selectedCameraId: string | null
  gridSize: 1 | 4 | 9
  // action 与 state 放在同一个 store 里
  selectCamera: (id: string | null) => void
  setGridSize: (n: UiState["gridSize"]) => void
}

export const useUiStore = create<UiState>()((set) => ({
  selectedCameraId: null,
  gridSize: 4,
  selectCamera: (id) => set({ selectedCameraId: id }),
  setGridSize: (n) => set({ gridSize: n }),
}))
```

规则：

- **按领域切分 store，不做单一大 store**。全局的放 `src/stores/`，只服务一个 feature 的放 `src/features/<name>/store.ts`。
- **action 定义在 store 内部**，组件不直接调 `set`。组件里出现 `useUiStore.setState(...)` 是绕过封装。
- **store 里不放派生数据**。能从其它字段算出来的在选择器里算，或用 `useMemo`。
- **store 里不放服务端数据**。

---

## 选择器：订阅粒度决定性能

这条在 Heimdall 上尤其重要 —— 实时预览页有 8 路播放器同时挂载。

```ts
// ❌ 订阅整个 store：任何字段变化都让这个组件重渲染
const store = useUiStore()

// ❌ 每次返回新对象：即使值没变也会重渲染
const { a, b } = useUiStore((s) => ({ a: s.a, b: s.b }))

// ✅ 取单个原始值
const gridSize = useUiStore((s) => s.gridSize)

// ✅ 需要多个值就分别取
const a = useUiStore((s) => s.a)
const b = useUiStore((s) => s.b)

// ✅ 确需返回对象时用 useShallow
const { a, b } = useUiStore(useShallow((s) => ({ a: s.a, b: s.b })))
```

**规则**：选择器返回原始值。返回对象或数组必须配 `useShallow`，否则每次渲染都是新引用，`memo` 全部失效。

---

## 实时数据（WebSocket）

事件流从 WebSocket 推送过来。它既不是纯客户端状态，也不是请求-响应式的服务端状态。

约定：

- **WebSocket 连接生命周期由一个专门的 store 或 provider 管理**，全局只建一条连接。不要每个组件自己 `new WebSocket`。
- **连接状态（connecting / open / closed / reconnecting）放 Zustand**，UI 据此显示连接指示。
- **推送进来的事件写入有容量上限的固定缓冲**。事件是持续到来的，无上界数组会让长时间挂载的监控页面内存持续增长。

  ```ts
  // src/stores/eventStreamStore.ts
  // 简易有界队列示例（生产环境极高频推送场景可封装基于固定数组+游标的 RingBuffer 避免浅拷贝）
  const MAX_BUFFERED = 500
  appendEvent: (ev) => set((s) => ({
    recent: [ev, ...s.recent.slice(0, MAX_BUFFERED - 1)],
  })),
  ```

- **重连必须有指数退避**，与后端约定一致，见 [../backend/api-guidelines.md](../backend/api-guidelines.md)。
- 历史事件通过分页接口拉取，**实时推送只负责增量**。两者的合并逻辑要写清楚，避免重复条目（按事件 ID 去重）。

---

## 服务端数据获取

**当前未引入数据获取库**。在选型确定前，约定如下：

- 每个资源写一个 hook，放在 feature 的 `hooks/` 下：`useEvents()`、`useCameras()`。
- hook 统一返回 `{ data, loading, error, refetch }` 形状，保证后续换成库时改动集中。
- 请求函数集中在 `src/lib/api/`，**组件和 hook 都不直接写 `fetch` URL**。
- **必须处理组件卸载后的响应**（用 `AbortController`），否则会有"卸载后 setState"警告和潜在泄漏。

```ts
// src/lib/api/events.ts
export async function fetchEvents(q: ListEventsQuery, signal?: AbortSignal): Promise<EventDto[]> { ... }
```

这是过渡方案。如果发现在手写缓存、去重、失效逻辑，说明该引入库了 —— 那时按待验证事项决策，不要继续加码手写。

---

## 持久化

需要跨刷新保留的（布局宫格数、筛选偏好）用 Zustand 的 `persist` 中间件，存 `localStorage`。

**规则**：只持久化 UI 偏好。**不要持久化服务端数据快照** —— 恢复出来的是过期数据，比没有更糟。

---

## 禁止事项

- ❌ 服务端数据存进 Zustand store
- ❌ 订阅整个 store（`useStore()` 不带选择器）
- ❌ 选择器返回新对象却不用 `useShallow`
- ❌ 组件里调 `useStore.setState()`
- ❌ 多个组件各自建 WebSocket 连接
- ❌ 无上界的事件数组
- ❌ store 里存能算出来的派生值
- ❌ 持久化服务端数据

---

## 待验证事项

- [ ] **服务端状态方案选型**：手写 hook 够不够，还是引入 TanStack Query。判断依据是是否出现缓存失效、请求去重、后台刷新这类需求
- [ ] 事件列表的实时推送与分页历史的合并策略（游标对齐方式）
- [ ] 是否需要离线/断网状态下的 UI 降级
