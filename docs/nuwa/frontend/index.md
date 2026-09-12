# 前端开发规范

适用于 `web/` 的 Vite + React + TypeScript SPA。依赖版本见 [package.json](../../../web/package.json)，仓库契约见 [AGENTS.md](../../../AGENTS.md)。

## 入口

所有前端专题的完整索引与触发条件已收敛在 [规范导航](../guides/index.md)，以该页为准。

对接接口时同时阅读 [API 规范](../backend/api-guidelines.md) 和 [跨层数据流](../guides/cross-layer-thinking-guide.md)。

## Quality Check

- 执行 `AGENTS.md` 的 Web 门禁，格式化先于 lint、类型检查、测试和构建。
- 按 [质量规范](./quality-guidelines.md) 验证用户可观察行为；实时页面检查无关事件不重建播放器。
- 双主题、三语、键盘操作和卸载清理均按变更范围验证；明确记录未运行或失败的检查。
