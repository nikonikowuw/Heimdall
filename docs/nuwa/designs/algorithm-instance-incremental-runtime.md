# Design: 算法实例增量运行时与两阶段配置提交 (Incremental Algorithm Instance Runtime)

> **状态**: Implemented（后端 Phase 1-4、前端收敛状态展示与整体下发的乐观并发保护均已落地）
> **目标**: 将算法实例的参数、启停、增删与模型替换从摄像头整路管线生命周期中解耦。
> **关联规范**: [全局约定](../guides/conventions.md)、[架构概览](../guides/architecture-overview.md)、[并发模型](../backend/concurrency-guidelines.md)、[数据库规范](../backend/database-guidelines.md)、[API 规范](../backend/api-guidelines.md)、[算法 SDK](../backend/algo-sdk-guidelines.md)、[媒体管线](../backend/media-pipeline.md)

---

## 1. 背景与结论

当前任务同步以完整的 `StartCameraPipelineParams` 为比较和启动单位。算法实例集合变化时，`TaskRuntimeCoordinator` 会停止并重新启动整路摄像头分析管线；`AnalysisPump::start_multi_worker` 在创建时一次性构造所有 `DecodeSlot`、`ControlSlot` 和推理循环。

这保证了全量配置快照的一致性，但把以下两种不同级别的变化混为一谈：

```text
摄像头管线变化：连接、解码器、码流能力、共享帧格式
算法实例变化：参数、抽帧频率、Worker、模型和实例启停
```

本设计采用以下结论：

1. 添加或取消一个算法，只增删对应的算法实例槽和 Worker，不重启解码器、摄像头连接或其他算法 Worker。
2. `analysisFps` 通过分析泵的抽帧 governor 更新，不重建 Worker。
3. 置信度、类别、NMS 和算法私有阈值优先调用 Worker 所属 OS 线程内的 `instance_update_config` 热更新；插件不支持时，仅替换目标实例 Worker。
4. 模型路径、算法版本和算法私有输入预处理变化，默认只替换目标实例 Worker；只有改变共享解码器能力或全局帧契约时才重建摄像头分析管线。
5. “应用参数”持久化用户期望配置并立即尝试运行时生效；任务“保存配置”继续负责任务级结构配置和实例集合事务保存。
6. 运行时结果必须显式返回 `applied`、`pending` 或 `failed`，不能因为数据库写入成功就伪装成算法已经生效。

这里的“两阶段提交”是“数据库期望配置提交 + 运行时收敛”的应用层协议，不是要求 SQLite 与 NPU 参与严格的分布式两阶段提交。

---

## 2. 当前实现与差异

### 2.1 已有能力

当前代码已经具备增量设计所需的一部分基础：

- `AnalysisPump` 将解码循环的 `DecodeSlot` 与控制面的 `ControlSlot` 分开管理。
- `ControlSlot` 持有算法标识、配置 JSON、`InferenceWorkerHandle`、托管 `InferenceWorker`、推理任务句柄和指标。
- `PipelineManager::replace_pump_worker_for_algo` 可以替换指定算法的 Worker，不要求重启解码器。
- `InferenceWorker` 使用专用 OS 线程和单线程 Tokio runtime，使 NPU/FFI session 绑定在创建线程内。
- C ABI 已有 `instance_update_config`，算法 SDK 的 `AlgoPlugin::update_config` 已预留动态配置入口。
- `algorithm_instances` 已有 `actual_status` 和 `status_message`，可作为运行状态展示基础。

对应实现入口：

- [`AnalysisPump`](../../../crates/pipeline/src/pump.rs)
- [`PipelineManager`](../../../crates/pipeline/src/manager.rs)
- [`InferenceWorker`](../../../crates/infer/src/worker.rs)
- [`InferenceBackend`](../../../crates/infer/src/backend.rs)
- [`AlgoPlugin`](../../../crates/algo-sdk/src/plugin.rs)
- [`algorithm_instances` Model](../../../crates/db/src/entity/algorithm_instance.rs)

### 2.2 当前缺口

- `SharedControlSlots` 使用初始化时建立的 `Vec<ControlSlot>`，没有运行时 `add` / `remove`。
- 解码循环持有本地 `DecodeSlot` 集合，没有接收动态拓扑变更命令的协议。
- `InferenceBackend` 尚未暴露实例级配置更新方法。
- Coordinator 主要提供整路启动、停止和完整参数比较，实例级路由最终仍调用 `sync_pipeline_for_camera`，从而触发全量同步。
- 前端 `activeInstances` 仍有以 `algorithmId` 作为 key 的路径，不能保证同一实例的独立生命周期语义；运行时 API 必须以 `instanceId` 为唯一身份。
- 数据库同时存在任务级规则/门控和算法实例规则/门控字段，存在双来源风险；目标设计必须指定唯一运行时来源。

