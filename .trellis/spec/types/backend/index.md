# types — 规范入口

适用于 `crates/types`：共享领域类型与 `FrameRef` 等跨 crate 契约；不承载 IO、数据库或平台 SDK。仓库级契约见 [AGENTS.md](../../../../AGENTS.md)。

## Pre-Development Checklist

1. [全局约定](../../guides/conventions.md)：13 位毫秒时间、`[0,1]` 归一化坐标、DTO 契约与队列硬约束。
2. [架构概览](../../guides/architecture-overview.md)：跨 crate 共享契约必须下沉到本层，依赖保持单向。
3. 类型变更同时核对所有消费方：各 Rust crate 与前端 `web/src/types/` 的守卫和归一化逻辑。

## Quality Check

- 执行 [AGENTS.md](../../../../AGENTS.md) 的 Rust/Native 门禁。
- 契约字段变更属于跨层变更：同步消费方与 [类型与时间规范](../../web/frontend/type-safety.md)，未覆盖项在交付说明列出。
