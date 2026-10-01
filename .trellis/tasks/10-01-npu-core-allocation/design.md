# NPU 放置架构设计

> 修订草案 / 待确认。本文所有新增类型、符号、字段和模块均为拟议接口，不是现有 API。需求见 prd.md，HTTP 契约见 api.md，实施门禁见 implement.md。

## 1. 架构决策

采用单进程控制面分配、执行组所有权和版本化插件协商。不引入分布式调度器、每帧调度 RPC 或自动迁移控制环。

```text
任务实例 affinity（持久化期望）
  -> Pipeline 协调器（配置版本、实例启动/替换）
  -> infer PlacementManager
       DeviceInventory：复用现有设备发现，生成不可变拓扑快照
       PlacementSolver：纯函数，校验并选择候选
       CoreLedger：短锁事务提交、有界 reservation、代际
  -> 执行组 GroupOwner（模型资源的真实生命周期）
       独立模型：一个实例执行组
       共享人脸：同一包代际的一个共享硬件 Worker/多 session 组
  -> algo-sdk：分离宿主元数据和业务配置
  -> 插件硬件 Worker：创建/设置/查询/销毁 RKNN context
  -> 版本化回执 -> 宿主只读诊断快照
```

### 分层

- types：平台无关 AffinityIntent、运行态快照及错误分类；不放 rknn C enum 或硬件调用。
- db：实例 affinity 持久化、配置事务和 revision；不存 reservation 或硬件瞬时负载。
- infer：设备 inventory、分配、协议适配、资源生命周期；硬件 ABI/平台差异止于此与插件 HAL。
- pipeline：业务成员生命周期、配置收敛、帧边界替换；不计算 mask、不探测 SoC。
- algo-sdk：独立的版本化 wire 类型与边界解析，不依赖宿主 types；双侧通过协议 fixture 保持一致。
- api：参数/鉴权/DTO/错误映射，只读宿主快照；app：装配唯一 manager 和容量配置。
- web：既有数据类型、归一化与保存兼容；不新增调度 UI。

## 2. 身份与资源所有权

### 三类身份

1. MemberId：业务实例或离线请求的唯一成员身份。
2. ExecutionGroupId：实际共享硬件资源的组身份，不能直接使用 algorithm_id。
3. ReservationId：进程 boot ID + 单调序列，绑定 packageGeneration、groupGeneration、topologyGeneration；外部作为不透明字符串。

共享组 key 使用规范化包身份 + 已加载制品代际 + 插件声明的 sharing domain。不得把 affinity 加入共享 key，以免冲突被悄悄转成重复加载。独立模式增加实例身份。包原地升级必须产生新代际，不复用旧模型组。

新协议插件声明 instance 或 packageShared 执行粒度。未知老插件按 conservative package scope 记录，状态 unverified；不能宣称真实 context 数已知。

### AlgoLease 与 reservation 分工

AlgoLease 保留包热态保活职责；PlacementMemberLease 记录业务成员；GroupOwner 持有唯一硬件 reservation。共享成员只增引用，不重复分配核心。Warmup 也是该组的保活成员，不另造一个无配置模型池。

独立模型插件不应为预热另外复制完整模型：实际生产 Worker 的成功初始化可完成预热；需要缓存时缓存同一执行资源，而非再创建一份。实施前核对不同插件的现有 warmup 语义。

### 释放顺序

停止接单 -> 排空/取消未执行请求 -> 完成在途调用 -> 销毁插件实例与共享 session -> 确认插件内部线程退出 -> 释放 reservation。资源退出前保留动态库引用。

宿主外壳 Drop、超时、成员归零都不能单独证明资源已释放。共享组在 60s 冷却期仍占驻留/执行组额度；冷却无在途工作时可主动回收，必须等待退出后才能再准入。

## 3. 配置与放置语义

实例字段 affinity：auto 默认 spread；manual 必须有 deviceId 与 coreIndex。详见 api.md。不引入 Task 继承，不提供裸 mask API。

