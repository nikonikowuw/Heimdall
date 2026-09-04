# API 规范

> Axum HTTP + WebSocket。前端是唯一消费者，契约由本文件和 [../frontend/type-safety.md](../frontend/type-safety.md) 共同约束。

> ⚠️ **状态：立项约定（尚未经代码验证）**
> 首批路由落地后需回填真实端点清单与 DTO 示例，并删除本提示。

---

## 分层

```
crates/api/src/
├── lib.rs
├── error.rs          # ApiError + IntoResponse，见 error-handling.md
├── state.rs          # AppState：共享句柄（db, pipeline, config）
├── middleware/       # 极简 JWT / API-Key 鉴权中间件、操作日志
│   ├── auth.rs
│   └── oplog.rs
├── dto/              # 请求/响应类型，只有 serde 派生，无业务逻辑
│   ├── mod.rs
│   ├── auth.rs       # 管理员登录、修改密码
│   ├── camera.rs     # 摄像头拉流参数
│   ├── task.rs       # 任务配置、ROI/Mask/Line 规则
│   ├── alarm.rs      # 告警事件
│   ├── capture.rs    # 目标抓拍与人脸/车牌通行
│   └── system.rs     # 系统状态(CPU/NPU/内存/存储)、网络配置、API Key
├── routes/           # 路由模块划分
│   ├── mod.rs        # Router 组装
│   ├── auth.rs       # /api/v1/auth (login, change-password)
│   ├── camera.rs     # /api/v1/cameras (CRUD, probe, deduce-substream)
│   ├── live.rs       # /api/v1/live (HTTP-FLV / Enhanced FLV H.265 / H.264 实时分发)
│   ├── task.rs       # /api/v1/tasks
│   ├── alarm.rs      # /api/v1/alarms
│   ├── capture.rs    # /api/v1/captures
│   ├── observation.rs# /api/v1/observations/faces, /plates
│   ├── system.rs     # /api/v1/system (info, network, reboot, api-key)
│   └── ws.rs         # /api/v1/ws/events
└── static_files.rs   # 前端 SPA 静态资源内嵌服务 (rust-embed)
```

约定：

- **handler 只做三件事**：提取参数、调用领域服务/仓储、把结果映射成 DTO。
- **DTO 与领域类型分离**。写显式的 `From<AlarmRecord> for AlarmDto`。
- **`api` 不依赖 `sea-orm` 的复杂查询 DSL**，仅持有连接句柄并调用 `db` 的 repository 函数。

---

## AppState

```rust
// crates/api/src/state.rs
#[derive(Clone)]
pub struct AppState {
    pub db: DatabaseConnection,
    /// 核心视频分析管线控制句柄（启停摄像头拉流、热更新 ROI/Mask 规则）
    pub pipeline: Arc<PipelineHandle>,
    /// 实时事件广播通道（将管线触发的告警与通行记录广播给在线 WebSocket 客户端）
    pub event_broadcaster: broadcast::Sender<WsMessage>,
    pub config: Arc<ArcSwap<Config>>,
}
```

规则：`AppState` 必须是廉价 `Clone`（内部全是 `Arc` 或句柄）。不要往里塞大对象。

---

## 路由与命名

| 约定 | 规则 |
| ------ | ------ |
| 前缀 | 所有 API 挂在 `/api/v1` 下 |
| 资源名 | 复数 kebab-case：`/api/v1/cameras`、`/api/v1/events` |
| 路径参数 | `/api/v1/cameras/{id}` |
| 动作类接口 | `POST /api/v1/cameras/{id}/restart`（动词作为子资源） |
| HTTP-FLV 实时流 | `GET /api/v1/live/{id}.flv?stream=main\|sub&token={jwt}`（支持 H.264 与 Enhanced FLV H.265） |
| WebRTC WHEP | `POST /api/v1/webrtc/whep?cameraId={id}&stream=main\|sub`、`DELETE /api/v1/webrtc/whep/{sessionId}` |
| WebSocket | `/api/v1/ws/events`、`/api/v1/ws/stream/{camera_id}` |
| 静态资源 | 根路径 `/`，SPA fallback 到 `index.html` |

**SPA fallback 是必需的**：前端是客户端路由，直接访问 `/events/123` 时后端必须返回 `index.html` 而不是 404。

