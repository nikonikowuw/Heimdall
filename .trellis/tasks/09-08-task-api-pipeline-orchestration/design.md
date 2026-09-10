# 技术方案设计：任务 API 启停编排与状态同步 (design.md)

## 1. Context & Goals (背景与目标)

### 1.1 问题与现状分析
在完成子任务 `09-08-task-algo-binding-contract` 与 `09-08-task-runtime-lifecycle-coordinator` 之后，系统已具备以下底层能力：
1. **数据与契约层**：`analysis_tasks` 与 `algorithm_instances` 拥有原子双写、状态码 (`TaskStatus`) 与错误信息持久化机制；
2. **运行时协调层**：`TaskRuntimeCoordinator` 提供了摄像机级的双流生命周期管理、主码流 RingBuffer 挂载、子码流 Pump/Decoder/InferenceWorker 启动、回滚与取消保护。

然而，目前 API 控制平面（`crates/api/src/routes/task.rs`）仍然处于早期占位状态：
- **未真正点火启动管线**：`update_task` 仅调用了 `state.pipeline.set_camera_rules` 和 `state.pipeline.set_ai_active`，仅在内存中翻转布尔标记，从未触发 `TaskRuntimeCoordinator` 启动硬件解码和分析驱动泵；
- **存在状态“假激活”风险**：当用户请求 `desiredEnabled: true` 时，无论底层流媒体是否可达、算法是否正常初始化，数据库与返回值均停留在初始状态或未反映真实运行时状况；若启动异常，用户无法获取诊断信息；
- **删除顺序倒置**：当前 `delete_task` 先执行数据库记录删除，再调用管线清理；如果管线清理阻塞或崩溃，可能残留活动解码线程与流订阅，形成孤儿资源；
- **缺乏可注入测试机制**：API 层测试直接依赖完整环境，缺乏面向运行时协调器的 Mock 隔离机制，无法在无真实 RTSP 摄像头与无物理 NPU/VPU 硬件的环境下验证端到端启停与异常降级分支。

### 1.2 核心目标
1. **控制平面唯一收敛**：Task API 成为任务配置与生命周期控制的唯一入口，不再允许前端或上层越过 Task API 直接操作实例或硬件句柄；
2. **点火与优雅启停编排**：当 `desiredEnabled = true` 时，自动解析摄像头码流与编码格式并调用 `TaskRuntimeCoordinator` 启动双流分析；当 `desiredEnabled = false` 时平稳停止并释放资源；
3. **真实状态双向同步与防假激活**：启动成功后原子更新 `Task` 与 `AlgorithmInstance` 为 `Running(2)`；启动失败时保留用户的 `desired_enabled = true` 意图，将实际状态置为 `Error(5)` 并写入诊断错误信息，杜绝假激活；
4. **安全级联删除**：严格遵循“先停运行时与流订阅，再删除数据库记录”的时序，并保证幂等性；
5. **可注入 Mock 架构**：通过抽象 `TaskRuntimeService` Trait，使 `AppState` 支持注入 `MockTaskRuntimeService`，提供高覆盖率的 API 自动化集成测试。

---

## 2. Architecture & Boundaries (架构分层与依赖边界)

### 2.1 依赖关系与分层定位
```text
┌────────────────────────────────────────────────────────┐
│                   Web Frontend / Client                │
└───────────────────────────┬────────────────────────────┘
                            │ HTTP /api/v1/tasks
                            ▼
┌────────────────────────────────────────────────────────┐
│               crates/api (Control Plane)               │
│  - routes/task.rs: Handler (参数提取、DTO 映射、服务编排)  │
│  - state.rs: AppState (持有 Arc<dyn TaskRuntimeService>)│
└─────────────┬────────────────────────────┬─────────────┘
              │ 业务/运行时编排             │ 状态与配置持久化
              ▼                            ▼
┌───────────────────────────────┐ ┌──────────────────────┐
│  crates/pipeline              │ │  crates/db           │
│  - TaskRuntimeCoordinator     │ │  - TaskRepo          │
│  - PipelineManager            │ │  - CameraRepo        │
│  - TaskRuntimeService Trait   │ │  - AlgorithmRepo     │
└─────────────┬─────────────────┘ └──────────────────────┘
              │ 设备侧流媒体与推理驱动
              ▼
┌───────────────────────────────┐
│  crates/media & crates/infer  │
│  (StreamHub, Decoders, NPU)   │
└───────────────────────────────┘
```

