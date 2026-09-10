# 技术方案设计：实时画面AI检测框与跟踪元数据流转及Canvas2D叠加渲染 (Design)

## 1. 架构与目录边界 (Architecture & Boundaries)

本任务打通后端高频航迹追踪元数据到前端播放器 Canvas 2D 叠加渲染链路，划分为前后端独立域及共享契约：

| 侧 | 目录 / 包 | 职责范围 |
|---|---|---|
| **backend** | `crates/types` | 声明 WebSocket 主题常量 `TOPIC_CAMERA_TRACKS = "camera.tracks"` 及传输 DTO |
| **backend** | `crates/pipeline` | `PipelineManager` 提供 `has_preview_subscribers` 视口感知查询能力 |
| **backend** | `crates/api` | 新增 `TrackDispatchService`：订阅航迹事件、10~15 FPS 节流、空帧过渡优化、视口感知按需广播 |
| **backend** | `crates/app` | `main.rs` 启动时注册并启动 `TrackDispatchService` 后台常驻 Worker |
| **frontend** | `web/src/lib/trackStore.ts` | 极轻量全局航迹订阅总线，维护各摄像头最新追踪框，解耦 React 组件渲染周期 |
| **frontend** | `web/src/features/live/` | `LivePlayer.tsx` 自动接入 `trackStore`，通过现有 rAF 60fps 引擎绘制识别框与轨迹 |
| **frontend** | `web/src/features/live/LivePage.tsx` | 监听 `camera.tracks` 主题，零 State 开销转存至 `trackStore` |
| **frontend** | `web/src/features/tasks/` | `LiveRulesStudio.tsx` 挂载 `<LivePlayer />` 自动呈现动态识别框，实现规则标定所见即所得 |

---

## 2. 数据流与时序设计 (Data Flow & Sequence)

```text
[VPU 硬解码 + NPU 推理 + ByteTrack 航迹跟踪]
                  │
                  ▼
         crates/pipeline/pump.rs
                  │ (每帧推理产出 PipelineTrackEvent)
                  ▼
      PipelineManager.publish_analysis_event(Tracks)
                  │ (tokio::broadcast 无阻塞发布)
                  ▼
      crates/api/TrackDispatchService
                  ├── 1. 检查视口活跃度：pipeline.has_preview_subscribers(camera_id)
                  │      └─ false: 立即丢弃，零序列化与零广播开销
                  ├── 2. 状态机与节流控制：
                  │      ├─ 非空帧：限制最小发送间隔 66ms (~15 FPS)
                  │      └─ 空帧过渡：检测框刚离开时立即下发一次空 tracks: []，随后抑制
                  └── 3. 序列化为 WsBroadcastEvent { topic: "camera.tracks", payload }
                  │
                  ▼ (WebSocket: /api/v1/ws/events)
        前端 WebSocket 接收端 (LivePage / App)
                  │ (零 useState 触发，避免高频 DOM 重排)
                  ▼
            trackStore.setTracks(cameraId, tracks)
                  │ (直接更新内部 Ref)
                  ▼
       LivePlayer.tsx (Canvas 2D 离屏绘制引擎)
                  │ (requestAnimationFrame 60fps 独立渲染循环)
                  ▼
      Canvas: 发光矩形框 + 置信度胶囊 + 历史运动尾迹
```

---

## 3. 关键机制与权衡 (Trade-offs & Mechanics)

### 3.1 零负载视口感知 (Zero-Load when Idle)
- **机制**：利用 `PipelineManager` 已有的 `preview_count: AtomicUsize`。客户端每次建立 WebCodecs 或 FLV 播放时 `increment_preview`，断开连接时 `decrement_preview`。
- **收益**：即便后台有 8 路摄像头在执行常驻 AI 分析，若当前用户未在网页端打开实时预览（`preview_count == 0`），后端完全跳过 Track JSON 序列化与 WebSocket 分发，网络与序列化开销为 0。

### 3.2 动态节流与空帧边缘触发 (Throttling & Edge-triggered Clear)
- **节流**：推理帧率通常为 20~25 FPS，而人眼对检测框更新在 15 FPS 已非常平滑。节流至 ~15 FPS（66ms 最小间隔）减少约 40% 的 WebSocket 小包压力。
- **边缘清空**：若直接丢弃所有超出节流周期的帧，当画面中目标消失时可能残留最后一帧的幽灵框。因此引入边缘触发逻辑：上一帧有目标、当前帧无目标时，**强制立即发送一次 `tracks: []`**，确保旧框瞬间消失；随后如果持续无目标，则进入静默抑制状态。

### 3.3 前端零 React 响应式开销 (Zero-React-Re-render Overlay)
- **原则**：严禁在 `LivePage` 或 `LivePlayer` 中使用 `useState(tracks)`。高频 15fps 的 state 变化会导致 React 组件树每秒重渲染数十次，导致视频播放卡顿与 CPU 飙高。
- **方案**：引入极轻量 `trackStore` 单例，内部使用 `Map<string, TrackedBBox[]>` 与 Set 观察者模式。`LivePlayer` 内部原有的 `requestAnimationFrame` (60fps) 直接读取 Ref 中的最新数据，实现真正的零 React 组件重渲染、硬件加速 Canvas 2D 平滑重绘。

---

## 4. 兼容性与迁移 (Compatibility)

- `LivePlayer.tsx` 现有 `trackedObjects?: TrackedBBox[]` 接口完全保留，若外部显式传入 props 则优先使用，未传入时自动从 `trackStore` 订阅实时数据。
- 坐标规范统一使用 `[x1, y1, x2, y2]` 归一化浮点数 `[0.0, 1.0]`，与 `LivePlayer` 现有的画框逻辑 100% 对齐。