---

## 3. 目标架构

### 3.1 运行时分层

```text
CameraRuntime
├── CameraStreamSession / StreamHub 订阅
├── VideoDecoder
├── 共享原生 FrameRef 分发
└── AnalysisPump
    ├── DecodeSlot(instanceId = A)
    ├── DecodeSlot(instanceId = B)
    ├── ControlSlot(instanceId = A)
    │   └── InferenceWorker A / 推理循环 A
    └── ControlSlot(instanceId = B)
        └── InferenceWorker B / 推理循环 B
```

`CameraRuntime` 负责摄像头连接、分析码流和共享解码资源。`AlgorithmInstanceSlot` 负责单个实例的抽帧、Worker、算法配置、代数和运行状态。

算法实例的增删、参数更新和模型替换不能触碰 `CameraStreamSession` 或 `VideoDecoder`，除非变更明确要求重新协商共享帧能力。

### 3.2 实例身份

所有控制、持久化和前端状态以 `instanceId` 为主键：

- `algorithmId` 表示算法包身份，用于加载包和展示名称。
- `instanceId` 表示任务中某一次算法实例绑定，用于更新、删除、状态和结果隔离。
- 当前数据库仍约束同一任务下不能重复挂载相同 `algorithmId`；该约束不改变 `instanceId` 作为运行时主键的要求。

`algorithmId` 不得作为跨摄像头或跨任务的 Worker 控制 key。

### 3.3 分析泵控制面

为 `AnalysisPump` 增加有界控制命令通道或等价的单一变更入口。命令由摄像头运行时控制器串行提交，在解码帧边界处理：

```rust
pub enum PumpCommand {
    AddInstance(PreparedInstance),
    RemoveInstance { instance_id: String },
    SetEnabled { instance_id: String, enabled: bool },
    SetAnalysisFps { instance_id: String, target_fps: u32 },
    UpdateWorkerConfig { instance_id: String, config_json: String },
    ReplaceWorker(PreparedReplacement),
}
```

以上代码是协议示意，不代表立即新增同名公共类型。具体实现必须满足：

- 控制通道有固定容量；满载时返回 `pending` 或明确的忙错误，不能静默丢失配置命令。
- 同一实例的连续参数更新可以合并为最新 revision，但旧请求必须得到可解释的 `pending` 或 `failed` 结果。
- 控制命令只在帧边界切换抽帧槽和结果代数，避免半帧状态。
- 不在持有 `ControlSlot` 锁时执行 FFI、Worker 创建、Worker 关闭或 `.await`。

---

## 4. 实例生命周期

### 4.1 添加算法实例

```text
校验算法与参数
    |
    v
获取算力租约并准备 Worker（专用阻塞线程）
    |
    +-- 失败 -> 不影响现有实例，返回 failed
    |
    v
SQLite 事务写入实例期望配置和 revision
    |
    v
在下一帧边界加入 DecodeSlot、ControlSlot 和推理循环
    |
    v
更新 appliedRevision，返回 applied
```

新增 Worker 默认采用“先准备、后挂载”的方式，避免新算法初始化失败时破坏正在运行的实例。准备阶段需要临时 NPU/硬件资源；资源不足时保留旧运行时，不强行停止其他算法。

如果摄像头任务尚未启动，新增实例只需持久化期望状态，运行时状态为 `stopped/applied`；任务启动时再按照同一恢复流程创建 Worker。

### 4.2 禁用算法实例

禁用与删除分开处理。禁用保留参数、规则和实例 ID，只停止对应推理：

```text
停止向目标 DecodeSlot 投递新帧
    |
    v
在帧边界标记 draining，阻止旧结果继续发射
    |
    v
等待目标推理循环退出
    |
    v
在阻塞线程关闭 Worker，释放实例租约
    |
    v
更新 enabled=false、appliedRevision 和运行状态
```

其他算法、解码器和摄像头连接保持运行。若目标 Worker 关闭超时，必须隔离挂死线程并返回 `failed` 或 `pending`，不能阻塞整路管线的其他实例。

### 4.3 删除算法实例

删除属于结构性变更，仍必须通过任务/实例 Repository 的 SQLite 事务完成。推荐使用两步语义：

