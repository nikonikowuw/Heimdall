# 后端开发规范

适用于 Rust workspace、算法插件及必要的 C/C++ 硬件垫片。仓库契约见 [AGENTS.md](../../../AGENTS.md)。

## Pre-Development Checklist

- 阅读 [架构概览](../guides/architecture-overview.md)，确认职责、依赖方向与修改文件。
- 按下表读取专题；涉及接口或帧边界时加读 [跨层检查](../guides/cross-layer-thinking-guide.md)。
- 涉及逐帧工作、队列或写盘时，完成 [资源预算](../guides/edge-constraints-guide.md)。
- 对照专题中的实现入口；未落地设计与真机待验证项不能当作现有 API。

| 专题                                            | 触发条件                                 |
| ----------------------------------------------- | ---------------------------------------- |
| [目录与配置](./directory-structure.md)          | 新建模块/crate、调整依赖或配置           |
| [错误处理](./error-handling.md)                 | 错误枚举、传播、降级或退出               |
| [日志](./logging-guidelines.md)                 | 日志字段、span、采样与落盘               |
| [数据库](./database-guidelines.md)              | 表、迁移、查询、批处理与证据清理         |
| [API](./api-guidelines.md)                      | HTTP、WS、DTO、鉴权与审计                |
| [并发](./concurrency-guidelines.md)             | async/线程、队列、共享状态、停机         |
| [推理后端](./inference-backends.md)             | 后端接入、模型加载与调度                 |
| [媒体管线](./media-pipeline.md)                 | RTSP、解码、帧内存、门控与流分发         |
| [FFI](./ffi-guidelines.md)                      | unsafe、ABI、句柄与构建绑定              |
| [算法 SDK](./algo-sdk-guidelines.md)            | 插件、C ABI、预处理、交付与沙箱          |
| [检测与告警契约](./detection-alarm-contract.md) | 检测载荷、坐标、规则告警、证据状态与迁移 |
| [质量检查](./quality-guidelines.md)             | 验证、测试与审查                         |

## Quality Check

- 按 [质量规范](./quality-guidelines.md) 选择测试，执行 `AGENTS.md` 的 Rust/Native 门禁，格式化先于检查。
- 确认跨层契约、错误路径、容量上限和资源释放；未运行或失败的检查写入交付说明。

平台 API 与转换工具细节查 Apple 官方文档、`rknn-pro` 或 `ascend-pro`；spec 仅保留项目约束和已确认的兼容性要点。