- **单向依赖原则**：`crates/api` 依赖 `crates/pipeline`、`crates/db`、`crates/media`、`crates/types`。底层 crate 严禁反向依赖 `crates/api`；
- **极薄 Handler 契约**：Handler 严禁直接调用 FFI、硬件 SDK 或创建解码器；所有的音视频与硬件交互完全封装在 `pipeline` 的协调器内部；
- **仓储事务边界**：涉及 `analysis_tasks` 与 `algorithm_instances` 的双写与双删逻辑完全收敛在 `db::TaskRepo` 内部事务中，API 层不裸露 SQL 或 DSL。

---

## 3. Core Data Flow & State Machine (核心数据流与状态机)

### 3.1 任务状态流转定义
沿用 `types::TaskStatus` 定义的标准状态码：
- `Stopped (0)`：任务已停止，无活跃解码与推理线程；
- `Starting (1)`：正在连接流媒体并初始化解码器与推理实例（过渡态）；
- `Running (2)`：分析驱动泵常驻运行，持续接收子码流帧并执行推理；
- `Degraded (3)`：因过热或系统限流进入降频/降级状态；
- `Reconnecting (4)`：网络流异常断开，媒体层重连中；
- `Error (5)`：启动失败或严重故障，保留 desired 意图，提供诊断信息。

### 3.2 PUT /api/v1/tasks/{cameraId} 执行时序 (点火与重配置)

```text
Client                  routes/task.rs           TaskRepo             CameraRepo        TaskRuntimeCoordinator
  │                          │                      │                     │                     │
  │── PUT /tasks/{camId} ───►│                      │                     │                     │
  │   (desiredEnabled=true)  │                      │                     │                     │
  │                          │── 1. 校验输入入参 ───│                     │                     │
  │                          │── 2. 默认算法补齐 ───│                     │                     │
  │                          │                      │                     │                     │
  │                          │── 3. 原子保存意图 ──►│                     │                     │
  │                          │   (save_task_and_sync_instance)            │                     │
  │                          │                      │                     │                     │
  │                          │── 4. 更新几何规则 ──────────────────────────────────────────────►│ (PipelineManager)
  │                          │                      │                     │                     │
  │                          │── 5. 查询摄像头 ──────────────────────────►│                     │
  │                          │◄─ (rtsp_url, codec) ───────────────────────│                     │
  │                          │                      │                     │                     │
  │                          │── 6. 检查现有运行并停止旧配置 (若有变更) ────────────────────────►│
  │                          │                      │                     │                     │
  │                          │── 7. 启动管线点火 ──────────────────────────────────────────────►│
  │                          │   (start_camera_pipeline)                  │                     │
  │                          │                      │                     │                     │
  │                          │   [ 分支 A: 启动成功 ]                     │                     │
  │                          │◄── Ok(generation) ───────────────────────────────────────────────│
  │                          │── 8a. 更新状态为 Running ────────────────►│ (update_status)      │
  │                          │── 9a. set_ai_active(true) ──────────────────────────────────────►│
  │◄─ 200 OK (Running) ──────│                      │                     │                     │
  │                          │                      │                     │                     │
  │                          │   [ 分支 B: 启动失败 ]                     │                     │
  │                          │◄── Err(coordinator_err) ─────────────────────────────────────────│
  │                          │── 8b. 更新状态为 Error + 诊断信息 ───────►│ (update_status)      │
  │                          │── 9b. set_ai_active(false) ─────────────────────────────────────►│
  │◄─ 200 OK (Error) ────────│ (保留 desiredEnabled=true, actualStatus=5) │                     │
```

