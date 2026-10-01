# NPU 放置架构设计

> 修订草案 / 待确认。本文所有新增类型、符号、字段和模块均为拟议接口，不是现有 API。需求见 prd.md，HTTP 契约见 api.md，实施门禁见 implement.md。D1（实例独立执行、同模型物理权重共享）已由用户确认；原共享串行组停机 Q1 已撤销。完整接口与实施仍需后续批准。

## 1. 架构决策

采用单进程控制面分配、实例执行所有权、独立权重驻留所有权和版本化插件协商。共享权重不经过共享逐帧推理队列。

```text
DB 实例 affinity/revision -> Pipeline -> infer PlacementManager
  DeviceInventory + PlacementSolver + ExecutionLedger + WeightLedger
                             |
     +-----------------------+-----------------------+
     |                       |                       |
实例 A 专用 Worker      实例 B 专用 Worker      实例 C 专用 Worker
 私有 detector/session   私有 detector/session   私有 detector/session
 私有 embedder/session   私有 embedder/session   私有 embedder/session
 私有 IO/workspace       私有 IO/workspace       私有 IO/workspace
 宿主分配 core 0         宿主分配 core 1         宿主分配 core 2
     |                       |                       |
     +---- 同模型只读权重引用（不传递逐帧推理请求）----+
             detector WeightOwner + embedder WeightOwner
             模型加载/派生/回收专用控制 Worker（有界）
```

优先复用宿主已有实例 OS Worker 执行插件全部同步推理，移除人脸插件内部共享串行推理 Actor，不再叠加一条每实例内部推理线程。WeightOwner 的控制线程只处理低频根资源管理，不执行所有摄像头的推理。实例数/线程槽在启动配置中有硬上限，不按帧生线程。

### 分层

- types：平台无关 AffinityIntent、运行态快照及错误分类；不放 rknn C enum 或硬件调用。
- db：实例 affinity 持久化、配置事务和 revision；不存 reservation 或硬件瞬时负载。
- infer：设备 inventory、分配、协议适配、资源生命周期；硬件 ABI/平台差异止于此与插件 HAL。
- pipeline：业务实例生命周期、配置收敛、帧边界替换；不计算 mask、不探测 SoC。
- algo-sdk：独立的版本化 wire 类型与边界解析，不依赖宿主 types；双侧通过协议 fixture 保持一致。
- api：参数/鉴权/DTO/错误映射，只读宿主快照；app：装配唯一 manager 和容量配置。
- web：既有数据类型、归一化与保存兼容；不新增调度 UI。

## 2. 身份与资源所有权

### 四类资源身份

1. ExecutionGroupId：一个实例运行代际拥有的一组必要模型 session；离线 Worker 是独立 execution group，不是摄像头成员。
2. WeightId：同一模型的一份物理权重根资源；一个执行组通常引用多个 WeightId。modelKey 是资源计划中的规范化制品键，多个业务角色的别名先映射到同一制品键；角色映射由插件声明，不能假设 detector 与 embedder 是同一模型。
3. ReservationId：执行预算身份；WeightReservationId：权重根预算身份。两类均使用 boot ID + 单调序列，独立释放。
4. InstanceId/runGeneration：业务/执行尝试身份；离线请求使用有界 request ID，不把人员信息放进 ID。

WeightKey = 已加载动态库/Runtime 共享域 + device identity + 模型 SHA-256 + 影响权重兼容性的初始化配置/profile + 制品代际。不得含 instanceId/coreIndex；不能因 A/B 核心不同复制根权重。同一包代际的重复 RawAlgoLibrary 打开须关联同一驻留 registry/已加载库身份；不同动态库里的 static/Arc 不天然共享，不能把跨库同路径当成同一份。

Execution key 包含 instance/run 身份；候选替换有新 reservation。WeightId 与 execution/group generation 不混用，首个使用者不是权重生命周期的主人。不同制品/不兼容配置/设备/加载域不合并权重；跨域重复费用如实计入，不声称进程全局一份。

新协议分别声明 executionIsolation 与 weightSharing 能力，不能继续用一个 packageShared 枚举同时表达二者。老插件只给 unknown/legacy 诊断，不作为 D1 的可用实现；不能在新架构准入失败后隐式切到 legacy。

### 代际及版本边界