---

## 请求与响应格式

- 一律 JSON，字段名 **camelCase**（前端是 TypeScript，用 `#[serde(rename_all = "camelCase")]` 统一转换，不要让前端做适配）。
- **时间戳与时区规范（全链路 UTC 毫秒基准）**：
  - 系统内所有整型时间戳一律约定为 **13 位 UTC Unix 毫秒整数**（Rust 为 `i64`，TS 为 `number`），系统内不存在任何秒级整型时间戳传输。
  - **响应信封**：统一使用业界标准 **`timestamp`**。
  - **业务实体绝对时间点**：使用自然语义命名，如事件触发时刻 `timestamp`、创建与更新时刻 `createdAt` / `updatedAt`。
  - **游标分页参数**：统一使用 `before`（或 `after`），值为 UTC 毫秒时间戳。
  - **相对时长（Durations / Delays / Timeouts）**：保留单位后缀，如 `timeoutMs`、`durationMs`、`intervalMs`、`latencyMs`，消除时长单位歧义。
  - 后端只存 UTC，不存储也不传输时区信息，前端负责按用户本地时区显示。与数据库存储一致，见 [database-guidelines.md](./database-guidelines.md)。
- **服务端响应国际化 (i18n) 契约**：
  - 客户端通过标准 HTTP 请求头 `Accept-Language` 传递当前界面语言偏好（支持 `zh-CN`, `zh-TW`, `en` 等）；
  - 服务端全局中间件 `i18n_response_middleware` 自动拦截所有 JSON 响应，根据状态码（`code`）将 `message` 转换为目标语言，并在响应头附带标准 `Content-Language: <lang>`；
  - 杜绝非中文界面弹出硬编码中文错误的问题，保证 Web 控制台、API Client 与三方集成行为高度统一。

### 统一响应体

所有 API 响应遵循统一格式：

```json
// 成功（单个对象）
{
  "code": 0,
  "message": "success",
  "data": { "id": "cam-01", "name": "前门" },
  "timestamp": 1704123456000
}

// 成功（列表）
{
  "code": 0,
  "message": "success",
  "data": {
    "items": [ ... ],
    "hasMore": true
  },
  "timestamp": 1704123456000
}

// 错误
{
  "code": 20001,
  "message": "摄像头不存在: cam-99",
  "data": null,
  "timestamp": 1704123456000
}
```

| 字段 | 类型 | 说明 |
|------|------|------|
| `code` | `number` | `0` 表示成功，非 0 表示具体错误（5 位数字，按模块分段） |
| `message` | `string` | 成功时固定 `"success"`，错误时为人类可读描述 |
| `data` | `T \| null` | 成功时返回数据，错误时为 `null` |
| `timestamp` | `number` | 服务器当前 UTC 毫秒时间戳，用于前端校准本地时间 |

### 错误码体系

错误码为 5 位数字，按模块分段：

| 范围 | 模块 | 说明 |
|------|------|------|
| `0` | 成功 | 请求成功 |
| `10000-19999` | 认证授权 | 登录、Token、权限 |
| `20000-29999` | 摄像头 | 设备管理、拉流、配置 |
| `30000-39999` | 算法任务 | 任务创建、规则、ROI/Mask |
| `40000-49999` | 告警事件 | 告警查询、导出、统计 |
| `50000-59999` | 系统 | 配置、网络、存储、升级 |
| `90000-99999` | 内部错误 | 服务器内部故障 |

### 错误码明细

#### 认证授权（1xxxx）

| 错误码 | HTTP 状态 | 说明 |
|--------|----------|------|
| `10001` | 401 | 未登录（无 Token） |
| `10002` | 401 | Token 已过期 |
| `10003` | 401 | Token 无效（格式错误或签名不匹配） |
| `10004` | 403 | 无权限访问该资源 |
| `10005` | 403 | 系统未初始化，需先完成开箱向导 |
| `10006` | 403 | 系统已初始化，禁止重复调用初始化接口 |
| `10007` | 400 | 用户名或密码错误 |
| `10008` | 400 | 新密码不符合强度要求 |

#### 摄像头（2xxxx）

