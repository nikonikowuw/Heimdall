# algo-sdk — 规范入口

适用于 `crates/algo-sdk`：算法插件契约、C ABI 虚表、帧契约、沙箱与宿主侧加载。仓库级契约见 [AGENTS.md](../../../../AGENTS.md)。

## Pre-Development Checklist

1. 阅读 [算法 SDK 与 C ABI](./algo-sdk-guidelines.md)：构建边界、状态与生命周期、导出接口、帧契约（ns 单位）、状态码与共享底库 ABI。
2. FFI 契约变更同时阅读 [FFI 边界](../../guides/ffi-guidelines.md) 与 [全局约定](../../guides/conventions.md)（防御性错误处理、资源释放）。
3. 插件交付物、六步沙箱自检与归档安全以 [AGENTS.md](../../../../AGENTS.md) 项目契约为准。
4. 抽象复用判断参考 [代码复用思考指南](../../guides/code-reuse-thinking-guide.md)。

## Quality Check

- 执行 [AGENTS.md](../../../../AGENTS.md) 的 Rust/Native 门禁；算法包不属于根 workspace，按 AGENTS.md 中列出的平台 manifest 单独执行。
- 按 [质量检查](../../guides/quality-guidelines.md) 覆盖：ABI 布局断言、错误路径、资源释放与沙箱回归。
