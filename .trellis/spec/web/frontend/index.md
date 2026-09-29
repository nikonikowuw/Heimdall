# web — 规范入口

适用于 `web/` 的 Vite + React + TypeScript SPA。依赖版本见 [package.json](../../../../web/package.json)，仓库级契约见 [AGENTS.md](../../../../AGENTS.md)。

## Pre-Development Checklist

1. 按变更范围读取本层专题：
   - [目录与 i18n](./directory-structure.md)：文件归属、模块边界、三语语言包
   - [组件](./component-guidelines.md)：接口与归属、实时页面、播放器生命周期、交互
   - [Hook](./hook-guidelines.md)：订阅与清理、渲染期约束、逐帧路径
   - [状态管理](./state-management.md)：Zustand 划分、WS 增量、服务端资源
   - [类型与时间](./type-safety.md)：DTO、边界收窄、联合类型、时间显示
   - [样式](./styling-guidelines.md)：主题 token、排印、视频叠加
   - [错误处理](./error-handling.md)：API client、401、连接恢复
2. 对接接口时同时阅读 [API 规范](../../api/backend/api-guidelines.md) 与 [全局约定](../../guides/conventions.md)（camelCase、13 位毫秒、`[0,1]` 坐标）。
3. 跨层数据（WS payload、DTO）变更阅读 [跨层思考指南](../../guides/cross-layer-thinking-guide.md)；高频流渲染遵循本层组件/状态规范的 Canvas 独立重绘闭环。

## Quality Check

- 执行 [AGENTS.md](../../../../AGENTS.md) 的 Web 门禁（format → lint → typecheck → test → check:cycles → build），零 Warning。
- 按 [质量检查](./quality-guidelines.md) 验证用户可观察行为；实时页面检查无关事件不重建播放器。
- 双主题、三语、键盘操作和卸载清理均按变更范围验证；明确记录未运行或失败的检查。
