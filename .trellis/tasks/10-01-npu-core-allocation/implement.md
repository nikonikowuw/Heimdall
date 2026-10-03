# 实施与验证计划

> planning / 待确认。用户授权继续优化任务文档、以工业级交付为目标；本计划不是启动授权。需求以 prd.md 为准，设计和 API 均为草案。

## 0. 开工门禁与变更边界

- [x] 用户确认默认 auto、严格 manual；非法配置拒绝，修改失败保留旧健康 Worker，重启绑定失效仅该实例失败。
- [x] D1：用户确认一实例一专用 Worker、独立 session、宿主分核并共享物理权重；旧 Q1 整组停机前提撤销，不再实现其分支。
- [ ] 用户确认最新 PRD/design/API 完整摘要，包含执行/权重双层所有权、资源计划/离线扩展、共享 profile 拒绝语义及权重消费者分页；单项确认不触发启动。
- [ ] 完成该摘要之后的明确实施批准，再执行 task.py start；当前保持 planning，不创建/激活子任务。
- [ ] 保留工作区已有修改；不混入前端抽屉、摄像头搜索/i18n 等无关任务。
- [ ] 按目标包 spec index/checklist 读取规范；上下文清单见 implement.jsonl/check.jsonl。现行 spec 的默认组合 mask/共享串行 Worker 与新架构差异见 research.md，不提前修改现行规范。
- [ ] 阶段 A 冻结 ABI、执行/清理语义；板端错误码和合法 mask 证据在 C 硬件接入前确认，不能把裸 -13 或任意非零当可恢复错误。
- [ ] **冻结前置（2026-10-02 明确）**：架构已确立首选路线 A（控制面创建并独占移交），ABI 清理回执与所有权胶囊默认以路线 A 推进冻结；A0 阶段优先实测路线 A 的 4 条判定标准，若失败则切换路线 B 后备；本项与 A0 绑定，不阻塞 T01-T05 等硬件无关工作。
- [ ] 确认 placement 默认核心与 auto 亲和降级不得放宽 `FallbackPolicy::RequireHardware`（现行契约见 `.trellis/spec/algo-sdk/backend/algo-sdk-guidelines.md`，实现落点 `crates/algo-sdk/src/runtime/fallback.rs`）；新增会话入口必须经 `RuntimeSession::open_with_policy`。

最小行为缺口扩大为“独立实例执行 + 真实共享权重 + 宿主准入/生命周期”的闭环；不能只加 round-robin 或保留内部共享串行 Worker。修改边界限定为以下文件族及直接测试：

| 归属               | 预期修改范围与理由                                                                                                                     |
| ------------------ | -------------------------------------------------------------------------------------------------------------------------------------- |
| infer              | worker.rs、package.rs、c_abi、npu 及必要 placement 模块：完成确认、owner、inventory、账本与协议                                        |
| algo-sdk / RKNN 包 | macros/plugin/runtime/models/yolo/model 及人脸实例 session/权重 provider：移除内部串行推理 Actor，接通共享根与派生，不保留隐式自主分核；`InitContext` 新增字段需同步 10 处 yolo.rs 测试构造点与 3 平台本地 runner/探针 |
| types / db         | 任务字段、错误与快照；algorithm_instance/task repository 与前向迁移：三态、身份、原子配置与版本条件回写                                |
| pipeline / app     | coordinator/manager、TaskRuntimeService 及调用方、app reconcile/main/config：恢复批次、部分成功、配置/生命周期装配                     |
| api / web          | 任务与系统路由/DTO、task_service/state、错误与三语映射；web 共享类型、归一化和保存测试，不新增 UI                                      |

不修改模型输入/量化、媒体格式、帧 PTS、检测语义、告警协议、同任务算法唯一性；不引入分布式调度、跨卡执行、每帧 JSON/调核或无证据性能优化。

## 1. 交付拆分与协作

以下为建议交付单元，不表示已创建子任务。若后续拆分，当前任务负责 R01-R14/AC01-AC25 总体验收，子任务逐个规划、批准、实现及验证；依赖写入各子计划，不仅依赖 parent 关系。