| 标识                      | 变化条件                                               | 禁止混用                                                           |
| ------------------------- | ------------------------------------------------------ | ------------------------------------------------------------------ |
| packageGeneration         | 已加载制品身份变更                                     | 路径相同不代表同一制品；禁止原地覆盖仍在使用的模型/动态库          |
| groupGeneration           | 同组执行资源重建                                       | 权重引用变化不当作执行换代；不得释放旧代际来抵新代际峰值           |
| weightGeneration          | 同 WeightKey 根资源重新创建                            | 子实例换核不换根；旧根及隔离子未清理不得以新根抵账                 |
| topologyGeneration        | 设备身份、可用核心、Runtime 映射或亲和能力发生实质变化 | 普通利用率采样不换代；无关设备变更可复核后继续，不无条件杀死健康组 |
| targetRevision            | 启动/应用请求取得的持久化实例版本                      | 不得在完成时改为数据库最新版；0 可能是合法初始版本，缺失须显式表达 |
| runGeneration / attemptId | 当前进程内一次启动/配置执行                            | 同一 revision 重试也隔离迟到结果；不等同帧 PTS                     |

已申请 reservation 永远按其原身份释放，不能因为拓扑已换代而拒绝旧资源的合法清理。准入/Ready 发布检查与 release 检查分开。

### AlgoLease、执行 owner 与权重 owner

AlgoLease 保留包代际和热态意图；ExecutionOwner 持有私有 Worker/session 与 execution reservation；WeightOwner 持有根 context、权重 reservation、动态库及派生引用。三者不能用一个 ref_count 替代。

权重 registry 用单航班 loading slot，短锁内登记，锁外专用 Worker 初始化。有限等待者的取消不取消其他已获准使用者；所有等待者撤销且初始化已在 FFI 中时，owner 继续持有至安全清理。每个派生 session 在创建前即持有对应权重强租约，确认销毁后才释放。派生成功后的所有权移交、失败清理与完成回执槽位必须在准入时保留，不能把 child 的回收依赖于可能已满的普通请求队列；停止时先关新请求，控制 Worker 仍处理已保留的清理工作。

Warmup 只预热/保活根权重，不再为共享而创建一套完整业务实例。根 context 可能自带 internal/IO 开销，必须计费；不能宣称它是零开销纯权重对象。必要探测/预热推理使用受限临时执行资源，完成后清理，不把根 context 并发充作生产 session。根权重可按现有 60s 意图冷却，停用实例私有执行资源不跟着全量保留。

### 释放顺序与取消

停止实例接单 -> 丢弃未执行旧帧/结束在途调用 -> 在所属 Worker 清理输出/IO 导入与子 context（依目标 SDK 的精确释放顺序）-> 清理确认 -> 释放执行 reservation 与子租约。最后一个子资源/派生操作结束且无 warmup 冷却保活后，权重控制 Worker 才销毁根 context -> 清理确认 -> 释放权重 reservation -> 允许卸库。

首实例退出不销毁根；任何子 session 隔离、派生/析构未确认时均保留对应权重和库。宿主外壳 Drop、引用计数归零、通道断开都不是硬件清理证明。根资源清理超时独立隔离并占额，不能通过重新创建同 WeightKey 逃避不确定状态。

当前离线路径 `package.rs:1570-1597` 把 lease 留在 async future 且随后按名重新查包。须将同代际 package、执行 reservation、权重 lease 一起移入真正同步执行任务；取消只结束等待，不回收仍在执行的资源。

## 3. 配置与放置语义

实例字段 affinity：auto 默认 spread；manual 必须有 deviceId 与 coreIndex。详见 api.md。不引入 Task 继承，不提供裸 mask API。

- spread：按已分配 execution group 数最少选择，同分确定性轮转；这是放置启发式，不是实时最空闲。
- 每实例独立选择核心；共享物理核心受容量限制，不代表独占。
- manual：严格检查设备/核心/模型能力，不取模、不自动迁移其他实例。同权重的 core 0 与 core 2 请求互不冲突。
- 换核：仅重建目标实例子 context，不在业务 update_config 内调核，不改变 WeightKey/root。额外私有预算充足时创建候选，在 Ready/版本栅栏通过后帧边界切换，旧实例排空退出；失败保留旧健康实例。
- 容量不足不自动停止任何实例；明确失败后用户可显式停目标实例再启动。兄弟实例继续运行，不能把整组停机作为共享权重的前提。
- 若目标 Runtime 跨核派生实际复制权重或必须停全部子 session 才能派生/销毁，则该 profile 不满足 D1，不能隐藏在换核实现里。

### 有界恢复批次

