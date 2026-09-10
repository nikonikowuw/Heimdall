# PRD: 任务算法包绑定与分析驱动泵启动调度 (Task-Algo Binding & Sub-Stream Analysis Pump Scheduling)

## 1. Goal

打通业务任务配置（Task）、算法包运行时注册表（`AlgoRegistry`）与底层多媒体分析驱动泵（`SubStreamAnalysisPump`）之间的业务装配鸿沟。实现用户在 Web 控制台启用布防任务时，系统自动拉取对应算法包、初始化专用常驻推理线程、拉起子码流解码器并驱动分析泵常驻运行，在任务禁用或删除时安全优雅回收所有硬件与线程资源。

---

## 2. Background & Problem Statement

当前系统底层虽然已完整实现了 `SubStreamAnalysisPump`、`InferenceWorker` 与 `AlgoRegistry`，并且单测能驱动前向推理，但在上层 API 业务装配中存在关键断点：
1. **API 层未真正“点火”启动驱动泵**：
   - `crates/api/src/routes/task.rs` 中的 `update_task` 仅调用了 `state.pipeline.set_camera_rules(...)` 和 `state.pipeline.set_ai_active(...)`，修改了原子布尔值，却从未调用 `pipeline.start_analysis_pump_with_worker`；
   - 生产环境中没有真实的数据流被解码并送入推理引擎。
2. **任务与算法包松散脱节**：
   - 前端任务布防仅传递了几何规则（ROI/Line/Mask）与移动侦测参数，未明确绑定具体的算法包（`algorithm_id`）；
   - 数据库中 `tasks` 表与 `algorithm_instances` 表未完全联动，缺少统一的运行时编排通道。
3. **冷启动自愈缺环**：
   - 系统重启后，已持久化为 `desired_enabled = true` 的布防任务未能自动恢复运行，处于假激活状态。
4. **主码流 RingBuffer 未挂载**：
   - `attach_main_stream` 从未被业务代码调用，导致告警触发时无法从主码流缓存中提取关键帧。

---

## 3. Detailed Requirements

### R1. 任务数据模型与接口扩展 (`crates/types`, `crates/db`, `crates/api`)
- **R1.1 算法绑定属性扩展**：
  - 在 `TaskConfigDto` 与数据库实体中增加 `algorithm_id: Option<String>`、`analysis_fps: Option<i32>` 与 `algo_params: Option<serde_json::Value>`；
  - 当用户未显式指定 `algorithm_id` 时，系统自动回退并匹配当前已激活的通用检测算法（如 `general_detection`）。
- **R1.2 任务与算法实例（`AlgorithmInstance`）状态同步**：
  - 保存任务时，同步在 `algorithm_instances` 表创建或更新对应记录，统一维护 `actual_status`（运行中/已停止/异常）。

### R2. 业务管线启动与生命周期装配 (`crates/api/src/routes/task.rs`)
- **R2.1 任务启动与点火装配 (`start_camera_pipeline`)**：
  - 当任务设为 `desired_enabled = true` 时：
    1. **解析码流与编解码器**：从数据库查询摄像头主/子码流 RTSP 地址与编码格式（H.264/H.265）；
    2. **挂载主码流 RingBuffer**：通过 `StreamHub` 订阅主码流数据包，调用 `pipeline.attach_main_stream`，为按需高清抓拍提供 NALU 环形缓冲；
    3. **装配算法包与常驻推理线程**：从 `state.algo_registry` 获取激活的算法包实例，调用 `pkg.create_instance(cam_id, config)` 并包装为 `InferenceWorker::new(Arc::new(inst))`；
    4. **获取子码流会话并创建硬件解码器**：通过 `StreamHub` 获取或订阅子码流会话（`CameraStreamSession`），通过 `media::create_decoder` 构建硬件解码器；
    5. **启动分析驱动泵**：调用 `pipeline.start_analysis_pump_with_worker(...)`，传入配置参数与采样率；
    6. 同步更新任务与实例的 `actual_status = 1`（运行中）。
- **R2.2 任务停止与优雅资源释放 (`stop_camera_pipeline`)**：
  - 当任务设为 `desired_enabled = false` 或删除任务时：
    1. 调用 `pipeline.stop_analysis_pump(&camera_id).await`，平稳退出解码与推理循环；
    2. 释放主码流 RingBuffer 订阅；
    3. 同步更新数据库 `actual_status = 0`（已停止）。

### R3. 服务冷启动任务自愈恢复 (`crates/app/src/reconcile.rs` / `crates/app/src/main.rs`)
- **R3.1 激活任务自愈扫描**：
  - 在系统启动、算法包装载至 `AlgoRegistry` 完成后，自动扫描所有 `desired_enabled = true` 且摄像头状态正常的任务；
  - 自动调用管线点火逻辑，恢复所有分析驱动泵运行，实现断电或重启后的自愈保活。