| 单元             | 顺序/输出                                                   | 可独立验收与回滚点                                      |
| ---------------- | ----------------------------------------------------------- | ------------------------------------------------------- |
| A0 可行性证据    | 与 A/B 可并行研究，C 接线前完成对应 profile                 | 官方合同、根子线程/生命周期与物理共享成立才接生产       |
| A 安全与协议基线 | 首先；错误复现、生命周期/版本基线、可选 ABI fixture         | 不启用亲和也可验证析构/版本/部分成功；基础 ABI 完全不变 |
| B 宿主管理       | 依赖 A；inventory、独立执行/权重双层 owner/ledger、容量     | mock 验证不变量；不接新生产插件前不改变运行架构         |
| C SDK/插件       | 依赖 A/B 协议及 A0 profile；共享根+独立子 session、完整回执 | 缺共享证明不准入，默认核心也不放弃权重共享              |
| D 配置和诊断闭环 | 依赖 B/C；DB/API/Web/recovery 全路径                        | 前向迁移保留旧数据；关闭 auto spread 不取消 manual      |
| E 板端与发布     | 依赖 A-D；性能、长稳、灰度与回滚证据                        | 未通过的型号/制品/runtime profile 不宣称生产支持        |

与 `09-30-edge-torch-algo-ecosystem` 的 SDK 交付共同涉及 `runtime/mod.rs`、`runtime/platforms/rockchip.rs`、`models/yolo.rs`、InitContext 和算法包。A 阶段登记接口/文件负责人及合入基线；C 开始前重新搜索实际 session 创建调用图。

**2026-10-02 基线登记（原“任务被移往 archive”的假定已过期）**：该任务已 `completed`（2026-10-01）并归档于 `.trellis/tasks/archive/2026-10/09-30-edge-torch-algo-ecosystem/`（无 briefing 产物），其交付代码已合入 dev：`4a9a797`（吸收 RKNN runtime）、`d50487b`（可组合 transforms 与通用 YOLO 流水线）、`aa0cb7c`（cls 激活语义）、`da594ab`（插件 tracing 桥）、`1cae773`（硬件必需回退策略）。

直接后果：

- 「登记接口/文件负责人及合入基线」现在**可立即执行**，不必再等；B/C 开始前只需重取一次调用图。
- `crates/algo-sdk/src/runtime/fallback.rs` 属于新增的已交付契约面，placement 设计必须与它对齐（见 research.md S04）。
- `runtime/mod.rs` / `rockchip.rs` / `models/yolo.rs` 仍是活跃修改区（如 `1cae773` 改动 `resolve_policy` 与 `RknnSessionOptions`），因此 H 表的行号锚已整体失效，全部改用符号锚（见 research.md 漂移表）。

## A0. 权重共享可行性证据（R01,R09,R13,R14）

- [ ] 用部署匹配头文件/Runtime 动态符号确认 rknn_dup_context；不能只测任意开发库。公共 V2.3.2 官方证据已记录 research.md，但不替代 BSP 证据。
- [ ] 板端最小独立验证：每模型一个根、N 个 child；正确双指针签名、独立核心/IO、根控制线程到目标 Worker 一次性转移、兄弟运行时派生/销毁、先子后根清理。
- [ ] 共享 vs 独立 rknn_init 基线只用于隔离实验；记录 1/2/3/N 内存增量、根私有开销、Runtime query 与 DMA/CMA/PSS。后者不作为生产 fallback。
- [ ] 无法证明跨核共享/私有 workspace 隔离的 profile 阻塞，不降级产品需求；SDK 无 transfer 依据先调整并评审线程归属，不滥用 unsafe Send。**（T41/T42/T43）**
- [ ] **（P0 首项）线程归属实测验证（优先路线 A，后备路线 B）**（`design.md` §6.3.1）：优先验证首选路线 A「控制线程 dup 后一次性移交实例线程，并在实例线程内设核/推理/销毁」；逐条记录跨线程使用与设核的返回码、耗时与兄弟实例并发推理正确性；若路线 A 通过则正式确认采纳路线 A，若路线 A 发生 TLS/跨线程异常则验证后备路线 B「实例线程内 dup、控制侧串行化并发派生」；四条判定标准逐条给结论，输出选路结论与理由；两条均不成立则该 profile 阻塞。

A0 不阻止 A/B 的硬件无关安全和账本工作，但 C 的生产接线/发布依赖对应 profile 证据。不同型号独立签发，不以 RK3588 结果覆盖 RK3568/RK3576。

