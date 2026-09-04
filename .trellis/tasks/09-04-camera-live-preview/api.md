# Camera & Live Preview API Contract (接口契约与数据模型)

所有业务接口统一基于 `/api/v1`，遵循统一 JSON 信封格式（实时流媒体传输端点除外）：
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
      "subRtspUrl": "rtsp://192.168.1.100:554/live/ch1",
      "remark": "主要出入口",
      "transportPolicy": "auto",
      "lastProbeStatus": "healthy",
      "lastProbeAt": 1788508800000,
      "lastProbeErrorCode": "",
      "lastSuccessAt": 1788508800000,
      "lastCodec": "h264",
      "lastWidth": 1920,
      "lastHeight": 1080,
      "lastFps": 25.0,
      "createdAt": 1788508800000,
      "updatedAt": 1788508800000
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
- **行为**：数据入库，`cameraId` 自动生成 UUID，初始 `lastProbeStatus = "never"`，自动尝试推导子码流并在后台触发异步探活。

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
- **响应数据**：触发后台探活并通过 WebSocket 广播结果。

### 1.6 子码流规则自动推导
- **路由**：`POST /api/v1/cameras/deduce-substream`
- **鉴权**：需登录
- **请求体**：
```json
{
  "rtspUrl": "rtsp://admin:123456@192.168.1.100:554/h264/ch1/main/av_stream"
}
```
- **响应数据**：
```json
{
  "code": 0,
  "message": "success",
  "data": {
    "primarySubUrl": "rtsp://admin:123456@192.168.1.100:554/h264/ch1/sub/av_stream",
    "candidates": [
      {
        "vendor": "海康威视 (Hikvision)",
        "subUrl": "rtsp://admin:123456@192.168.1.100:554/h264/ch1/sub/av_stream",
        "description": "标准 H.264/H.265 子码流"
      }
    ]
  },
  "timestamp": 1788508800000
}
```

---

## 2. Enhanced FLV / MSE 实时流媒体接口

### 2.1 HTTP-FLV 实时流分发 (Chunked Transfer)
- **路由**：`GET /api/v1/live/:camera_id` (或 `/:camera_id/flv`)
- **Query 参数**：
  - `stream`: 可选 `"main"` (默认主流) 或 `"sub"` (子码流)
  - `token`: 可选 JWT Token 认证（若无法设置 HTTP Header 时可用）
- **响应头**：
  - `Content-Type: video/x-flv`
  - `Transfer-Encoding: chunked`
- **协议支持**：
  - H.264: `AVCDecoderConfigurationRecord` + FLV Video Tag
  - H.265 (HEVC): **Enhanced FLV (FourCC `hvc1`)** `HEVCDecoderConfigurationRecord` + ExVideoTag
  - 秒开注入：首次连接注入 Sequence Header + 完整 GOP 关键帧序列。

### 2.2 WS-FLV WebSocket 实时流通道
- **路由**：`GET /api/v1/live/:camera_id/ws`
- **Query 参数**：同上
- **行为**：建立 WebSocket 连接并以二进制帧形式推送 FLV Header 与视频 Tag。

---

## 3. WebSocket 实时事件契约

- **端点**：`GET /api/v1/ws/events`
- **广播事件**：
  1. `camera.probe_updated`：
  ```json
  {
    "topic": "camera.probe_updated",
    "payload": {
      "cameraId": "550e8400-e29b-41d4-a716-446655440000",
      "status": "healthy",
      "codec": "h264",
      "width": 1920,
      "height": 1080,
      "fps": 25.0,
      "errorCode": ""
    },
    "timestamp": 1788508800000
  }
  ```
  2. `camera.telemetry` (高频遥测)：
  ```json
  {
    "topic": "camera.telemetry",
    "payload": {
      "cameraId": "550e8400-e29b-41d4-a716-446655440000",
      "activeTracks": 3,
      "personCount": 2,
      "carCount": 1,
      "motionScore": 0.35,
      "isMotionGated": false
    },
    "timestamp": 1788508800000
  }
  ```
