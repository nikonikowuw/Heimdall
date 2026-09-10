# API 接口契约：告警与抓拍证据三支柱及 WebSocket 实时广播 (api.md)

## 1. Scope (范围边界)

- **Backend**：
  - `crates/api/src/alarm_service.rs` (新增后台消费与持久化 Worker)
  - `crates/api/src/routes/alarm.rs` (告警列表查询与状态流转)
  - `crates/api/src/routes/evidence.rs` (抓拍记录查询与证据图片安全静态分发)
  - `crates/api/src/routes/ws.rs` (WebSocket 实时消息通道)
- **Frontend**：
  - `web/src/features/live/LivePage.tsx` (实时中心告警事件消费、提示音与悬浮卡片)
  - `web/src/features/alarms/AlarmsPage.tsx` (告警中心记录展示、核验与证据详情)
  - `web/src/types/index.ts` (前端领域类型定义)

---

## 2. Conventions (规范契约)

- **协议与路由前缀**：所有 HTTP RESTful API 挂载在 `/api/v1` 下。
- **根信封格式**：
  ```json
  {
    "code": 0,
    "message": "success",
    "data": T,
    "timestamp": 1741100000000
  }
  ```
- **时间规范**：绝对时间戳统一采用 13 位 UTC Unix 毫秒整数（`i64` / `number`）。
- **状态命名**：
  - 告警状态统一为 `unprocessed`（待处理）与 `processed`（已处理）；
  - 严重级别统一为 `warning`（一般告警）与 `critical`（紧急告警）；
  - 规则类型统一为 `roi`（区域入侵）与 `line`（绊线越界）。

---

## 3. Endpoints (HTTP 接口)

### 3.1 告警分页查询
- **Method / Path**: `GET /api/v1/alarms`
- **Query Params**:
  - `cameraId` (string, optional): 摄像头 ID 筛选
  - `status` (string, optional): `unprocessed` | `processed`
  - `startTime` (number, optional): 开始时间戳 (ms)
  - `endTime` (number, optional): 结束时间戳 (ms)
  - `limit` (number, optional, default 20)
  - `offset` (number, optional, default 0)
- **Response Data**: `Vec<AlarmDto>`
  ```json
  [
    {
      "id": 1024,
      "eventId": "c73a8867-b5bb-41db-bc4a-9ef854e4c278",
      "cameraId": "CAM-01",
      "alarmTypeId": "rule_0",
      "occurredAt": 1741100000000,
      "targetLabel": "person",
      "confidence": 0.95,
      "trackId": 42,
      "bboxJson": "{\"x1\":0.1,\"y1\":0.2,\"x2\":0.4,\"y2\":0.6}",
      "imageId": "img-uuid",
      "imageRelPath": "CAM-01/full_1741100000000.jpg",
      "cropImageId": "crop-uuid",
      "cropImageRelPath": "CAM-01/crop_1741100000000.jpg",
      "ruleType": "roi",
      "severity": "warning",
      "status": "unprocessed",
      "handledAt": null,
      "createdAt": 1741100000050
    }
  ]
  ```

### 3.2 告警处理状态更新
- **Method / Path**: `PUT /api/v1/alarms/{id}/status`
- **Request Body**:
  ```json
  {
    "status": "processed"
  }
  ```
- **Response Data**: `AlarmDto`（更新后的记录，`handledAt` 包含核验时间戳）。
- **Side Effect**: 后端向 WebSocket 全局广播 `alarm.status_changed` 事件。

### 3.3 行迹抓拍分页查询
- **Method / Path**: `GET /api/v1/evidence/captures`
- **Query Params**:
  - `cameraId` (string, optional)
  - `targetLabel` (string, optional)
  - `startTime` (number, optional)
  - `endTime` (number, optional)
  - `limit` (number, optional, default 20)
  - `offset` (number, optional, default 0)
- **Response Data**: `Vec<CaptureDto>`

### 3.4 证据图片静态分发
- **Method / Path**: `GET /api/v1/evidence/image/{*path}?token=...`
- **Response**: 二进制图片流（`image/jpeg`），防路径穿越与未授权访问。

---

## 4. WebSocket 广播事件契约

WebSocket 连接端点为 `GET /api/v1/ws`。消息格式统一为：

```json
{
  "topic": "<topic_name>",
  "payload": { ... },
  "timestamp": 1741100000000
}
```

### 4.1 新增违规告警广播 (`alarm.triggered`)
- **Topic 常量**: `alarm.triggered`
- **广播触发时机**: 当 `AlarmDispatchService` 成功将告警与抓拍记录写入 SQLite 后触发。
- **Payload Schema**:
  ```json
  {
    "id": 1024,
    "eventId": "c73a8867-b5bb-41db-bc4a-9ef854e4c278",
    "cameraId": "CAM-01",
    "cameraName": "东大门通道",
    "targetLabel": "person",
    "ruleType": "roi",
    "ruleName": "区域入侵 #1",
    "severity": "warning",
    "cropImageRelPath": "CAM-01/crop_1741100000000.jpg",
    "imageRelPath": "CAM-01/full_1741100000000.jpg",
    "occurredAt": 1741100000000
  }
  ```
- **前端消费行为 (`LivePage.tsx`)**:
  - 调用 `playAlarmChime()` 播放警报提示音；
  - 弹出悬浮卡片展示特写切图、摄像头名称、规则类型与违规目标标签；
  - 点击“查看凭证”可快速跳转至告警中心。

### 4.2 告警处理状态流转广播 (`alarm.status_changed`)
- **Topic 常量**: `alarm.status_changed`
- **广播触发时机**: 当用户在告警中心调用 `PUT /api/v1/alarms/{id}/status` 将告警核验为 `processed` 后触发。
- **Payload Schema**:
  ```json
  {
    "id": 1024,
    "eventId": "c73a8867-b5bb-41db-bc4a-9ef854e4c278",
    "status": "processed",
    "handledAt": 1741100010000
  }
  ```
- **前端消费行为 (`LivePage.tsx`)**:
  - 若悬浮卡片正在显示该 `eventId` 或 `id`，立即自动关闭悬浮卡片，完成闭环。
