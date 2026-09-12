# 规范导航

仓库级契约与验证命令见 [AGENTS.md](../../../AGENTS.md)；本目录补充项目专属的实现约束。

## Pre-Development Checklist

1. 阅读 [全局约定](./conventions.md)，确认时间、坐标、队列、资源释放等跨层硬约束。
2. 阅读 [架构概览](./architecture-overview.md)，确认代码归属和依赖方向。
3. 按变更范围进入下方对应专题列表，逐个读取。
4. 涉及接口、帧、配置或持久化边界时，加读 [跨层数据流](./cross-layer-thinking-guide.md)。

## 跨层指南

| 指南 | 触发条件 |
| --- | --- |
| [全局约定](./conventions.md) | 时间、坐标、队列、资源释放、测试、存储保护 |
| [架构概览](./architecture-overview.md) | 确认 crate 职责、数据流、平台边界 |
| [边缘资源约束](./edge-constraints-guide.md) | 新增逐帧工作、队列、缓存、写盘或阻塞调用 |
| [代码复用](./code-reuse-thinking-guide.md) | 新增公共逻辑、修改常量、枚举或重复解析 |
| [跨层数据流](./cross-layer-thinking-guide.md) | 修改 FFI、DTO、事件、坐标、配置或数据库映射 |

## 后端专题

适用于 Rust workspace、C/C++ 硬件垫片及算法插件。

| 专题 | 触发条件 |
| --- | --- |
| [目录与配置](../backend/directory-structure.md) | 新建模块/crate、调整依赖或配置 |
| [错误处理](../backend/error-handling.md) | 错误枚举、传播、降级或退出 |
| [日志](../backend/logging-guidelines.md) | 日志字段、span、采样与落盘 |
| [数据库](../backend/database-guidelines.md) | 表、迁移、查询、批处理与证据清理 |
| [API](../backend/api-guidelines.md) | HTTP、WS、DTO、鉴权与审计 |
| [并发](../backend/concurrency-guidelines.md) | async/线程、队列、共享状态、停机 |
| [推理后端](../backend/inference-backends.md) | 后端接入、模型加载与调度 |
| [媒体管线](../backend/media-pipeline.md) | RTSP、解码、帧内存、门控与流分发 |
| [FFI](../backend/ffi-guidelines.md) | unsafe、ABI、句柄与构建绑定 |
| [算法 SDK](../backend/algo-sdk-guidelines.md) | 插件、C ABI、预处理、交付与沙箱 |
| [检测与告警契约](../backend/detection-alarm-contract.md) | 检测载荷、坐标、规则告警、证据状态 |
| [质量检查](../backend/quality-guidelines.md) | 验证、测试与审查 |

## 前端专题

适用于 `web/` 的 Vite + React + TypeScript SPA。

| 专题 | 触发条件 |
| --- | --- |
| [目录与 i18n](../frontend/directory-structure.md) | 新文件、feature 边界、翻译资源 |
| [组件](../frontend/component-guidelines.md) | 组件接口、播放器、Canvas、交互 |
| [样式](../frontend/styling-guidelines.md) | 主题、token、排印、响应式叠加 |
| [Hook](../frontend/hook-guidelines.md) | 逻辑复用、订阅、清理与依赖 |
| [状态](../frontend/state-management.md) | Zustand、服务端资源、WS 增量 |
| [错误处理](../frontend/error-handling.md) | API 错误、401、渲染隔离与重连 |
| [类型与时间](../frontend/type-safety.md) | DTO、边界校验、联合类型、时间显示 |
| [质量检查](../frontend/quality-guidelines.md) | 测试、构建、性能、视觉与可访问性 |

## Quality Check

- 检查需求是否落在正确层，按受影响层执行质量门禁。
- 审查意见先核对实际来源、调用路径和设计意图，再判断优先级；不把推测当作缺陷。
- 确认测试能因目标行为被破坏而失败，避免只复述实现的断言。

## 规范维护

- 一项约束只在所属专题完整定义，其他文档用链接引用；索引只做导航，指南只列检查点。
- 保留输入输出、单位、校验、错误、所有权、容量和验证点；通用教程、整段实现和重复禁令不进入 spec。
- 完整类型、配置默认值和枚举以源文件链接承载；短例只解释不直观的边界。
- 临时方案、调试过程和单次性能测量放 `docs/` 或 PR/Issue 记录；未实现设计须明确标注，不能写成现有能力。
- 发现文档与代码不符时记录差异，以可验证事实修正文档；不得借此放宽 `AGENTS.md` 的硬约束。
- 保持现有文件路径与有效链接，变更后检查索引及锚点有效性。