## A. 冻结跨层协议与故障基线（R01,R02,R04-R07,R12,R13,R14）

- [ ] 先建立 T01-T05、T10、T15-T17、T22-T24 的稳定失败测试。使用 barrier/oneshot/可控 mock 析构，不能靠长 sleep 碰运气；所有测试线程在断言后解除阻塞并回收，必要的挂死模拟置于可终止子进程。
- [ ] 修复退出确认语义：正常退出、初始化失败、startup/exit 通道断开、TLS/析构阻塞共用有界清理流程。Err/Disconnected 不直接无限 join；reaper 在锁外 join/析构/日志。
- [ ] 明确 guard 的创建、跨线程移交、取消与隔离路径；构造 future 取消不能释放实际执行中的 owner。
- [ ] 补齐真实 targetRevision 在 launch/apply/恢复中的传递，移除“0=未知后读最新版”的路径；Applied/Failed/Pending 使用 Repository 条件更新与 affected rows。
- [ ] 冷启动与 app 恢复建立每实例 outcome；placement 失败不 shutdown 健康 Worker、不统一标错。保持现有任务状态聚合枚举与媒体消费者隔离。
- [ ] types 定义 affinity、实例/组快照、错误，保留缺失/null/替换三态和有限枚举。
- [ ] 冻结可选扩展的精确 C 函数签名、size/version/offset、最大载荷、资源计划、权重根 prepare/release、实例/离线 session、初始化/双层清理回执及缓冲区不足规则；清理回执在 instance 销毁后由仍存活上下文查询。增加实际双侧布局测试，不仅 Rust 自比较。
- [ ] 四种宿主/插件新旧组合 fixture；旧插件 deny_unknown_fields、新插件无注入、宏未声明能力、毁坏/错误代际回执均覆盖。确定宿主错误码和 wire reason 映射，无裸硬件码泛化。
- [ ] **回退策略交汇回归**：注入 placement 元数据后 `is_self_test` 硬门仍为 `RequireHardware`（不被 placement 字段/显式声明/`.env` 翻越）；`runtimeManaged` 与 auto 亲和降级路径不产生 `debug_cpu_fallback_path` 模拟会话；新会话入口全部经 `RuntimeSession::open_with_policy`。
- [ ] **`InitContext` 扩展波及面清单**：新增 placement 字段前先冻结构造形状（建议 `#[non_exhaustive]` + builder）并一次性更新 10 处 `models/yolo.rs` 测试构造点、`macros.rs::export_algo!`、rk3568/rk3576/rk3588 三平台的 `run_local.rs`/`face_fusion_probe.rs`/`hardware_infer_test.rs`；不得改写 `fallback_policy_override` 既有语义。**（T44）**

Review gate：基础 AvAlgoAbi 64 位 96 字节及已有布局/签名不变；未启用调度时原业务行为除明确修复外不变；没有未经评审的新抽象/依赖。A 的协议 fixture 通过后才进入驱动接线。T41-T44 属 A/C 边界用例，可在 A 阶段先以硬件无关形式立稳定失败测试。**注意**：T41/T44 可在 A 阶段以 mock 落地，但 T42/T43 的**最终判定值**依赖 A0 的板端对照实验，不得用 mock 结果代替选路结论。

## B. 宿主资源管理（R01,R03,R04,R08,R09,R12,R13）

- [ ] 复用 npu 探测基础，在源头拆分 discovery/capability/telemetry，保留错误、null、采样范围和时间；兼容旧概览调用，不把默认一核/补零当可信数据。
- [ ] 设备验证/指标采样在固定专用 Worker；topologyGeneration 与普通采样更新分离，库存有上限，handler 只读快照。
- [ ] 纯 PlacementSolver + 短锁 CoreLedger 原子提交；同分轮转、严格 manual、幂等执行/权重引用与释放；旧 reservation 清理不能被新拓扑代际拒绝。
- [ ] recovery barrier：同批稳定排序、manual 先预留再分配 auto；各实例核心不求交；权重单航班不继承第一实例核心；离线/预热不绕行，barrier 不等待全部网络/硬件完成。
- [ ] ExecutionOwner/WeightOwner 各自 reservation，与 AlgoLease 分离；包/根/执行/run/拓扑代际独立校验；warmup 只保活根，根附带私有开销计费。
- [ ] 固定上限覆盖执行组、权重根、设备、核心、派生数、宿主/控制 Worker、离线请求、等待者、初始化及隔离；容量满快速拒绝。maxQuarantinedGroups 作为熔断阈值，硬容器上界覆盖所有已准入者同时失败，不丢 owner。
- [ ] 制品/runtime 关联内存 profile；根权重/根私有量与子 session/workspace/IO 分开，原子准入并分别释放。unknown legacy 每个可能创建资源入口保守计费，不算新架构能力。无可靠预算不进入受保障生产准入，不以第一份未知模型免费放行。
- [ ] 缓存实例/执行组/权重根/消费者快照，最新失败无 reservation 也可查；日志节流、指标低基数；released 历史不无限积累。

