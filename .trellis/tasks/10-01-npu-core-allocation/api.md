# NPU 放置 API 契约

> draft，2026-10-01 工业级规划修订；待用户确认。以下为提案，不是现有接口。D1 独立执行+物理权重共享已确认，原共享组维护 Q1 已撤销。以下 HTTP 扩展仍待完整评审，不新增强制停组写端点。

## Scope

- Backend：types、db、infer、pipeline、api、app；职责见 design.md。
- Frontend：web 共享任务类型、归一化及保存回归；不新增 affinity 编辑器或调度页面。
- Shared：本文件由主会话维护，前后端实现者不得自行改契约。

## Conventions / Auth

沿用 `/api/v1`、camelCase、Accept-Language 与既有 ApiResponse：成功根信封为 `{ "code": 0, "message": "success", "data": T, "timestamp": ms }`，失败 `data=null`。绝对时间为 UTC Unix 毫秒 number/i64，可缺失的采样时间为 null；相对时长使用 Ms 后缀。版本计数为 JS 安全整数，达到范围上限须显式报错，不截断；boot/group/reservation/attempt 身份使用不透明字符串。

任务写入和新增诊断接口均挂现有 protected router，使用 `require_auth`/管理员 JWT；REST 客户端使用 Authorization: Bearer。当前不是细粒度 RBAC 系统，不虚构现有“系统读取权限”。沿用失效凭据、未初始化和审计处理；不公开内部路径、fd、指针、token 或完整底层错误文本，不增加公开旁路。

## Shared models

### AffinityIntent

```json
{ "mode": "auto", "policy": "spread" }
```

```json
{ "mode": "manual", "deviceId": "rknn-npu0", "coreIndex": 1 }
```

| 字段      | 约束                                                                                                |
| --------- | --------------------------------------------------------------------------------------------------- |
| mode      | auto / manual，未知值拒绝                                                                           |
| policy    | 仅 auto 可用；省略默认 spread，本期只接受 spread                                                    |
| deviceId  | 仅 manual，inventory 暴露的不透明 ID；1..128 UTF-8 字节，拒绝首尾空白与控制字符，不自动 trim 改身份 |
| coreIndex | 仅 manual，非负整数、受检转换为 u32，并在选定设备/插件 profile 可调度集合内                         |

manual 不表示独占；不接受裸 mask、pinned_core/pinned_device、越界取模或自动换卡。auto 不允许 deviceId/coreIndex，manual 不允许 policy；此对象未知字段拒绝，避免拼写错误变成默认值。业务 algoParams 顶层出现宿主保留键 `__heimdall_placement` 一律拒绝，包括旧单算法桥接和实例级更新入口。

同模型实例独立分配核心，不能因共享权重把不同 coreIndex 判冲突。恢复按同批 manual 先预留、auto 再分配，容量不足明确失败；权重加载单航班不决定派生实例核心。物理共享为新架构的必需能力，不添加让用户静默关闭它的 affinity 字段。

### 任务实例写入与兼容身份

现有 TaskAlgorithmInstanceDto 响应新增 `affinity`，总是归一化完整对象。请求字段为三态：

| 请求             | 已有实例             | 新实例      |
| ---------------- | -------------------- | ----------- |
| 缺失 affinity    | 保留当前事务内的旧值 | auto/spread |
| affinity: null   | 重置 auto/spread     | auto/spread |
| affinity: object | 校验后替换           | 校验后采用  |

- 显式 instanceId：必须是目标 task 的已有实例，algorithmId 一致；未知/跨任务/算法错配/重复 ID 返回 400，不按名称重新指向另一个实例。
- 缺失或空 instanceId：保留旧客户端行为，在目标 task 内按唯一 algorithmId 匹配；已有实例保留原 ID/affinity，否则由服务器创建 ID。保持 `UNIQUE(task_id, algorithm_id)`，不开放同任务重复算法。
- algorithmInstances 显式数组仍为集合替换，空数组清空；省略且未表达旧单算法意图时保留。旧 algorithmId/algoParams 桥接分支保持已验证行为，对仍被保留的实例不得清除 affinity。
- 三态和 ID 约束传至 Repository，在一个配置事务内完成合并；不能 API 先读再把“保留”变为陈旧全量覆盖。
- `PUT /tasks/{cameraId}/enabled` 仍只翻转总闸，不改 affinity 或实例分闸。
- 响应中的 placement、desiredRevision/appliedRevision/applyState 等运行字段不能作为客户端写入的授权事实。使用现有请求/响应兼容策略忽略只读回传字段，不允许它们覆盖宿主状态。

