# 实施方案清单：实时画面AI检测框与跟踪元数据流转及Canvas2D叠加渲染 (Implementation Plan)

## 1. 实施步骤清单

### 阶段一：共享契约与后端实现 (Backend Domain)
- [ ] **B1. 共享类型与主题定义 (`crates/types`)**：
  - 在 `crates/types/src/event.rs` 中定义 `TOPIC_CAMERA_TRACKS: &str = "camera.tracks"` 并导出；
  - 在 `crates/types/src/detection.rs` 中定义 `TrackDto` 与 `CameraTracksPayload`，保证字段与前端 `TrackedBBox` 一致。
- [ ] **B2. 视口订阅感知能力扩展 (`crates/pipeline`)**：
  - 在 `PipelineManager` 中新增 `has_preview_subscribers(&self, camera_id: &str) -> bool`。
- [ ] **B3. 航迹分发与节流服务实现 (`crates/api`)**：
  - 新增 `crates/api/src/track_service.rs`，实现 `TrackDispatchService`；
  - 订阅 `pipeline.subscribe_analysis_events()`，处理 `PipelineAnalysisEvent::Tracks`；
  - 视口按需检查：无客户端预览时直接丢弃；
  - 节流与边缘清空：单摄像头最小间隔 66ms，目标离开时触发单次清空广播；
  - 在 `crates/api/src/lib.rs` 导出 `TrackDispatchService`。
- [ ] **B4. 应用装配与后台启动 (`crates/app`)**：
  - 在 `crates/app/src/main.rs` 中初始化并启动 `TrackDispatchService.start_worker()`。
- [ ] **B5. 后端单测与集成验证 (`crates/api/tests`)**：
  - 新增 `track_dispatch_tests.rs`，覆盖视口感知、节流限频与目标清空逻辑。

### 阶段二：前端总线与 Canvas 渲染联动 (Frontend Domain)
- [ ] **F1. 前端共享类型与主题定义 (`web/src/types/index.ts`)**：
  - 在 `WS_TOPICS` 中新增 `CAMERA_TRACKS: 'camera.tracks'`；
  - 声明 `CameraTracksPayload` 类型。
- [ ] **F2. 全局轻量航迹状态总线 (`web/src/lib/trackStore.ts`)**：
  - 实现基于观察者模式的 `trackStore` 单例，管理各摄像头最新航迹数据。
- [ ] **F3. 播放器 Canvas 2D 自动接入 (`web/src/features/live/components/LivePlayer.tsx`)**：
  - `LivePlayer` 内部挂载 `trackStore.subscribe(cameraId, ...)`；
  - 保持现有 `requestAnimationFrame` 60fps 绘制循环零重渲染运行。
- [ ] **F4. 实时监控页 WebSocket 数据流转 (`web/src/features/live/LivePage.tsx`)**：
  - 在 WebSocket `onmessage` 监听 `WS_TOPICS.CAMERA_TRACKS`，写入 `trackStore.setTracks`。
- [ ] **F5. 交互式布防画板视觉闭环 (`web/src/features/tasks/components/LiveRulesStudio.tsx`)**：
  - 确保画板在预览状态下自动呈现动态检测框与轨迹。

### 阶段三：端到端质量验证与门禁检查
- [ ] **Q1. 后端代码检查与格式化**：
  - `cargo fmt --all -- --check`
  - `cargo clippy --all-targets -- -D warnings`
  - `cargo test --workspace`
- [ ] **Q2. 前端代码检查与构建验证**：
  - `pnpm lint`
  - `pnpm typecheck`
  - `pnpm build`

---

## 2. 验证命令

```bash
# 后端
cargo test -p api --test track_dispatch_tests
cargo clippy --all-targets -- -D warnings
cargo fmt --all -- --check

# 前端
cd web
pnpm typecheck
pnpm lint
pnpm test
pnpm build
```

---

## 3. 回滚方案

若集成出现性能波动或高频丢包，可通过关闭 `TrackDispatchService` Worker 或前端取消 `trackStore` 订阅快速回退，主视频流与告警通道完全不受影响。
