# media — 规范入口

适用于 `crates/media`：RTSP 接入、硬件解码、帧内存、StreamHub 与流分发。仓库级契约见 [AGENTS.md](../../../../AGENTS.md)。

## Pre-Development Checklist

1. 阅读 [媒体管线](./media-pipeline.md)：三大路径、帧与所有权、接入与分发、门控、快照硬件编码；录像切片章节为规划设计，尚未实现。
2. 帧内存与对齐：[全局约定](../../guides/conventions.md)（DMA-BUF 堆选择优先级、CMA 预算、热路径零分配）。
3. 执行归属与停机：[并发模型](../../guides/concurrency-guidelines.md)（DMA-BUF/RGA 操作仅在专用 OS Worker）。
4. DMA-BUF 导入与平台绑定：[FFI 边界](../../guides/ffi-guidelines.md)。
5. 错误与降级：[错误处理](../../guides/error-handling.md)；帧路径采样降噪：[日志](../../guides/logging-guidelines.md)。

## Quality Check

- 执行 [AGENTS.md](../../../../AGENTS.md) 的 Rust/Native 门禁；硬件依赖测试标记 `#[ignore]`。
- 按 [质量检查](../../guides/quality-guidelines.md) 覆盖：投递/丢弃策略、重连（GOP 恢复）、资源释放与停机路径。