1. 运行时先将目标槽置为 `draining`，停止产生新结果。
2. 数据库事务删除实例关系；提交成功后完成 Worker 关闭和槽位移除。
3. 数据库事务失败时恢复目标槽的接收状态。
4. 若删除已提交但 Worker 关闭超时，目标槽继续保持不可发帧的隔离状态，后台按超时策略回收；服务重启后不会根据已删除记录恢复它。

如果产品交互只需要暂时取消分析，前端应调用禁用接口，而不是删除实例。

### 4.4 Worker 替换

模型路径、算法版本或算法私有输入格式变化使用目标实例级替换：

```text
读取旧实例 effective generation
    |
    v
创建新 Worker 和新 session
    |
    +-- 失败 -> 保留旧 Worker，状态 failed
    |
    v
在帧边界替换 worker_holder 并递增 generation
    |
    v
旧推理循环停止接收新帧
    |
    v
专用阻塞线程关闭旧 Worker
```

替换成功前旧 Worker 继续提供服务。若平台无法同时保留新旧 Worker 的硬件资源，不得默认停止旧 Worker 后再盲目创建新 Worker；应返回明确的容量失败，或进入目标实例级维护状态，不能影响其他实例。

替换会重置目标算法的插件私有状态。旧 generation 的检测结果必须被丢弃，不能混入新 Worker 的结果；其他算法的 generation 和跟踪状态不受影响。

---

## 5. 参数更新分类

| 变更 | 运行时动作 | 是否重建整路摄像头管线 |
| --- | --- | --- |
| `analysisFps` | 在 `DecodeSlot` 帧边界更新 governor | 否 |
| 置信度、目标类别、NMS、算法阈值 | Worker 线程调用 `update_config`；不支持时替换目标 Worker | 否 |
| 告警标签、时序阈值 | 优先热更新算法实例或目标规则状态 | 否 |
| 模型路径、模型版本 | 准备并替换目标 Worker | 否 |
| 算法私有预处理尺寸/色彩格式 | 替换目标 Worker，由算法包按 `FrameRef` 自行预处理 | 否 |
| 添加算法实例 | 新增一个实例槽和 Worker | 否 |
| 禁用算法实例 | 只停止目标实例 | 否 |
| 删除算法实例 | 只移除目标实例槽和 Worker | 否 |
| 摄像头 URL、码流来源、解码器类型 | 重建摄像头/媒体运行时 | 通常是 |
| 共享 `FrameRef` 能力、全局解码格式或 stride 契约 | 重新协商解码管线 | 是 |
| 主/子码流分析模式 | 按媒体管线能力重新配置，必要时重建解码路径 | 可能是 |

“输入格式变化”必须区分算法私有预处理和共享解码器输入契约。宿主不为所有算法强制一个全局模型输入尺寸；算法包负责从原生 `FrameRef` 选择自己的预处理尺寸、裁切、色彩转换和归一化。

---

## 6. Worker 控制协议

### 6.1 热更新边界

`InferenceBackend` 增加可选的实例配置更新能力，默认返回“不支持”，由 `InferenceWorker` 将控制消息投递到自身 OS 线程：

```text
API / Coordinator
    -> 有界 WorkerControl 通道
        -> Worker OS 线程
            -> AlgoPlugin::update_config
                -> C ABI instance_update_config
```

更新调用必须在该 Worker 的硬件上下文所属线程执行。不能从 Tokio worker、API handler 或其他算法线程直接调用平台 SDK/FFI。

热更新成功后，新的配置从下一个安全帧边界开始生效。更新失败时，Worker 必须继续使用原 effective 配置，不能处于半更新状态。

### 6.2 不支持热更新时的回退

插件返回 `NotImplemented` 或能力声明表明配置需要重建时：

1. 保留旧 Worker 和旧 effective 配置。
2. 根据新配置准备新 Worker。
3. 准备成功后在帧边界替换目标槽。
4. 准备失败则保留旧 Worker，并将期望配置标为 `failed`。

“不支持原地更新”不是失败本身；只要目标实例级 Worker 替换成功，最终状态仍为 `applied`。

### 6.3 `analysisFps`

`analysisFps` 不属于插件 session 配置，直接作用于 `DecodeSlot` 的 `AnalysisFpsGovernor`。变更只需在下一帧边界替换 governor 参数，不创建或销毁 `InferenceWorker`。

