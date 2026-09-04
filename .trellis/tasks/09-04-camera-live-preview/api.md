# Camera & WebRTC Live Preview API Contract (接口契约与数据模型)

所有接口统一基于 `/api/v1`，遵循统一 JSON 信封格式（WHEP 媒体协商端点除外）：
```json
{
  "code": 0,
  "message": "success",
  "data": {},
  "timestamp": 1747584000000
}
```

---

## 1. 摄像头管理与探活接口

### 1.1 获取摄像头列表
- **路由**：`GET /api/v1/cameras`
- **鉴权**：需携带 Bearer Token
- **响应数据 `data`**：`Array<CameraVo>`
```json
{
  "code": 0,
  "message": "success",
  "data": [
    {
      "id": 1,
      "cameraId": "550e8400-e29b-41d4-a716-446655440000",
      "name": "库房正门东区",
      "protocol": "rtsp",
      "rtspUrl": "rtsp://192.168.1.100:554/live/ch0",
      "subRtspUrl": "",
      "remark": "主要出入口",
      "transportPolicy": "auto",
      "lastProbeStatus": "success",
      "lastProbeAt": "2026-09-04T08:00:00Z",
      "lastProbeErrorCode": "",
      "lastSuccessAt": "2026-09-04T08:00:00Z",
      "lastCodec": "h264",
      "lastWidth": 1920,
      "lastHeight": 1080,
      "lastFps": 25.0,
      "createdAt": "2026-09-04T08:00:00Z",
      "updatedAt": "2026-09-04T08:00:00Z"
    }
  ],
  "timestamp": 1788508800000
}
```

### 1.2 添加摄像头
- **路由**：`POST /api/v1/cameras`
- **鉴权**：需登录（记操作审计日志 `module: camera, action: create`）
- **请求体**：
```json
{
  "name": "库房正门东区",
  "rtspUrl": "rtsp://192.168.1.100:554/live/ch0",
  "subRtspUrl": "",
  "remark": "主要出入口",
  "transportPolicy": "auto"
}
```
- **行为**：数据入库，`cameraId` 自动生成 UUID，初始 `lastProbeStatus = "never"`，后台自动触发一次异步探活，响应立即返回创建成功的实体。

### 1.3 更新摄像头
- **路由**：`PUT /api/v1/cameras/:cameraId`
- **鉴权**：需登录（记操作审计日志 `module: camera, action: update`）
- **请求体**：同上
- **行为**：更新配置，若 `rtspUrl` 发生变化自动触发一次异步探活。

### 1.4 删除摄像头
- **路由**：`DELETE /api/v1/cameras/:cameraId`
- **鉴权**：需登录（记操作审计日志 `module: camera, action: delete`）
- **行为**：停止该摄像头拉流及关联任务，级联清理相关流会话与数据库记录。

### 1.5 手动触发探活
- **路由**：`POST /api/v1/cameras/:cameraId/probe`
- **鉴权**：需登录
- **响应数据**：返回探活任务已触发并在后台执行（或直接返回 probe 结果）。

---

## 2. WebRTC (WHEP) 流媒体协商接口

### 2.1 WHEP SDP 协商交换
- **路由**：`POST /api/v1/webrtc/whep?cameraId=:cameraId`
- **请求头**：
  - `Content-Type: application/sdp`
  - `Authorization: Bearer <token>`
- **请求体**：浏览器生成的 SDP Offer 字符串
- **响应头**：
  - `Content-Type: application/sdp`
  - `Location: /api/v1/webrtc/whep/:sessionId`
- **响应体**：服务端生成的 SDP Answer 字符串
- **HTTP 状态码**：
  - 成功：`201 Created`
  - 摄像头不存在：`404 Not Found`
  - 协商失败：`500 Internal Server Error`

### 2.2 WHEP 断开会话
- **路由**：`DELETE /api/v1/webrtc/whep/:sessionId`
- **请求头**：
  - `Authorization: Bearer <token>`
- **HTTP 状态码**：`200 OK` (或 `204 No Content`)
- **行为**：销毁对应 PeerConnection，递减对应摄像头的观众计数。

---

## 3. WebSocket 实时事件契约

- **端点**：`GET /api/v1/ws/events`
- **新增事件类型**：
  1. `camera.probe_updated`：
  ```json
  {
    "type": "camera.probe_updated",
    "data": {
      "cameraId": "550e8400-e29b-41d4-a716-446655440000",
      "status": "success",
      "codec": "h264",
      "width": 1920,
      "height": 1080,
      "fps": 25.0,
      "errorCode": ""
    }
  }
  ```
  2. `camera.telemetry` (高频遥测)：
  ```json
  {
    "type": "camera.telemetry",
    "data": {
      "cameraId": "550e8400-e29b-41d4-a716-446655440000",
      "activeTracks": 3,
      "personCount": 2,
      "carCount": 1,
      "motionScore": 0.35,
      "isMotionGated": false
    }
  }
  ```