Review gate：T06-T14/T19-T21/T25-T27/T31/T33-T35/T37-T38 验证额度守恒，无锁内 IO/FFI/await、无无限重试；mock 竞态任意顺序均不重复释放或漏账。

## C. SDK 与全部 RKNN 插件（R01,R04-R06,R09,R10,R13,R14）

- [ ] SDK 宏单次剥离 __heimdall_placement；InitContext、本地 runner、测试构造点与 update_config 同步。元数据错误不默默回退 Config 默认值。
- [ ] checked narrowing、字段/位校验、版本和身份比对；普通业务热更新不能改变 group/reservation。
- [ ] 完整路径：GenericDetector -> RuntimeSession -> RknnSession、手写通用/消防检测、人脸各平台独立 rknn.rs；SharedWeights/Core 不得再次覆盖宿主 placement。
- [ ] **placement 选项走 `#[non_exhaustive]` builder**：`RknnSessionOptions` 已具备该形态（`Default` + `with_fallback_policy`），新增核心选择必须同形，不得新增可绕过准入的 `pub` 字段路径。
- [ ] 删除生产 RKNN_CORE_MASK 环境覆盖及未经协商的固定 mask，同步 .env.example/GUIDE，保留明确本地无注入行为。
- [ ] 当前环境覆盖读点已变化：`RknnSessionOptions` 新增 `core_mask`/`fallback_policy` 后，掩码覆盖逻辑现位于 `RknnSession::open_or_fallback`（符号锚）；人脸包独立读点在 `algo-packages/rknn/rk3588/face_recognition/src/rknn.rs`；两处均需移除，不得只改一处。
- [ ] 建立真实 RKNN shared-weight provider，root 单航班；WeightKey 不含 core/instance，同包重复打开不能有第二份 registry。rknn_dup_context 检查返回值，派生操作串行但推理不走根控制队列。
- [ ] 每实例独占必要 child sessions/IO/workspace，在现有宿主 Worker 推理；删除人脸插件共享串行推理 Actor，不机械增加第二层每实例线程。可写图像池只复用空闲独占租约。
- [ ] WeightOwner 保留根至所有 child/在途派生/隔离子清理确认；取消移交/部分创建失败有回收 owner。不得依赖首实例、静态变量析构或先毁根的未验证行为。
- [ ] 准入要求权重共享：符号缺失/能力未验证/派生失败不 fallback 独立 rknn_init、共享推理 Worker 或 CPU。
- [ ] 全部必要 session 聚合回执，明确可选 session 集合及预算；任一必需模型不满足 manual 则本执行组初始化失败并清理。
- [ ] 根据目标头文件/已加载库证据分类 set_core_mask 失败；auto 最多一次从同根重新派生默认 child，仍满足共享要求；手动、不匹配/OOM/未知致命错误不吞掉。
- [ ] 离线可选 session 扩展与独立有界 Worker 接线，不借用摄像头 context；同域模型引用同一权重根，lease 与包强引用移入有界实际执行任务；取消、包替换和冷却期间保持同代际；内部 Worker 退出/清理 panic 能在回执中诊断。
- [ ] 安装自检持有限临时设备预算，进程取消/超时后须确认 wait/退出才释放；不得重跑已受信包全量自检或扩大硬件放置声明。

Review gate：同模型三个实例必须是三个独立 execution group，各自分核并关联同一权重根（多模型分别关联）；不是一个串行 Worker。T10-T14/T20-T21/T28-T29/T31-T38/T42 通过，SDK 接受不冒充硬件采样/物理内存实测。

## D. 持久化、运行时收敧、API 与 Web 兼容（R02,R07,R08,R10,R12）