FPS 的范围和默认值继续遵循 `TaskAlgorithmInstanceConfig` 的现有校验与归一化规则，不在运行时设计中重新定义 `0` 的语义。

---

## 7. 两阶段配置提交与状态模型

### 7.1 期望配置与实际配置

数据库中的实例参数代表用户期望配置，不代表当前 Worker 已经使用该配置。运行时维护目标实例的 effective snapshot；数据库只需持久化 revision 和结果状态即可在重启后恢复。

建议为 `algorithm_instances` 增加前向迁移字段：

```sql
ALTER TABLE algorithm_instances
    ADD COLUMN desired_revision INTEGER NOT NULL DEFAULT 0;

ALTER TABLE algorithm_instances
    ADD COLUMN applied_revision INTEGER NOT NULL DEFAULT 0;

ALTER TABLE algorithm_instances
    ADD COLUMN runtime_apply_state INTEGER NOT NULL DEFAULT 0;
```

`runtime_apply_state` 使用显式稳定值：

```text
0 = applied
1 = pending
2 = failed
```

已有 `actual_status` 继续表示算法实例的健康生命周期（`Stopped`、`Starting`、`Running`、`Degraded`、`Error`），不与配置应用状态混用。`status_message` 保存可展示的失败或等待原因。

### 7.2 应用参数流程

```text
阶段 A：持久化期望配置
  1. 按 instanceId 读取实例
  2. 校验参数 Object、FPS、算法能力和大小上限
  3. 单一 SQLite 事务写入参数并 desiredRevision += 1
  4. 状态置为 pending

阶段 B：运行时应用
  5. Coordinator 串行处理该 camera/instance 的 revision
  6. 热更新或准备目标 Worker 替换
  7. 在帧边界安装配置/新 Worker
  8. 成功后写 appliedRevision = desiredRevision、state = applied
  9. 失败后保留旧 effective Worker、state = failed、记录原因
```

阶段 A 成功而阶段 B 失败时，数据库保留用户最后一次期望配置，运行时继续使用旧配置。刷新或服务重启后，启动恢复流程检测 `desiredRevision != appliedRevision`，再次尝试应用，而不是丢失用户设置。

如果运行时操作过程中用户提交了更高 revision，旧操作完成后不得覆盖新配置；旧结果只能标记为过时，Coordinator 必须继续处理最新 revision。

### 7.3 API 返回状态

实例级写接口返回统一 API envelope，`data` 至少包含：

```json
{
  "instanceId": "inst-01",
  "desiredRevision": 12,
  "appliedRevision": 12,
  "applyState": "applied",
  "actualStatus": "running",
  "statusMessage": "",
  "effectiveAtMs": 1788825600000
}
```

状态语义：

| `applyState` | 含义 | 前端行为 |
| --- | --- | --- |
| `applied` | 期望 revision 已在目标运行时生效 | 更新本地资源缓存，关闭参数提交中状态 |
| `pending` | 已持久化，正在排队、创建 Worker 或等待安全帧边界 | 显示处理中，订阅/轮询实例状态 |
| `failed` | 期望配置已持久化，但目标 Worker 未能使用它 | 保留表单值，展示原因，提供重试；不能显示为成功 |

传输请求成功不等于 `applyState=applied`。客户端必须按状态字段分支，不根据 HTTP message 文本判断。

---

## 8. API 与前端职责

### 8.1 写路径只有任务级保存

实例级写接口（`POST/PUT/DELETE /api/v1/tasks/instances*`、`PUT .../enabled`、`GET .../runtime`）已移除。所有实例变更——新增、改参、启停、删除——都表现为「整份任务配置一次下发」：

`PUT /api/v1/tasks/{cameraId}` 通过 `TaskRepo::save_task_with_instances` 在单个 SQLite 事务中保存任务名称、启停意图、实例集合（含各自 `analysisFps`/`algoParams`/`enabled`）、任务级规则与运动门控。

选择这个形状的理由：

- 算法实例的唯一权威副本是 `algorithm_instances` 行，不存在「任务级配置」与「实例级配置」两份数据；
- 整体下发不产生额外代价——未变更实例在 Coordinator 的逐实例 diff 中判定为 `Noop`，不重建 Worker、不重新申请租约、不重启解码器；
- 请求体大小与实例数量线性相关，仅数十 KB 量级，不构成瓶颈；真正的开销在「收到报文后乱重启」，而这已被 diff 消除。

因此控制面的不变量是：**任何写接口都只提交期望配置，收敛统一由 `sync_pipeline_for_camera` 从已提交的持久化配置驱动**。禁止任何入口用请求体临时拼装 `StartCameraPipelineParams`。

