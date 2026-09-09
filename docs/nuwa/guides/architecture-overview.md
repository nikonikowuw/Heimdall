# 架构概览

Heimdall 在边缘设备本地完成多路视频分析、规则告警和证据管理。架构硬约束见 [AGENTS.md](../../../AGENTS.md)。

## crate 依赖方向

依赖保持单向；具体模块位置见 [后端目录结构](../backend/directory-structure.md)。

| 层                           | 职责与边界                                             |
| ---------------------------- | ------------------------------------------------------ |
| `types`                      | 共享领域类型、`FrameRef`；不承载 IO、数据库或平台 SDK  |
| `db`                         | SQLite、迁移、Repository；向上提供持久化接口           |
| `media` / `infer`            | 媒体接入与推理；互不依赖，通过 `types::FrameRef` 交汇  |
| `pipeline`                   | 编排、门控、后处理、跟踪、规则判定与证据生成           |
| `api`                        | HTTP/WS 适配、DTO、控制句柄；不直接操作 SQL DSL 或硬件 |
| `app`                        | 配置、依赖装配、启动与退出                             |
| `algo-sdk` / `algo-packages` | 独立算法插件契约与实现；插件不依赖宿主业务 crate       |

平台差异收敛在媒体、推理及其 FFI 实现内，上层不加平台 feature 分支。算法插件边界见 [算法 SDK](../backend/algo-sdk-guidelines.md)。

## 运行时数据流

```text
子码流 → 硬件解码 → FrameRef → 门控/推理 → 后处理/跟踪/规则判定
主码流 → 裸 NALU RingBuffer → 告警时按需解码 → 全景图与 10% 扩边特写
业务事件 → 有界写入缓冲 → SQLite / 证据文件 → HTTP 查询与稀疏 WS 通知
```

- 默认仅子码流常驻解码与推理；主码流不常驻解码。
- 帧路径、平台载体及允许读回的场景统一见 [媒体管线](../backend/media-pipeline.md)。
- 网络 IO 与硬件执行分离，所有队列有界；线程归属和停机见 [并发模型](../backend/concurrency-guidelines.md)。
- 插件内部状态与 Pipeline 的全局跟踪/规则状态独立，不共享内部 track ID。
- 业务证据分为 Alarms、Captures、Recognitions；存储约束见 [数据库规范](../backend/database-guidelines.md)。

## 工程与交付

- Rust workspace 的成员与依赖版本由 [Cargo.toml](../../../Cargo.toml) 统一管理。
- HTTP/WS 使用 Axum + Tokio，持久化使用 SeaORM + SQLite + Refinery。
- 前端为 Vite + React + TypeScript SPA，具体依赖见 [web/package.json](../../../web/package.json)。
- 生产 SPA 通过 [static_files.rs](../../../crates/api/src/static_files.rs) 的 `rust-embed` 内嵌，保持单二进制交付；算法包独立管理。
- 平台目标为 Apple Silicon、Rockchip 和 Ascend；CPU 仅作物理无硬件环境的显式调试回退。
- 专有驱动确需 C/C++ 时使用极薄 `native/` 垫片，不承载业务编排。

实现入口用于定位代码，不代表所有平台已通过真机验证；性能和兼容性结论必须附对应环境的验证记录。