### R4. 前端交互与状态回显 (`web/src/features/tasks/`)
- **R4.1 算法包选择与配置**：
  - 在 `CreateTaskModal.tsx` 与 `LiveRulesStudio.tsx` 中增加算法包下拉选择组件（联动 `algoApi.list()`），默认选中激活的检测算法；
  - 在任务卡片（`TaskCameraCard.tsx`）上直观展示当前绑定的算法名称、版本、运行帧率及驱动泵实时运行状态胶囊。

---

## 4. Key Files & Architecture Touchpoints

- `crates/types/src/task.rs`：扩展任务配置模型定义
- `crates/db/src/entity/task.rs` / `crates/db/src/repository/task.rs`：字段支持与查询更新
- `crates/api/src/routes/task.rs`：任务启用/禁用逻辑改造，接入驱动泵装配
- `crates/app/src/reconcile.rs`：冷启动任务自动恢复
- `web/src/features/tasks/components/CreateTaskModal.tsx`：增加算法选择器
- `web/src/features/tasks/components/TaskCameraCard.tsx`：算法状态呈现

---

## 5. Acceptance Criteria

1. **接口驱动点火验证**：
   - 通过 Web 页面或调用 `PUT /api/v1/tasks/{cameraId}` 将任务设为 `desiredEnabled: true` 后：
   - 后端日志输出“子码流分析驱动泵启动成功”与“主码流 RingBuffer 已挂载”；
   - `pipeline.is_analysis_pump_running(cameraId)` 确认为 `true`；
   - 驱动泵指标 `frames_decoded` 与 `frames_inferred` 随时间稳定递增。
2. **算法热重载验证**：
   - 在算法页面上传或激活新算法包时，运行中的分析驱动泵能够通过 `reload_algorithm_on_pumps` 优雅无损热替换推理句柄。
3. **安全关停验证**：
   - 停用或删除任务后，驱动泵与常驻推理线程在 1 秒内安全回收，无内存泄漏、无死锁、无文件描述符残留。
4. **测试门禁**：
   - `cargo clippy --all-targets -- -D warnings` 无警告通过；
   - 补充 API 层任务启动驱动泵的端到端集成测试并通过。

---

## 6. 子任务拆解与依赖

本父任务只维护总体需求、跨子任务契约和最终集成验收；具体实现拆分为以下 6 个可独立验收的子任务：

```text
[task-algo-binding-contract]
          │
          ├──────────────┐
          ▼              ▼
[runtime-lifecycle]  [targeted-hot-reload]
          │              │
          ▼              │
[api-pipeline-orchestration]
          │
          ▼
[cold-start-task-recovery]
          │
          └──────────────► [frontend-task-algorithm-status]
```

1. `09-08-task-algo-binding-contract`：任务算法字段、显式状态码、数据库迁移、Task/AlgorithmInstance 一致性和输入校验。
2. `09-08-task-runtime-lifecycle-coordinator`：按摄像头统一管理主/子码流订阅、RingBuffer attach task、decoder、InferenceWorker、pump 启停、回滚和幂等。
3. `09-08-task-api-pipeline-orchestration`：Task API 调用统一运行时，完成启停、删除、状态同步和 API 集成测试。
4. `09-08-task-cold-start-task-recovery`：算法注册完成后的任务扫描、算法回退、健康检查、限并发恢复和失败降级。
5. `09-08-task-targeted-algorithm-hot-reload`：定向算法热替换、Worker owner 生命周期、旧 worker 回收和失败保留旧模型。
6. `09-08-task-frontend-task-algorithm-status`：前端算法选择、FPS/参数配置、运行状态回显和移除 Task/Instance 双写。

依赖规则写在各子任务 PRD 中，父子关系不等同于自动阻塞；每个子任务开始实现前仍需完成自己的设计、API 契约和验证计划。

`09-08-alarm-evidence-persistence-ws-broadcast` 与 `09-08-realtime-detection-metadata-canvas-overlay` 暂作为并行兄弟任务保留，不纳入本父任务的 children。它们依赖运行时层确定统一的分析事件出口：告警任务消费 alarm/snapshot 事件，检测框任务消费 tracked metadata；三者不得各自定义冲突的 pump 输出协议。

### 父任务最终集成验收

- 6 个子任务完成并通过各自门禁。
- 通过真实或可控 RTSP 流验证启用任务后 pump、RingBuffer、解码和推理指标持续运行。
- 验证重复启用、停用、删除、启动失败和进程重启后的资源与状态一致性。
- 验证算法激活只影响绑定该算法的任务，不破坏无关摄像头。
- 通过 Rust workspace、Web lint/typecheck/test/build 以及跨层 API 集成测试。
