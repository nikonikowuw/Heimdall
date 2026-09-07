# API 规范 (API Guidelines)

> 基于 Axum HTTP + WebSocket。契约由本文件和 [../frontend/type-safety.md](../frontend/type-safety.md) 共同约束。

---

## 1. 分层架构与职责边界

```
crates/api/src/
├── error.rs          # ApiError 与 IntoResponse 统一错误映射
├── state.rs          # AppState：共享句柄（db, pipeline, event_broadcaster, config）
├── middleware/       # JWT/API-Key 鉴权、AuditLog 审计、i18n 响应转译、错误捕获
├── dto/              # 请求/响应 DTO（仅含 serde 派生，零业务逻辑）
├── routes/           # 路由模块（auth, cameras, live, tasks, alarms, captures, system, ws）
└── static_files.rs   # 前端 SPA 静态资源内嵌服务 (rust-embed)
```

- **Handler 职责三原则**：仅负责“提取入参”、“调用领域服务/Repository”、“将结果转换为 DTO”。严禁在 Handler 中编写复杂业务判定。
- **DTO 与领域模型严格隔离**：严禁直接将数据库 Model 或硬件帧描述符序列化返回给前端。
- **数据库解耦**：`api` 层不直接依赖 `sea-orm` 的复杂查询 DSL，所有持久化访问统一由 `crates/db` 的 Repository 提供。
- **AppState 廉价 Clone**：内部仅持有连接池、Channel Sender 或 `Arc<T>` 句柄，严禁注入重量级大对象。

---

## 2. 路由与资源命名规范

- **统一前缀**：所有业务 HTTP API 挂载在 `/api/v1` 下。
- **资源复数与 Kebab-case**：如 `/api/v1/cameras`、`/api/v1/tasks`、`/api/v1/alarms`。
- **动作类接口**：采用动词作为子资源，如 `POST /api/v1/cameras/{id}/restart`。
- **实时流媒体与长连接端点**：
  - HTTP-FLV 实时流：`GET /api/v1/live/{id}.flv?stream=main|sub&token={jwt}`（支持 H.264 与 Enhanced FLV H.265）；
  - WebRTC WHEP：`POST /api/v1/webrtc/whep?cameraId={id}&stream=main|sub`、`DELETE /api/v1/webrtc/whep/{sessionId}`；
  - WebSocket 广播：`/api/v1/ws/events`；
  - 静态资源 Fallback：根路径 `/`，任何非 `/api/` 路由 fallback 返回 `index.html`。

---

## 3. 请求响应信封与时间戳铁律

### 3.1 统一根响应信封
所有 RESTful JSON 响应必须统一包装为如下信封结构：

```rust
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ApiResponse<T> {
    pub code: u32,             // 0 表示成功，非 0 表示业务错误码
    pub message: String,       // 成功为 "success"，错误为国际化可读提示
    pub data: Option<T>,       // 成功时为数据负载，错误时固定为 null
    pub timestamp: i64,        // 13 位 UTC Unix 毫秒时间戳
}
```

### 3.2 时间戳与单位硬性规范
- **全链路 13 位 UTC 毫秒基准**：系统内传输的所有绝对时间戳一律为 **13 位 UTC Unix 毫秒整数**（Rust 为 `i64`，TS 为 `number`）。系统内严禁出现整型秒级时间戳或带时区的字符串时间戳；
- **时长单位显式命名**：所有相对时长变量与字段必须显式附带单位后缀，如 `timeoutMs`、`latencyMs`、`durationMs`、`intervalMs`；
- **字段风格**：传输字段一律采用 **camelCase**（通过 `#[serde(rename_all = "camelCase")]` 映射）。

---

## 4. 错误码体系与 i18n 契约

错误码为 5 位数字，按业务模块严格分段（具体枚举定义见 `crates/api/src/error.rs`）：