### 实例视图：配置状态与当前硬件分离

现有 algorithmInstances[] 保留 `desiredRevision`、`appliedRevision`、`applyState`，新增 `placement`。不把这些实例字段嵌进组资源记录。

以下为实例响应的相关字段子集，表示“新配置失败，旧资源仍在运行”：

```json
{
  "instanceId": "instance-1",
  "affinity": { "mode": "manual", "deviceId": "rknn-npu0", "coreIndex": 2 },
  "desiredRevision": 5,
  "appliedRevision": 4,
  "applyState": "failed",
  "placement": {
    "bootId": "boot-7",
    "snapshotGeneration": 19,
    "runtime": {
      "groupId": "group-7",
      "reservationId": "boot-7:42",
      "runtimeRevision": 4,
      "assigned": { "deviceId": "rknn-npu0", "coreIndex": 1 },
      "applicationStatus": "acknowledged",
      "reason": null
    },
    "lastAttempt": {
      "attemptId": "boot-7:attempt-8",
      "targetRevision": 5,
      "state": "failed",
      "reason": "capacityExceeded",
      "candidateReservationId": null,
      "updatedAt": 1790812800000
    }
  }
}
```

| 字段                                           | 含义与空值                                                                   |
| ---------------------------------------------- | ---------------------------------------------------------------------------- |
| affinity                                       | 持久化 requested 意图，不由运行分配反写                                      |
| desiredRevision / appliedRevision / applyState | 沿用配置收敛；历史相等不证明本进程存在硬件，停用也可完成配置收敛             |
| placement.bootId / snapshotGeneration          | 本次 manager 内存快照来源；进程重启后 bootId 改变                            |
| placement.runtime                              | 当前槽实际使用的资源关联，未启动/已停止为 null；修改失败保留旧 runtime       |
| runtime.runtimeRevision                        | 当前资源对应的真实配置版本，不取数据库最新版补齐                             |
| runtime.assigned                               | 运行资源的宿主分配，未知设备时可 null；Runtime 管理核心时 coreIndex=null     |
| runtime.applicationStatus                      | acknowledged / runtimeManaged / degraded / unverified；不能用新请求回显生成  |
| placement.lastAttempt                          | 当前 boot 最新一次执行尝试；未尝试为 null，只有失败而无 reservation 也可表示 |
| lastAttempt.state                              | pending / applied / failed；仅描述 targetRevision，不替代现行实例 applyState |
| lastAttempt.candidateReservationId             | 已创建候选的资源身份，否则 null；失败后可能关联仍在隔离的资源                |
| reason                                         | 稳定机器枚举或 null；不匹配本地化日志文本                                    |

GET 以 DB 意图加当前 manager 快照按 instanceId 组合，不调用 FFI；快照中的 runtimeRevision/targetRevision 不因 DB 更新而重标。跨 DB/manager 不承诺原子事务读取，消费者按版本辨识差异。停机后 runtime=null；失败资源若仍隔离继续在组列表可查。最新尝试按现有实例容量有界保存，实例删除后清除其诊断关联，隔离资源仍由独立 reservation 持有。

### GroupPlacementSnapshot（独立执行资源）

保留 groupId 命名以表示一个实例的一组模型 session，不再表示多摄像头共享推理线程：

```json
{
  "groupId": "group-7",
  "reservationId": "boot-7:42",
  "groupGeneration": 3,
  "topologyGeneration": 2,
  "kind": "instance",
  "instanceId": "instance-1",
  "executionIsolation": "instance",
  "assigned": { "deviceId": "rknn-npu0", "coreIndex": 1 },
  "applicationStatus": "acknowledged",
  "lifecycle": "ready",
  "privateSessionCount": 2,
  "inflightCount": 0,
  "weightRefs": [
    { "modelKey": "detector", "weightId": "weight-1", "weightGeneration": 1 },
    { "modelKey": "embedder", "weightId": "weight-2", "weightGeneration": 1 }
  ],
  "reason": null,
  "updatedAt": 1790812800000
}
```