| 错误码 | HTTP 状态 | 说明 |
|--------|----------|------|
| `20001` | 400 | 摄像头 RTSP 连接失败 |
| `20002` | 400 | RTSP 码流协议交互异常 |
| `20003` | 400 | 视频 SPS 参数集解析失败 |
| `20004` | 408 | 摄像头探活超时 |
| `20005` | 500 | 硬件解码器初始化失败 |
| `20006` | 500 | 视频帧硬件解码失败 |
| `20007` | 400 | 不支持的视频编解码格式 |
| `20008` | 404 | 摄像头流会话未找到 |
| `20009` | 500 | 底层硬件帧句柄错误 |
| `20010` | 500 | 网络与系统 IO 错误 |

#### 算法任务（3xxxx）

| 错误码 | HTTP 状态 | 说明 |
|--------|----------|------|
| `30001` | 404 | 任务不存在 |
| `30002` | 400 | 任务配置无效（缺少必填字段） |
| `30003` | 404 | 算法包未安装 |
| `30004` | 409 | 摄像头已被其它任务占用 |
| `30005` | 400 | ROI/Mask 坐标超出视频分辨率范围 |
| `30006` | 400 | Line 规则至少需要两个点 |
| `30007` | 500 | 推理后端不可用（当前平台未编译该后端） |
| `30008` | 408 | 推理超时 |

#### 告警事件（4xxxx）

| 错误码 | HTTP 状态 | 说明 |
|--------|----------|------|
| `40001` | 404 | 告警事件不存在 |
| `40002` | 400 | 查询时间范围无效（开始时间 > 结束时间） |
| `40003` | 400 | 导出时间范围超过上限（最多 7 天） |
| `40004` | 500 | 抓拍图片文件缺失 |

#### 系统（5xxxx）

| 错误码 | HTTP 状态 | 说明 |
|--------|----------|------|
| `50001` | 500 | 系统信息读取失败 |
| `50002` | 400 | 网络配置无效（IP 格式错误） |
| `50003` | 500 | 网络配置应用失败 |
| `50004` | 500 | 系统重启失败 |
| `50005` | 500 | 存储配额设置失败 |
| `50006` | 400 | API Key 格式无效 |
| `50007` | 500 | 升级包验证失败 |
| `50008` | 500 | 升级过程中出错 |

#### 内部错误（9xxxx）

| 错误码 | HTTP 状态 | 说明 |
|--------|----------|------|
| `90001` | 500 | 数据库操作失败 |
| `90002` | 500 | WebSocket 广播失败 |
| `90003` | 500 | 文件系统操作失败 |
| `99999` | 500 | 未知内部错误 |

### 错误响应实现

```rust
// crates/api/src/response.rs
use serde::Serialize;
use chrono;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ApiResponse<T: Serialize> {
    pub code: u32,
    pub message: String,
    pub data: Option<T>,
    pub timestamp: i64,
}

impl<T: Serialize> ApiResponse<T> {
    pub fn success(data: T) -> Self {
        Self {
            code: 0,
            message: "success".to_string(),
            data: Some(data),
            timestamp: chrono::Utc::now().timestamp_millis(),
        }
    }
}

impl ApiResponse<()> {
    pub fn error(code: u32, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            data: None,
            timestamp: chrono::Utc::now().timestamp_millis(),
        }
    }
}

/** handler 统一返回类型 */
pub type AppResult<T> = Result<Json<ApiResponse<T>>, ApiError>;
```

```rust
// crates/api/src/error.rs
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde_json::json;

#[derive(Debug, thiserror::Error)]
pub enum ApiError {
    #[error("摄像头不存在: {0}")]
    CameraNotFound(String),

    #[error("RTSP 连接超时")]
    RtspTimeout(String),

    #[error("请求参数无效: {0}")]
    BadRequest(String),

    #[error("参数校验失败")]
    Validation { field: String, message: String },

    #[error("内部错误")]
    Internal(#[from] anyhow::Error),
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let (code, status) = match &self {
            ApiError::CameraNotFound(_)          => (20001, StatusCode::NOT_FOUND),
            ApiError::RtspTimeout(_)             => (20003, StatusCode::REQUEST_TIMEOUT),
            ApiError::BadRequest(_)              => (90001, StatusCode::BAD_REQUEST),
            ApiError::Validation { .. }          => (90001, StatusCode::BAD_REQUEST),
            ApiError::Internal(_)                => (99999, StatusCode::INTERNAL_SERVER_ERROR),
        };
        let resp = ApiResponse::<()>::error(code, self.to_string());
        (status, Json(resp)).into_response()
    }
}

/** 构造校验错误的辅助函数 */
pub fn validation_error(field: impl Into<String>, message: impl Into<String>) -> ApiError {
    ApiError::Validation {
        field: field.into(),
        message: message.into(),
    }
}
```

