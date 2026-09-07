# 前端错误处理与容灾规范 (Error Handling Guidelines)

> 覆盖 React 渲染崩溃、异步异常、HTTP API 拦截与实时流媒体链路自愈。
> 核心原则：**渲染绝不白屏、接口统一解包、401 自动驱逐、网络波动有界自愈**。

---

## 1. 四道防御分层架构

```text
┌────────────────────────────────────────────────────────┐
│ 第 1 道防线：React 渲染层崩溃兜底 (Global & Local ErrorBoundary)│ ── 杜绝白屏，局部崩溃视口隔离
├────────────────────────────────────────────────────────┤
│ 第 2 道防线：未捕获异步异常监听 (window.unhandledrejection)    │ ── 拦截脱离 React 生命周期的异步错误
├────────────────────────────────────────────────────────┤
│ 第 3 道防线：统一 HTTP API 客户端拦截体系 (apiClient.ts)      │ ── 统一解包信封、401 登出、错误分级
├────────────────────────────────────────────────────────┤
│ 第 4 道防线：媒体流与 WebSocket 链路自愈机制                  │ ── 状态机感知与指数退避有界重连
└────────────────────────────────────────────────────────┘
```

---

## 2. React 渲染异常与 ErrorBoundary 隔离

1. **应用根级兜底 (`GlobalErrorBoundary`)**：
   - 挂载于 `web/src/main.tsx` 最顶层，捕获整树未拦截的致命崩溃；
   - 渲染 HUD 风格故障面板，展示错误摘要并提供“重启控制台”恢复按钮；
2. **局部高危视口隔离 (`Local ErrorBoundary`)**：
   - **高危渲染单元隔离**：视频 Canvas 2D 检测框层、Three.js 背景等容易发生 Context 丢失的组件必须单独包裹局部 ErrorBoundary；
   - **隔离边界契约**：局部渲染崩溃仅在对应视口内展示降级重试占位卡片，**严禁使外层控制台与导航树级联崩溃**。

---

## 3. 全局未捕获异步异常监听

在 `web/src/main.tsx` 入口处统一监听全局异步未捕获异常，防止脱离 React 生命周期的 Promise 错误导致静默故障：

```ts
window.addEventListener('unhandledrejection', (event) => {
  console.error('[UNHANDLED PROMISE REJECTION]', event.reason);
});

window.addEventListener('error', (event) => {
  console.error('[GLOBAL RUNTIME ERROR]', event.message, event.filename, event.lineno);
});
```

---

## 4. 统一 HTTP API 客户端拦截体系 (`apiClient`)

所有网络请求必须经由 `src/lib/api.ts` 发起，严格执行统一解包与错误分类：

### 4.1 强类型业务错误定义
```ts
export class ApiError extends Error {
  constructor(
    public readonly code: number,
    message: string,
    public readonly timestamp?: number,
  ) {
    super(message);
    this.name = 'ApiError';
  }
}
```

### 4.2 拦截流水线规则
1. **自动附带凭证**：自动从 `useAuthStore` 提取 Token 注入 `Authorization: Bearer <token>`；
2. **网络离线拦截**：捕获 `fetch` 底层异常，网络断开时抛出 `ApiError(-1, "网络连接中断或边缘网关无响应")`；
3. **401 强制登出**：HTTP 状态码为 `401` 时，立即调用 `useAuthStore.getState().logout()` 清除凭据并跳转登录页；
4. **信封解包与业务校验**：
   - 必须校验 `json.code === 0`，成功时直接返回 `json.data`（消除调用方逐层解构负担）；
   - 若 `json.code !== 0`，自动抛出 `new ApiError(json.code, json.message, json.timestamp)`；
5. **静默控制 (`silentToast`)**：
   - 常规读取请求默认弹出全局轻量 Toast，调用方无需写样板 `try/catch`；
   - 表单提交支持传入 `{ silentToast: true }`，由调用方在局部 `catch (err)` 将错误精准投递到具体输入框红字。

---

## 5. 实时媒体流与 WebSocket 链路自愈

### 5.1 WebSocket 重连规范 (`/api/v1/ws/events`)
- **指数退避重试**：初始间隔 1s，倍乘 1.5，最大重连间隔封顶 30s，附加微小随机抖动（Jitter）；
- **状态感知指示**：Zustand 维护 `wsStatus: 'connecting' | 'connected' | 'disconnected'`，页面顶部显示微型状态指示灯（琥珀色重连，绿色正常）。

### 5.2 视频播放器与流媒体自愈
- **连接状态监听**：针对 MSE 播放器与 WebRTC 监听底层状态回调；
- **自愈行为**：
  - 发生解码卡死或丢包断流时，等待 3s 容错；若未恢复，自动销毁旧实例并重建播放流；
  - 断流期间播放器窗口呈现优雅的信号中断条纹占位与重连倒计时，禁止黑屏无响应。

---

## 6. 禁止事项 (Iron Rules)

- ❌ 在业务组件内直接使用原生裸 `fetch`（必须经由 `apiClient` 统一拦截）
- ❌ 发生 401 鉴权失效后仍保留已失效的 Token
- ❌ 高危独立视口未包裹局部 `ErrorBoundary` 导致整页白屏
- ❌ 无上限的 WebSocket 狂暴重连（必须设置最大 30s 封顶的指数退避）
- ❌ 在异步请求的 `catch` 块中静默吞掉致命异常
