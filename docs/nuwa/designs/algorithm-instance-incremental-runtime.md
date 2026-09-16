# Design: 算法实例增量运行时与两阶段配置提交 (Incremental Algorithm Instance Runtime)

> **状态**: Draft（架构设计，尚未实现）
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

### 8.1 实例级接口

保留并收敛现有实例资源接口：

- `POST /api/v1/tasks/instances`：新增实例，返回实例身份和运行时应用状态。
- `PUT /api/v1/tasks/instances/{instanceId}`：应用参数、FPS以及需要实例级更新的配置。
- `PUT /api/v1/tasks/instances/{instanceId}/enabled`：立即启用或禁用目标实例。
- `DELETE /api/v1/tasks/instances/{instanceId}`：删除实例并清理目标 Worker。
- `GET /api/v1/tasks/instances/{instanceId}/runtime`：查询期望 revision、实际 revision、应用状态和错误原因。

Handler 只负责参数提取、调用实例服务和 DTO 映射；Worker 准备、运行时 diff、硬件调用和阻塞关闭归 `pipeline`/`infer`。

所有更新必须使用 `instanceId`。请求体中的 `algorithmId` 仅用于新增时选择算法包，不能作为已有实例的更新定位符。

### 8.2 任务级保存

`PUT /api/v1/tasks/{cameraId}` 继续通过 `TaskRepo::save_task_with_instances` 在单个 SQLite 事务中保存：

- 任务名称和任务启停意图；
- 算法实例的加入、移除和绑定关系；
- 任务级规则、运动门控和码流模式；
- 其他会改变任务结构的字段。

任务保存提交后，由 Coordinator 计算实例集合 diff，调用增量 `add/remove/replace/update`；只有媒体输入契约变化才走整路重建。

任务保存请求不得用页面打开时的旧参数覆盖已经通过“应用参数”提交的新 revision。推荐将任务 DTO 中的实例部分收敛为 `instanceId + algorithmId + membership/enabled`，参数由实例级接口维护。过渡期若仍接受完整实例参数，必须携带 revision 并拒绝 stale write。

### 8.3 前端状态

- `AlgoParamDrawer` 的“应用参数”直接调用实例级 API，不再调用任务级保存回调来承担持久化。
- `LiveRulesStudio` 维护任务结构的未保存状态，并清晰区分“参数已应用”和“任务结构未保存”。
- `activeInstances` 使用 `instanceId` 做 key，`algorithmId` 只作为展示和算法包查找字段。
- API 返回 `pending/failed` 时不乐观地把实例标成运行成功；服务端资源状态由专用 hook 或查询缓存管理，不放入高频 UI state。
- 所有可见状态和错误文案进入 i18n；不在组件内拼接服务端错误文本。

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

## 14. 未决决策

- 算法包 manifest 是否显式声明每个参数的 `hot`、`replace` 或 `restart` 能力，还是统一尝试热更新后按 `NotImplemented` 回退。
- 是否允许同一任务挂载同一个 `algorithmId` 的多个不同 `instanceId`；若放开，数据库唯一约束和前端编辑器需要同步调整。
- 当硬件资源不足以同时保留旧、新 Worker 时，产品是否接受目标实例短暂停机；默认策略是保留旧 Worker 并返回资源失败。
- 规则和运动门控的实例级兼容字段何时迁移删除，避免任务级和实例级双写。

在这些决策完成前，不应把整路重建继续作为算法实例 Apply 的默认行为；现有全量路径只作为冷启动、媒体能力变化和增量同步失败后的受控回退路径。