- [ ] 前向 affinity_json 迁移，旧行 auto；事务内合并三态，显式 instanceId 校验任务/算法归属，旧无 ID 客户端按同任务唯一算法匹配。
- [ ] 保留 algorithmInstances 集合替换、旧单算法桥接、总闸/分闸分离及 UNIQUE(task_id, algorithm_id)。省略 affinity 不因全量表单保存而变 auto。
- [ ] **不变量回归**：`desiredRevision` 仅在有效期望字段变化时递增；affinity 归一化等价（如 `auto` 与 `auto/spread`）不创建新组；规则镜像等无关字段变更不引起硬件重建；`PUT /tasks/{cameraId}/enabled` 只翻转总闸，不修改 affinity/实例分闸/实例身份。
- [ ] 在 `RknnSessionOptions` 已是 `#[non_exhaustive]` 的前提下确认 self-test 硬门不可被翻越：placement 注入不得引入新的构造旁路（与 A 阶段回退策略回归共用用例）。
- [ ] manual 校验在任何写入前；同次 streamMode 与任务/实例配置纳入一次仓储事务，revision 冲突不留下 camera 半更新。
- [ ] 实例级参数更新/创建/删除和任务整体保存/启停/冷启动均走受管理入口；不能绕过保留元数据校验或原子回写。
- [ ] 运行槽记录 runtimeRevision，候选/结果以 targetRevision+attempt 栅栏提交；新配置失败保留旧槽。DB 写回失败不提前释放已切换资源；重试队列和次数有界。
- [ ] api.md 六个端点按契约接线：任务 GET/PUT、执行组、权重根/消费者、拓扑；实际 protected JWT 路由、稳定错误与国际化，不在 handler 调 SDK。
- [ ] web 类型/共享归一化、taskDraft/LiveRulesStudio 往返；旧服务器缺字段只表示未知；不新增 UI、不重建无关视频播放器。
- [ ] 完成 T15-T24/T30，覆盖无资源失败、旧资源继续运行、停用历史 applied、删除后迟到结果、跨请求快照变化。

Review gate：保存和应用严格区分；兼容客户端不损失配置；任务聚合 Error 不停止健康实例；HTTP 400/409/503 的拒绝事务无副作用。T39/T40 在本阶段落地。

## E. 板端验收、发布与规范闭环（R11,R13,R14；AC01-AC25）

> **注入完整性（拆分前须知）**：本任务 `implement.jsonl` 引用的 `algo-sdk-guidelines.md`（64369 B）单文件超 `max_file_bytes`（32768），被截断后丢失尾部「硬件回退策略 (`FallbackPolicy`)」与「Apple Silicon」等章节；且 jsonl 清单会先耗尽 `max_total_bytes`（131072），导致 `research.md`/`api.md`/`design.md`/`implement.md` 降级为索引行（仅路径+reason+大小），子代理拿不到正文。**拆分子任务时必须同步瘦身 jsonl 清单与拆分 design.md**，否则子代理实际拿到的是不完整契约。详见本节末尾「上下文注入基线」。

- [ ] 对每个实际发布 RK3568/RK3576/RK3588 + 制品 + Runtime/BSP profile 完成 research.md evidence packet；未验证平台不作支持承诺。
- [ ] 测量前冻结负载模型/输入、启动并发和所有超时/容量、P95/吞吐/丢帧/控制面响应预算及容许回归范围，由产品/维护者确认；字段有缺项则不能签发验收结论。
- [ ] 每种 Runtime 默认/单核 spread/受支持组合策略至少 3 轮同负载对照，固定预热和测量窗口，保留原始记录；不根据结果回调门槛。
- [ ] 输出吞吐、端到端 P50/P95、排队/丢帧、CPU、RSS/PSS、CMA/设备内存、fd、温度/频率及实际 session 集合；包含同模型共享权重多实例、人脸多阶段、多模型并行和离线负载；旧共享串行架构/独立全量加载仅作对照，不作为新架构验收。
- [ ] 启停/配置修改/失败重试至少 100 轮；长稳至少 8h。预热平台期后线程/fd/驻留内存无未解释的持续增长，正常清理后账本回基线；挂死对象有账且不继续扩增。
- [ ] 验证无设备/权限不足/未知拓扑/设备消失、OOM、fallback 失败、同一制品下库指纹不匹配、并发隔离阈值突破；板端故障注入先有恢复与取证方案，不在生产板盲目破坏驱动。
- [ ] 先验收物理共享与独立执行，再发布：shadow -> 已验证 auto spread -> manual。新插件无注入仍要求共享权重/独立 session，只采用 Runtime 默认核心，必须将这一行为变化纳入新旧宿主组合基线，而非称为完全无行为变化。
- [ ] 演练关闭 auto spread、停止后重启、进程异常退出恢复及旧宿主回滚。manual 必须先显式解除/停用并记录运维确认；回滚旧共享串行架构同样须告知 D1 保证失去并停用相关实例，不能无声降级，DB 不破坏性回退；隔离资源未退出不能借开关解除预算。**（AC10）**
- [ ] 更新已实现且验证的 infer/algo-sdk/db/api/pipeline/app/web 相关 spec；未实现项留任务文档。记录根/独立 workspace 与板端检查的实际环境和未跑项。**（AC11）**

