# 前端开发规范

适用于 `web/` 的 Vite + React + TypeScript SPA。依赖版本见 [package.json](../../../web/package.json)，仓库契约见 [AGENTS.md](../../../AGENTS.md)。

## Pre-Development Checklist

- 阅读 [架构概览](../guides/architecture-overview.md)，按下表选取相关专题。
- 对接接口时同时阅读 [API 规范](../backend/api-guidelines.md) 和 [跨层检查](../guides/cross-layer-thinking-guide.md)。
- 先确认状态归属、播放器隔离、事件缓冲上限和订阅清理责任。
- 界面沿用主题 token、Lucide 与 i18n；所有可见文本接入三语翻译。

| 专题                                    | 触发条件                          |
| --------------------------------------- | --------------------------------- |
| [目录与 i18n](./directory-structure.md) | 新文件、feature 边界、翻译资源    |
| [组件](./component-guidelines.md)       | 组件接口、播放器、Canvas、交互    |
| [样式](./styling-guidelines.md)         | 主题、token、排印、响应式叠加     |
| [Hook](./hook-guidelines.md)            | 逻辑复用、订阅、清理与依赖        |
| [状态](./state-management.md)           | Zustand、服务端资源、WS 增量      |
| [错误处理](./error-handling.md)         | API 错误、401、渲染隔离与重连     |
| [类型与时间](./type-safety.md)          | DTO、边界校验、联合类型、时间显示 |
| [质量检查](./quality-guidelines.md)     | 测试、构建、性能、视觉与可访问性  |

## Quality Check

- 执行 `AGENTS.md` 的 Web 门禁，格式化先于 lint、类型检查、测试和构建。
- 按 [质量规范](./quality-guidelines.md) 验证用户可观察行为；实时页面检查无关事件不重建播放器。
- 双主题、三语、键盘操作和卸载清理均按变更范围验证；明确记录未运行或失败的检查。
