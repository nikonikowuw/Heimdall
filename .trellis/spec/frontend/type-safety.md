# 前端类型安全规范 (Type Safety Guidelines)

> 基于 TypeScript 严格模式。核心目标：**前端类型与 Rust 后端 DTO 绝对对齐，消灭运行时隐式类型崩溃**。

---

## 1. tsconfig 编译基线

必须开启以下核心规则，严禁因编码繁琐而关闭：

```jsonc
{
  "compilerOptions": {
    "strict": true,
    "noUncheckedIndexedAccess": true,      // 数组下标索引 arr[0] 推导为 T | undefined，防御越界
    "noUnusedLocals": true,
    "noUnusedParameters": true,
    "noFallthroughCasesInSwitch": true,
    "verbatimModuleSyntax": true
  }
}
```

---

## 2. 与后端 DTO 对齐铁律

后端所有 DTO 集中定义于 `src/types/api.ts`，字段名采用 camelCase，与后端 `#[serde(rename_all = "camelCase")]` 逐字对齐：

### 2.1 统一信封与分页契约
```ts
// src/types/api.ts
export interface ApiResponse<T> {
  code: number        // 0 = 成功，非 0 = 业务错误码
  message: string     // 成功为 "success"，错误为人类可读提示
  data: T | null      // 成功返回负载，错误固定为 null
  timestamp: number   // 13 位 UTC Unix 毫秒
}

export interface PaginatedData<T> {
  items: T[]
  total: number
  page: number
  pageSize: number
  totalPages: number
}

export interface CursorParams {
  limit?: number      // 默认 50，上限 500
  before?: number     // 游标时间戳（13 位 UTC 毫秒）
  cameraId?: string
}
```

### 2.2 字段映射规范
- **时间戳必须是 `number`**：DTO 中的时间字段必须显式声明为 `number`（13 位 UTC Unix 毫秒），**严禁声明为 `string` 或 `Date`**；
- **空值映射**：后端 `Option<T>` 在 JSON 中序列化为 `null`，前端 DTO 必须声明为 `T | null`，不得声明为 `T | undefined`；
- **坐标归一化**：目标检测框坐标必须使用归一化区间 `[0.0, 1.0]`：
  ```ts
  export interface BoundingBox {
    x: number; y: number; width: number; height: number;
  }
  ```

---

## 3. 网络边界与类型断言收敛

- **断言隔离**：`as` 类型断言**仅允许出现在 `src/lib/api/` 的网络请求封装中**，且必须附带来源注释。**严禁在组件、hooks 或业务工具函数中使用 `as SomeType` 断言逃逸**；
- **严禁 `any`**：禁止在代码中出现 `any`。对于未定类型必须使用 `unknown`，并通过自定义类型守卫（Type Guard）收窄后使用。

---

## 4. 可辨识联合与编译期穷尽检查

- **WebSocket 消息分发**：以 `type` 字段驱动，`switch` 语句中**必须包含 `never` 穷尽检查**，确保后端新增消息事件类型时前端直接在编译期报错阻断：
  ```ts
  export type WsMessage =
    | { type: "alarm.reported"; payload: AlarmDto }
    | { type: "capture.reported"; payload: CaptureDto }
    | { type: "task.reconciled"; payload: { taskId: string; status: string } }
    | { type: "stream.lagged"; payload: { skipped: number } }

  function dispatchWs(msg: WsMessage) {
    switch (msg.type) {
      case "alarm.reported":   return handleAlarm(msg.payload)
      case "capture.reported": return handleCapture(msg.payload)
      case "task.reconciled":  return handleTask(msg.payload)
      case "stream.lagged":    return handleLagged(msg.payload)
      default: {
        const _exhaustive: never = msg // 后端新增类型未处理时编译失败
        return _exhaustive
      }
    }
  }
  ```
- **异步状态管理**：使用显式互斥联合类型 `AsyncState<T>`，禁止用多个 `boolean` 标志组合（如 `{ loading: true, data: ... }`）引发非法状态：
  ```ts
  type AsyncState<T> =
    | { status: "idle" }
    | { status: "loading" }
    | { status: "success"; data: T }
    | { status: "error"; error: string }
  ```

---

## 5. 时间格式化规范

- **严禁手动截取**：严禁写 `new Date(ts).toISOString().slice(0, 10)` 等跨浏览器兼容性极差的手动字符串切片；
- **统一国际化格式化**：统一使用 `Intl.DateTimeFormat` 或通过 `src/lib/time.ts` 工具函数格式化时间，自动适配浏览器语言与本地时区。

---

## 6. 类型存放组织规范

| 类型类别 | 存放位置 | 规则 |
|---------|---------|------|
| **后端 DTO** | `src/types/api.ts` | 与后端完全对齐的唯一权威信源 |
| **全局领域模型** | `src/types/` | 跨多个 feature 共用的前端领域抽象 |
| **Feature 专属类型** | `src/features/<name>/types.ts` | 仅在本 feature 内流转的状态/模型 |
| **组件 Props** | 紧邻组件文件内 | 声明于组件正上方，不集中到全局 `types/` |

---

## 7. 禁止事项 (Iron Rules)

- ❌ 使用 `any` 类型
- ❌ 在 `src/lib/api/` 之外使用 `as` 类型断言
- ❌ 使用 `@ts-ignore`（若确需临时规避，使用 `@ts-expect-error` 并注明缺陷跟踪链接）
- ❌ 使用非空断言操作符 `!`（必须使用可选链 `?.` 或显式条件判空）
- ❌ DTO 中时间戳字段使用 `string` 或 `Date` 类型（必须为 `number`）
- ❌ 关闭 tsconfig 中任何 strict 检查选项
- ❌ 手动通过字符串切片格式化日期
