# PRD: 实时画面AI检测框与跟踪元数据流转及Canvas2D叠加渲染 (Real-time Detection & Tracking Metadata Stream Overlay)

## 1. Goal

打通后端算法推理/航迹跟踪输出的动态元数据（Bounding Boxes、TrackID、类别置信度、轨迹序列）到前端播放器的低延迟传输链路。激活并闭环前端 `LivePlayer` 已有的 Canvas 2D 高帧率离屏渲染引擎，在实时监控（`LivePage`）与交互布防画板（`LiveRulesStudio`）上无损直观呈现动态 AI 目标框与轨迹尾迹。

---

## 2. Background & Problem Statement

当前系统的实时视觉呈现存在断层：
1. **后端实时元数据被静默丢弃**：
   - `crates/pipeline/src/pump.rs` 中，`pipeline_mgr.process_detections` 吐出的活跃航迹追踪列表 `_tracked` 被直接用下划线丢弃：
     ```rust
     let (_tracked, alarms) = pipeline_mgr_for_infer
         .process_detections(&cam_id_for_infer, detections, timestamp)
         .await;
     ```
   - 没有向任何实时通道（WebSocket / 数据流）推送当前帧的检测目标。
2. **前端渲染组件“空转”**：
   - `web/src/features/live/components/LivePlayer.tsx` 已经用 HTML5 Canvas 2D 和 `requestAnimationFrame` 实现了极高性能的 60fps 识别框、置信度胶囊和历史运动轨迹（Fading Trail）绘制逻辑；
   - 但父组件 `LivePage.tsx` 与 `LiveRulesStudio.tsx` 在挂载 `<LivePlayer />` 时，从未传递 `trackedObjects` 数据；
   - 导致用户在界面上只能看到纯粹的原始视频，看不到任何 AI 识别框，无法验证算法是否正在正常检测。

---

## 3. Detailed Requirements

### R1. 后端检测元数据流转与按需限流推送 (`crates/pipeline`, `crates/api`)
- **R1.1 元数据广播接口与按需节流**：
  - 在 `PipelineManager` 中维护每路摄像头的最新活跃航迹快照（`current_tracks: Arc<RwLock<HashMap<String, Vec<TrackedObject>>>>`）；
  - 当驱动泵完成一帧检测并更新跟踪器后，原子刷新该快照；
  - 针对实时连接的客户端，通过 WebSocket 广播主题 `camera_detections` 周期性（建议 10~15 FPS 节流，避免过多小包拥塞）推送归一化检测坐标：
    ```json
    {
      "topic": "camera_detections",
      "payload": {
        "cameraId": "CAM-01",
        "timestamp": 1757300000120,
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
      "timestamp": 1757300000120
    }
    ```
- **R1.2 视口按需推送优化 (Zero-Load when Idle)**：
  - 仅当某路摄像头当前有活跃的实时预览订阅者（`preview_count > 0`）时才向 WebSocket 广播元数据，无客户端观看时完全关闭网络序列化开销。

### R2. 前端高性能消费与 Canvas 2D 叠加 (`web/src/features/live/`)
- **R2.1 零 React 响应式开销的状态接入**：
  - `LivePage.tsx` 在 WebSocket 消息处理中监听 `camera_detections`；
  - 采用 `useRef<Map<string, TrackedBBox[]>>` 存储各摄像头的最新追踪框，**严禁将高频元数据直接存入 React `useState`**（防止引起高频 DOM 重排与丢帧）；
  - 将 `trackedObjects` 传递给当前播放的 `<LivePlayer />` 实例。
- **R2.2 自适应分辨率与平滑插值渲染**：
  - Canvas 尺寸通过 `ResizeObserver` 动态与视频尺寸像素级对齐；
  - 保持发光边框（青色代表 person，绿色代表 vehicle 等）、置信度胶囊标签与历史移动尾迹的清晰绘制。

### R3. 交互式布防画板视觉闭环 (`web/src/features/tasks/components/LiveRulesStudio.tsx`)
- **R3.1 动态边画规则边看目标穿透**：
  - 在 `LiveRulesStudio.tsx` 布防配置模式下，实时叠加显示 AI 检测框与目标穿透轨迹；
  - 用户绘制 ROI 区域或绊线时，能即时看到检测框是否落在多边形内或穿过绊线，极大提升规则标定的精准度与直观性。

---

## 4. Key Files & Architecture Touchpoints

- `crates/pipeline/src/pump.rs`：抽取 `tracked_objects` 并触发分发
- `crates/pipeline/src/manager.rs`：维护每路摄像头的最新航迹与订阅
- `crates/api/src/state.rs` & `crates/api/src/routes/ws.rs`：节流广播 `camera_detections`
- `web/src/features/live/LivePage.tsx`：订阅元数据并传导给播放器
- `web/src/features/live/components/LivePlayer.tsx`：Canvas 2D 离屏绘制
- `web/src/features/tasks/components/LiveRulesStudio.tsx`：画板叠加实时动态目标

---

## 5. Acceptance Criteria

1. **实时框动态呈现**：
   - 启动布防任务且有目标出现在视频画面中时，前端实时播放器上能清晰看到青色/绿色矩形检测框紧跟目标移动；
   - 目标上方显示 Track ID、类别名称（如 `#12 person`）及置信度百分比。
2. **轨迹线动态延伸**：
   - 目标连续运动时，底部中心点呈现半透明渐变历史尾迹线。
3. **高帧率与极低 CPU 开销**：
   - 前端页面保持 60fps 流畅运行，无卡顿，Chrome DevTools 性能面板中无频繁的 React Commit / Re-render 尖峰。
4. **布防联动所见即所得**：
   - 在 `LiveRulesStudio` 画板中标定规则时，画面同时呈现 AI 动态检测框，便于用户根据目标大小和路径精准调整规则边界。
5. **门禁检查**：
   - `pnpm lint`、`pnpm typecheck`、`cargo clippy` 均无警告通过。