### 全局错误捕获中间件

```rust
// crates/api/src/middleware/error_capture.rs
use axum::{
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use tower::{Layer, Service};

#[derive(Clone)]
pub struct ErrorCaptureLayer;

impl<S> Layer<S> for ErrorCaptureLayer {
    type Service = ErrorCaptureService<S>;
    fn layer(&self, inner: S) -> Self::Service {
        ErrorCaptureService { inner }
    }
}

#[derive(Clone)]
pub struct ErrorCaptureService<S> {
    inner: S,
}

impl<S, ReqBody> Service<axum::http::Request<ReqBody>> for ErrorCaptureService<S>
where
    S: Service<axum::http::Request<ReqBody>, Response = axum::response::Response> + Clone + Send + 'static,
    S::Future: Send,
    ReqBody: Send + 'static,
{
    type Response = axum::response::Response;
    type Error = S::Error;
    type Future = Pin<Box<dyn Future<Output = Result<Self::Response, Self::Error>> + Send>>;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.inner.poll_ready(cx)
    }

    fn call(&mut self, req: axum::http::Request<ReqBody>) -> Self::Future {
        let mut inner = self.inner.clone();
        Box::pin(async move {
            match inner.call(req).await {
                Ok(response) => Ok(response),
                Err(err) => {
                    // 捕获未处理的 panic 或 tower 层错误
                    tracing::error!(error = ?err, "未捕获的内部错误");
                    let resp = ApiResponse::<()>::error(99999, "内部错误");
                    Ok((StatusCode::INTERNAL_SERVER_ERROR, Json(resp)).into_response())
                }
            }
        })
    }
}
```

### handler 使用示例

```rust
use crate::response::{AppResult, ApiResponse};

// ✅ 使用 AppResult<T> 作为返回类型，? 自动传播错误
async fn get_camera(
    Path(id): Path<String>,
    State(st): State<AppState>,
) -> AppResult<CameraDto> {
    let camera = repository::find_camera(&st.db, &id)
        .await?                          // anyhow::Error → ApiError::Internal
        .ok_or_else(|| ApiError::CameraNotFound(id.clone()))?;
    Ok(Json(ApiResponse::success(camera.into())))
}

// ✅ 校验失败时用辅助函数
async fn create_camera(
    Json(input): Json<CreateCameraDto>,
    State(st): State<AppState>,
) -> AppResult<CameraDto> {
    if input.name.is_empty() {
        return Err(validation_error("name", "名称不能为空"));
    }
    if !input.rtsp_url.starts_with("rtsp://") {
        return Err(validation_error("rtspUrl", "RTSP 地址格式无效"));
    }
    // ...
}

// ✅ 批量校验（收集所有错误）
async fn create_task(
    Json(input): Json<CreateTaskDto>,
    State(st): State<AppState>,
) -> AppResult<TaskDto> {
    let mut errors = Vec::new();

    if input.camera_id.is_empty() {
        errors.push(validation_error("cameraId", "摄像头 ID 不能为空"));
    }
    if input.algorithm_id.is_empty() {
        errors.push(validation_error("algorithmId", "算法 ID 不能为空"));
    }
    if input.roi.is_empty() {
        errors.push(validation_error("roi", "ROI 区域不能为空"));
    }

    if !errors.is_empty() {
        return Err(errors.remove(0));  // 返回第一个错误，或聚合返回
    }
    // ...
}
```

### 分页策略与响应

系统严格区分两类分页策略：

1. **高频流式数据（告警事件、通行记录、系统操作日志）**：
   - 随时间单调递增，数据规模大；
   - **强制采用游标分页**（`before` + `limit`），禁止使用 `offset` 分页，杜绝深度翻页性能劣化与高频写入导致的跳页问题。
2. **静态实体配置（摄像头设备列表、算法任务列表）**：
   - 数据总量少（通常几十条以内）；
   - **允许采用传统页码分页**（`page` + `pageSize`）或全量返回。

