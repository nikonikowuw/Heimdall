# 源码证据、差异与硬件验证边界

> 2026-10-01；源码静态评审，不代表板端结果。行号可能随实现变化，路径/符号为主要定位依据。

## 已确认源码事实

| 入口                                                                              | 事实                                                                | 对设计的约束                                                |
| --------------------------------------------------------------------------------- | ------------------------------------------------------------------- | ----------------------------------------------------------- |
| crates/infer/src/package.rs / LeaseState, AlgoLease, acquire_lease, release_lease | algorithm_id 聚合保活、共享 Warmup、60s 冷却                        | 不把 AlgoLease 直接当独立核心 reservation                   |
| crates/infer/src/worker.rs / stop, quarantine_worker, WORKER_STARTUP_TIMEOUT      | 启动超时、退出隔离、底层线程可继续存活                              | 超时不是硬件资源释放；额度必须保留                          |
| algo-packages/rknn/rk3588/face_recognition/src/lib.rs:38-104                      | 按 canonical package_root 的 Weak 共享 registry；初始化在检查锁之外 | 当前是共享串行组，D1 要改为单航班共享权重根+实例独立 child  |
| 同包 worker.rs:189-220                                                            | 内部硬件线程，Drop 中 join                                          | 需核对内外层退出所有权，不能只回收宿主外壳                  |
| 同包 plugin.rs:65、extract.rs:297                                                 | 实时实例与离线提取均获取 shared_models                              | 需改为独立实时/离线执行资源，共享权重；离线 ABI 不带 config |
| crates/algo-sdk/src/macros.rs:462-580                                             | 整个 JSON 直接反序列化插件 Config                                   | 保留元数据需在 SDK 宏边界剥离                               |
| crates/algo-sdk/src/plugin.rs / InitContext                                       | 当前没有 placement 字段                                             | Rust SDK 源码 API 需显式演进并覆盖本地 runner               |
| crates/algo-sdk/src/c_abi.rs / AvAlgoAbi, AvAlgoLibraryInfo                       | 96 字节基础虚表；library_query 固定结构无通用扩展载荷               | 不能往旧结构塞字段，需加法式可选扩展                        |
| crates/infer/src/c_abi/loader.rs                                                  | 已按可选符号加载 face/gallery 能力                                  | 可复用此模式，但 placement 扩展是新增设计                   |
| crates/infer/src/npu/{mod,rknn,ascend,monitor}.rs                                 | 已有设备抽象、SoC 发现、指标                                        | 复用 inventory，不另造探测分支                              |
| crates/pipeline/src/coordinator.rs:1073,1437；manager.rs:1727                     | 多个 Worker 创建路径                                                | 统一所有权入口，不只改 Warmup                               |
| crates/db/src/entity/algorithm_instance.rs；repository/task.rs                    | 实例持久化和 desired/applied revision                               | 放置期望沿现有事务及 Apply 流程                             |
| crates/api/src/routes/task.rs；routes/system/overview.rs                          | 既有任务字段和系统概览                                              | 不破坏根信封；新增读取快照不让 handler 接触 SDK             |
| web/src/features/tasks/taskDraft.ts、components/LiveRulesStudio.tsx               | 保存会重新构建实例对象                                              | 必须验证省略字段保留，避免手动配置被旧编辑流程抹掉          |

## 原方案应替换的结论

1. 仅给外层线程分核 -> D1 要求实例专用 Worker/独立 context；共享权重单独管理，不能保留内部串行瓶颈。
2. algorithm_id 释放 -> 唯一 reservation + generation 幂等释放。
3. 越界取模 -> 已确认严格 manual 明确错误。
4. 任意非零改 warning -> 按目标 Runtime 错误分类，只有已验证的可恢复错误降级。
5. 空核心 max(1) -> 区分无设备、未知拓扑与已知单核。
6. spread/shared 两种并列 policy -> spread 是放置偏好，共享由有限准入约束。
7. 返回 mask 就是实际生效 -> requested/assigned/acknowledged/telemetry 分离。
8. 只加 JSON helper -> SDK 宏解析、InitContext、共享 Worker、离线与回执都需接线。
9. 只改 SDK 环境变量 -> RK3588 人脸独立 rknn.rs 也读取 RKNN_CORE_MASK。
10. u8 device_id 等于物理卡号 -> inventory 稳定 ID 与 runtime index 分离。

