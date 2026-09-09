# Implementation Plan: 分析任务运行时协调与双流资源生命周期

## 1. 任务概述

本任务在 `crates/pipeline` 中实现 `TaskRuntimeCoordinator` 协调器，提供按摄像头隔离的端到端运行时装配，保证主码流 RingBuffer、子码流驱动泵、硬件解码器、InferenceWorker 和 StreamHub 订阅的原子启停、幂等与故障反向回滚。同时建立统一分析事件出口通道。

---

## 2. 实施步骤与检查点

### Step 1: 分析事件出口模型定义与 PipelineManager 广播通道接入
- [ ] 在 `crates/pipeline/src/manager.rs` 中定义 `PipelineAlarmEvent`、`PipelineTrackEvent` 与 `PipelineAnalysisEvent` 枚举。
- [ ] 在 `PipelineManager` 中新增 `analysis_event_tx: tokio::sync::broadcast::Sender<PipelineAnalysisEvent>`，容量设为 1024。
- [ ] 提供订阅方法 `pub fn subscribe_analysis_events(&self) -> tokio::sync::broadcast::Receiver<PipelineAnalysisEvent>`。
- [ ] 在 `pump.rs` 的推理后处理循环中：
  - 航迹更新后构造 `PipelineTrackEvent` 并向 `analysis_event_tx` 广播；
  - 规则引擎触发告警且快照落地后构造 `PipelineAlarmEvent` 并向 `analysis_event_tx` 广播。

### Step 2: 协调器参数与实体结构定义
- [ ] 创建 `crates/pipeline/src/coordinator.rs`。
- [ ] 定义 `StartCameraPipelineParams`：
  - 包含 `camera_id`、主/子码流 URL 及 Codec、`TransportPolicy`、`algorithm_id`、`algo_params`、`target_fps`、`motion_gate_enabled`。
- [ ] 定义 `ActiveRuntimeEntry`：
  - 持有 `generation`、`main_stream_key`、`main_attach_handle`、`sub_stream_key`、`sub_session`、当前配置及创建时间。
- [ ] 定义 `CoordinatorError`（实现 `thiserror::Error`）：
  - 区分 `InvalidParam`、`StreamHubError`、`AlgorithmNotFound`、`InstanceCreationFailed`、`PipelineError` 等。

### Step 3: 启动点火与原子回滚实现
- [ ] 在 `coordinator.rs` 中实现 `TaskRuntimeCoordinator::start_camera_pipeline`：
  - 步骤 1：参数校验；
  - 步骤 2：读写锁保护下的幂等性检查；
  - 步骤 3：`stream_hub.subscribe` 主码流；
  - 步骤 4：`pipeline_mgr.attach_main_stream` 启动 RingBuffer 写入；
  - 步骤 5：`stream_hub.get_or_create_session` 获取子码流并设置 `ai_task_enabled = true`；
  - 步骤 6：`media::create_decoder` 构建视频解码器；
  - 步骤 7：通过 `tokio::task::spawn_blocking` 调用算法包 `create_instance` 并构建 `InferenceWorker`；
  - 步骤 8：调用 `pipeline_mgr.start_analysis_pump_with_worker`；
  - 步骤 9：组装 `ActiveRuntimeEntry` 并持久化到映射表。
- [ ] 严格反向回滚（Rollback Stack）：任一步发生错误时，逆序执行释放，确保不残留孤儿 Handle、泄漏的 StreamHub 订阅或内存。

### Step 4: 优雅停止与生命周期查询
- [ ] 实现 `TaskRuntimeCoordinator::stop_camera_pipeline(&self, camera_id: &str)`：
  - 移除运行时 entry；
  - 停止子流驱动泵并等待 Worker 退出；
  - 恢复子流 `ai_task_enabled = false`；
  - 取消并 `await` 主码流 attach handle；
  - 主动 `stream_hub.unsubscribe(main_key).await`；
  - 标记 AI 状态为 false 并释放 idle 解码器。
- [ ] 实现状态查询与指标接口：
  - `is_pipeline_running(&self, camera_id: &str) -> bool`
  - `get_runtime_info(&self, camera_id: &str) -> Option<RuntimeInfo>`
  - `stop_all(&self)`

### Step 5: 模块导出与集成测试
- [ ] 在 `crates/pipeline/src/lib.rs` 中导出 `coordinator` 相关类型。
- [ ] 在 `crates/pipeline/tests/` 中编写集成测试 `coordinator_lifecycle_tests.rs`：
  - 测试 1：双流启动与停止全周期，验证 RingBuffer 包接收与 pump 指标增长；
  - 测试 2：启动失败注入（如非法算法ID）的回滚与 0 泄漏断言；
  - 测试 3：重复启动幂等性；
  - 测试 4：事件订阅通道能接收到实时航迹与告警。

---

## 3. 验证与门禁命令

```bash
cargo fmt --all -- --check
cargo clippy --all-targets -- -D warnings
cargo test -p pipeline
```