#### 关键分支决策细节：
1. **默认算法自动回退**：仅当 `desiredEnabled=true` 且未显式传递 `algorithmId` 时，从当前已注册且数据库存在的可运行算法包中选择；若无可用算法，返回 `400 Bad Request`，不写入任务意图。`desiredEnabled=false` 时保留已有算法绑定，不凭空选择算法；
2. **子码流 RTSP 推导回退**：
   - 优先使用 `camera.sub_rtsp_url`；
   - 若为空，调用 `media::deduce_primary_sub_stream(&camera.rtsp_url)` 进行推导；
   - 若无法推导，则直接使用主码流 `camera.rtsp_url` 回退，确保各类单流摄像头均可运行；
3. **编码格式映射**：
   - 解析 `camera.last_codec`，包含 `"265"` 则映射为 `CodecType::H265`，否则默认 `CodecType::H264`；
4. **幂等与变更检测**：
   - 若 coordinator 报告当前已有该摄像头的 active runtime：
     - 比对运行参数（算法 ID、FPS、RTSP 地址、参数）；
     - 若参数一致，直接保持当前运行并返回，实现幂等；
     - 若关键参数发生变更，先调用 `coordinator.stop_camera_pipeline(&camera_id)` 安全停止旧运行时，再重新启动新配置；
5. **停用分支 (`desired_enabled == false`)**：
   - 调用 `coordinator.stop_camera_pipeline(&camera_id)` 停止驱动泵与流订阅；
   - 调用 `pipeline.set_ai_active(&camera_id, false)`；
   - 调用 `TaskRepo::update_status(&state.db, &camera_id, TaskStatus::Stopped.as_i32(), "")` 同步 Stopped 状态。

### 3.3 DELETE /api/v1/tasks/{cameraId} 执行时序 (安全级联删除)

```text
Client                  routes/task.rs        TaskRuntimeCoordinator     PipelineManager       TaskRepo
  │                          │                         │                        │                 │
  │── DELETE /tasks/{camId} ─►│                         │                        │                 │
  │                          │── 1. 停止分析运行时 ───►│                        │                 │
  │                          │   (stop_camera_pipeline)│                        │                 │
  │                          │◄── Ok (幂等无害) ───────│                        │                 │
  │                          │                         │                        │                 │
  │                          │── 2. 清理管线与上下文 ──────────────────────────►│                 │
  │                          │   (stop_task & set_camera_rules & set_ai_active) │                 │
  │                          │                         │                        │                 │
  │                          │── 3. 删除数据库记录 ─────────────────────────────────────────────►│
  │                          │   (delete_task_and_instance)                     │                 │
  │                          │◄── rows_affected ──────────────────────────────────────────────────│
  │                          │                         │                        │                 │
  │                          │ [ rows_affected == 0 ]  │                        │                 │
  │◄─ 404 Not Found ─────────│                         │                        │                 │
  │                          │ [ rows_affected > 0 ]   │                        │                 │
  │◄─ 200 OK ────────────────│                         │                        │                 │
```
- **核心约束**：先释放底层资源，再删除数据库实体。即使任务已在停止状态，`stop_camera_pipeline` 具备完全幂等性，保证在任何状态下删除均不会引发悬挂线程或流泄漏。

---

## 4. Detailed Module & Interface Design (接口与模块详细设计)

### 4.1 运行时服务抽象 (`TaskRuntimeService`)
在 `crates/pipeline/src/coordinator.rs` 中定义抽象接口，实现控制层与运行时实现的解耦：