### 8.2 收敛结果的返回位置

没有实例级查询接口，收敛状态随任务资源一起返回：

- `PUT /api/v1/tasks/{cameraId}` 与 `GET /api/v1/tasks/{cameraId}` 的 `algorithmInstances[]` 逐实例携带 `instanceId`、`desiredRevision`、`appliedRevision`、`applyState`、`statusMessage`、`enabled`、`actualStatus`；
- `GET /api/v1/tasks` 的摘要同样携带 `applyState`，供任务卡片展示未生效实例。

写入成功与生效成功必须分开看：`applyState` 才是判据，HTTP 200 只代表期望配置已持久化。

### 8.3 整体下发的并发保护（已实现）

整体下发是「整个数组覆盖」，两个客户端同时编辑同一任务时会丢失更新：A 删掉实例 X 后，B 用不含 X 的旧快照保存，X 会重新出现；A 改过的参数同理被静默回退。

防护采用**任务级配置版本号**（ETag / `If-Match` 语义），而不是逐实例 `desiredRevision` 校验。理由是整体下发的正确性单位就是「整份配置」：实例集合的增删、任务级规则与门控都在同一次写入中覆盖，仅校验实例自身的代际无法发现「对方删除了一个本次请求里根本不存在的实例」。

- **版本号**：`analysis_tasks.config_revision`（V16 迁移，整数，新任务从 1 起）；
- **递增时机**：配置写入一律 +1——`save_task_with_instances_txn`（唯一面向客户端的整体下发入口，重复提交同一份内容也会推进，因为它同样是「一次覆盖写」）、`update_instance_and_sync_task`、`delete_instance_and_sync_task`。运行时簿记（`update_task_runtime_state`、`update_status`、`sync_task_actual_status_txn`、`mark_apply_*`）一律不递增，否则状态轮询会让客户端在途保存永远冲突；
- **协议**：`GET /tasks/{cameraId}` 与 `GET /tasks` 返回 `configRevision`；`PUT /tasks/{cameraId}` 可携带 `configRevision`，服务端在事务内比对，不匹配则拒绝写入并返回 **HTTP 409 + 业务码 40903**，响应 `data` 为 `null`（不回传伪快照）；
- **省略即不校验**：`configRevision` 可选，未携带时按旧行为写入。这是对脚本与旧客户端的兼容路径，也意味着它只是「配合才能生效」的防护，不是强一致约束；
- **任务已被删除**：客户端携带非 0 版本号但库中无该任务时按冲突处理（`actual = 0`），否则一次删除会被别的会话静默反转成新建；
- **`configRevision: 0` 表示「读取时该通道还没有任务」**：快速创建用它做乐观断言，通道已有任务（版本从 1 起）时被拒绝——`analysis_tasks.camera_id` 是 UNIQUE，并发创建若不拦截会把对方已配置好的实例与防区整个覆盖。

前端恢复动作（工作台）：

- 冲突提示为常驻红色徽标（`SaveFeedback.kind = 'conflict'`），不自动消失，避免用户以为已经保存成功；
- 冲突响应不携带配置快照：API 根信封约定错误时 `data` 为 `null`，恢复所需的最新配置由前端重新 `GET` 获取；
- **载入最新**：重新 `GET` 任务配置，按服务端快照覆盖本地编辑态并刷新版本号（丢弃本地修改）；
- **覆盖保存**：先 `GET` 取回最新版本号，再用本地内容重新下发——先读后写，避免用陈旧版本号反复被拒；
- 任务列表的布防开关同样回传列表快照的 `configRevision`；冲突时静默丢弃本次开关动作并刷新列表，不弹错误，因为用户并没有在编辑配置；
- 快速创建模态框的冲突提示单独成句（「该通道已被其他会话创建了 AI 任务，请刷新列表后重试」），不把通用的「请载入最新配置后重试」透给用户——创建场景没有可载入的配置。

### 8.4 前端状态（已实现）

判定逻辑收敛在纯函数 [`applyState.ts`](../../../web/src/features/tasks/applyState.ts)：`summarizeInstanceApply` 把逐实例 `applyState` 归纳为「已生效 / 排队中 / 未生效」三态 + 未生效明细。判定边界与服务端一致——停用实例不参与判定，缺失 `applyState` 视为已生效，避免把未知状态渲染成故障。

