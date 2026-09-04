# 类型安全

> TypeScript 严格模式。核心问题：**前端类型和 Rust 后端的 DTO 如何保持同步。**

> ⚠️ **状态：立项约定（尚未经代码验证）**
> 类型同步方案尚未选定，见文末待验证事项。首批类型落地后需回填真实定义并删除本提示。

---

## tsconfig 基线

```jsonc
{
  "compilerOptions": {
    "strict": true,
    "noUncheckedIndexedAccess": true,      // arr[0] 是 T | undefined
    "noUnusedLocals": true,
    "noUnusedParameters": true,
    "noFallthroughCasesInSwitch": true,
    "verbatimModuleSyntax": true,
    "erasableSyntaxOnly": true
  }
}
```

`noUncheckedIndexedAccess` 会带来一些额外的判空，但它挡住的正是"事件列表为空时取 `events[0]` 崩溃"这类真实 bug。**不要因为麻烦就关掉。**

---

## 与后端 DTO 对齐

后端返回 camelCase JSON，时间戳是 Unix 毫秒整数，见 [../backend/api-guidelines.md](../backend/api-guidelines.md)。

### 统一响应类型

```ts
// src/types/api.ts

/** 统一响应体 */
export interface ApiResponse<T> {
  code: number        // 0 = 成功，非 0 = 错误
  message: string     // 成功时固定 "success"，错误时为描述
  data: T | null      // 成功时返回数据，错误时为 null
  timestamp: number   // 服务器当前 UTC 毫秒时间戳
}

/** 分页数据（配置类资源） */
export interface PaginatedData<T> {
  items: T[]
  total: number       // 总记录数
  page: number        // 当前页码（从 1 开始）
  pageSize: number    // 每页条数
  totalPages: number  // 总页数
}

/** 分页请求参数（配置类资源） */
export interface PageParams {
  page?: number       // 默认 1
  pageSize?: number   // 默认 20，上限 100
}

/** 游标分页请求参数（流式事件与日志大表） */
export interface CursorParams {
  limit?: number      // 默认 50，上限 500
  before?: number     // 游标时间戳（UTC 毫秒）
  cameraId?: string
}

/** 错误码常量 */
export const ErrorCode = {
  // 摄像头
  CAMERA_NOT_FOUND: 20001,
  INVALID_RTSP_URL: 20002,
  RTSP_TIMEOUT: 20003,

  // 任务
  TASK_NOT_FOUND: 30001,
  INVALID_TASK_CONFIG: 30002,
  INFERENCE_TIMEOUT: 30008,

  // 内部错误
  DATABASE_ERROR: 90001,
  UNKNOWN_ERROR: 99999,
} as const

export type ErrorCode = (typeof ErrorCode)[keyof typeof ErrorCode]
```

```ts
// src/types/api.ts
export interface BoundingBox {
  x: number      // 归一化 [0.0, 1.0]
  y: number
  width: number
  height: number
}

export interface DetectedObject {
  label: string
  score: number
  bbox: BoundingBox
}

export interface AlarmDto {
  id: string                   // 引擎生成的唯一全局事件 ID
  cameraId: string
  cameraName?: string
  algorithmId: string
  alarmTypeId: string
  timestamp: number            // UTC Unix 毫秒，与视频 PTS 对齐
  objects: DetectedObject[]
  imageRelPath: string | null  // 相对数据路径，如 "var/images/..."
}

export interface FaceObservationDto {
  id: string
  cameraId: string
  trackId: number
  personName: string
  similarity: number
  faceBbox: BoundingBox
  imageRelPath: string | null
  faceImageRelPath: string | null
  timestamp: number
}

export interface PlateObservationDto {
  id: string
  cameraId: string
  trackId: number
  plateText: string
  confidence: number
  plateBbox: BoundingBox
  imageRelPath: string | null
  timestamp: number
}
```

**当前手写维护，规则如下**：

- **所有后端 DTO 集中在 `src/types/api.ts`**，不散落在各 feature 里。改后端接口时前端只需要改一个文件。
- **字段名与后端 `#[serde(rename_all = "camelCase")]` 后的结果逐字对齐**。
- **时间戳类型是 `number`**。在类型上就区分开，避免有人传字符串进来。
- 后端的 `Option<T>` 对应 `T | null`（serde 默认序列化为 `null`），**不是 `T | undefined`**。这个区别在 `exactOptionalPropertyTypes` 下会实际报错。

手写维护是过渡方案。接口数量上去后按待验证事项决策自动生成。

---

## 边界处理：网络数据不是可信类型

`fetch().json()` 返回 `any`。直接 `as EventDto` 是在骗编译器 —— 后端改了字段，前端在运行时才炸。

```ts
// ❌ 断言不是校验
const data = (await res.json()) as EventDto[]

// ✅ 至少在 API 层收口，让不安全断言只出现在一个地方
// src/lib/api/events.ts
export async function fetchEvents(q: ListEventsQuery, signal?: AbortSignal): Promise<EventDto[]> {
  const res = await http.get("/api/v1/events", { params: q, signal })
  return res as EventDto[]   // 唯一的信任点，附注释说明
}
```