```rust
#[async_trait::async_trait]
pub trait TaskRuntimeService: Send + Sync {
    /// 启动单路摄像机分析管线
    async fn start_camera_pipeline(
        &self,
        params: StartCameraPipelineParams,
    ) -> Result<u64, CoordinatorError>;

    /// 停止单路摄像机分析管线
    async fn stop_camera_pipeline(&self, camera_id: &str) -> Result<bool, CoordinatorError>;

    /// 停止所有活跃的分析管线
    async fn stop_all(&self) -> Result<(), CoordinatorError>;

    /// 查询单路摄像机运行时概要
    async fn get_runtime_info(&self, camera_id: &str) -> Option<CameraPipelineRuntimeInfo>;

    /// 列出所有摄像机的运行时概要
    async fn list_runtime_infos(&self) -> Vec<CameraPipelineRuntimeInfo>;

    /// 检查指定摄像机是否有活跃运行时
    async fn has_active_runtime(&self, camera_id: &str) -> bool;
}

#[async_trait::async_trait]
impl TaskRuntimeService for TaskRuntimeCoordinator {
    async fn start_camera_pipeline(
        &self,
        params: StartCameraPipelineParams,
    ) -> Result<u64, CoordinatorError> {
        self.start_camera_pipeline(params).await
    }

    async fn stop_camera_pipeline(&self, camera_id: &str) -> Result<bool, CoordinatorError> {
        self.stop_camera_pipeline(camera_id).await
    }

    async fn stop_all(&self) -> Result<(), CoordinatorError> {
        self.stop_all().await
    }

    async fn get_runtime_info(&self, camera_id: &str) -> Option<CameraPipelineRuntimeInfo> {
        self.get_runtime_info(camera_id).await
    }

    async fn list_runtime_infos(&self) -> Vec<CameraPipelineRuntimeInfo> {
        self.list_runtime_infos().await
    }

    async fn has_active_runtime(&self, camera_id: &str) -> bool {
        self.has_active_runtime(camera_id).await
    }
}
```

### 4.2 AppState 装配扩展 (`crates/api/src/state.rs`)
在 `AppState` 中注入 `Arc<dyn TaskRuntimeService>`：

```rust
pub struct AppState {
    pub db: DatabaseConnection,
    pub pipeline: Arc<PipelineManager>,
    pub stream_hub: Arc<StreamHub>,
    pub algo_registry: Arc<AlgoRegistry>,
    pub task_coordinator: Arc<dyn pipeline::TaskRuntimeService>,
    // ...
}
```
- `AppState::new_with_limit` 默认组装基于实装 `TaskRuntimeCoordinator` 的实例；
- 扩展 `with_task_coordinator(mut self, coordinator: Arc<dyn TaskRuntimeService>) -> Self` 构造器，便于单元与 API 集成测试中注入 Mock。

### 4.3 Mock 服务实现与测试隔离
提供可在单元/集成测试中复用的 `MockTaskRuntimeService`：
- 能够记录 `start_camera_pipeline` 和 `stop_camera_pipeline` 的调用参数与次数；
- 支持预设特定摄像机启动成功（返回 generation）或模拟各类错误（如参数错误、算法未找到、流媒体连接超时等）；
- 允许动态修改 `has_active_runtime` 和 `get_runtime_info` 返回值以配合查询接口测试。

---

## 5. Concurrency & Thread Safety (并发安全与锁设计)

1. **Camera 级生命周期互斥锁**：
   - `TaskRuntimeCoordinator` 内部维护了 `operation_locks: HashMap<String, Arc<TokioMutex<()>>>`；
   - 针对同一 `camera_id` 的并发 PUT 请求，会在 coordinator 内部按 camera 串行排队执行，彻底杜绝并发点火产生双重解码器或竞争性 Worker；
2. **无锁化 Handler**：
   - HTTP Handler 中不创建、不持有任何阻塞型 `std::sync::Mutex` 或 `RwLock`，所有异步流程仅使用 Tokio 的非阻塞通信与无锁异步接口；
3. **数据库事务与短锁作用域**：
   - 任务配置持久化与状态更新完全交由 `TaskRepo` 内部的 SeaORM 事务（`db.transaction`）完成，短事务迅速释放，不横跨耗时的高延迟媒体/网络 I/O。

---

## 6. Error Handling & HTTP Status Code Mapping (异常处理与状态映射)