- `LiveRulesStudio`：保存后用**响应中的逐实例收敛结果**决定反馈，不再无条件显示"已保存并生效"。
  - `applied`：绿色瞬时提示，3 秒后自动消失；
  - `pending` / `failed`：琥珀 / 红色常驻提示，附服务端原因（悬停逐行展示）与「重试」按钮——重试即重新下发同一份期望配置触发再次收敛；
  - 响应未携带逐实例状态时保持沉默，不宣称已生效；
  - 进入工作台时即根据加载到的任务配置恢复上次的未收敛态势，不必再保存一次才发现问题。
- `TaskCameraCard`：与运行状态正交地展示 `配置排队中` / `配置未生效` 轻量徽标（仅未生效时出现，悬停给出原因），日常已生效状态不产生噪声。
- `activeInstances` 以 `algorithmId` 索引本地编辑态，落库后以响应中的 `instanceId` 为准做展示与键值。
- 所有可见状态与错误文案进入 i18n；服务端原因作为数据逐行展示，不在组件内拼装面向用户的句子。

---

## 9. 规则和门控字段归属

当前实例表包含 `rules_json` 和 `motion_gate_json`，任务表也有对应任务级字段。目标设计必须选定唯一 canonical source：

- 全局任务 ROI/Mask/Line 和摄像头级运动门控归任务配置，由任务级保存提交。
- 算法私有阈值、类别和时序参数归算法实例 `params_json`，由实例级 Apply 提交。
- 如果继续兼容实例表中的规则/门控列，迁移期间只能作为镜像字段，在同一事务内同步；运行时只能读取一份规范化后的配置。

规则和门控的运行时更新同样应在帧边界切换，不能在空间规则判定过程中修改共享对象。

---

## 10. 并发、资源与结果隔离

1. 每个摄像头的拓扑操作按顺序执行；不同摄像头之间可以并行。
2. 不持有数据库连接、实例槽锁或泵锁执行 FFI、Worker 创建、Worker 关闭或 `.await`。
3. 所有 Worker 创建和销毁都在专用 OS 线程或 `spawn_blocking` 路径执行，不能运行在 Tokio worker。
4. 新 Worker 先获取硬件资源租约；资源不足时只拒绝目标实例操作，不驱逐其他实例。
5. 添加、替换和删除都递增目标实例 generation。检测结果携带 generation，消费端丢弃旧 generation 的迟到结果。
6. 帧通道、控制通道和事件缓冲均有固定容量；帧路径优先丢弃旧帧，控制命令不得静默丢弃。
7. 常驻推理仍沿 `FrameRef` 传递原生设备 buffer。算法实例增量变更不允许引入 CPU 像素拷贝、CPU 色彩转换或软解。
8. Worker 停机遵循超时和隔离策略；硬件挂死时不能无限等待，也不能提前释放仍被使用的句柄。

推荐指标：

```text
algorithm_instance_apply_total{state,operation}
algorithm_instance_apply_latency_ms{operation}
algorithm_instance_pending_count
algorithm_instance_generation
algorithm_instance_worker_replace_total{result}
algorithm_instance_stale_result_total
algorithm_instance_worker_shutdown_timeout_total
```

日志至少包含 `camera_id`、`instance_id`、`algorithm_id`、`desired_revision`、`applied_revision`、`generation` 和 `operation`，不得记录算法参数中的敏感值或明文凭证。

---

## 11. 故障与恢复策略

| 场景 | 期望配置 | 运行时动作 | 结果 |
| --- | --- | --- | --- |
| 参数校验失败 | 不写入 | 不操作 Worker | HTTP 4xx，旧配置继续运行 |
| SQLite 事务失败 | 不变 | 不操作 Worker | 返回数据库错误 |
| 热更新成功 | 已提交 | 当前 Worker 在帧边界切换 | `applied` |
| 插件返回 NotImplemented | 已提交 | 准备目标实例替换 Worker | 替换成功则 `applied` |
| 新 Worker 创建失败 | 已提交 | 保留旧 Worker | `failed`，旧配置继续运行 |
| 添加实例资源不足 | 已提交或等待准入 | 不影响已有实例 | `pending` 或 `failed` |
| 删除时 Worker 关闭超时 | 已删除 | 目标槽禁止发帧并隔离回收 | `pending/failed`，其他实例继续运行 |
| 服务在运行时安装后崩溃 | 已提交 | 重启比较 desired/applied revision | 自动重试，避免丢配置 |
| 旧 revision 晚于新 revision 完成 | 新配置已提交 | 丢弃旧结果，继续处理新 revision | 新 revision 最终决定状态 |