app 读取启用/媒体恢复资格内的实例快照，经 pipeline 交 infer。按稳定 instanceId 排序，同批先校验并预留 manual，再按剩余容量分配 auto；同核满额按固定顺序明确拒绝，跨不同核心不求约束交集。未来才恢复的摄像头不长期占额。

对 WeightKey 单航班初始化，但不以首实例核心绑定整个根资源的派生选择。容量准入的短 recovery barrier 阻止离线/warmup 抢占尚未完成的批次规划，不等待全部摄像头 IO/FFI；超限立即 busy。各实例持独立 targetRevision/attempt，某个克隆失败只失败该实例，不回滚其他成功实例。若共同根初始化失败，所有依赖该根的本次尝试明确失败，其他模型继续运行。

不再设计 Q1 的共享串行人脸组维护端点或组级换核。

### 设备身份

复用 npu::NpuDevice 发现结果。deviceId 是宿主 inventory 暴露的稳定逻辑字符串；runtimeDeviceIndex 是内部运行时映射，不是 PCIe 物理卡号保证。拓扑携带来源、可用性、generation 和 affinity capability。

未发现设备为空集合；未知核心集合不能 max(1) 伪造可绑定核心。RKNN 默认 auto 可在可用但不可绑定的设备上以 runtimeManaged 运行，仍消耗设备级预算；manual 拒绝。CPU/Core ML 非本任务的核放置目标，不伪装成 RKNN。

“复用”指共享探测适配器和缓存，不复用已丢失信息的默认值。当前 `npu/rknn.rs:185-205,349-367` 的默认一核/补零/复制全局负载，以及 `monitor.rs:68-72` 的吞错，须在源头改为显式状态，再适配既有概览响应；不能在新接口外层用 Option 恢复已经伪造的值。

- Discovery：设备存在性、身份来源、最近验证时间、错误类别；权限不足/探测失败不等于空设备集合。
- Capability：设备核心集合与“选定插件/模型/Runtime profile 支持的核心选择”分离；设备支持不保证该制品支持。绑定白名单依据实际加载库的指纹与制品身份，不能只凭系统同名 `.so`。
- Telemetry：逐核、全局两种采样范围不混用；缺失字段为 null，保留 last-success 时间及 stale。硬件采样 unavailable 不使可信静态拓扑自动失效。

发现/定期验证/遥测采样使用固定专用 Worker、有限等待和有界快照；不在 Tokio worker 或 handler 同步读设备/调用 SDK。稳定 ID 的来源变化必须显式表现为 identityChanged，不可把旧 manual 悄悄映射到另一设备。

## 4. 双层账本、并发与容量

一个短持有 Mutex<LedgerState> 内原子预留执行资源和缺失的权重根预算，已有 WeightKey 只加派生保留引用。不在持锁期间做 IO/FFI/await/join；SDK 根操作由专用有界控制队列串行，实例逐帧路径不经过该队列。锁内选择/提交而非先读后写，避免多请求同时创建相同根或突破核心上限。

配置上限覆盖：execution groups（全局/设备/核心）、weight owners、每权重子 session 数、宿主 Worker 槽、初始化/派生并发、离线 Worker/请求/等待者、缓存/库存规模、初始化/停止/回执 deadline、设备内存预算。禁止按请求无限扩线程；数值范围/层级/溢出在 app 装配时校验，优先复用已有配置。

```text
预算 = Σ 每个 WeightOwner(一份权重 + 根 context 内部开销)
     + Σ 每个 ExecutionOwner(私有 context/workspace/IO/预处理与队列)
     + 未归入上述项的宿主媒体/池保留开销
峰值还包括：初始化临时量 + 候选替换私有量 + 新旧制品根 + 隔离资源
```

上述是去重后的内存分类，不是直接相加每个 context 的 RKNN_QUERY_MEM_SIZE：Runtime 可能对每个子 context 重复报告逻辑权重大小。根 context 的 internal/IO 不属于权重，不能漏计；相同可写 workspace/IO 不跨并发实例复用。共享预处理池必须给每个在途任务独占 buffer 租约且计入总峰值。

根、私有 execution 分别持 reservation，分别清理确认后扣账。reserved/initializing/ready/cooling/draining/quarantined 均占额；某子失败后根仍被其他子使用时只退该子的私有预算。旧拓扑 generation 不阻止旧资源合法 release(id,generation)。不以 algoId 批量删除。