| 范围 | 模块 | 典型错误码 |
|------|------|-----------|
| `0` | 成功 | 请求成功 |
| `10000-19999` | 认证与权限 | `10001` 未登录, `10002` Token 过期, `10005` 未初始化, `10006` 已初始化防重入 |
| `20000-29999` | 摄像头媒体 | `20001` RTSP 连接失败, `20004` 探活超时, `20005` 硬件解码器失败 |
| `30000-39999` | 算法任务 | `30001` 任务不存在, `30005` 规则坐标超界, `30007` 硬件推理后端未编译 |
| `40000-49999` | 告警事件 | `40001` 告警不存在, `40003` 导出超限, `40004` 抓拍物理文件缺失 |
| `50000-59999` | 系统运维 | `50002` 网络配置非法, `50004` 重启失败, `50006` API Key 无效 |
| `90000-99999` | 基础设施 | `90001` 数据库异常, `90002` WS 广播失败, `99999` 未知内部错误 |

- **服务端国际化 (i18n)**：中间件读取 `Accept-Language` 请求头（支持 `zh-CN`, `zh-TW`, `en`），根据状态码动态转译响应中的 `message`，并在响应头携带 `Content-Language`。

---

## 5. 分级分页规范

| 数据特征 | 分页策略 | 查询参数 | 约束 |
|---------|---------|---------|------|
| **高频流式数据**（告警、抓拍、操作审计日志） | **强制游标分页 (Cursor)** | `before: Option<i64>` (UTC毫秒), `limit: u32` (默认 50, 上限 500) | **严禁在大表上使用 offset 分页**，杜绝深度翻页性能劣化 |
| **静态实体配置**（摄像头列表、算法任务、用户） | 传统页码分页 (Page) | `page: u32` (默认 1), `pageSize: u32` (默认 20, 上限 100) | 数据量小，支持全量或页码查询 |

---

## 6. 开箱向导 (OOBE) 安全契约

系统严禁硬编码出厂弱密码（如 `admin/admin123`），采用动态无感初始化流程：
1. **未初始化探测**：`GET /api/v1/auth/init-status`（公开接口）返回 `{ "initialized": false }`；除此接口与静态资源外，其余所有业务接口一律拦截返回 401/428；
2. **初始化提交**：`POST /api/v1/auth/initialize` 接收自定义管理员账号与高强度密码（PBKDF2-HMAC-SHA256 加盐计算），直接下发初始 JWT Token；
3. **永久防重入锁死**：一旦 `initialized == true`，该接口**永久锁死**并返回 `403 Forbidden` (`code: 10006`)，防止重入攻击。

---

## 7. 操作审计中间件 (`AuditLogLayer`) 契约

- **自动拦截与异步落盘**：对受保护 REST 路由的所有写操作（`POST`/`PUT`/`PATCH`/`DELETE`）自动捕获，通过独立 `tokio::spawn` 写入数据库，写日志失败不影响业务响应；
- **完整路径保留**：必须通过 `request.extensions().get::<OriginalUri>()` 提取完整 path 与 query，严禁使用被 `nest` 剥离后的 `Request::uri()`；
- **IP 提取顺序**：`X-Forwarded-For` 首地址 -> `X-Real-IP` -> `ConnectInfo<SocketAddr>` -> `unknown`；
- **脱敏与限额**：Body 截断至最多 2048 个 UTF-8 字符，二进制上传与登录密码字段严禁明文入库。

---

## 8. WebSocket 实时事件与元数据同步

- **消息格式**：`{ "type": "<domain>.<action>", "payload": { ... } }`。
  - 事件类型：`alarm.reported`, `capture.reported`, `face.observed`, `plate.observed`, `task.reconciled`。
- **音画/元数据对齐**：WebSocket 推送的检测框必须包含对应的视频帧毫秒级 `timestamp` (PTS)，前端以此建立滑动队列与播放器渲染帧精确对齐，消除飘框。
- **慢客户端降级**：广播通道使用 `tokio::sync::broadcast`，慢客户端触发 `Lagged` 时主动丢弃过旧消息，严禁反压阻塞全局推流。

---

## 9. 禁止事项 (Iron Rules)

- ❌ 在 Handler 中编写领域业务判定或复杂计算
- ❌ 数据库 Model 或平台句柄直接作为 DTO 序列化输出
- ❌ 在 `api` 层直接书写 `sea-orm` 查询 DSL
- ❌ 在高频流式数据（告警/抓拍/日志）接口中使用 `offset` 分页
- ❌ 传输秒级时间戳或带时区的字符串时间戳（必须 13 位 UTC 毫秒）
- ❌ 在 WebSocket 推送中使用无界的慢队列阻塞系统
- ❌ 将内部底层 Panic 或原始数据库异常堆栈透传给前端响应
