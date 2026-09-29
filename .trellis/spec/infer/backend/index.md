# infer — 规范入口

适用于 `crates/infer`：推理引擎抽象、平台后端（RKNN / Core ML / CANN）与算法包加载。仓库级契约见 [AGENTS.md](../../../../AGENTS.md)。

## Pre-Development Checklist

1. 阅读 [推理后端](./inference-backends.md)：实现入口、后端选择与模型、输出与后处理、算力租约与生命周期。
2. 阅读 [推理运行时健康](./inference-runtime-health.md)：活性判定边界、指标口径、Worker 代际栅栏、在途心跳。
3. 上下文与线程模型：[并发模型](../../guides/concurrency-guidelines.md)；内存预算与 CMA 复用：[全局约定](../../guides/conventions.md)。
4. 算法包边界与 C ABI：[算法 SDK 与 C ABI](../../algo-sdk/backend/algo-sdk-guidelines.md)；FFI 安全：[FFI 边界](../../guides/ffi-guidelines.md)。

## Quality Check

- 执行 [AGENTS.md](../../../../AGENTS.md) 的 Rust/Native 门禁；硬件依赖测试标记 `#[ignore]`。
- 活性/指标口径变更必须与 [推理运行时健康](./inference-runtime-health.md) 的验证节对齐。