运行时状态写入失败不能回滚已经发生的硬件操作。恢复逻辑必须是幂等的：重复执行同一 revision 不得创建重复 Worker、重复实例槽或重复告警。

---

## 12. 实施阶段

### Phase 0：身份和状态基础

- 前端、API、Pipeline 控制路径统一使用 `instanceId`。
- 增加 desired/applied revision 与运行时应用状态迁移。
- 保留现有全量启动路径作为冷启动和回退路径。
- 为 `AlgorithmInstanceDto` 增加应用状态字段。

### Phase 1：Worker 配置控制

- 为 `InferenceBackend` 和 Worker 增加配置更新控制消息。
- 在固定 OS 线程调用 `AlgoPlugin::update_config` / `instance_update_config`。
- 区分热更新成功、NotImplemented、FFI 错误和超时。
- 完成单 Worker 热更新和目标 Worker 替换测试。

### Phase 2：AnalysisPump 动态槽位

- 将 `ControlSlot` 和 `DecodeSlot` 的寻址从 `algorithm_id` 迁移到 `instance_id`。
- 增加有界 `add/remove/set_fps` 控制协议。
- 在帧边界处理槽位变更和 generation fence。
- 保持解码器和其他实例不变，增加资源释放和超时隔离测试。

### Phase 3：Coordinator 增量同步

- 将完整配置比较改为媒体配置比较 + 实例集合 diff。
- 复用已有 `replace_worker_with_owner` 实现目标实例级模型替换。
- `sync_pipeline_for_camera` 改为调用增量同步服务。
- 只有媒体连接、解码能力或全局帧契约变化才调用整路重建。

### Phase 4：API 与前端交互

- 实例级 Apply 返回 `applied/pending/failed`。
- 参数抽屉移除对任务保存的隐式依赖。
- 任务结构保存显示未保存状态，并防止旧参数覆盖新 revision。
- 增加刷新、重启、热更新失败和重试的用户可观察测试。

---

## 13. 验收标准

1. 摄像头已有算法 A 运行时添加算法 B，A 的 Worker、解码器、PTS 连续性和告警状态不被重启影响。
2. 删除或禁用 B 时，A 持续产生检测结果；B 的迟到结果全部被 generation fence 丢弃。
3. 修改 `analysisFps` 不创建或销毁 Worker，新的采样频率在后续帧边界生效。
4. 热参数更新由固定 Worker 线程执行；插件不支持热更新时只替换目标 Worker。
5. 新模型加载失败时旧 Worker 继续运行，API 返回 `failed`，数据库保留用户期望配置。
6. 任务结构保存仍由单个 SQLite 事务提交，失败时不会留下孤儿实例或半个实例集合。
7. 服务重启后，`desiredRevision != appliedRevision` 的实例自动进入恢复流程并可重试。
8. 任一算法实例的 FFI、Worker 或关闭超时不会阻塞其他实例、媒体连接或 Tokio worker。
9. 常驻推理路径没有因动态管理引入 CPU 像素拷贝、软解或无界队列。
10. API、WebSocket 和前端类型均以 `instanceId`、13 位 UTC 毫秒和统一状态守卫传递运行时状态。

---

## 14. 决策与实现落点

### 14.1 已定决策

- **单一收敛入口**：任务级保存与实例级写接口都只提交期望配置到数据库，收敛统一走 `sync_pipeline_for_camera`——媒体契约与实例集合都从已提交的持久化配置解析。禁止任何入口用请求体临时拼装 `StartCameraPipelineParams`，否则同一路管线会因媒体签名不一致（如请求体未携带 `motionGate` 而库里是默认值）被判定为「媒体变更」而整路重建，这正是增量设计要消除的行为。
- **不做 manifest 能力声明**：统一先尝试 `instance_update_config`，插件返回 `NotImplemented` / `AV_ERR_NOT_IMPLEMENTED` 时降级为「只替换目标实例 Worker」。避免为每个参数维护额外元数据。
- **按 `instanceId` 寻址**：抽帧槽、控制槽、租约与 Worker 句柄均以实例 ID 为键；`algorithmId` 只用于加载包与展示。数据库唯一约束暂不放开「同一任务多实例同算法」，待编辑器与约束同步调整后再放开。
- **资源不足不驱逐其他实例**：新 Worker 分配失败时旧 Worker 继续服务，目标实例返回 `failed` 并附 `status_message`；不做「杀掉旧实例腾内存」的激进策略。
- **规则/门控归属**：任务级 ROI/Mask/Line 与摄像头级运动门控为 canonical source，由任务级保存提交；实例表同名列为迁移期镜像字段，运行时不读取。
- **并发保护用任务级版本号而非逐实例代际**：整体下发的正确性单位是整份配置（含实例集合的增删），只有任务级单数字能覆盖全部覆盖写字段；逐实例校验会漏判「对方删除了本次请求里不存在的实例」。