## 本轮新增源码证据与工业级缺口

以下 H 项为静态源码证据或由契约推导的设计缺口，未执行故障复现、未完成真机测试；测试合同见 implement.md 的 T 编号。P0 指本任务交付的阻断缺口，不表示已在生产复现。

| ID/级别              | 定位与已观察事实                                                                                                                                                                                | 必须补齐的合同 / 验证                                                   |
| -------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ----------------------------------------------------------------------- |
| H01 / P0             | `crates/infer/src/worker.rs:526-540,631-645`：backend 作用域析构前发送 exit，停止方收到后直接 join；`worker.rs:457-471,555-572` 错误/断开分支同样 join；人脸 `worker.rs:203-214` 内部 Drop join | 清理确认、线程完成和 deadline 分开；T01-T03                             |
| H02 / P0             | `crates/pipeline/src/coordinator.rs:1403-1454`：单实例失败 shutdown_workers；`crates/api/src/routes/task.rs:972-980` 统一错误；`crates/app/src/reconcile.rs:225-313` 恢复准备可整任务失败       | 冷启动每实例 outcome、成功成员继续服务；T05/T24                         |
| H03 / P0             | `coordinator.rs:440-449` launch 版本记 0；API `task.rs:695-705` 执行后读当前 revision；`crates/db/src/repository/algorithm_instance.rs:197-265` 先读后写，失败回写不带 revision                 | 不可变目标票据、条件 UPDATE 和同 revision attempt 栅栏；T04/T18/T22-T23 |
| H04 / 已撤销旧约束   | 旧设计让共享串行组的 auto 首次绑定限制后续 manual                                                                                                                                               | D1 下独立 child 可不同核；改测单航班权重与有界批次准入，T09             |
| H05 / P0（迁移缺口） | 人脸 lib.rs 按包共享内部推理 Worker，extract.rs 只带 package_root；当前不具备每实例共享权重派生/离线执行句柄                                                                                    | 改为权重根+独立实时/离线子 session；Q1 撤销，T28/T31-T38                |
| H06 / P1             | `crates/infer/src/npu/rknn.rs:185-205,349-367` 默认一核/补零/复制全局负载；`monitor.rs:68-72,86-106` 丢弃错误并在 Tokio 定时任务内同步采样                                                      | 源头三态 discovery/capability/telemetry，拓扑与采样代际分离；T25        |
| H07 / P1             | `crates/db/src/repository/task.rs:33-38,234-242,295-301` 无实例 ID 入参、按 algorithm_id 匹配；`crates/types/src/task.rs:358-369` 禁重复算法                                                    | 显式 ID 归属校验、旧无 ID 兼容、保留 UNIQUE 语义；T16-T17               |
| H08 / P1             | `crates/algo-sdk/src/models/yolo.rs:255-260` -> `runtime/mod.rs:98-106` 固定默认 options；`model.rs:69-78` 可自主轮转                                                                           | 不遗漏通用模板，不重复自主调度；T29；与活跃 SDK 生态任务协作            |
| H09 / P0             | `crates/infer/src/package.rs:1570-1597` lease 在 async future，执行 closure 仅持 package/bytes，且租约后重新查 package                                                                          | lease/package 必须同代际一起移入实际执行任务；T20                       |
| H10 / P0             | `crates/api/src/routes/task.rs:343-357` 实例校验前持久化 streamMode                                                                                                                             | 无效 affinity 和 revision 冲突不得留下 camera 半写入；T15               |
| H11 / P1（设计推导） | 原 API PlacementSnapshot 混合实例 revision 与组资源；准入前失败无 reservation                                                                                                                   | 实例当前槽/最后尝试与组快照拆开，执行/权重/消费者分页有界；T30          |
| H12 / P0             | `crates/infer/src/worker.rs:70-80` 隔离 Vec 当前直接追加，无准入硬上界；共享真实粒度与未知旧插件不是同一事实                                                                                    | 熔断阈值不能作为丢 owner 上限；旧插件每潜在入口计费；T26-T27            |

### 已确认与尚待决策