隔离数量是熔断阈值而非可丢弃 owner 的容器裁剪线；所有已准入者同时挂死仍全部保留。任一根/执行隔离阈值触发则停止新准入，包括离线、warmup、自检和重试；根预算及依赖边不因阈值清零。

模型 profile 与实际制品/库指纹、实际 session 集合绑定；未知预算不按零准入。可预见扩增先预约；SDK 内部分配无法逐次拦截，实测超出预算时记录 overBudget、停止新准入并安全清理/隔离，不截断实耗或宣称逻辑额度是物理硬限额。外部进程资源只能观测。

默认/降级核心模式的执行组仍占设备预算，核心竞争保守估计，不伪造每核零占用。权重 owner 自身不因被三个核使用就计成三份权重，也不作为正在推理的独立核心组。

## 5. 生命周期状态机

```text
Reserved -> Initializing -> Ready <-> Cooling -> Draining -> Released
    |             |            |                   |
    +-------------+------------+-------------------+-> Quarantined
    （取消/失败：无资源时 Released，否则先 Draining）       |
                                      确认清理及退出后 Released
```

ExecutionOwner 和 WeightOwner 分别使用此状态机，依赖边决定权重最早释放时机；Cooling 主要用于权重保活。失败是一次应用的结果，不是证明资源消失的生命周期终态。状态与 applicationStatus 分离：新架构 Ready 需要独立 execution 与已验证共享权重；亲和状态可以 acknowledged/runtimeManaged/degraded，manual 仅 acknowledged。legacy 的 unverified 不算 D1 Ready。

- Reserved：额度先占用，未创建硬件；取消可直接 RAII 回滚。
- Initializing：启动 owner 独立于请求 future；取消发出停止意图，不销毁仍在 FFI 中的资源。
- Ready：当前 group/package/run 票据才可发布；拓扑变化先复核本次分配仍有效，普通采样不使票据失效。过期结果转清理，不能发布给新实例。
- Cooling：权重无在途派生/子执行使用，仅剩有界 warmup 冷却保活；新的权重 lease 作废对应代际定时器。
- Draining：停止接单并保留额度，等待实际资源退出。
- Quarantined：线程未退出或资源清理无法确认，保留 reservation、库及 owner；达到隔离熔断阈值停止新准入，不无限重试重建。不能 TTL 清账。
- Released：确认实际退出后释放一次。进程崩溃重启时不重放旧 reservation，只从 DB 意图重建。

权重控制 Worker 同样适用。移除旧共享推理线程时，其 Drop/join 旧问题须由正常/失败/退出测试覆盖；不能只调整宿主计数。

### 完成信号与清理证明

- `worker.rs:526-540` 当前先发 exit 再析构 backend；必须改成在所属线程内完成后端清理后发送完成信号。停止方和初始化失败方都使用同一有界退出协议，不得在 Err/Disconnected 分支直接无期限 join。
- completion 消息仅作唤醒，最终 join 前仍确认线程结束；TLS 析构、panic 展开和内部线程退出同样受 deadline 约束。超时只把 owner 转交 supervisor，不在 Drop 中无限等待。
- SDK 扩展声明销毁语义：受管理 v1 插件必须在最后一个受管理资源持有者释放时同步完成内部线程/session 清理，不允许未申报的 detached Worker。库关闭不能早于清理。老插件须以经审核的生命周期 profile 建立此约束；无证明时保守保留额度和库引用，不假称已回收。
- C ABI `instance_destroy` 无返回值且可能捕获 panic；因此宿主线程退出本身仍不充分。新扩展需要可查询的执行资源及权重根各自的清理回执，存于仍存活的 library/resource 上下文，不读取已销毁 instance 指针；清理 panic/回执丢失保留为 cleanupUnverified。ABI 签名在阶段 A 冻结。
- reaper 在短锁内仅移出已完成对象；join、库析构、日志在锁外。thread finished 与 resource cleanup confirmed 分开记录；未确认清理不得为了腾额度重启同组。
- 正常停止、启动错误、最后权重引用退出、动态库替换、应用退出共用此规则。进程级退出以明确 deadline 结束服务；驱动卡死是否可恢复由板端验证，不能承诺杀进程一定清除设备故障。

## 6. 基础 ABI 不变的插件协议

### 6.1 必须解决的现有入口限制

现有 export_algo! 直接将完整 config_json 反序列化为插件 Config；InitContext 没有原始 JSON 或 placement。仅新增 extract_placement_core_mask() 不会让现有插件自动得到配置。