- spread：新执行组在可用核心中按 assigned group count 最少选择，同分使用持久于 manager 的轮转游标。这是放置启发式，不叫实时最空闲。
- sharing：允许共享但受 admission limits 约束，与 spread 不是互斥枚举。人工指定也不承诺独占。
- manual：设备/核心/能力/共享冲突明确错误；不取模、不自动迁移其他组。
- 共享组已有 placement：auto 加入现状；manual 仅在与已确认 placement 相容时加入。不相容返回 conflict；需要调整时运维先停整个组。
- 运行中 affinity 改变：走资源重建，不调用普通业务 update_config 修改硬件归属。对没有其他成员的组，按容量和停机边界执行替换；不做隐式 stop-first 中断。无峰值容量时标记失败并提示先停后改。

### 设备身份

复用 npu::NpuDevice 发现结果。deviceId 是宿主 inventory 暴露的稳定逻辑字符串；runtimeDeviceIndex 是内部运行时映射，不是 PCIe 物理卡号保证。拓扑携带来源、可用性、generation 和 affinity capability。

未发现设备为空集合；未知核心集合不能 max(1) 伪造可绑定核心。RKNN 默认 auto 可在可用但不可绑定的设备上以 runtimeManaged 运行，仍消耗设备级预算；manual 拒绝。CPU/Core ML 非本任务的核放置目标，不伪装成 RKNN。

## 4. 账本、并发与容量

采用一个短持有 Mutex<LedgerState>，在锁内校验快照代际、选择并提交 reservation，防止并发启动都选择同一最低负载核。仅内存操作在锁内；SDK、IO、await、join、日志落盘在锁外。初版不引入无必要的 actor 或无锁复杂度。

账本按 reservation 索引，按 device/core 保存聚合计数；成员集合独立有界。release(id,generation) 幂等，禁止按 algorithm_id 批量删除。重复加入同一 member/run generation 返回已有成员或冲突，不重复计数。

启动配置必须提供有限的：maxExecutionGroups、maxGroupsPerDevice、maxGroupsPerCore、maxMembersPerGroup、maxConcurrentInitializations、maxQuarantinedGroups、设备模型驻留预算。实现阶段先复用已有容量配置，缺失项才新增；具体发布值由板端 profile 确定，不能把草案数字当硬件能力。

内存预算包括 session 权重/workspace/IO、预处理池、Warmup、新旧代际重叠与隔离资源。未知用量使用经审核的保守 profile，不以 0 准入；没有可靠 profile 时禁止自动扩增同包独立组。逻辑预算不声称精确替代 CMA allocator 状态，分配失败仍需安全回滚。

Runtime 默认/降级组不属于某一独占核心。设备级额度照常占用，对每核竞争评估采用保守影响，不能记为“全核零占用”。外部进程负载只能采样观测，不在本进程账本控制范围。

## 5. 生命周期状态机

```text
Reserved -> Initializing -> Ready -> Cooling -> Draining -> Released
                |            |                      |
                +-> Failed --+                      +-> Quarantined
                                                        -> Released（确认退出）
```

状态与 applicationStatus 分离：Ready 的组可以 acknowledged、runtimeManaged、degraded 或 legacyUnverified；manual 只有满足要求才 Ready。

- Reserved：额度先占用，未创建硬件；取消可直接 RAII 回滚。
- Initializing：启动 owner 独立于请求 future；取消发出停止意图，不销毁仍在 FFI 中的资源。
- Ready：仅当前 group/package/topology generation 可提交；过期结果转清理，不能发布给新实例。
- Cooling：最后一个活动业务成员退出，Warmup 按既有冷却策略保活；新成员只取消对应代际定时器。
- Draining：停止接单并保留额度，等待实际资源退出。
- Quarantined：线程未退出，保留 reservation、库及 owner；到达隔离上限停止新准入，不无限重试重建。不能 TTL 清账。
- Released：确认实际退出后释放一次。进程崩溃重启时不重放旧 reservation，只从 DB 意图重建。