### 14.2 实现落点

| 层 | 落点 | 关键契约 |
| --- | --- | --- |
| DB | `V15__algorithm_instance_apply_revision.sql`、`V16__task_config_revision.sql`、`AlgorithmInstanceRepo::{mark_apply_applied, mark_apply_pending, mark_apply_failed, list_unapplied}` | `desired_revision` / `applied_revision` / `runtime_apply_state`；回写幂等且不倒退；`analysis_tasks.config_revision` 仅由配置写入推进 |
| infer | `InferenceBackend::update_config`、`InferError::Unsupported`、`WorkerControl::UpdateConfig`、`InferenceWorkerHandle::update_config` | 热更新在有界控制通道内执行，1s 客户端超时熔断 |
| pump | `PumpCommand`（Add/Remove/SetAnalysisFps）、`DecodeSlot.target_fps`、`ControlSlot.{target_fps, worker_generation}`、`AnalysisPump::{add,remove,set_fps,replace}_instance*` | 拓扑变更只在解码帧边界生效；Worker 替换后旧代际结果计入 `stale_results` 并丢弃 |
| coordinator | `MediaContractSignature`、`InstanceDesiredConfig`、`InstanceApplyOutcome`、`TaskRuntimeCoordinator::{apply_instance_config, remove_instance_runtime, sync_camera_instances, requires_media_restart}` | 实例集合变更走增量；仅媒体契约变化才整路重建 |
| api | `sync_pipeline_for_camera`（返回 `Option<CameraInstanceSyncOutcome>`）是唯一收敛入口，唯一写接口 `PUT /tasks/{cameraId}` 与任务查询都经由它；`TaskConfigDto.configRevision` / `TaskSummaryDto.configRevision`；`task_runtime_updates`、`persist_instance_outcomes`、`resolve_outcome_revision`；`DbError::RevisionConflict` → `ApiError::ConfigRevisionConflict`（409 / 40903） | 任务响应逐实例返回 `desiredRevision` / `appliedRevision` / `applyState` / `statusMessage`，未收敛不伪装成功；代际记 0（集合 diff 语义）时按库中当前期望代际收敛 |
| 测试 | `pipeline/tests/{pump_incremental_instance_tests,coordinator_incremental_tests}.rs`、`api/tests/instance_apply_state_tests.rs`、`db/tests/task_repo_tests.rs`、`web/src/features/tasks/taskDraft.test.ts`、`web/src/lib/api.test.ts` | 增删/改帧率不重启解码器、代际栅栏丢弃迟到结果、失败如实回写、陈旧版本 409 且不落库、运行时簿记不推进版本号、创建断言与冲突识别 |

### 14.3 已知取舍

- `algorithm_instances.status_message` 同时承载实例健康文案（「运行中」「等待运行时挂载该算法实例」）与收敛失败原因，两者共用一列。控制面与前端必须以 `applyState` 为生效判据，`statusMessage` 只作为补充说明；若后续需要严格区分，应拆分 `apply_message` 列而不是继续复用。
- 实例级 HTTP 接口已全部移除，只保留任务级保存；`TaskRepo` 层保留的实例级写方法（`update_instance_and_sync_task` / `set_instance_enabled_and_sync_task` / `delete_instance_and_sync_task` / `add_instance_to_task`）当前只被数据库层测试使用，如需彻底清理应连同测试一起评估。
- 任务级版本号与实例级 `desiredRevision` 是两套独立编号：前者管「谁基于最新配置写入」，后者管「运行时是否用上了期望配置」。二者语义不同但不冲突，不要把 `configRevision` 当作实例收敛状态的判据。
- `configRevision` 可省略即不校验，因此并发保护只对「配合的客户端」生效；若将来要强制，需要在 API 层区分内部调用与用户客户端（例如独立的一组内部写接口）。

### 14.4 仍待处理

- 实例表 `rules_json` / `motion_gate_json` 镜像列的迁移删除时机。
- 重启恢复流程是否需要在 `app` 冷启动对账中显式消费 `list_unapplied`。