拟改为 SDK 边界一次解析 object：剥离 __heimdall_placement，受检构建 PlacementContext，再将剩余 object 交给插件 Config。InitContext 增加 Rust 侧可选 placement；这不改 C ABI，但属于 SDK 源码 API 变化，所有初始化构造点和本地 runner 必须同步。

wire v1 例子（内部字段沿用 snake_case，与 REST 分离）：

```json
{
  "confidence": 0.5,
  "__heimdall_placement": {
    "version": 1,
    "reservation_id": "boot-7:42",
    "group_id": "opaque-group",
    "generation": 3,
    "device_id": "rknn-npu0",
    "runtime_device_index": 0,
    "strategy": "pinned",
    "core_mask": 2,
    "required": true,
    "weight_sharing": "required",
    "weight_bindings": [
      {
        "model_key": "detector-artifact",
        "weight_id": "weight-1",
        "generation": 1
      },
      {
        "model_key": "embedder-artifact",
        "weight_id": "weight-2",
        "generation": 1
      }
    ]
  }
}
```

宿主生成元数据；用户 algoParams 出现该键一律拒绝。字段缺失可兼容老宿主，字段存在但版本/类型/范围错误必须失败；mask checked narrowing、拒绝非法位和不一致字段。不读取进程 RKNN_CORE_MASK。普通 update_config 不得修改 reservation；对未变化元数据只校验后剥离，不触发重绑。

### 6.2 能力与回执（拟议可选扩展）

采用与现有 gallery optional symbol 一致的加载模式，新增独立版本化 placement 扩展，不修改 AvAlgoAbi、AvAlgoLibraryInfo 或实例参数布局。

扩展覆盖：创建前查询执行隔离/共享能力与有界模型资源计划（模型 key/哈希/兼容 profile）；权重根 prepare/release；实例创建后的子 session/核心/权重绑定回执；执行及根资源销毁后的清理回执。宿主先对资源计划和可信 profile 核预算，再下发 weight_bindings，不能先初始化后补账。正式函数签名、POD 字段大小/偏移和最大载荷须在实施第一阶段用双侧协议测试冻结；未冻结不得接入硬件代码。接口只能返回调用者拥有缓冲区中的固定上限数据，不跨 ABI 传 Rust 引用/Vec/String。能力查询不得惰性加载模型。

协议冻结清单：size/version、resource kind、execution/weight reservation、package/group/weight generation、可识别的 operation/status/reason、字符串编码与长度、payload 最大字节、零长度/空指针语义、缓冲区不足时的所需长度、未知枚举处理、调用线程与串行要求。长度先校验后分配；不可按插件给出的任意长度增长；不跨库释放内存。wire 顶层 generation 为 groupGeneration，weight_bindings 内 generation 为 weightGeneration，包身份由加载库和 reservation 联合校验，不把一个字段解释成多种代际。

回执至少包含 reservation/group generation、状态、requested/accepted core selection、reason enum 与 SDK status；各必要 session 的应用结果必须聚合，不能只报检测器成功而漏掉 embedder。初始化回执在实例所属 Worker 串行读取；执行和根资源的清理回执由仍持库引用的有界 supervisor Worker 串行读取。两者只读缓存，不在 HTTP 路由调用 FFI，不执行逐帧硬件查询。清理记录容量覆盖所有已准入执行组及权重根，在宿主确认读取前不能丢失；确认后的记录回收协议同属阶段 A 的 ABI 冻结范围。

回执须含有界 model/session -> WeightId/root generation 关联、执行隔离状态、私有 session 数、共享机制/证据 profile 与 requested/accepted core。实例核心确认不代替权重共享确认。缺失必要共享/清理能力或回执时，新架构 auto/manual 均失败；不以 unverified 放行。格式非法/错代际视为协议错误，不能把 AV_OK 当所有约束已满足。老插件无扩展时仅沿原 legacy 路径诊断，不注入未知元数据；新架构准入返回明确不支持。

新插件在老宿主/本地 runner 无注入时使用 Runtime 默认核心，但仍走插件内有界的共享权重 provider 和独立 session，不退回旧串行 Actor；本地 owner/额度与宿主受管理域明确区分，不宣称已有宿主 placement 账本。缺少共享能力应明确失败，不能为兼容自动复制权重。只有显式实现扩展的插件才声明支持；导出宏不可替所有插件假定支持。

