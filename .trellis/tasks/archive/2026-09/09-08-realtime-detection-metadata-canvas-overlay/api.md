# API 契约：实时画面AI检测框与跟踪元数据流转及Canvas2D叠加渲染

- 任务：.trellis/tasks/09-08-realtime-detection-metadata-canvas-overlay
- 状态：confirmed
- 确认时间：2026-09-10
- 作者：主会话撰写，用户确认

## 1. Scope（目录边界）

| 侧 | 目录 | 规则 |
|---|---|---|
| frontend | `web/src/` | 前端实现域，包含 `lib/trackStore.ts`, `features/live/`, `features/tasks/` |
| backend  | `crates/api/`, `crates/pipeline/`, `crates/app/` | 后端实现域，包含 `TrackDispatchService`, `PipelineManager` |
| shared   | `crates/types/`, `web/src/types/` | 共享类型与主题契约，仅主会话可改 |

## 2. Conventions（全局约定）

- WebSocket 端点：`/api/v1/ws/events?token=<jwt>`
- 消息信封格式：`{ "topic": string, "payload": T, "timestamp": number }`
- 时间格式：13 位 UTC Unix 毫秒整数（Rust 使用 `i64`，TypeScript 使用 `number`）
- 坐标规范：全量归一化为 `[0.0, 1.0]` 浮点区间；BBox 格式采用数组 `[x1, y1, x2, y2]`，节省高频传输带宽。

## 3. Shared models（共享数据模型）

### TrackedBBox
| 字段 | 类型 | 必填 | 说明 |
|---|---|---|---|
| trackId | number (u64) | ✅ | ByteTrack 稳定航迹编号 |
| label | string | ✅ | 目标类别（如 "person", "car"） |
| confidence | number (f32) | ✅ | 识别置信度 (0.0 ~ 1.0) |
| bbox | [number, number, number, number] | ✅ | 归一化坐标 `[x1, y1, x2, y2]` |
| trajectory | [number, number][] | ❌ | 历史底边中心点运动轨迹，可选 |

### CameraTracksPayload
| 字段 | 类型 | 必填 | 说明 |
|---|---|---|---|
| cameraId | string | ✅ | 摄像头 ID（如 "CAM-01"） |
| timestamp | number (i64) | ✅ | 视频帧对应的时间戳 (毫秒) |
| tracks | TrackedBBox[] | ✅ | 当前帧活跃的跟踪目标列表，清空时为 `[]` |

## 4. WebSocket Topics

### TOPIC: `camera.tracks`
- 描述：流式下发单路摄像头的实时 AI 目标检测框与历史航迹。
- 传输频率：按需节流（上限约 15 FPS / 最小间隔 66ms），无预览客户端时完全静默。
- 消息示例：
```json
{
  "topic": "camera.tracks",
  "payload": {
    "cameraId": "CAM-01",
    "timestamp": 1741100000120,
    "tracks": [
      {
        "trackId": 12,
        "label": "person",
        "confidence": 0.89,
        "bbox": [0.15, 0.22, 0.35, 0.68],
        "trajectory": [[0.25, 0.65], [0.25, 0.68]]
      }
    ]
  },
  "timestamp": 1741100000120
}
```

- 目标离开画面时的清空消息示例：
```json
{
  "topic": "camera.tracks",
  "payload": {
    "cameraId": "CAM-01",
    "timestamp": 1741100000800,
    "tracks": []
  },
  "timestamp": 1741100000800
}
```

## 5. Auth / Session

客户端通过 `/api/v1/ws/events?token=<JWT>` 连接后，自动接收已订阅/正在预览的摄像头元数据。

## 6. 变更记录

| 日期 | 变更内容 | 状态 |
|---|---|---|
| 2026-09-10 | 初始草案生成，定义 `camera.tracks` 主题及 `CameraTracksPayload` 数据结构 | confirmed |
