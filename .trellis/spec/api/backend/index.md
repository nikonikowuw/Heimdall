# api — 规范入口

适用于 `crates/api`：Axum HTTP/WS 适配、DTO 映射、静态资源内嵌与控制句柄；handler 只提取参数、调用领域服务并映射 DTO，不写业务判定、不碰 SQL DSL 与硬件调用。仓库级契约见 [AGENTS.md](../../../../AGENTS.md)。

## Pre-Development Checklist

1. 阅读 [API 规范](./api-guidelines.md)：分层、路由、JSON/时间/错误、人员底库与批量导入契约。
2. 信封、时间与字段命名：[全局约定](../../guides/conventions.md)（`/api/v1`、camelCase、13 位毫秒、`T | null`）。
3. 错误映射与日志：[错误处理](../../guides/error-handling.md)、[日志](../../guides/logging-guidelines.md)。
4. 前端消费方对齐：[类型与时间](../../web/frontend/type-safety.md)。

## Quality Check

- 执行 [AGENTS.md](../../../../AGENTS.md) 的 Rust/Native 门禁；契约测试覆盖信封、错误与边界校验。
- 接口变更时同步核对 web 消费方，并按 [质量检查](../../guides/quality-guidelines.md) 的审查重点自查。
