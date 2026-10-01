# 源码证据、差异与硬件验证边界

> 2026-10-01；源码静态评审，不代表板端结果。行号可能随实现变化，路径/符号为主要定位依据。

## 已确认源码事实

| 入口 | 事实 | 对设计的约束 |
| --- | --- | --- |
| crates/infer/src/package.rs / LeaseState, AlgoLease, acquire_lease, release_lease | algorithm_id 聚合保活、共享 Warmup、60s 冷却 | 不把 AlgoLease 直接当独立核心 reservation |
| crates/infer/src/worker.rs / stop, quarantine_worker, WORKER_STARTUP_TIMEOUT | 启动超时、退出隔离、底层线程可继续存活 | 超时不是硬件资源释放；额度必须保留 |
| algo-packages/rknn/rk3588/face_recognition/src/lib.rs:38-104 | 按 canonical package_root 的 Weak 共享 registry；初始化在检查锁之外 | 粒度是共享执行组；需要单航班初始化，防临时双份模型 |
| 同包 worker.rs:189-220 | 内部硬件线程，Drop 中 join | 需核对内外层退出所有权，不能只回收宿主外壳 |
| 同包 plugin.rs:65、extract.rs:297 | 实时实例与离线提取均获取 shared_models | 必须一起纳入组管理；离线 ABI 不自带 config_json |
| crates/algo-sdk/src/macros.rs:462-580 | 整个 JSON 直接反序列化插件 Config | 保留元数据需在 SDK 宏边界剥离 |
| crates/algo-sdk/src/plugin.rs / InitContext | 当前没有 placement 字段 | Rust SDK 源码 API 需显式演进并覆盖本地 runner |
| crates/algo-sdk/src/c_abi.rs / AvAlgoAbi, AvAlgoLibraryInfo | 96 字节基础虚表；library_query 固定结构无通用扩展载荷 | 不能往旧结构塞字段，需加法式可选扩展 |
| crates/infer/src/c_abi/loader.rs | 已按可选符号加载 face/gallery 能力 | 可复用此模式，但 placement 扩展是新增设计 |
| crates/infer/src/npu/{mod,rknn,ascend,monitor}.rs | 已有设备抽象、SoC 发现、指标 | 复用 inventory，不另造探测分支 |
| crates/pipeline/src/coordinator.rs:1073,1437；manager.rs:1727 | 多个 Worker 创建路径 | 统一所有权入口，不只改 Warmup |
| crates/db/src/entity/algorithm_instance.rs；repository/task.rs | 实例持久化和 desired/applied revision | 放置期望沿现有事务及 Apply 流程 |
| crates/api/src/routes/task.rs；routes/system/overview.rs | 既有任务字段和系统概览 | 不破坏根信封；新增读取快照不让 handler 接触 SDK |
| web/src/features/tasks/taskDraft.ts、components/LiveRulesStudio.tsx | 保存会重新构建实例对象 | 必须验证省略字段保留，避免手动配置被旧编辑流程抹掉 |

## 原方案应替换的结论

1. 每管线分核 -> 按真实执行组分配，管线作为成员。
2. algorithm_id 释放 -> 唯一 reservation + generation 幂等释放。
3. 越界取模 -> 推荐严格 manual 明确错误。
4. 任意非零改 warning -> 按目标 Runtime 错误分类，只有已验证的可恢复错误降级。
5. 空核心 max(1) -> 区分无设备、未知拓扑与已知单核。
6. spread/shared 两种并列 policy -> spread 是放置偏好，共享由有限准入约束。
7. 返回 mask 就是实际生效 -> requested/assigned/acknowledged/telemetry 分离。
8. 只加 JSON helper -> SDK 宏解析、InitContext、共享 Worker、离线与回执都需接线。
9. 只改 SDK 环境变量 -> RK3588 人脸独立 rknn.rs 也读取 RKNN_CORE_MASK。
10. u8 device_id 等于物理卡号 -> inventory 稳定 ID 与 runtime index 分离。

## 资料入口与尚未验证的事实

本轮已读取本机 rknn-pro 的 multi-model-scheduling 参考及 ascend-pro API 路由指导。它们是研究导航，不替代目标设备证据。

待核实官方入口：
- https://github.com/airockchip/rknn-toolkit2/blob/master/rknpu2/runtime/Linux/librknn_api/include/rknn_api.h
- https://github.com/airockchip/rknn-toolkit2
- 华为目标设备与 CANN 版本对应的 AscendCL context/device 文档。

本轮未获取目标板 Runtime/header/driver，也未完成上述线上官方文档的独立核验。因此不把具体错误码、支持 mask、API 失败后 context 状态或性能收益写成已验证结论。前序交流中对 -13 的确定性说明没有在本次工具记录中完成来源核验；实施必须以选定 BSP 头文件与加载库证据纠正该口径。

当前 SDK 对 -13 的注释与容忍分支，以及 RK3568 人脸的特殊处理，是需要核对的候选问题，不是跨所有 Runtime 已确认的错误码事实。

模型转换证据门禁：not-applicable（本任务不改变模型输入/量化/预处理）。但板端性能测试仍须记录模型 SHA-256、转换目标、Runtime/driver、热状态与输入负载；不得推断图精度或权重复制成本。

## 板端 evidence packet

每个发布 profile 单独保存：设备型号、系统/BSP、实际加载 librknnrt 路径及版本、驱动版本、匹配 rknn_api.h、模型标识与哈希、CMA/内存预算、合法 mask 的真实返回码、Runtime 默认/单核比较、初始化/退出/错误注入记录。

禁止把 RK3568 某板 16MB CMA 当作所有 RK3568 的硬件常量。没有 profile 的平台不能进入已验证 spread/manual 白名单。

## Spec 差异处理

现行 algo-sdk spec 仍包含 RK3576/RK3588 默认组合 mask 约定；本任务拟改为宿主协商。只有实现、兼容回归及板端证据完成后，才更新对应 spec；本轮不把未实现设计推广为仓库既有能力。