| 场景 | 错误原因 | DB Task 状态 | DB Instance 状态 | HTTP Status | Response Data |
| :--- | :--- | :--- | :--- | :--- | :--- |
| **参数不合法** | analysisFps < 0 或 > 60、algoParams 非对象 | 不入库 | 不入库 | `400 Bad Request` | `null` |
| **显式算法不存在** | algorithmId 在 algorithms 表中查无记录 | 不入库 | 不入库 | `404 Not Found` | `null` |
| **无可用默认算法** | 启用请求未指定算法且没有已注册可运行算法 | 不入库 | 不入库 | `400 Bad Request` | `null` |
| **摄像头不存在** | camera_id 在 cameras 表中不存在 | 不入库 | 不入库 | `404 Not Found` | `null` |
| **流连接/解码失败** | coordinator.start_camera_pipeline 抛出错误 | 意图入库，actual=5 | 意图入库，actual=5 | `200 OK` | `TaskConfigDto (actualStatus=5, statusMessage="启动分析管线失败: ...")` |
| **正常启用成功** | coordinator 成功启动 pump 与 RingBuffer | actual=2 | actual=2 | `200 OK` | `TaskConfigDto (actualStatus=2, statusMessage="")` |
| **正常停用成功** | desiredEnabled=false 成功释放运行时 | actual=0 | actual=0 | `200 OK` | `TaskConfigDto (actualStatus=0, statusMessage="")` |
| **删除不存在任务** | 删除时数据库中无对应 camera_id | 无变化 | 无变化 | `404 Not Found` | `null` |
| **正常删除任务** | 成功停止运行时并删除任务及其实例 | 物理删除 | 物理删除 | `200 OK` | `null (success envelope)` |

> 注：依据 Nuwa 设计哲学，用户保存任务时若配置参数合法，系统应持久化用户的“期望配置”（`desired_enabled = true`）。若因网络断开、摄像头离线或硬件瓶颈导致无法立即启动，系统不应粗暴抛出 500 导致用户配置丢失，而是应将实际运行状态置为 `Error(5)` 并附带诊断说明，使前端可以明确呈现告警并等待重试或冷启动恢复自愈。

---

## 7. Verification & Testing Plan (验证与测试方案)

### 7.1 测试矩阵与用例清单
在 `crates/api/src/routes/task.rs` 与集成测试中覆盖以下场景：

1. **`test_task_create_and_enable_pipeline_success`**：
   - 插入摄像头与算法数据；
   - `PUT /api/v1/tasks/{cameraId}` 设置 `desiredEnabled: true`；
   - 验证 mock coordinator 接收到正确的 URL、编码与算法参数；
   - 验证返回 DTO 中 `actualStatus == 2 (Running)`，且数据库中 `actual_status == 2`。
2. **`test_task_enable_pipeline_failure_preserves_desired_records_error`**：
   - 模拟 coordinator 返回 `CoordinatorError::Pipeline` 异常；
   - 验证返回 DTO 中 `desiredEnabled == true`，`actualStatus == 5 (Error)`，`statusMessage` 包含错误说明；
   - 验证数据库中 Task 与 AlgorithmInstance 同步记录 Error，杜绝假激活。
3. **`test_task_disable_stops_pipeline_and_sets_stopped`**：
   - 对已运行的任务调用 `PUT` 设置 `desiredEnabled: false`；
   - 验证 mock coordinator 的 `stop_camera_pipeline` 被成功调用；
   - 验证返回 DTO 中 `actualStatus == 0 (Stopped)`，数据库状态同步为 0。
4. **`test_task_idempotent_enable`**：
   - 重复两次提交完全相同的启用配置；
   - 验证 coordinator 识别为相同配置，不触发重复启动或停止；
   - 再修改 FPS，验证旧运行时被回收后启动新配置。
5. **停止失败保护**：
   - 模拟 coordinator 停止失败；
   - 验证任务实际状态为 `Error(5)`，仍保留活跃运行时，不伪造 `Stopped`。
6. **`test_task_delete_stops_pipeline_before_db_removal`**：
   - 发送 `DELETE /api/v1/tasks/{cameraId}`；
   - 验证 mock coordinator 的 `stop_camera_pipeline` 在数据库删除操作前被调用；
   - 再次发送 DELETE 验证返回 404 Not Found。

### 7.2 质量门禁执行命令
```bash
cargo fmt --all -- --check
cargo clippy -p api --all-targets -- -D warnings
cargo test -p api
cargo test --workspace
```