| 字段                | 契约                                                                                                                  |
| ------------------- | --------------------------------------------------------------------------------------------------------------------- |
| kind / instanceId   | instance 对应一个业务实例；offline 为独立离线 Worker，instanceId=null                                                 |
| executionIsolation  | instance / unknown；unknown 仅表示 legacy，不能冒充新架构就绪                                                         |
| privateSessionCount | 已创建且未确认释放的本执行组私有子 context 数，未知为 null；不含共享根                                                |
| weightRefs          | 有界 modelKey/WeightId/generation 关联；modelKey 为资源计划中的规范化模型制品键，业务角色别名先映射，不是服务器文件名 |
| applicationStatus   | pending / acknowledged / runtimeManaged / degraded / unverified / failed；仅描述亲和确认                              |
| lifecycle           | reserved / initializing / ready / cooling / draining / quarantined；released 移出列表                                 |
| inflightCount       | 本执行组未结束调用数；计数为 0 不证明 context 已销毁                                                                  |

acknowledged 仅代表 SDK/插件核心确认，不是物理权重证明或硬件占用。runtimeManaged 为主动默认核心策略；degraded 为已验证亲和错误后的有界回退，reason 必填；unverified 仅 legacy 兼容显示。新架构 Ready 要求执行隔离及所有必要 weightRefs 的已验证共享关联；manual 还必须 acknowledged。

一个实例可同时有旧执行组与候选/隔离组，它们是不同 reservation/run，不是两个业务实例。当前运行槽在任务响应中明确选择；不能让候选覆盖旧组的版本。未知旧插件按保守计费单位展示，不声称实际独立 session 数。

### WeightSnapshot（权重根资源）

```json
{
  "weightId": "weight-1",
  "weightReservationId": "boot-7:weight-1",
  "weightGeneration": 1,
  "deviceId": "rknn-npu0",
  "sharingDomainId": "library-domain-2",
  "modelKey": "detector",
  "sharingStatus": "verified",
  "mechanism": "rknnDupContext",
  "evidenceProfileId": "profile-5",
  "lifecycle": "ready",
  "consumerGroupCount": 3,
  "childSessionCount": 3,
  "pendingDerivations": 0,
  "quarantinedChildCount": 0,
  "warmupHeld": true,
  "memory": { "weightBudgetBytes": 8000000, "rootPrivateBudgetBytes": 2000000 },
  "reason": null,
  "updatedAt": 1790812800000
}
```

例子内字节数仅为 JSON 格式示例，不是任何模型的已测预算。

| 字段                                              | 契约                                                                                                                          |
| ------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------- |
| weightId / weightReservationId / weightGeneration | 权重资源、预算和根代际身份，独立于执行组；重建不复用旧资源票据                                                                |
| sharingDomainId / deviceId / modelKey             | 共享边界与规范化模型键；同路径/同名不同域不能合并，不返回路径/fd/指针                                                         |
| sharingStatus                                     | pending / verified / unverified / unsupported / failed；verified 需绑定已审核目标 profile 和当前根/子回执，不只读取插件布尔值 |
| mechanism                                         | rknnDupContext / unknown；本期不宣称尚未接入的共享机制                                                                        |
| evidenceProfileId                                 | 经验证模型/Runtime/设备合同的不透明标识，缺失为 null；不代表 GET 时重新测过物理内存                                           |
| lifecycle                                         | 同资源生命周期枚举；冷却保活/隔离继续计费；released 不保留无界历史                                                            |
| consumerGroupCount                                | 已关联该根的不同执行组数，包含初始化/隔离；一个组引用多个角色不重复计组                                                       |
| childSessionCount                                 | 成功派生且未确认释放的子 context 数，包含隔离子对象；不含根 context                                                           |
| pendingDerivations                                | 已准入但未完成的派生数；取消等待者不等于此数可提前减少                                                                        |
| quarantinedChildCount                             | childSessionCount 中清理未确认的隔离子集                                                                                      |
| warmupHeld                                        | 根资源是否有独立预热/冷却保活，不表示额外完整推理实例                                                                         |
| memory                                            | 去重后的权重和根私有预算（Bytes、非负 JS 安全整数）；未知字段 null，不是实测 RAM/CMA；未知预算不准入新架构                    |

WeightSnapshot **没有 assigned/coreIndex、实例 revision 或统一 applyState**。多实例可以不同核、不同版本，共享只读权重；释放或换核某个实例不改变其他实例的 placement。根资源与组分别分页，不把无界消费者列表嵌入快照。

### WeightConsumerSnapshot

```json
{
  "groupId": "group-7",
  "reservationId": "boot-7:42",
  "kind": "instance",
  "instanceId": "instance-1",
  "childSessionCount": 1,
  "pendingDerivations": 0
}
```

