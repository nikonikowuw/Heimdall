# 规范导航

工程规范体系总入口。仓库级硬契约与验证门禁见 [AGENTS.md](../../../AGENTS.md)；AI 协作工作流见 [.trellis/workflow.md](../../../.trellis/workflow.md)。

## Pre-Development Checklist

1. 阅读 [全局约定](./conventions.md)：时间、坐标、队列通道、CMA/内存预算、DTO 契约、资源释放与存储保护硬约束。
2. 阅读 [架构概览](./architecture-overview.md)：代码归属、依赖方向、运行时数据流与抽象复用原则。
3. 按下方「包与分层入口」进入目标包的 `index.md`，逐个读取其中清单指向的文件；跨多个包时全部读取。
4. 跨层数据变更（DTO/WS/FFI/插件 ABI）同时阅读 [跨层思考指南](./cross-layer-thinking-guide.md)；新增工具与抽象前阅读 [代码复用思考指南](./code-reuse-thinking-guide.md)。

## 共享指南索引

| 指南 | 触发条件 |
| --- | --- |
| [全局约定](./conventions.md) | 时间、坐标、队列通道、边缘内存预算、DTO 契约、资源释放、存储保护 |
| [架构概览](./architecture-overview.md) | Crate 职责边界、单向依赖、三大路径拓扑、抽象与复用原则 |
| [错误处理](./error-handling.md) | 错误枚举、传播、降级或退出 |
| [日志](./logging-guidelines.md) | 日志字段、span、采样与落盘 |
| [并发](./concurrency-guidelines.md) | async/线程、队列、共享状态、停机 |
| [FFI](./ffi-guidelines.md) | unsafe、ABI、句柄与构建绑定 |
| [目录与配置](./directory-structure.md) | 新建模块/crate、调整依赖或配置 |
| [质量检查](./quality-guidelines.md) | 验证、测试与审查 |
| [跨层思考指南](./cross-layer-thinking-guide.md) | 变更跨 3+ 层、多消费者、新增载荷或配置字段 |
| [代码复用思考指南](./code-reuse-thinking-guide.md) | 重复模式、新增工具、修改常量或配置 |

## 包与分层入口

| 包 | 代码路径 | spec 层 |
| --- | --- | --- |
| algo-sdk | `crates/algo-sdk` | [backend](../algo-sdk/backend/index.md) |
| types | `crates/types` | [backend](../types/backend/index.md) |
| db | `crates/db` | [backend](../db/backend/index.md) |
| media | `crates/media` | [backend](../media/backend/index.md) |
| infer | `crates/infer` | [backend](../infer/backend/index.md) |
| pipeline | `crates/pipeline` | [backend](../pipeline/backend/index.md) |
| api | `crates/api` | [backend](../api/backend/index.md) |
| app | `crates/app` | [backend](../app/backend/index.md) |
| web | `web/` | [frontend](../web/frontend/index.md) |

平台 API 与转换工具细节查 Apple 官方文档、`rknn-pro` 或 `ascend-pro`；spec 仅保留项目约束和已确认的兼容性要点。

## 规范维护

- 一项约束只在所属文档完整定义，其他文档用链接引用；索引只做导航，指南只列检查点。
- 保留输入输出、单位、校验、错误、所有权、容量和验证点；通用教程、整段实现与重复禁令不进入 spec。
- 完整类型、配置默认值和枚举以源文件链接承载；短例只解释不直观的边界。
- 临时方案、调试过程和单次性能测量放 `docs/` 或 PR/Issue 记录；未实现设计须明确标注，不能写成现有能力。
- 规范更新随 Trellis 工作流沉淀（Phase 3.3 / `trellis-update-spec`）；发现文档与代码不符时以可验证事实修正文档，不得借此放宽 `AGENTS.md` 硬约束。
- 保持文件路径与有效链接，变更后检查索引及锚点有效性。
