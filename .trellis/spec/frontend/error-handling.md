# 前端错误处理与容灾规范

> 覆盖 React 渲染崩溃、异步未捕获异常、HTTP API 统一拦截与低延迟媒体流自愈。
> 核心原则：**渲染绝不白屏、接口统一信封解包、401 自动驱逐、网络波动有界自愈。**

---

## 四道防御分层架构

边缘计算无人值守场景下，前端必须具备强大的自愈能力与清晰的错误职责边界：

```text
┌────────────────────────────────────────────────────────┐
│ 第 1 道防线：React 渲染层崩溃兜底 (Global ErrorBoundary)│ ── 杜绝白屏，提供局部/全局 HUD 引擎重启
├────────────────────────────────────────────────────────┤
│ 第 2 道防线：未捕获异步异常 (window.unhandledrejection)│ ── 拦截脱离 React 生命周期的异步幽灵错误
├────────────────────────────────────────────────────────┤
│ 第 3 道防线：统一 HTTP API 客户端 (apiClient 拦截体系)  │ ── 统一解包信封、401 自动登出、错误分级
├────────────────────────────────────────────────────────┤
│ 第 4 道防线：低延迟媒体流与 WebSocket 链路自愈         │ ── WebRTC / WS 状态机感知与指数退避重连
└────────────────────────────────────────────────────────┘
```

---

## 1. 第一道防线：React 渲染异常与 ErrorBoundary

### 1.1 架构分级

1. **应用根级兜底 (`GlobalErrorBoundary`)**：
   - 挂载于 `web/src/main.tsx` 最外层；
   - 捕获整个组件树未拦截的致命渲染崩溃；
   - 展示科技感 HUD 故障面板（`CrashHUD`），包含错误堆栈摘要、系统时间戳、以及“重启控制台 / 刷新”恢复按钮。
2. **高危独立视口隔离 (Local ErrorBoundary)**：
   - 用于易受 GPU 显存耗尽、Context 丢失影响的独立渲染单元（如 `GargantuaCanvas`、视频分析 Canvas 2D 叠加层）；
   - 局部崩溃仅在视口内显示降级占位卡片，**严禁使外层控制台与导航树崩溃**。

### 1.2 标准实现模式 (`web/src/components/ErrorBoundary.tsx`)

```tsx
import React, { Component, type ErrorInfo, type ReactNode } from 'react'

interface Props {
  children: ReactNode
  fallback?: (error: Error, reset: () => void) => ReactNode
}

interface State {
  hasError: boolean
  error: Error | null
}

export class ErrorBoundary extends Component<Props, State> {
  public state: State = {
    hasError: false,
    error: null,
  }

  public static getDerivedStateFromError(error: Error): State {
    return { hasError: true, error }
  }

  public componentDidCatch(error: Error, errorInfo: ErrorInfo) {
    console.error('[CRITICAL UI RENDER CRASH]:', error, errorInfo)
  }

  public reset = () => {
    this.setState({ hasError: false, error: null })
  }

  public render() {
    if (this.state.hasError && this.state.error) {
      if (this.props.fallback) {
        return this.props.fallback(this.state.error, this.reset)
      }
      return (
        <div className="lens-glass flex min-h-[320px] w-full flex-col items-center justify-center rounded-3xl p-8 text-center">
          <div className="font-mono text-xs font-bold tracking-widest text-rose-500 uppercase">
            SYSTEM CRITICAL // RENDER CRASH
          </div>
          <p className="mt-2 text-sm text-[var(--text-secondary)]">{this.state.error.message}</p>
          <button
            onClick={this.reset}
            className="mt-6 rounded-xl bg-[var(--accent)] px-4 py-2 font-mono text-xs text-white"
          >
            REBOOT VIEWPORT // 重启视口
          </button>
        </div>
      )
    }
    return this.props.children
  }
}
```

---

## 2. 第二道防线：未捕获异步异常监听

在应用启动入口（`web/src/main.tsx`）监听全局未捕获 Promise 与运行时异常，避免静默失败：

```ts
// 捕获未处理的 Promise rejection (如异步请求未 catch)
window.addEventListener('unhandledrejection', (event) => {
  console.error('[UNHANDLED PROMISE REJECTION]', event.reason)
  // 屏蔽浏览器控制台默认红标噪音（根据需要），并分流至应用遥测日志
})

// 捕获资源加载失败或全局脚本执行错误
window.addEventListener('error', (event) => {
  console.error('[GLOBAL RUNTIME ERROR]', event.message, event.filename, event.lineno)
})
```

---

## 3. 第三道防线：统一 HTTP API 客户端 (`apiClient`)

### 3.1 后端统一响应信封与前端强类型契约

后端在 `crates/api/src/response.rs` 约定统一信封格式，前端在 `web/src/types/api.ts` 中完全对齐：