验收判定：正确性/资源守恒/兼容性有任一失败则拒绝发布；性能未达冻结预算则不得启用该 profile 的 spread。8h 是本任务最小长稳门槛，不等于已证明产品寿命或 MTBF。

## 故障与回归矩阵

T 编号为拟实施测试合同，不是已存在测试名；先检索复用现有 fixture，再落地到对应 crate/tests。mock 测试无硬件依赖，板端用例单独标记。

| ID  | 注入/场景                                               | 必须断言                                               | AC                  |
| --- | ------------------------------------------------------- | ------------------------------------------------------ | ------------------- |
| T01 | 推理正常、backend Drop 阻塞                             | 停止期限内隔离、真实清理前占额                         | AC04,AC12           |
| T02 | factory 成功后 runtime 创建失败且析构阻塞               | Err 分支不无界 join                                    | AC12                |
| T03 | startup/exit 通道断开、panic/TLS 析构延迟               | 通道断开不等于资源释放；不虚假 completed               | AC12                |
| T04 | revision N 执行中保存 N+1，N 成功/失败/pending 迟到     | N+1 的状态/原因/版本均不被覆盖；条件更新 0 行可诊断    | AC15                |
| T05 | 同摄像头多算法冷启动，单实例 manual 失败                | 正常算法继续服务，顺序互换等价                         | AC13                |
| T06 | reservation 后、线程启动前取消                          | 无 session，预算释放一次                               | AC04                |
| T07 | rknn_init 阻塞/初始化等待 future 取消                   | owner 保活、额度不丢、无无界重试                       | AC04,AC18           |
| T08 | 同时多个组选择最低计数核心                              | 原子准入、同分轮转、所有上限不超额                     | AC01,AC03           |
| T09 | 同批 manual core0/core2 与 auto，改变输入顺序、离线抢先 | 独立约束；同批 manual 先准入、权重单航班不绑定整个组   | AC14                |
| T10 | 并发首次请求同模型权重、部分等待者取消                  | 单航班一个根；实例有独立 child，取消不连坐             | AC01,AC08,AC21      |
| T11 | 必需多模型部分成功后失败                                | 部分资源清理或隔离，全组不假报 Ready                   | AC05,AC08           |
| T12 | set_core_mask 不支持/未知/OOM、默认回退也失败           | 仅已验证 auto 一次回退，manual/致命错误拒绝            | AC05                |
| T13 | 无能力/拒绝未知 JSON 的旧插件、新插件无注入             | 新旧四组合、正确未确认/unsupported                     | AC06,AC08           |
| T14 | 错版本/错组/错代际/截断/超长应用或清理回执              | 不发布伪确认，不超分配、不读已销毁指针                 | AC06,AC12           |
| T15 | 非法 manual + streamMode、revision 冲突                 | camera/task/instance/revision 全部不变                 | AC16                |
| T16 | 旧客户端缺 affinity/缺 ID、null reset、旧算法桥接       | 保留/重置正确，实例身份不漂移                          | AC07,AC16           |
| T17 | 显式跨任务 ID、算法错配、同任务重复算法                 | 明确拒绝，无部分写入                                   | AC16                |
| T18 | 同 revision 新 attempt，旧 Worker 结果/错误迟到         | run/attempt 栅栏生效，不污染健康和规则链               | AC04,AC15           |
| T19 | 权重冷却被新实例复用、旧拓扑资源释放                    | 根定时器代际安全；子和根分别幂等释放                   | AC04,AC21           |
| T20 | 离线执行 future 取消/同时包替换                         | 执行任务持有原 lease/package，结束前不回收             | AC08,AC18           |
| T21 | 自检进程取消、挂住、wait 尚未完成                       | 临时预算和进程 owner 保留，生产账本不被子进程改写      | AC08,AC18           |
| T22 | DB 提交成功后硬件失败/切换后 DB 写回失败                | 真实旧/新槽可诊断，资源不提前释放，重试有界            | AC15,AC17           |
| T23 | 新进程读历史 applied、禁用/未运行、实例删除后旧结果     | 不伪造当前硬件确认、不复活实例                         | AC15,AC17           |
| T24 | 旧算法失败、其他摄像头及同路算法运行、媒体输入失败      | 准确区分实例与媒体故障域                               | AC13                |
| T25 | 未知 SoC、部分/全局负载、权限不足、遥测过期             | null/范围准确；采样不改变拓扑代际、不阻塞 Tokio        | AC09,AC17           |
| T26 | 临近隔离阈值的全部已准入者同时卡死                      | 全部 owner 留存，停止新准入，不清表或失去句柄          | AC03,AC18           |
| T27 | 老插件多实例+warmup、未知预算、宿主外层线程耗尽         | 保守计费，不把 unknown 当免费共享                      | AC03,AC18           |
| T28 | A 换核，B/C 推理与离线执行持续                          | 只替换 A child；同权重不复制，不停兄弟实例             | AC02,AC04,AC08,AC13 |
| T29 | GenericDetector/RuntimeSession/手写插件/SharedWeights   | 所有 session 收到宿主选择，不自主覆盖                  | AC06,AC08           |
| T30 | API 认证、执行/权重/消费者分页、失败、Web 往返          | 信封/枚举/空值/上限正确，共享证据不混为核心确认        | AC07,AC09,AC17      |
| T31 | 同模型创建 1/2/3/N 独立实例，mock 计数与板端分层        | 每模型一个根+N child；每实例私有预算，非一个串行 Actor | AC01,AC19,AC20      |
| T32 | 不同输入并发，延迟读取某实例输出，另实例执行            | IO/workspace/预处理租约不覆盖；结果等价性通过          | AC19                |
| T33 | 停止首实例、乱序销毁 child、最后 child 析构阻塞         | 根由独立 owner 持有；先子后根；隔离不提前释放          | AC04,AC21           |
| T34 | 缺 dup 符号/profile、派生失败、auto 亲和降级            | 新架构不复制权重/串行回退；默认 child 仍从同根派生     | AC05,AC22           |
| T35 | 同包重复打开、不同制品/设备/域/初始化配置               | 同 key 单根，不相容 key 不误共享；旧新根分别计费       | AC08,AC20,AC21      |
| T36 | 1/2/3/N 共享 vs 独立 init，预热、增删与峰值             | SDK+分配证据+多源内存联合验证，不直接累加逻辑权重查询  | AC20                |
| T37 | 根到 Worker 移交取消、root 操作阻塞、私有分配失败       | 独占移交，无线程并发用同 context；所有 owner/预算保留  | AC12,AC18,AC21      |
| T38 | root 清理回执丢失、child 错 WeightId/代际、未知共享状态 | 不发布 D1 Ready、不卸库/误清账；API 如实报告           | AC06,AC09,AC21,AC22 |
| T39 | `auto` 与 `auto/spread` 往返归一化；仅修改规则镜像后保存 | 不创建新执行组、不重建硬件、revision 不递增；placement 保持 | AC25 |
| T40 | 总闸 `PUT /tasks/{id}/enabled` 与非法 affinity 并存 | 仅总闸翻转；affinity/实例分闸/实例 ID 不变；非法 affinity 仍整体拒绝 | AC25 |
| T41 | placement 注入 + `is_self_test=true`；runtimeManaged/auto 降级且无运行时 | 策略恒为 `RequireHardware`；不产生模拟会话；错误仍为 `ModelLoad`→`-5` | AC24 |
| T42 | 首选路线 A 实测（控制面 dup 后独占移交、实例线程设核与推理）；后备路线 B 验证（实例内加锁 dup） | 选定路线下全部操作在所属 Worker 完成，无跨线程并发误用；另一路线结论入档 | AC23 |
| T43 | 混用路线：一个实例走 A、另一实例走 B（仅实验环境，不得进入生产 profile） | 明确拒绝或明确隔离；不静默产生两个共享语义不清的 root | AC23,AC06 |
| T44 | `InitContext` 新增字段后旧构造点编译与本地 runner 运行 | 所有构造点同步；本地无注入行为与 D1 前一致（仅核心选择不同） | AC06,AC08 |

