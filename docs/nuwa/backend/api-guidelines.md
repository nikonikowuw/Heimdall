# API 规范

入口：[路由装配](../../../crates/api/src/routes/mod.rs)、[成功信封](../../../crates/api/src/response.rs)、[错误映射](../../../crates/api/src/error.rs)。

## 分层

- Handler 只提取参数、调用领域服务/Repository、映射 DTO；SQL DSL、业务判定、硬件和阻塞计算归所属层。
- DTO 不直接暴露数据库 Model 或硬件句柄；定义跟随所属路由/服务模块。
- `AppState` 只持连接池、Sender、`Arc` 等廉价克隆句柄；资源在应用装配期注入。

## 路由

业务接口统一 `/api/v1`，资源用复数 kebab-case；动作作为子资源，例如 `POST /cameras/{id}/restart`。

| 通路               | 当前入口                                                                    |
| ------------------ | --------------------------------------------------------------------------- |
| HTTP-FLV           | `GET /api/v1/live/{id}.flv?stream=main\|sub&token={jwt}&audio=true\|false&video=true\|false`，也支持 `{id}/flv`；`audio` 默认 false，`video` 默认 true，H.265 WebCodecs 回退时可用 `audio=true&video=false` 请求 AAC-only 音轨 |
| WS-FLV / WebCodecs | `/api/v1/live/{id}/ws`、`/api/v1/live/{id}/webcodecs?stream=main\|sub&token={jwt}`                       |
| 媒体流健康快照     | `GET /api/v1/system/media/streams/{stream_key}`，返回 `StreamHealthSnapshot`                |
| 业务事件 WS        | `/api/v1/ws/events`                                                         |
| 证据图片           | `/api/v1/evidence/image/...`；规范化后校验路径仍在证据根目录内              |
| SPA                | 非 `/api/` 路径 fallback 到 `index.html`                                    |

流端点细节以 [live.rs](../../../crates/api/src/routes/live.rs) 为准。旧 WHEP 示例尚未注册，不作为现有接口使用。

## 任务配置写入

任务配置有两条写入口，共用同一个 `analysis_tasks.config_revision`：

| 入口 | 语义 | 载荷 |
| --- | --- | --- |
| `PUT /api/v1/tasks/{cameraId}` | 整份配置覆盖写：名称、布防意图、防区、门控、算法实例集合 | 完整配置，可带 `configRevision` |
| `PUT /api/v1/tasks/{cameraId}/enabled` | 状态动词：只翻转布防总闸，不改动其余配置 | `{ "enabled": bool }`，必填 |

- `algorithmInstances` 是集合替换语义：不在数组中的实例会被删除；省略该字段表示保留现有集合。`statusMessage`、`streamMode` 同理由服务端保留现值。
- `configRevision`：响应必带；请求可省略，省略即不做版本校验。`0` 表示「读取时该通道还没有任务」，供快速创建做乐观断言。不匹配返回 `409` + `40903`，且错误响应 `data` 为 `null`。
- 两条入口都只提交期望配置：运行时收敛一律由 `sync_pipeline_for_camera` 从已提交的持久化配置驱动，禁止任何入口用请求体拼装 `StartCameraPipelineParams`。
- 布防总闸（`analysis_tasks.desired_enabled`）与实例分闸（`algorithm_instances.enabled`）是两个独立开关：撤防不写回分闸，运行与否由总闸判定。

## JSON、时间与错误

```json
{ "code": 0, "message": "success", "data": {}, "timestamp": 1788825600000 }
```

- 所有 REST JSON 响应复用根信封；失败时 `code != 0`、`data: null`，禁止另造包装。
- 字段与时间约定见 [全局约定](../guides/conventions.md)。
- 错误码为稳定的 5 位模块码，`0` 成功；沿用领域 `error_code()` 和 `ApiError` 的既有映射，不在文档另建错误码副本。
- HTTP 状态与业务码都需正确。客户端按码分支，不匹配消息文本；内部错误细节只记录到日志。
- [i18n 中间件](../../../crates/api/src/middleware/i18n.rs) 读取 `Accept-Language`，支持 `zh-CN` / `zh-TW` / `en`，返回 `Content-Language`；字典按业务模块拆分。

## 告警与证据