每行是一条权重根到执行组的有界关联；同组对该根多个角色合并计数。kind 与 GroupPlacementSnapshot 一致，离线 instanceId=null；详情按 groupId 查询执行列表，不暴露人员/文件名。根还在初始化时可出现 pendingDerivations>0、childSessionCount=0。

该视图取代前一版未实施的 GroupMemberSnapshot；没有“停共享执行组所有摄像头”的操作语义。

### TopologySnapshot

```json
{
  "bootId": "boot-7",
  "topologyGeneration": 2,
  "snapshotGeneration": 19,
  "observedAt": 1790812800000,
  "status": "available",
  "reason": null,
  "devices": [
    {
      "deviceId": "rknn-npu0",
      "backend": "rknn",
      "availability": "available",
      "source": "deviceTreeAndDriver",
      "verifiedAt": 1790812800000,
      "stale": false,
      "affinityCapability": "verified",
      "reason": null,
      "cores": [
        { "coreIndex": 0, "assignedGroupCount": 1 },
        { "coreIndex": 1, "assignedGroupCount": 1 },
        { "coreIndex": 2, "assignedGroupCount": 1 }
      ],
      "usage": {
        "confirmedExecutionGroups": 3,
        "weightOwners": 2,
        "privateSessions": 6,
        "unknownAccountingUnits": 0,
        "quarantinedUnits": 0
      },
      "telemetry": {
        "scope": "perCore",
        "sampledAt": 1790812800000,
        "stale": false,
        "reason": null,
        "utilizationPercent": null,
        "cores": [
          { "coreIndex": 0, "utilizationPercent": 0 },
          { "coreIndex": 1, "utilizationPercent": 35 },
          { "coreIndex": 2, "utilizationPercent": null }
        ]
      }
    }
  ]
}
```

| 字段                        | 约束                                                                                                                                                                                           |
| --------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| status                      | available / unavailable / unknown / truncated；完整确认无设备为 available + devices=[]，不是探测错误吞空                                                                                       |
| observedAt / verifiedAt     | 快照观察时间/设备能力最近确认时间；未确认时 verifiedAt=null                                                                                                                                    |
| availability                | available / unavailable / unknown；权限不足、identityChanged 等见 reason                                                                                                                       |
| source                      | 有限来源枚举，阶段 A 冻结；不返回 sysfs 绝对路径                                                                                                                                               |
| affinityCapability          | verified / unsupported / unknown；设备级信息不是某个模型 manual 必然成功的保证                                                                                                                 |
| cores                       | 已知核心集合；未知为 null，不用 max(1) 造一核                                                                                                                                                  |
| usage                       | 已确认执行组与权重根、私有 session 与未知计费单位分列；根 context 不计入 privateSessions；quarantinedUnits 按执行/根 reservation 计数，而非受影响实例数；为宿主账本，不是全系统 session 利用率 |
| telemetry.scope             | perCore / device / unavailable；全局采样仅填 utilizationPercent，cores=[]，不复制到每核                                                                                                        |
| telemetry.sampledAt / stale | 最后成功采样时间及是否陈旧；从未成功则 null/true；刷新失败不得改写为新鲜零值                                                                                                                   |

每核 assignedGroupCount 仅计宿主明确分配到该核的未释放组，不是实测占用；它与 telemetry.utilizationPercent 完全独立。runtimeManaged/unknown 的保守竞争计费在设备级计数体现，不能伪装成精确单核绑定；assignedGroupCount=0 不代表该核实际空闲。设备/核心数组受 inventory 配置硬上限限制，超限 status=truncated 并停止依赖完整拓扑的绑定准入。

## Endpoints

| 方法/路径                                           | 变更 | 请求/响应                                                       |
| --------------------------------------------------- | ---- | --------------------------------------------------------------- |
| GET /api/v1/tasks/{cameraId}                        | 扩展 | algorithmInstances[].affinity 与 placement，其他字段保持        |
| PUT /api/v1/tasks/{cameraId}                        | 扩展 | affinity 三态、configRevision，保持保存响应语义                 |
| GET /api/v1/system/npu/topology                     | 新增 | data=TopologySnapshot                                           |
| GET /api/v1/system/npu/placements                   | 新增 | page/pageSize；可选 instanceId/deviceId/groupId；独立执行组分页 |
| GET /api/v1/system/npu/weights                      | 新增 | page/pageSize；可选 deviceId/weightId；权重根分页               |
| GET /api/v1/system/npu/weights/{weightId}/consumers | 新增 | page/pageSize；权重消费者分页                                   |

