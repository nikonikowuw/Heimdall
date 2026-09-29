# pipeline — 规范入口

适用于 `crates/pipeline`：管线编排、门控、后处理、跟踪、规则判定与证据生成。仓库级契约见 [AGENTS.md](../../../../AGENTS.md)。

## Pre-Development Checklist

1. 阅读 [检测、告警与证据契约](./detection-alarm-contract.md)：检测载荷、ByteTrack、几何判定、证据三支柱与状态流转。
2. 帧路径与证据环：[媒体管线](../../media/backend/media-pipeline.md)；坐标与队列上限：[全局约定](../../guides/conventions.md)。
3. 证据落库与淘汰：[数据库规范](../../db/backend/database-guidelines.md)；错误与降级：[错误处理](../../guides/error-handling.md)。
4. 线程/通道与停机协调：[并发模型](../../guides/concurrency-guidelines.md)。

## Quality Check

- 执行 [AGENTS.md](../../../../AGENTS.md) 的 Rust/Native 门禁。
- 按 [检测、告警与证据契约](./detection-alarm-contract.md) 的测试节与 [质量检查](../../guides/quality-guidelines.md) 选择测试：跟踪连续性、冷却防抖、状态流转。