生产接线须覆盖 `models/yolo.rs::GenericDetector::init -> RuntimeSession::open -> RknnSession`，不能只改手写 plugin.rs；用可选 placement/options 路径传递受检请求，不在通用模板硬编码平台 mask。`model.rs::SharedWeights/Core` 的本地轮转不是宿主账本，受管理 session 不得再次自主分核。与 `09-30-edge-torch-algo-ecosystem` 共用这些入口，阶段 A 冻结接口及文件归属，C 阶段先复核合入后的调用图。

### 6.3 RKNN 共享权重实现路线与 Worker 归属

官方 RKNNRT V2.3.2 手册 p21–22 明确 `rknn_dup_context(rknn_context *context_in, rknn_context *context_out)` 创建同模型新 context 以复用权重，适用于多线程执行；这是首选路线，来源固定版本/哈希见 research.md。头文件本身描述较简略，不能继续说“官方没有共享权重合同”；但文档支持不代表部署 BSP 已验证。

每个模型 WeightOwner 持有独立根 context，不作为摄像头生产 session。控制 Worker 串行完成根初始化、`rknn_dup_context(&root, &child)` 及最终根销毁；成功创建的 child 以 HAL 内部独占所有权胶囊一次性交给指定实例 Worker，此后设置核心、绑定独占 IO、推理与销毁均在该 Worker。中途取消/发送失败，胶囊由受控清理路径接管，不能泄漏或只丢裸句柄。

一次性转移不是跨线程并发使用；正式实现前须以目标 Runtime 资料和测试确认 transfer、并发派生/兄弟运行、销毁顺序可行。不得仅因句柄是整数就 unsafe impl Send/Sync；原始 context 不进入宿主安全层或 REST，根永不并发被多个克隆操作访问。若目标 SDK 无法满足此线程模型，先补可证明的线程归属方案并评审，不能退回共享串行推理。

绑定在每个 child 上执行；派生可能继承的默认状态需显式校验/设置，不让源 context 的核心决定所有实例。根保持到所有子 context、进行中的派生以及隔离子对象清理确认之后再销毁，不依赖“先毁根也安全”的未验证假设。

不使用 `RKNN_FLAG_SHARE_WEIGHT_MEM` 作为同模型复制的快捷替代：V2.3.2 手册说明它主要服务旧式多分辨率/去权重模型，此任务不改模型制品/转换。`rknn_set_weight_mem`、外部分配和共享 internal/workspace 不纳入默认路线；不能为省内存把并发可写资源混成一份。

`SharedWeights<ModelWeights>` 目前只有 Arc/轮转封装与 mock 实现，生产 RuntimeSession 仍独立 open。新增真实 RKNN shared-weight provider 并接通 GenericDetector/手写插件，复用现有模型抽象而非再造上层调度器。受管理 session 只用宿主选择，移除 helper 的第二次自主轮转；静态 Arc 不证明物理共享。必要 trait/生命周期改动与关联 SDK 任务的已合入基线协调。

### 6.4 人脸实时、预热、离线与自检

人脸 SharedModels 改为共享模型制品/权重 provider，不再持共享逐帧推理 Worker。每个 FaceRecognizer 的 detector/embedder 及实际需要的可选模型子 session 在本实例 Worker 内使用。注册专用检测模型仅分配给确实需要的执行资源，不能按摄像头盲目复制一套离线模型工作区；保持已验证的检测/识别语义。

离线提取使用独立有界 Worker/session，引用相同域内的相同模型 WeightId，不借用正在执行的摄像头 context。旧 av_algo_extract_face 无 placement/session 参数，需加法可选离线执行扩展：绑定 execution reservation、包/运行代际、weight bindings 到明确的离线 session 句柄，再提交请求。基础导出保留给旧调用者，不能按 package_root 猜选某个摄像头或默认新建全权重。精确 ABI 在 A 阶段冻结，宿主新路径不绕开准入。

安装自检子进程不能共享生产进程的权重，这属于明确的跨进程非目标：单独计临时设备预算，确认子进程退出后释放。不能为安装验证挤占生产已保留容量；不重复全量自检已受信包。自检结果不作为生产多实例共享证据。

## 7. 错误与降级

不按“任意非零 -> warn”实现。亲和降级与共享能力失败分开：