前稿 placements/{groupId}/members 端点撤销（尚未实现，不是删除既有生产 API）；由 weights 与消费者列表准确展示共享关系。没有新的换核/停组写端点。

分页 data 固定为 `{bootId,snapshotGeneration,page,pageSize,total,items}`。默认 page=1/pageSize=20，page>=1、pageSize=1..100；溢出/非法参数 400，末页以外空 items。ID 过滤参数 1..128 UTF-8 字节、拒绝控制字符/首尾空白。组按 groupId，根按 weightId，消费者按 groupId 稳定排序。固定上限快照，实时分页不保证跨请求一致性；snapshotGeneration 改变时可刷新。

组/根过滤无匹配为 200 空 items；消费者端点指定权重已释放/不存在为 404。无 reservation 的实例失败查任务 placement.lastAttempt，不以空组列表抹掉错误。GET 不触发加载、销毁、清理或硬件采样。

## Errors and Apply semantics

沿用既有五位业务错误码机制和 ApiError。阶段 A 在 error_code() 权威定义中分配未占用数字，并形成 HTTP/领域错误/回执 reason 对照 fixture；本文件不臆造数值。机器 reason 至少覆盖 invalidAffinity、identityMismatch、topologyUnavailable、unsupportedAffinity、weightSharingUnsupported、weightSharingUnverified、weightDerivationFailed、capacityExceeded、initializationTimeout、cleanupUnverified、protocolMismatch、legacyPlugin、sdkAffinityUnsupported、deviceUnavailable、recoveryBusy；有限集合在协议冻结时逐项确认，不允许自由文本取代枚举。

| 分类                                         | HTTP/处理                                                                        |
| -------------------------------------------- | -------------------------------------------------------------------------------- |
| JSON/字段/范围/保留元数据/实例身份非法       | 400，任何配置写入前拒绝                                                          |
| configRevision 冲突                          | 409 + 既有 40903，camera/task/instance 均不部分写入                              |
| 查询指定资源不存在                           | 404；过滤列表无匹配为 200 空列表                                                 |
| manual 保存前设备/核心明确不存在或明确不支持 | 400；解释合法范围/能力，不换核                                                   |
| 保存所需拓扑不可用/过期无法验证              | 503；不写配置                                                                    |
| 新架构所需共享能力明确不支持                 | 保存前可验证时 400；初始化阶段按 lastAttempt/Apply 失败，不自动 legacy           |
| 缺共享验证 profile / 容量或恢复 busy         | 同步准入 503；已提交配置按运行时失败反馈                                         |
| PUT 已提交后硬件不可满足                     | 保留保存响应语义；applyState/lastAttempt 报告 pending/failed，不假报整个 DB 回滚 |
| 未认证/失效凭据/未初始化                     | 沿用 protected router 既有行为，不新增鉴权旁路                                   |

保存前校验必须覆盖同次 streamMode 等所有配置副作用；有效保存之后仍由运行时复核。新配置失败保留旧健康 runtime，targetRevision 绑定真实尝试；旧成功/失败不得改写新 revision。重启失败只影响对应实例；任务聚合 Error 不意味着健康实例停机。停用/未运行的历史 appliedRevision 不是 SDK 确认，严禁据此生成 acknowledged。

普通业务参数热更新不改 placement。换核只重建目标实例子 session 并复用原兼容权重；失败不停止兄弟实例。auto 亲和降级不能把 weightSharing 从必需变为可选，不提供隐式复制权重、共享串行或 CPU 回退。

## Compatibility / Changelog

- 2026-10-01 draft：实例 affinity、严格 manual、诊断查询；替代原 pinned_core/pinned_device 与 spread/shared 混合语义。
- 本轮修订：分离实例配置/运行槽/尝试与组资源；补完整拓扑空值语义、真实 revision 与原子拒绝、旧无 ID 客户端和既有单算法桥接、实际 JWT 鉴权边界。
- 旧客户端省略字段不会清空新配置；Web 回归覆盖 taskDraft、LiveRulesStudio 重建对象、null reset 和旧服务端缺失新字段。未知运行状态不得被前端归一化成 acknowledged。
- 本次 D1 修订：执行组按实例独立；删除尚未实施的共享组成员端点，新增权重根/消费者分页；实例分核与物理共享证据分开。
- 用户已确认 D1、严格 manual / auto 受控亲和降级；完整 API 仍待确认。新增可视化编辑器不在本期范围。
