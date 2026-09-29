# 状态管理

## 状态归属

| 状态                                   | 归属                           |
| -------------------------------------- | ------------------------------ |
| 选中摄像头、布局、筛选、侧栏等 UI 状态 | 领域 Zustand store             |
| 摄像头配置、历史事件、任务等服务端资源 | 资源数据获取 hook              |
| 业务 WS 连接状态和实时增量             | 全局连接管理 + 有界缓冲        |
| 高频检测元数据/播放器实例              | ref、Worker 或 Canvas 生命周期 |

普通服务端资源不进入 Zustand，不在 store 重新实现请求缓存系统。

## Store 与选择器

- 按领域拆 store；action 和 state 同放，组件调用 action，不直接 `setState`。
- 不存可计算的派生状态；全局 store 放 `src/stores/`，私有 UI 状态留在 feature。
- 使用细粒度选择器；返回对象/数组时配 `useShallow`，不订阅整个 store。

```ts
const selectedId = useUiStore((s) => s.selectedCameraId)
const selection = useUiStore(useShallow((s) => ({ id: s.selectedCameraId, size: s.gridSize })))
```

## 业务事件连接

- 全局共享一条业务事件 WebSocket；独立媒体传输连接遵循播放器生命周期。
- 连接状态用同一状态机表达；重连退避有上限，见 [错误处理](error-handling.md#连接恢复)。
- 实时事件按条数或字节设固定上限，溢出丢旧，禁止持续 append 到无界数组。
- WS 只负责增量，历史用分页请求；合并按事件 ID 去重，保持游标和排序一致。

## 服务端资源与持久化

- 请求集中在 [lib/api.ts](../../../../web/src/lib/api.ts)，资源 hook 暴露数据、加载、错误、重取行为。
- 当前未引入服务端缓存库；出现复杂缓存失效、去重或后台刷新需求时单独评估，不向 Zustand 堆服务端快照。
- 卸载/参数变化时取消请求并防迟到响应，详见 [Hook 规范](hook-guidelines.md)。
- `persist/localStorage` 仅持久化布局、筛选等 UI 偏好，不持久化服务端数据快照。
- 实时页面检查一条无关事件不会引发播放器或整页重渲染，并验证缓冲长期运行不增长。

### 认证凭据的存储边界

凭据（Bearer Token）**只允许写入 `sessionStorage`**，见 [stores/auth.ts](../../../../web/src/stores/auth.ts)：

- `localStorage` 对同源脚本永久可读，而该凭据本身只有数小时至一天级的寿命。把长效可读的副本留在那里，收益（关标签页后免登录）与代价（任意一次 XSS 即可外带并异地重放）不对称。关闭标签页即失效是这里愿意付的代价。
- 凭据过期判定**只用服务端返回的 `expiresAt`**，不在前端重复推算 TTL（后端 `auth.token_ttl_hours` 可配，前端任何硬编码时长都是重复真相源）。
- `localStorage` 在此只保留两类非敏感数据：「记住我」的操作员账号名，以及 UI 偏好。
- 版本演进遗留的持久化凭据必须在 store 初始化时**主动回收**，不能只靠「新代码不再写」：旧副本仍会被后续请求携带，等于绕过收敛。同时清理 `heimdall-token-expires-at` 等新增键的旧位置。
- 会话恢复时必须校验 `expiresAt`：进程重启后 `sessionStorage` 仍在，直接带着已过期凭据进入工作区只会先吃一次 401 再被踢出。
- 「记住我」**默认不勾选**：默认勾选意味着共享终端上任何一次登录都会把操作员 ID 留在下一次打开的页面里。登录表单的其它交互约束见 [API 规范](../../api/backend/api-guidelines.md#登录页的交互契约)。
- 登出只清凭据，**保留**「记住我」的账号名（下次登录预填）；只有登录时未勾选「记住我」才清除它 —— 两者是独立意图。
- store 中的 `expiresAt` 参与会话判定，因此新增登录路径必须把服务端返回的 `expiresAt` 一路传到 `login()`，不得只存 token。