## 验证命令

先格式化后检查；保留非本任务变更。以下为实施期门禁，不在仅编辑任务文档时执行全仓格式化。

```bash
cargo fmt --all
cargo fmt --all -- --check
cargo clippy --all-targets -- -D warnings
cargo nextest run --workspace

for manifest in algo-packages/macos/Cargo.toml \
  algo-packages/rknn/rk3568/Cargo.toml algo-packages/rknn/rk3576/Cargo.toml \
  algo-packages/rknn/rk3588/Cargo.toml; do
  cargo fmt --manifest-path "$manifest" --all
  cargo clippy --manifest-path "$manifest" --workspace --all-targets -- -D warnings
  cargo nextest run --manifest-path "$manifest" --workspace
done
# InitContext/宏影响 macOS；没有对应目标环境时明确记录未跑，不能用根 workspace 替代。
# 若涉及 native C/H，先 clang-format 再检查；SDK 可用时补 all-features/平台测试。
# 硬件用例 #[ignore]，在指定板端按精确用例/过滤器单独运行并保留命令与结果。

cd web
pnpm format
pnpm lint
pnpm typecheck
pnpm test
pnpm check:cycles
pnpm build
```

## 本轮文档验证

仅检查 task JSON/JSONL、Markdown 格式/引用路径、JSON 例子、R/AC/T 覆盖和跨文档状态一致性。记录 D1 已确认/Q1 已撤销、官方 V2.3.2 共享合同已核验、目标板/ABI/性能仍待验证；不把文档检查写成实现验收。保持 task.json.status=planning，不执行 task.py start、归档或 Git 提交。