- 用户已确认 auto 默认、manual 严格失败、不取模、不静默换核；本轮进一步授权优化任务文档，不是产品实现批准。
- D1 已确认：一实例一专用 Worker、独立检测/特征 session、宿主分核且共享模型权重。原 Q1 前提被替代、已撤销，不是用户批准整组停机。新 API/双层生命周期与发布计划仍待最终评审。
- 当前鉴权由 `crates/api/src/routes/mod.rs:23-41` protected router 和 `middleware/auth.rs` 的管理员 JWT 提供；不声称已有细粒度 NPU RBAC 权限。
- `crates/algo-sdk/src/macros.rs` 的 instance_destroy 捕获 panic 且基础 ABI 没有销毁结果：仅宿主线程结束无法证明插件所有资源释放，因此可选扩展需明确清理回执和查询上下文生命周期。

## 官方依据与证据等级

### S01 — RKNNRT V2.3.2 完整手册（已在线核验）

- 来源：[Rockchip RKNNRT API Reference V2.3.2 EN](https://github.com/airockchip/rknn-toolkit2/blob/59a913d172e7f5ff03c9076e2ec7b1b1288ffd08/doc/04_Rockchip_RKNPU_API_Reference_RKNNRT_V2.3.2_EN.pdf)。固定仓库 tree commit：`59a913d172e7f5ff03c9076e2ec7b1b1288ffd08`。
- Git blob：`c94f5e51a552f063e387ab7afc14b07e9d9cca1a`；PDF SHA-256：`f222dddd811e93ba8bb2aa01788b0a20fe221a4e42bbbd4e1125da02402c5074`。通过 GitHub blob API 下载，PDFKit 提取逐页文本核验；未把临时下载作为仓库运行依赖。
- p21–22 的 rknn_dup_context 明确描述多线程同模型权重复用。原文短摘：**“Creates a new context for the same model, to reuse the weight of the model.”** 同节列出不适用 RV1106/RV1103/RV1106B/RV1103B/RK2118。
- 结论：rknn_dup_context 是有官方共享权重用途依据的首选机制，不能再只凭简短头文件说“官方未声明权重共享”。但目标 BSP 支持、具体驻留量、可写资源隔离、跨核与析构行为仍必须验证。
- p20–21：set_core_mask 面向 RK3588/RK3576，单核架构调用会报错。核心绑定能力与权重派生能力不能混为一谈；不能把 RK3568 的绑定 API 错误当 NPU 不存在。
- RKNN_FLAG_SHARE_WEIGHT_MEM 段描述旧式不同分辨率/去权重模型共享（remove_weight），不是本任务同模型独立实例的首选路线；不因一个 flag 名称就引入转换流程变化。

### S02 — 匹配官方头文件（已在线核验）

- [rknn_api.h 固定版本](https://github.com/airockchip/rknn-toolkit2/blob/59a913d172e7f5ff03c9076e2ec7b1b1288ffd08/rknpu2/runtime/Linux/librknn_api/include/rknn_api.h)。SHA-256：`c48e11a6f41b451a5fd1e4ad774ea60252d3d94f78bee9b21ea3d21b21deba9a`；该文件最近一次提交 `42aa1d426c0a9e0869b6374edba009f7208a1926`（SDK V2.3.2）。
- 声明 `int rknn_dup_context(rknn_context *context_in, rknn_context *context_out)`；两个参数均为指针，检查返回值后才能消费 child。
- rknn_context 在 `__arm__` 下是 uint32_t，其他分支 uint64_t；现有 Rust u64 绑定不能不经 ABI 检查推广到 armhf。目标位宽及 size/alignment/offset 必须双侧断言。
- RKNN_QUERY_MEM_SIZE 的 total_weight_size、total_internal_size、total_dma_allocated_size 等是 Runtime 定义口径；不能对每个 child 的逻辑 weight_size 简单求和证明物理复制，也不能仅看 RSS 证明设备内存共享。

### S03 — 当前生产接线（源码已核验）

`crates/algo-sdk/src/model.rs` 的 ModelWeights/SharedWeights 只有 Arc、轮转和 mock 实现；检索生产 crates/algo-packages 未发现 rknn_dup_context 接线。`runtime/mod.rs::RuntimeSession::open` 直接走 RknnSession::open_or_fallback；现有人脸内部 Worker 独占 detector/embedder（及可选模型），多摄像头共享队列。因此 D1 是真实执行模型改造，不是给现成物理共享开关加参数。

### 尚未验证

- 无选定目标板的 Runtime/header/driver/model profile；没有真机验证物理共享、跨线程移交、根/子销毁和跨核并发。手册能力不是板端结果。
- 手册未在上述段落充分定义源 context 可提前销毁的全部条件；采用根活到最后 child 清理确认的保守所有权，不试图提前释放。
- 未证明派生/销毁与兄弟 session 推理并发时的耗时、锁粒度；设计不能通过给所有推理加全局 mutex 来掩盖这一门禁。
- 旧 SDK 对裸 -13 的注释/忽略分支仍需目标错误表核对，不把所有非零归为“亲和不支持”。
- 本机 rknn-pro 参考基于头文件谨慎提示无共享保证；S01 补足完整手册证据，不改变“按部署版本测内存/生命周期”的要求。

模型转换证据门禁：not-applicable（本轮仅规划执行/权重所有权，不改变输入/量化/预处理）。板端测试仍记录制品 SHA-256、转换目标、模型签名、Runtime/driver 与负载；若实现触及 IO/preprocessing 契约须重新进入转换证据门禁，不能借本轮结论跳过。

## 板端 evidence packet

每个发布 profile 单独保存：设备型号、系统/BSP、实际加载 librknnrt 路径及版本/指纹、驱动版本、匹配 rknn_api.h、模型标识与哈希、CMA/内存预算、合法 mask 的真实返回码、Runtime 默认/单核比较、初始化/退出/错误注入记录。包含必需/可选模型和 root/child 集合、共享权重/根私有/workspace/IO/预处理池预算、外层线程和离线请求上限、全部 deadline 与资源计数；使用环境专属标识，不记录原始 machine-id。

实验前由维护者确认 profile 的负载、预热/测量窗口、吞吐和端到端 P95 下限/上限、允许回归范围、丢帧和控制面时限。每组至少 3 轮对照，启停至少 100 轮，最小长稳 8h；记录实际命令、原始测量、缺项和签字结论，不将任意草案数值当做平台性能。能力验证与性能实验分开：mask 被接受不意味着有收益，预算受控也不意味着精确掌握 allocator 状态。

禁止把 RK3568 某板 16MB CMA 当作所有 RK3568 的硬件常量。没有共享/隔离/预算 profile 的平台不能进入 D1 新架构；仅亲和能力不足可按已确认规则默认调度，不可解除共享要求。

### 物理共享与执行独立专项实验

1. 固定同一模型/Runtime/设备，分别运行共享方案 root+1/2/3/N child 与独立 rknn_init 对照；后者仅测试，不作为生产回退。
2. 记录实际 root/child 初始化调用与依赖、query 原始值、RSS/PSS、DMA-heap/设备分配、CMA 可用信息、fd 与预热后稳态/峰值。根 internal、临时模型字节缓存与媒体池须分项解释；不能把不同比例省内存一律叫权重只驻留一份。
3. 共享机制/可信生命周期与多源增量共同判定；传感器不可用应列明，证据不足不签发 verified。预先固定允许的 allocator 波动与私有增量上限，禁止测后移动门槛。
4. 相异输入并发、延迟取输出、独立停止/换核、派生失败、首实例退出与最后 child 隔离；确认输出不串帧、不覆盖，root 保留，兄弟实例不被控制面停掉。三核设备需证明不再只有一个插件推理队列；线程多/分核日志不等于并行吞吐已验收。
5. ABI/符号缺失、未知 profile、预算不足与 default-core 重试均验证 fail-closed；不得回退每实例全量权重或共享串行 Actor。

## Spec 差异处理

现行 algo-sdk/infer/conventions spec 仍包含默认组合 mask 和受限 CMA 人脸共享串行 Actor；本任务在用户 D1 确认后改为宿主协商、真实共享权重与实例独立 session。省内存目的保留，不再把串行 Actor 作为唯一实现。只有实现、兼容回归及板端证据完成后，才更新对应 spec；本轮不把未实现设计推广为仓库既有能力。

其他差异：现有部分 spec 对 CMA、退出保证或版本条件更新的概括不能代替源码证明；本任务按 H01/H03/H06/H10 补闭环，而非继续依赖注释。API spec 的“省略集合保留”须结合仍存在的旧单算法桥接分支验证；不在本任务顺手改变旧请求意图。按 design.md、implement.md 的明确边界实现后再同步现行规范。