共享插件内部线程同样适用。当前人脸 Worker Drop 中 join 的事实不能证明有界停机；必须跟踪外层隔离与内层资源的所有权，禁止只改宿主计数。

## 6. 基础 ABI 不变的插件协议

### 6.1 必须解决的现有入口限制

现有 export_algo! 直接将完整 config_json 反序列化为插件 Config；InitContext 没有原始 JSON 或 placement。仅新增 extract_placement_core_mask() 不会让现有插件自动得到配置。

拟改为 SDK 边界一次解析 object：剥离 __heimdall_placement，受检构建 PlacementContext，再将剩余 object 交给插件 Config。InitContext 增加 Rust 侧可选 placement；这不改 C ABI，但属于 SDK 源码 API 变化，所有初始化构造点和本地 runner 必须同步。

wire v1 例子（内部字段沿用 snake_case，与 REST 分离）：

```json
{"confidence":0.5,"__heimdall_placement":{"version":1,"reservation_id":"boot-7:42","group_id":"opaque-group","generation":3,"device_id":"rknn-npu0","runtime_device_index":0,"strategy":"pinned","core_mask":2,"required":true}}
```

宿主生成元数据；用户 algoParams 出现该键一律拒绝。字段缺失可兼容老宿主，字段存在但版本/类型/范围错误必须失败；mask checked narrowing、拒绝非法位和不一致字段。不读取进程 RKNN_CORE_MASK。普通 update_config 不得修改 reservation；对未变化元数据只校验后剥离，不触发重绑。

### 6.2 能力与回执（拟议可选扩展）

采用与现有 gallery optional symbol 一致的加载模式，新增独立版本化 placement 扩展，不修改 AvAlgoAbi、AvAlgoLibraryInfo 或实例参数布局。

扩展需要两个能力：创建前查询执行粒度/协议支持，创建后读取缓存的应用回执。正式函数签名、POD 字段大小/偏移和最大载荷须在实施第一阶段用双侧协议测试冻结；未冻结不得接入硬件代码。接口只能返回调用者拥有缓冲区中的固定上限数据，不跨 ABI 传 Rust 引用/Vec/String。

回执至少包含 reservation/group generation、状态、requested/accepted core selection、reason enum 与 SDK status；各必要 session 的应用结果必须聚合，不能只报检测器成功而漏掉 embedder。查询在所属 Worker 串行执行，只读初始化缓存，不在 HTTP 路由调用 FFI、不锁住推理执行硬件查询。

回执丢失/超时/代际不匹配：manual 初始化失败并进入安全清理；auto 标记 unverified，预算保守，不把 instance_create 返回 AV_OK 当掩码成功。老插件无扩展：不注入可能触发 deny_unknown_fields 的元数据，auto legacyUnverified，manual unsupported。

新插件在老宿主/本地 runner 无注入时使用 Runtime 默认策略，不宣称物理多核已生效。只有显式实现扩展的插件才声明支持；导出宏不可替所有插件假定支持。

### 6.3 共享人脸与离线提取

Warmup 必须携带组 placement，并在创建 shared_models 之前生效。shared_models 使用单航班初始化状态，避免检查 Weak 后在锁外各自初始化造成短时双份模型。锁内只登记 initializing slot；初始化在固定 Worker，等待者通过有界握手加入。

所有业务成员复用同组；配置不能在每帧更改 mask。独立 av_algo_extract_face 无实例 config：宿主执行前取得同组成员/保活 lease，完成组初始化并持有至同步提取真实结束；插件只消费已确认共享组。无宿主本地调用采用单独 legacy/runtimeManaged 组，不与不同绑定组悄然混合。

子进程安装自检不修改生产进程账本；宿主给自检进程一个有限设备级临时预算，子进程退出后释放。自检默认 Runtime 策略且不宣称生产 placement。已受信包冷启动继续遵守不重复全量自检的项目约定。

## 7. 错误与降级

不按“任意非零 -> warn”实现。