```json
// 传统页码分页响应（配置类资源）
{
  "code": 0,
  "message": "success",
  "data": {
    "items": [ ... ],
    "total": 100,
    "page": 1,
    "pageSize": 20,
    "totalPages": 5
  },
  "timestamp": 1704123456000
}
```

| 字段 | 类型 | 说明 |
|------|------|------|
| `items` | `T[]` | 当前页数据 |
| `total` | `number` | 总记录数 |
| `page` | `number` | 当前页码（从 1 开始） |
| `pageSize` | `number` | 每页条数（默认 20，上限 100） |
| `totalPages` | `number` | 总页数 |

**分页请求参数**：

```rust
/// 针对配置类资源的分页查询
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PageQuery {
    #[serde(default = "default_page")]
    pub page: u32,           // 默认 1
    #[serde(default = "default_page_size")]
    pub page_size: u32,      // 默认 20，上限 100，超过则钳制
}

/// 针对高频流式数据（告警/通行/日志）的游标查询
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CursorQuery {
    #[serde(default = "default_limit")]
    pub limit: u32,          // 默认 50，上限 500
    pub before: Option<i64>,  // 游标：返回此时间戳之前的记录（UTC 毫秒）
    pub camera_id: Option<String>,
}

fn default_page() -> u32 { 1 }
fn default_page_size() -> u32 { 20 }
fn default_limit() -> u32 { 50 }
```

---

## 输入校验

- 校验在 **handler 边界**完成，不要把未校验的输入传进领域层。
- 校验失败返回 `ApiError::BadRequest`，消息里说明哪个字段、为什么。
- 数值参数（`limit`、时间范围）超界时**钳制到合法范围**，不要直接报错 —— 前端传大了不是错误。
- 字符串参数要有长度上限，防止内存放大。

---

## WebSocket

Argus 的实时事件推送走 WebSocket，**不要让前端轮询列表接口**。

```rust
// crates/api/src/routes/ws.rs
async fn ws_handler(ws: WebSocketUpgrade, State(st): State<AppState>) -> Response {
    ws.on_upgrade(|socket| handle_socket(socket, st))
}
```

约定：

- 服务端用 `tokio::sync::broadcast` 分发事件。**慢客户端触发 `Lagged` 时跳过丢失的消息并发一条提示帧**，不阻塞其它客户端，也不无限缓冲。
- **统一消息格式**：`{ "type": "<领域>.<动作>", "payload": { ... } }`。
  - `alarm.reported`：新告警产生（含目标 Bounding Box、抓拍图片相对路径、时间戳）
  - `capture.reported`：通用目标抓拍上报
  - `face.observed`：人脸识别比对命中（含底库快照与置信度）
  - `plate.observed`：车牌通行识别
  - `task.reconciled`：摄像头/算法实例生命周期状态变化
- **必须有心跳**（服务端定期 ping），检测半开连接。边缘设备的网络环境不稳定。
- 客户端断开是常态，清理要幂等。

---

## 首次启动与开箱初始化向导 (First-Boot Setup)

边缘工控机/一体机严禁使用弱硬编码出厂密码（如 `admin123`），杜绝在代码逻辑中硬编码固定账号名为 `"admin"`。系统采用**双轨开箱与无感初始化（Out-of-the-Box Experience, OOBE）**流程：

```
首次开机启动
   │
   ├── [轨道 A：自动化部署 / 环境变量注入]
   │      检测到环境变量 ARGUS_ADMIN_PASSWORD（可选 ARGUS_ADMIN_USERNAME，缺省 admin）
   │      后端直接加盐哈希落库，持久化标记 initialized = true
   │
   └── [轨道 B：零弱密码交互式开箱向导 (OOBE)]
          未配置环境变量且数据库为空，系统保持 initialized = false
          前端拉取 GET /api/v1/auth/init-status ──> 返回 { "initialized": false }
          前端控制台强制呈现开箱初始化向导 (Setup Wizard)
          现场工程师输入自定义管理员账号与高强度密码
          POST /api/v1/auth/initialize
          落库并直接下发登录 Token 与系统基础参数，无感登入控制台
```

**关键安全与幂等契约**：