| 情况                                     | auto                                                        | manual       |
| ---------------------------------------- | ----------------------------------------------------------- | ------------ |
| 共享与隔离验证通过，SDK 接受核心         | acknowledged                                                | acknowledged |
| 已验证亲和设置不支持，共享仍满足         | 清理失败 child，最多一次从同根派生默认 child；成功 degraded | 失败         |
| 无共享符号/能力/profile、共享回执缺失    | 明确失败                                                    | 明确失败     |
| 派生失败、模型不匹配、OOM、未知 SDK 错误 | 原分类失败并清理本次部分资源                                | 同左         |
| 老插件仅有 legacy 能力                   | 不作为新架构启动；原兼容路径标明 unverified                 | unsupported  |
| fallback 再失败                          | 失败，不继续循环                                            | 不适用       |

默认重试也必须走共享根，禁止用每实例 rknn_init 偷偷满足 auto。一个必要模型失败就不发布该执行组 Ready；其他实例及其根引用不被回滚。权重初始化失败可影响依赖它的尝试，但不扩展为所有算法失败。

合法核心、RK3568 单核默认语义及错误码以实际部署头文件/库 profile 为准。V2.3.2 文档说明 set_core_mask 面向 RK3576/RK3588；不能把 RK3568 的该 API 失败直接解释为不存在 NPU。单核是否可确认 manual core 0 需独立、可审计的能力证明，不靠假成功。

## 8. 数据库、Apply 和恢复

### 保存与身份

新增 algorithm_instances.affinity_json（加法迁移、旧值 auto），与 params_json 分开。TaskRepo 在单事务内解析“未提交/重置/替换”，不能先在 API 读旧值再以全量对象回写；缺失值须一直传到事务内合并。不能通过 Option<T> 的普通反序列化合并缺失和 null。

当前仓储参数没有 instanceId，按 algorithm_id 匹配。新参数携带受检可选 ID：显式 ID 必须属于目标 task 且 algorithm_id 一致；不存在/跨任务/重复 ID 明确拒绝。旧客户端缺失/空 ID 时按同任务唯一 algorithm_id 匹配已有实例并保留 ID/affinity；新增由服务器生成。保持 `UNIQUE(task_id, algorithm_id)`，不按数组下标、不新增同算法多实例。

algorithmInstances 显式数组仍为集合替换；省略且无旧单算法意图时保留。当前 API 还有 algorithmId 旧桥接分支，须冻结原行为 fixture，再保证保留下来的实例 affinity 不被清空；不能简单宣称所有省略集合都绕过旧桥接逻辑。

结构/身份/可信拓扑校验在任何写操作前；manual 明确非法或无法验证时拒绝。`routes/task.rs:343-357` 当前提前写 streamMode，需把此次 camera 配置写入与任务/实例/configRevision 校验纳入同一仓储事务。DB 错误/乐观锁失败不留下部分更新，硬件/IO 不进入事务。有效配置提交后仍须在运行时重新准入，不把保存前检查当资源预留。

### 不可变应用票据与条件回写

每次应用持有 `(instanceId, targetRevision, runGeneration/attemptId, packageGeneration, reservationId?)`，数据来自提交后的同一快照。InstanceLaunchConfig、冷启动、增量更新、重试都传真实 targetRevision；不能再以 0 作“未知版本”并在结果返回时查询当前版本。

1. desiredRevision 只在有效期望字段变化时递增，单独规则镜像等无关字段不引起硬件重建；affinity 归一化等价时不创建新组。
2. 同实例应用由唯一 owner 维护当前 attempt 栅栏；同 revision 重试也有新 attempt。候选发布到当前运行槽前检查当前票据，过期候选只清理。该 owner 的 DB 回写同样串行有界，旧 attempt 的在途写入未完成前不得并发提交新 attempt 结果；不能仅在 await 前检查内存 token 后放任同 revision 乱序写回。
3. Applied/Failed/Pending 回写均在 Repository 使用带 `instance_id + desired_revision = targetRevision` 的条件更新，检查 affected rows；禁止先 SELECT 比较后无条件 UPDATE。失败理由同样受版本保护，删除后结果不复活实例。
4. 配置可在硬件切换与 DB 回写之间再次更新；此时记录真实运行槽的 runtimeRevision，DB 保持新意图 pending/failed，后续有界收敛处理。不得为制造相等而把旧回执贴到新 revision。
5. DB 回写失败不会使已创建硬件自动消失。保留 owner/当前运行快照和有界重试；无法恢复时明确失败，不能撤销一次已发生的切换或释放仍运行的资源。

保持 configRevision 乐观锁和已有 applyState 协议。历史 appliedRevision 表示此前配置收敛，不证明当前进程存在硬件；实例未运行/停用时不伪造 placement acknowledged。运行态始终由 manager 当前 boot/run 快照提供。

