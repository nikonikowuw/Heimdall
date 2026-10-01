# NPU 放置 API 契约

> draft，2026-10-01；待用户确认。以下为新增/变更提案，不是现有接口。

## Scope

- Backend：types、db、infer、pipeline、api、app；职责见 design.md。
- Frontend：web 共享任务类型、归一化及保存回归；不新增 affinity 编辑器或调度页面。
- Shared：本文件由主会话维护，前后端实现者不得自行改契约。

## Conventions / Auth

沿用 /api/v1、camelCase、现有鉴权/权限及 Accept-Language。成功和失败均使用既有 ApiResponse；失败 data=null。时间使用 UTC Unix 毫秒 number/i64。超时为 timeoutMs。拓扑无权限或未知指标不得伪装成零。

请求鉴权沿用任务配置写权限；诊断接口至少使用现有系统管理读取权限。禁止公开内部路径、fd、指针。实例/组 ID 是不透明字符串。

## Shared models

### AffinityIntent

```json
{"mode":"auto","policy":"spread"}
```

```json
{"mode":"manual","deviceId":"rknn-npu0","coreIndex":1}
```

- auto：policy 可省略，默认 spread；不允许携带 deviceId/coreIndex。
- manual：deviceId 非空且长度有界，coreIndex 为非负整数且在当前可调度集合内；不接受 policy、裸 mask 或 pinned_core 等旧草案字段。
- manual 不表示独占，不支持取模降级。
- public deviceId 来自 inventory，不能假定字符串等于 SDK 数字编号。
- 同组 auto 继承现有放置；同组不相容 manual 明确冲突。

### 任务实例字段

现有 TaskAlgorithmInstanceDto 新增 affinity；响应总是归一化后的完整对象。写请求需要三态：缺失=保留旧值，null=重置 auto，对象=替换。新实例缺失=auto。必须复用保留实例的 instanceId，不能按数组位置保留。

algorithmInstances 仍为集合替换；省略集合保留全部。不得为新字段改变既有配置语义。

### PlacementSnapshot

```json
{"instanceId":"instance-1","groupId":"group-7","reservationId":"boot-7:42","requested":{"mode":"manual","deviceId":"rknn-npu0","coreIndex":1},"assigned":{"deviceId":"rknn-npu0","coreIndex":1},"applicationStatus":"acknowledged","lifecycle":"ready","sharingScope":"packageShared","memberCount":3,"reason":null,"desiredRevision":4,"appliedRevision":4,"sampledAt":1790812800000}
```

- assigned 可空；runtimeManaged 不伪造 coreIndex，使用 null。
- applicationStatus：pending / acknowledged / runtimeManaged / degraded / unverified / unsupported / failed。
- lifecycle：reserved / initializing / ready / cooling / draining / quarantined；released 记录不无限保存。
- sharingScope：instance / packageShared / unknown。旧插件为 unknown + unverified。
- reason 为稳定机器枚举或 null；文案通过既有国际化映射，不让客户端匹配底层日志。
- requested 来自持久化期望；assigned 是当前运行资源，不由请求体回显生成。
- acknowledged 只代表插件确认 SDK 接受，不等于硬件利用率证据。

## Endpoints

| 方法/路径 | 变更 | 行为 |
| --- | --- | --- |
| GET /api/v1/tasks/{cameraId} | 扩展已有响应 | algorithmInstances[].affinity |
| PUT /api/v1/tasks/{cameraId} | 扩展已有请求 | 实例 affinity 三态更新；保留 configRevision 乐观锁 |
| GET /api/v1/system/npu/topology | 新增 | 有界设备/核心快照、能力、topologyGeneration、sampledAt/stale |
| GET /api/v1/system/npu/placements | 新增 | 分页执行组摘要；可按 instanceId/deviceId 过滤 |

placements 使用 page/pageSize（默认 1/20，pageSize 1..100）及 total/items；按稳定 groupId 排序。每组只返回 memberCount，不嵌入无界成员数组；instanceId 过滤返回该成员关联信息。分页是实时快照而非历史一致性事务，响应带 snapshotGeneration，变化时客户端可刷新。

topology 的设备/核心数组受已配置 inventory 上限限制；超限明确状态，不静默截断为完整拓扑。每核区分 assignedGroupCount 与 utilizationPercent，后者可 null，并有采样时间。API 只读内存快照，不在 handler 调硬件。

## Errors and Apply semantics

沿用现有五位业务错误码机制和 ApiError 映射；实施阶段在 error_code() 权威定义中分配未占用的码，禁止文档臆造与现有码冲突的数字。

| 分类 | HTTP/处理 |
| --- | --- |
| JSON/字段/范围/保留元数据非法 | 400，持久化前拒绝 |
| configRevision 冲突 | 409 + 既有 40903，不改行为 |
| 查询目标不存在 | 404 |
| 同步操作的共享组冲突/容量准入失败 | 409/503，复用领域分类映射 |
| PUT 已提交后硬件不可满足 | 保留既有任务保存响应语义，运行态 pending/failed；不得伪造整次 DB 回滚 |
| 未认证/无权限 | 既有 401/403 |

结构正确但设备离线/共享组变化的请求不能靠预检查保证生效。持久化 desiredRevision 后，由运行时复核；失败保留旧 appliedRevision 和可诊断原因。不得宣称任务 PUT 成功意味着掩码已成功设置。

## Compatibility / Changelog

- 2026-10-01 draft：实例 affinity、严格 manual、诊断查询；替代原 pinned_core/pinned_device 与 spread/shared 混合语义。
- 旧客户端省略字段不会清空新配置；Web 完整往返测试必须覆盖 taskDraft 与 LiveRulesStudio 的重建对象路径。
- API 契约需确认；新增可视化编辑器不在本期范围。