**规则**：类型断言只允许出现在 `src/lib/api/` 里，且要有注释。组件和 hook 里不允许 `as`。

是否引入运行时校验（zod / valibot）见待验证事项。判断依据：如果出现过"后端改字段导致前端白屏"的事故，就该上。

---

## 联合类型驱动状态

WebSocket 消息按 `type` 分发，用可辨识联合，让 `switch` 获得穷尽检查：

```ts
export type WsMessage =
  | { type: "alarm.reported"; payload: AlarmDto }
  | { type: "face.observed"; payload: FaceObservationDto }
  | { type: "plate.observed"; payload: PlateObservationDto }
  | { type: "task.reconciled"; payload: { taskId: string; status: string } }
  | { type: "stream.lagged"; payload: { skipped: number } }

function handle(msg: WsMessage) {
  switch (msg.type) {
    case "alarm.reported":   return onAlarm(msg.payload)
    case "face.observed":    return onFace(msg.payload)
    case "plate.observed":   return onPlate(msg.payload)
    case "task.reconciled":  return onTaskStatus(msg.payload)
    case "stream.lagged":    return onLagged(msg.payload)
    default: {
      const _exhaustive: never = msg   // 后端加了新消息类型 → 编译期报错
      return _exhaustive
    }
  }
}
```

**这个 `never` 检查是必需的**，不是炫技。后端加一种消息类型，前端能在编译期就知道自己漏处理了。

同样的模式用于加载状态：

```ts
type AsyncState<T> =
  | { status: "idle" }
  | { status: "loading" }
  | { status: "success"; data: T }
  | { status: "error"; error: string }
```

比 `{ data?: T; loading: boolean; error?: string }` 好 —— 后者允许 `loading: true` 同时 `data` 有值这种非法组合存在。

---

## 时间类型约定

系统内所有绝对时间戳统一约定为 **13 位 UTC Unix 毫秒整数**（TS 中类型为 `number`）。前端必须区分以下概念：

```ts
// src/types/api.ts

/** UTC Unix 毫秒 —— 后端返回的原始时间戳类型 */
type Timestamp = number

// 后端 DTO 中的时间字段统一标注
export interface AlarmDto {
  timestamp: Timestamp  // UTC 毫秒，前端按用户时区显示
  // ...
}
```

规则：

- **DTO 时间字段用 `number`，不用 `string`、不用 `Date`**。`Date` 对象有时区偏移副作用，容易在比较时出错。
- **显示时通过 `Intl.DateTimeFormat` 转换**，不要手写 `date.toLocaleString()`（部分浏览器行为不一致）。
- **禁止 `new Date(timestamp).toISOString().slice(0, 10)` 这类手动截取**。日期时间格式化集中到 `src/lib/time.ts`，所有组件引用同一函数。
- **UTC 偏移量**：如需显示 UTC+8 等偏移，用 `Intl.DateTimeFormat` 的 `timeZoneName: 'shortOffset'` 选项，不要手算。

```ts
// ❌ 手动截取，跨浏览器行为不一致
const date = new Date(ts).toISOString().slice(0, 10)

// ✅ 用 Intl 格式化，自动适配语言与时区
new Intl.DateTimeFormat('zh-CN', {
  year: 'numeric', month: '2-digit', day: '2-digit',
  hour: '2-digit', minute: '2-digit',
}).format(new Date(ts))
```

详见 [index.md 中的时间与时区](./index.md#时间与时区)。

---

## 类型放哪

| 类型 | 位置 |
|------|------|
| 后端 DTO | `src/types/api.ts` |
| 跨 feature 的领域类型 | `src/types/` |
| 只在一个 feature 内用 | `src/features/<name>/types.ts` |
| 组件 props | 组件文件内，组件正上方 |
| hook 返回值 | hook 文件内 |

**规则**：props 类型不要集中放 `types/`。它和组件是一体的，放一起才能一起改。

---

## 禁止事项

- ❌ `any`（确需时用 `unknown` 再收窄，并写注释说明）
- ❌ `as` 断言出现在 `src/lib/api/` 之外
- ❌ `@ts-ignore`（用 `@ts-expect-error` 并说明原因，它会在问题修复后提醒你删掉）
- ❌ 非空断言 `!`（用可选链或显式判空）
- ❌ 关闭 `strict` 相关的任何一项
- ❌ 时间戳用 `string` 类型（必须是 `number`，UTC 毫秒）
- ❌ 时间戳用 `Date` 类型存储（DTO 中只用 `number`）
- ❌ 用 `boolean` 标志组合表达互斥状态（用可辨识联合）
- ❌ 手动截取日期字符串（`toISOString().slice` 等，走 `src/lib/time.ts`）

---

## 待验证事项

- [ ] **类型同步方案**：手写 vs 后端 `utoipa` 生成 OpenAPI + `openapi-typescript` 生成 TS。接口超过 15 个建议上自动生成
- [ ] 是否引入运行时校验（zod / valibot），以及是全量校验还是只校验关键接口
- [ ] `exactOptionalPropertyTypes` 是否开启（更严格，但和部分库不兼容）