```ts
// 统一后端响应信封 (CamelCase)
export interface ApiResponse<T> {
  code: number
  message: string
  data: T
  timestamp: number // 13 位 UTC Unix 毫秒
}

// 统一分页列表信封
export interface PageData<T> {
  items: T[]
  total: number
  hasMore: boolean
}
```

### 3.2 强类型业务错误定义 (`ApiError`)

```ts
export class ApiError extends Error {
  public readonly code: number
  public readonly timestamp?: number

  constructor(code: number, message: string, timestamp?: number) {
    super(message)
    this.name = 'ApiError'
    this.code = code
    this.timestamp = timestamp
  }
}
```

### 3.3 核心拦截逻辑规范 (`web/src/lib/api.ts`)

`apiClient` 基于原生 `fetch` 封装，提供统一拦截处理：

```ts
import { useAuthStore } from '../stores/auth'
import type { ApiResponse } from '../types/api'

interface RequestOptions extends RequestInit {
  silentToast?: boolean // 是否静默错误提示（由调用方在局部自行接管）
}

export async function request<T>(endpoint: string, options: RequestOptions = {}): Promise<T> {
  const { silentToast = false, headers, ...rest } = options
  const token = useAuthStore.getState().token

  const reqHeaders: HeadersInit = {
    'Content-Type': 'application/json',
    ...(token ? { Authorization: `Bearer ${token}` } : {}),
    ...headers,
  }

  let res: Response
  try {
    res = await fetch(endpoint, { credentials: 'same-origin', headers: reqHeaders, ...rest })
  } catch (netErr) {
    // 1. 网络断开 / DNS 失败 / 后端离线
    const message = '网络连接中断或边缘网关无响应'
    if (!silentToast) {
      // 触发全局轻量 HUD Toast 提示
    }
    throw new ApiError(-1, message)
  }

  // 2. 拦截 401 Unauthorized（Token 过期或凭证被销毁）
  if (res.status === 401) {
    useAuthStore.getState().logout()
    throw new ApiError(401, '登录会话已过期，请重新验证身份')
  }

  // 3. HTTP 传输级非 2xx 报错
  if (!res.ok) {
    const errorMsg = `HTTP Error ${res.status}: ${res.statusText}`
    if (!silentToast) {
      // 触发全局轻量 HUD Toast 提示
    }
    throw new ApiError(res.status, errorMsg)
  }

  // 4. 解析统一 JSON 信封
  const json: ApiResponse<T> = await res.json()

  // 5. 业务级状态码非 0 校验
  if (json.code !== 0) {
    if (!silentToast) {
      // 触发全局轻量 HUD Toast 提示
    }
    throw new ApiError(json.code, json.message, json.timestamp)
  }

  return json.data
}
```

### 3.4 消费方规范（何时局部处理？何时静默？）

- **常规读请求（如获取告警列表、拉取摄像头列表）**：
  直接 `const cameras = await request<Camera[]>('/api/v1/cameras')`，无需编写冗长样板 `try/catch`，若出错由全局机制统一弹出 Toast，列表保持空状态。
- **精确表单提交（如创建分析任务、修改密码）**：
  传参 `{ silentToast: true }`，在页面局部 `try { ... } catch (err) { if (err instanceof ApiError) { ... } }`，将错误精准映射到具体表单字段红字，提供极致交互体验。

---

## 4. 第四道防线：低延迟媒体流与 WebSocket 自愈

### 4.1 WebSocket 重连规范 (`/api/v1/ws/events`)

- **退避策略**：采用带抖动的指数退避算法（Exponential Backoff with Jitter），初始 1s，倍乘 1.5，最大封顶 30s。
- **状态感知**：Zustand store 记录 `wsStatus: 'connecting' | 'connected' | 'disconnected'`。
- **页面感知**：界面顶部展示微型通信状态指示灯（琥珀色闪烁 = 正在自动重连，绿色 = 通畅）。

### 4.2 WebRTC (WHEP) 视频流自愈规范

- **状态监听**：监听 `RTCPeerConnection.onconnectionstatechange`；
- **自愈行为**：
  - 当状态为 `disconnected` 时：等待 3s，若未恢复则标记流断开；
  - 当状态为 `failed` 时：释放当前 PeerConnection，重新发起 WHEP SDP Offer 请求建立新通道；
  - 界面呈现优雅的信号中断条纹占位与重连倒计时。

---

## 检查清单

在代码审查或新增功能时确认：

- [ ] 根应用与高危视觉视口是否已包裹 `ErrorBoundary`？
- [ ] 所有 HTTP API 请求是否统一通过 `apiClient.ts` 发起？
- [ ] 401 响应是否能自动清空状态并平滑切回登录页面，无残留 Token？
- [ ] 页面中的高频长连接（WS / WebRTC）是否包含明确的有界重连与清理逻辑？