| 情况 | auto | manual |
| --- | --- | --- |
| 绑定受支持且 SDK 成功 | acknowledged | acknowledged |
| 已验证的亲和 API 不支持 | 有界重建默认 context，成功后 runtimeManaged/degraded | 失败 |
| 模型/平台不匹配、context 无效、OOM、未知错误 | 失败并保留原错误分类 | 失败 |
| 旧插件无能力协议 | legacyUnverified，不注入新字段 | unsupported |
| fallback 初始化失败 | 失败，不再次循环 | 失败 |

默认回退优先采用销毁失败绑定的独占 context 后重新初始化且不设置 mask，最多一次；只有目标 SDK 文档明确保证失败不改变 context 时才可优化为原 context 继续。必要模型应全组一致重建，预算与清理覆盖部分成功。

Runtime 错误码以目标头文件/库版本为准。当前源码对裸 -13 的解释需要验证，不将其写成跨版本契约。

## 8. 数据库、Apply 和恢复

新增 algorithm_instances.affinity_json（加法迁移、旧值 auto），与 params_json 分开。TaskRepo 在已有单事务内处理省略/显式 null，保留现有 algorithmInstances 集合替换语义。不能通过 Option<T> 的普通反序列化把“缺失”和“null”合并。

先校验结构，再提交期望 revision；硬件/共享冲突在运行时准入复核，避免把请求前检查当锁定资源。持续沿用 configRevision 乐观锁和 desiredRevision/appliedRevision。

DB 提交与硬件切换不可能是单一 SQLite 事务：成功保存可处于 pending/failed；旧健康 Worker 若仍运行须显示其 appliedRevision。候选新 Worker Ready 后在帧边界切换，旧代际迟到结果继续按现有栅栏丢弃。无资源时不销毁旧 Worker以制造“成功”。

设备丢失时拒绝新准入，旧资源进入可诊断状态，不自动迁移。配置修正/重启按当前拓扑重新求解。持久化只保存意图，不落库 transient mask 作为未来事实。

## 9. 观测、安全与性能

REST 快照来自 manager，不临时打开 NPU。包含 topologyGeneration、sampledAt、stale、group/member count、quarantine、application status；requested 与 assigned 明确区分。硬件利用率 unavailable 为 null，acknowledged 仅指插件/SDK 确认。

日志含不透明 group/reservation/instance ID、代际、状态转换、reason；按类首次与节流汇总。Prometheus 等计数标签只使用平台/状态/reason，不使用 instanceId 或 package path；细节走有界分页 API。

分配发生在启动/重配，不进入逐帧路径；不对已有 FrameRef 做像素拷贝。不增加每帧 SDK 设置或 JSON 解析。容量与重试均有上限。

## 10. 发布、回滚与未来边界

1. 协议和测试 fixture 先行，保持旧插件可运行但未确认。
2. 新插件发布能力与回执，宿主 shadow 记录拓扑/组归属，不改变生产绑定。
3. 对通过板端验证的 profile 启用 spread；未验证 profile 使用 runtimeManaged。
4. 开放 manual（仅明确支持且已验证的 profile）。
5. 出现回归可关闭新 auto spread，对新建 auto 组使用 Runtime 默认；不后台改写现存 manual，需停组/显式变更。

降级旧宿主前导出配置、停止/显式解除 manual 并告知用户旧版本不执行该约束。DB 使用前向兼容加法迁移，不删除 affinity 列。

Ascend 仅预留 device identity 与内存域校验：未来必须把 decoder/preprocess/model 的设备约束一起协商，不能通过 aclrtSetDevice 自动解决跨卡 device pointer。Rockchip core mask 不泛化为 Ascend core 语义。

## 11. 待冻结的技术门禁

- placement 可选扩展的精确双侧 ABI 与载荷上限：阶段 A 完成，未通过不得开始驱动接入。
- 设备 profile（合法 mask、fallback 错误、内存预算、容量默认值）：板端证据门禁，未验证不得宣称生产支持。
- 原 SDK spec 的默认组合 mask 要求与新策略存在差异：实现及板端验证后更新规范；本轮不把草案写成现行规范。