- **`GET /api/v1/auth/init-status`**：免鉴权公开接口，仅返回 `{ "initialized": bool }`。
- **`POST /api/v1/auth/initialize`**：
  - 仅在 `initialized == false` 时允许调用；
  - 一旦系统已完成初始化，该接口**永久锁死**并返回 `403 Forbidden`（错误码 `10006`），防止任何恶意重入篡改；
  - 初始化成功后，在同一个响应体内直接返回 JWT Token，前端无感登录，体验一气呵成。
- **动态用户名与密码安全**：
  - 单用户架构下，用户名由管理员自由配置，服务端通过 `find_by_username` 动态检索，不硬编码 `"admin"`；
  - 密码使用加盐 PBKDF2-HMAC-SHA256 计算，常数时间比对抵御时序攻击；
  - 基于 JWT + 毫秒级时间戳 `token_invalid_before` 联动内存原子整型缓存，修改密码或登出时立即让此前签发的所有凭据失效（$O(1)$ 复杂度）。
- **未初始化拦截**：在系统未完成初始化之前，除静态资源、`init-status`、`initialize` 之外的所有业务接口，鉴权中间件统一拦截并返回 `401/428`。

---

## 视频流与元数据同步

实时预览流不走普通 HTTP JSON 接口。作为边缘侧实时视频分析系统（对标 Frigate），核心目标是**低延迟（< 500ms）与画面/框体严格同步**：

- **选型倾向**：
  - 实时流优先评估 **WebCodecs (WebSocket fMP4/AnnexB)**、**MSE over WebSocket** 或 **WebRTC (WHEP)**。
  - **排除用传统 HLS 做实时监控**（切片分发延迟高达 2~6 秒以上，无法满足实时判定交互）；HLS 仅用于历史录像片段的按需点播。
- **流传输与控制面分离**：流数据端口或 WebSocket 路由独立，流带宽压力不可阻塞 RESTful 控制接口。
- **并发上限**：每路流的并发观看数要有上限，超过则拒绝新连接（边缘芯片硬件编码/转码通道有限）。
- **时间戳对齐契约（Frame PTS vs Event timestamp）**：
  - WebSocket 推送的识别结果（Bounding Box）必须携带对应帧的 `timestamp`（即采集/解码的 PTS 毫秒值）。
  - 前端以此时间戳为基准建立滑动队列，与播放器当前渲染帧对齐，杜绝“画面与检测框飘移脱节”。

---

## 静态资源（单二进制交付）

前端构建产物由 Axum 直接提供服务，以支撑单二进制极致部署：

- **生产环境（默认）**：采用 `rust-embed` 将 `web/dist` 下的 HTML/JS/CSS 静态资源直接编译嵌入进 Rust 主二进制文件。单文件交付，无需担心资源丢失或外部静态目录路径配置错误。
- **开发环境**：支持环境变量检测，若存在开发目录则通过 `tower-http::services::ServeDir` 实时读取或代理至 Vite 开发服务器（保留前端 HMR 快速热重载体验）。
- **SPA Fallback**：任何非 `/api/` 的未匹配前端路由（如 `/live`、`/alarms/123`），一律 fallback 返回 `index.html`，由前端 React Router 驱动客户端路由。

---

## 禁止事项

- ❌ handler 里写业务判定逻辑
- ❌ 领域类型直接 `Serialize` 返回给前端
- ❌ `api` 直接调用 `sea-orm` 查询 DSL
- ❌ 无 `limit` 的列表接口
- ❌ 在高频流式大表（告警/通行/日志）上使用 offset 分页（必须走游标分页）
- ❌ 时间戳返回字符串
- ❌ 内部错误细节返回给客户端
- ❌ 让前端轮询获取实时事件
- ❌ 在 handler 里做阻塞调用（见 [concurrency-guidelines.md](./concurrency-guidelines.md)）

---

## 待验证事项

- [ ] 视频流方案选型：WebCodecs (WS) vs MSE over WS vs WebRTC (WHEP)；真机测试端到端延迟与资源占用
- [ ] 静态资源方式：`rust-embed` vs `ServeDir`（生产编译嵌入，开发环境变量外挂）
- [ ] 是否生成 OpenAPI（`utoipa`），用于前端类型自动同步，见 [../frontend/type-safety.md](../frontend/type-safety.md)