### 部分成功与恢复

冷启动/恢复返回每实例 outcome，而不是遇到一个 placement 错误就 shutdown_workers。先保留已成功 Worker，为失败者记录机器原因；pipeline 挂载成功集合。媒体初始化失败才清理本次摄像头启动资源。所有实例失败则结束本次 AI 分析资源，不释放其他消费者的 StreamHub 租约。

API/app 恢复回写使用每实例结果，不调用统一错误覆盖健康实例。任务状态复用现有 `aggregate_task_instance_status`（Error 优先），该摘要不是停机命令。增量更新失败保留原健康 Worker；设备丢失拒绝新准入并诊断，不自动迁移。持久化只保存意图，不把 transient mask/reservation 当作重启事实。

实例执行组有预算且票据仍有效才在帧边界替换，旧结果/旧错误均按既有栅栏丢弃；共享权重不使实例共用 revision 或停机边界。

## 9. 观测、安全与性能

REST 快照来自 manager，不临时打开 NPU。执行组快照与权重根快照分开；前者含实例/核心/私有 session 与 WeightId 引用，后者含根代际/共享机制/证据/引用数，不能带一个共同核心。实例视图组合期望、当前槽与最近尝试，字段见 api.md。无 reservation 的失败也必须通过任务实例响应可查。分页结果受固定上限约束，发布 snapshotGeneration；实例列表与组列表不承诺跨请求事务一致性。

硬件利用率 unavailable 为 null，acknowledged 仅指插件/SDK 确认；保留 topologyGeneration、sampledAt、stale、execution/weight/隔离占用。逻辑预算、确认过的实际 session 数与硬件采样不得使用同一个指标名。

日志含不透明 group/reservation/instance ID、代际、状态转换、reason；按类首次与节流汇总。Prometheus 等计数标签只使用平台/状态/reason，不使用 instanceId 或 package path；细节走有界分页 API。

分配发生在启动/重配，不进入逐帧路径；不对已有 FrameRef 做像素拷贝。不增加每帧 SDK 设置或 JSON 解析。容量与重试均有上限。

## 10. 发布、回滚与未来边界

1. 协议和测试 fixture 先行，区分旧插件 legacy 兼容与新架构 capability/profile，不能以兼容代替达标。
2. 宿主先以 shadow 记录拓扑/组归属，不注入改变绑定的请求；新插件能力与回执再独立灰度。新插件在无注入时改用 Runtime 默认，本身也是需测试的行为变化，不能声称换插件完全不影响生产绑定。
3. 先通过权重共享/线程隔离/预算验证，再对通过核心验证的 profile 启用 spread；仅亲和不支持而共享已验证时可 runtimeManaged。缺少共享证明或安全预算不进入新架构生产准入。
4. 开放 manual（仅明确支持且已验证的 profile）。
5. 出现亲和回归可关闭新 auto spread，新 auto 使用 Runtime 默认但继续满足共享权重与独立执行；不后台改写 manual，不自动切 legacy。

降级旧宿主前导出配置、停止/显式解除 manual 并告知用户旧版本不执行该约束。旧架构不满足 D1，回滚前明确停用相关新架构实例并告知共享/执行保证将失去；不声称无损回滚。DB 使用前向兼容加法迁移，不删除 affinity 列。

Ascend 仅预留 device identity 与内存域校验：未来必须把 decoder/preprocess/model 的设备约束一起协商，不能通过 aclrtSetDevice 自动解决跨卡 device pointer。Rockchip core mask 不泛化为 Ascend core 语义。

## 11. 待冻结的技术门禁

- D1 架构已确认，旧 Q1 撤销；完整 API/兼容发布和最新摘要仍须评审，不据此启动实现。
- 可选扩展包含资源计划、权重根、实例、离线执行及各自清理回执；阶段 A 冻结精确 ABI、缓冲上限与线程归属，未通过不得接生产。
- 首先完成独立的目标板可行性验证：共享符号/生命周期、根到 child 移交、跨核并发与内存证据。板端不可行时阻塞该 profile，而非降低产品要求。
- 初始化/停止/回执/恢复 deadline、硬容量和性能阈值在实验前冻结，不用草案数字代替硬件结果。
- 现行 spec 的默认组合 mask、共享串行人脸 Worker 以及局部 SharedWeights 注释与新目标不同；此处记录迁移，不提前把规划写成当前规范。
