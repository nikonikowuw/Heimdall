# RGA 输入对齐诊断与连续失败追踪机制实现总结

## 实现概述

本实现为 Heimdall 项目的 RGA 预处理层添加了诊断日志与连续失败追踪机制，解决以下问题：

1. **诊断可观测性不足**：当 RGA 验证失败时，难以快速定位 stride 对齐、格式兼容等底层原因
2. **运维状态缺失**：连续失败时缺乏结构化的计数与阈值机制，无法接入监控体系

## 设计决策

RGA 连续失败是**硬件管线内部状态**，属于运维可观测性范畴，不应暴露给终端用户。因此：

- **不通过 `ResultEmitter` 推送前端**——`AV_RESULT_SYSTEM` 未定义，`notifier.rs` 未引入
- **通过 `tracing::error!` 结构化日志**——运维从服务日志 / ELK / Loki 获取
- **通过 `FailureTracker` 暴露状态**——供健康检查接口或 Prometheus metrics 读取

## 实现内容

### 1. 诊断日志增强

**位置**：`crates/algo-sdk/src/cv/platforms/rockchip/engine.rs`

- 在 `source_layout()` 各错误分支添加 `tracing::error!` 结构化日志，记录失败原因与上下文（frame_id、stride、format）
- 不在入口处添加 `debug!`——`source_layout()` 是逐帧热路径，正常帧的日志会产生不必要的 IO 和格式化开销

**日志示例**：
```log
ERROR RGA NV12 stride 不一致 frame_id=12346 stride_y=1921 stride_uv=1921
```

### 2. 连续失败追踪器

**位置**：`crates/algo-sdk/src/cv/platforms/rockchip/diagnostic.rs`

- `FailureTracker`：基于 `AtomicU64` 的线程安全纯计数器
- 连续失败计数：每帧失败递增，成功时 `record_success()` 重置并打恢复日志，flush 时 `reset()` 静默清零
- 总失败计数：累计失败次数
- 阈值判定：`is_threshold_exceeded()` 供调用方按需读取
- 不主动推送告警——错误日志由 `engine.rs` 的 `tracing::error!` 负责

```rust
pub struct DiagnosticConfig {
    pub failure_threshold: u64, // 默认 30 帧（约 1 秒 @30fps）
}
```

### 3. 算法插件集成

**位置**：`algo-packages/rknn/rk3568/safetyhelmet_detection/src/plugin.rs`

- `SafetyHelmetDetector` 直接持有 `FailureTracker`（无需 `Arc`，单实例独占）
- `process()` 失败时调用 `record_failure()`，成功时调用 `record_success()`
- `flush()` 时调用 `reset()` 静默清零连续计数（非成功语义，不打恢复日志）
- 告警由 `engine.rs` 的 `tracing::error!` 记录，不通过 emitter 推送

## 性能影响

| 组件 | 正常路径开销 | 异常路径开销 |
|------|-------------|-------------|
| 诊断日志 (error 级别) | 无（仅在错误分支触发） | 微小（格式化日志） |
| 失败追踪器 | 无（仅在成功时重置） | 极小（原子操作） |

**结论**：正常路径零开销，异常路径开销可忽略。

## 测试覆盖

- 单元测试：`FailureTracker` 基本功能、成功重置、冷却时间、状态查询
- `cargo check` 通过
- 算法包测试通过

## 文件变更清单

| 文件 | 变更类型 | 说明 |
|------|----------|------|
| `crates/algo-sdk/src/cv/platforms/rockchip/engine.rs` | 修改 | 添加诊断日志 |
| `crates/algo-sdk/src/cv/platforms/rockchip/diagnostic.rs` | 新增 | 失败追踪器模块 |
| `crates/algo-sdk/src/cv/platforms/rockchip/mod.rs` | 修改 | 导出 diagnostic 模块 |
| `crates/algo-sdk/src/cv/platforms/mod.rs` | 修改 | 导出追踪器类型 |
| `crates/algo-sdk/src/cv/mod.rs` | 修改 | 导出追踪器类型 |
| `crates/algo-sdk/src/lib.rs` | 修改 | 导出追踪器类型 |
| `crates/algo-sdk/Cargo.toml` | 修改 | 添加 parking_lot 依赖 |
| `Cargo.toml` | 修改 | 添加 workspace 依赖 |
| `algo-packages/rknn/rk3568/safetyhelmet_detection/src/plugin.rs` | 修改 | 集成失败追踪 |
| `docs/nuwa/backend/database-guidelines.md` | 修改 | 补充 N+1 查询模式 |

## 运维接入建议

`FailureTracker` 可在健康检查接口中暴露：

```rust
// 示例：健康检查端点中读取追踪器状态
let tracker = &task.failure_tracker;
if tracker.is_threshold_exceeded() {
    // 触发运维告警（钉钉/飞书 webhook、Grafana Alert 等）
}
let status = tracker.status();
// status.consecutive_failures, status.total_failures 可序列化为 JSON
```