2026-10-02 复核轮次另需确认：R14/AC23-AC25/T39-T44 已在 prd/implement 双侧落位；架构已确立首选路线 A（控制面创建后独占移交）与后备路线 B；research.md 的漂移表与 S04 已建立；design.md §6.3.1 首选/后备路线与 §11 的冻结前置已登记；api.md 无 REST 形状变化。

## 上下文注入基线（拆分前必读）

本任务的 `implement.jsonl` / `check.jsonl` 各有 19 条条目。按 `.pi/extensions/trellis/index.ts` 的实际截断规则（**保留头部、丢弃尾部**）模拟：

| 层级         | 限制         | 现状                                                                                              |
| ------------ | ------------ | ------------------------------------------------------------------------------------------------- |
| 单文件       | 32768 B      | `algo-sdk-guidelines.md`（64369 B）截断至 32768，**丢失尾部 31601 B**（含「硬件回退策略 (`FallbackPolicy`)」与「Apple Silicon」章节） |
| 单 artifact  | 65536 B      | `design.md`（~42.5 KB）未超限；但受总预算耗尽影响实际不被内联                                            |
| 总 payload   | 131072 B     | jsonl 清单在第 11 条处耗尽预算（剩 21487 B）                                                                  |

后果：`research.md`、`api.md`、`algorithm_instance` 相关 spec、`prd.md`、`design.md`、`implement.md` 全部降级为索引行。子代理实际拿到的是 10 份 spec 正文 + 一份被削尾的 SDK spec，**拿不到本任务最关键的契约文本**。

拆分时的必需动作：

1. **瘦身 jsonl 清单**：每个子任务只引用其真正需要的最小 spec 集，不复制父任务全量 19 条。
2. **拆分 design.md**：单文件降到 32768 B 以下（或拆为宿主侧/SDK 侧/发布侧多份子任务文档），否则无论怎么排预算都会被截断。
3. **不要依赖 jsonl 传递任务自身的 prd/design/implement**：artifacts 独立于 jsonl 预算，但一旦 jsonl 先耗尽总量，它们同样降级。把子任务自身契约放在子任务目录下并控制总量。
4. 父任务自身保持 planning 期间不受影响（本文件的核对靠的是直接读盘，不是注入）。
