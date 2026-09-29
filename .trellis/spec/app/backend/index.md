# app — 规范入口

适用于 `crates/app`：配置加载、依赖装配、启动与退出生命周期（单二进制 `heimdall`）。仓库级契约见 [AGENTS.md](../../../../AGENTS.md)。

## Pre-Development Checklist

1. [架构概览](../../guides/architecture-overview.md)：装配职责与单向依赖；业务逻辑不得落在本层。
2. [目录与配置](../../guides/directory-structure.md)：配置分层与文件归属。
3. [并发模型](../../guides/concurrency-guidelines.md)：停机协调与专用线程生命周期。
4. [错误处理](../../guides/error-handling.md)、[日志](../../guides/logging-guidelines.md)：启动失败可诊断、退出路径资源释放。

## Quality Check

- 执行 [AGENTS.md](../../../../AGENTS.md) 的 Rust/Native 门禁。
- 启动/退出路径必须验证：fd、线程与硬件上下文释放，失败原因可诊断。