告警成立、目标坐标、事件幂等与证据生成状态遵循 [检测结果、告警与证据契约](./detection-alarm-contract.md)。
当前 [AlarmDto](../../../crates/api/src/routes/alarm.rs) 使用平铺的 `imageId` / `cropImageId` 与 `bboxJson` 字符串；不要求改成 `evidence` / `target` 包装。
稳定 `ruleId` 与 `evidenceStatus` 是待实现的增量字段，接入时同步持久化、DTO、WS 类型和消费者；图片生成状态不能复用人工处理的 `status`。

[CaptureDto](../../../crates/api/src/routes/evidence.rs) / [RecognitionDto](../../../crates/api/src/routes/evidence.rs)
已暴露证据图来源：`imageSource` / `imageStream` / `imagePtsMs` / `fusedCount` / `templateQuality`。
其中 `imageSource` / `imageStream` 是强类型枚举（TS 联合类型），库内未知取值与空串统一序列化为 `null`；
`imagePtsMs` 为 `null` 表示与检测轴不可比或未记录，**不是** 0 时标。
新增枚举取值必须同时升级宿主与前端：旧前端对未知取值的行为是「不渲染徽标」，而非报错。
抓拍行恒有图（**无图不成行**，见 [数据库规范](./database-guidelines.md)）：`imageRelPath` 为空只能来自迁移前遗留行，不是运行期状态，前端图片占位仅作防御性保留。
`alarm_records` 尚未加同类列（告警不走峰值候选路径，`image_source` 无变化）；若需评估告警特写图的降级率，另立迁移。

## 快照编码系统配置

快照编码质量与裁剪参数统一由 `GET/PUT /api/v1/system/snapshot/config` 管理：
- DTO 采用 camelCase，按主码流/子码流双流区分全景与特写质量：`mainStreamPanoramicQuality`、`mainStreamCropQuality`、`subStreamPanoramicQuality`、`subStreamCropQuality` (范围 1-100)，以及 `cropPaddingRatio` (范围 0.0-0.5)；
- 配置变更由 SQLite 持久化并由 `SnapshotEngine` 执行无锁原子热更新，下次抓拍即刻生效；
- 前端设置页面统一收敛在存储设置页的「图片编码」分区，不创建独立 Tab。

## 分页

| 数据                     | 约定                                                                   |
| ------------------------ | ---------------------------------------------------------------------- |
| 告警、抓拍等持续增长数据 | `before: i64` 毫秒游标 + `limit`，通用默认 50、上限 500；避免深 offset |
| 摄像头、任务等配置资源   | `page` 默认 1，`pageSize` 默认 20、上限 100                            |

具体端点的已发布返回形状和更小上限以其 DTO/校验为准，不能推断所有列表都有 `items/total`。
现有 [操作日志接口](../../../crates/api/src/routes/oplog.rs) 仍使用 `offset` 且上限 100，这是与游标约定的差异；后续迁移需同步前端契约。

## 初始化与审计

- 无出厂弱密码。`GET /auth/init-status` 公开返回初始化状态；`POST /auth/initialize` 设置管理员、加盐 PBKDF2-HMAC-SHA256 密码并签发 JWT。
- 未初始化时业务接口拒绝访问（401/428）；初始化完成后再次初始化返回 403、`code: 10006`。
- 受保护写请求（POST/PUT/PATCH/DELETE）由审计中间件异步记录，审计失败不改变业务响应。
- 路径用 `OriginalUri` 保留完整 path/query；IP 按 `X-Forwarded-For` 首地址、`X-Real-IP`、`ConnectInfo`、`unknown` 获取。
- 审计 Body 最多 2048 个 UTF-8 字符；密码和二进制上传不明文入库。

## WebSocket

消息使用 `{ "type": "<domain>.<action>", "payload": ... }`，例如 `alarm.reported`、`capture.reported`；新增类型同步共享类型与消费者。
检测元数据的 `timestamp` 必须对应视频帧，消费方复用统一解码与归一化逻辑。
广播有界，慢客户端 `Lagged` 丢弃过旧消息，不反压推流；常态监控遵循稀疏告警通知，避免逐帧噪声。

验证覆盖信封/状态码、空值与边界、分页上限、初始化防重入、路径穿越和慢客户端降级。
