# 后端开发规范

适用于 Rust workspace、算法插件及必要的 C/C++ 硬件垫片。仓库契约见 [AGENTS.md](../../../AGENTS.md)。

## 入口

所有后端专题的完整索引与触发条件已收敛在 [规范导航](../guides/index.md)，以该页为准。

## Quality Check

- 按 [质量规范](./quality-guidelines.md) 选择测试，执行 `AGENTS.md` 的 Rust/Native 门禁，格式化先于检查。
- 确认跨层契约、错误路径、容量上限和资源释放；未运行或失败的检查写入交付说明。

平台 API 与转换工具细节查 Apple 官方文档、`rknn-pro` 或 `ascend-pro`；spec 仅保留项目约束和已确认的兼容性要点。
