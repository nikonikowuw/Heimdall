# Nuwa (女娲) 工程规范与架构体系

> **Heimdall 系统的技术规范、设计哲学与工程实现守则**  
> *Engineering Specifications & Architectural Standards for Heimdall*

---

## 🏛️ 设计哲学与不可妥协原则

Nuwa 规范体系为 Heimdall 提供统一的工程准绳。系统定位于工业级边缘视频分析与证据闭环，所有模块开发均需遵守以下不可妥协原则。完整约束见各专题文件，全局跨层约定见 [全局约定](./guides/conventions.md)。

1. **码流分析模式用户可控**：支持 `main`/`sub`/`auto` 三种模式，用户选择分析码流，`StreamHub` 复用单条物理连接。详见 [媒体管线](./backend/media-pipeline.md)。
2. **三大路径严格隔离**：常驻推理主路径保持设备原生 buffer 零拷贝；低频证据路径仅告警时 Device→Host readback；调试回退路径仅 CPU 保底。详见 [媒体管线](./backend/media-pipeline.md)。
3. **单二进制自包含交付**：`rust-embed` 内嵌 Web SPA，单端口开箱即用。
4. **空间坐标归一化与动态布防**：ROI/Mask/Line 坐标归一化为 `[0.0, 1.0]`，前端在动态实时流上叠加矢量交互层。详见 [全局约定](./guides/conventions.md)。
5. **存储安全与原子级联淘汰**：`statvfs` 监控水位，单一事务内同步销毁物理文件与 DB 记录。详见 [数据库规范](./backend/database-guidelines.md)。
6. **算法沙箱与防御性安全**：C ABI 虚表 + 六步沙箱自检 + Tar/Zip-Slip 防御。详见 [算法 SDK](./backend/algo-sdk-guidelines.md)。

---

## 📚 规范导航索引 (Specification Index)

### 1. 全局思考与跨层指南 (Guides)
| 文档 | 说明 | 重点关注 |
| :--- | :--- | :--- |
| [🧭 规范导航](./guides/index.md) | 规范体系入口与 Pre-Development Checklist | 需求归属、门禁检查点 |
| [🏗️ 架构概览](./guides/architecture-overview.md) | 系统层级划分、数据流向与边界 | Crate 职责边界、单向依赖原则 |
| [⏱️ 边缘资源约束](./guides/edge-constraints-guide.md) | 边缘环境算力、内存与 IO 预算 | 逐帧时间预算、队列上限、防阻塞 |
| [🔄 跨层数据流](./guides/cross-layer-thinking-guide.md) | FFI、DTO、WebSocket 与 DB 契约 | 13 位 Unix 毫秒、归一化坐标、无损映射 |
| [🧩 代码复用思考](./guides/code-reuse-thinking-guide.md) | 公共逻辑抽象与防过度设计 | 共享类型提取、单一职责 |

### 2. 后端工程规范 (Backend Specs)
适用于 Rust Workspace、C/C++ 硬件垫片及算法沙箱集成。

| 文档 | 说明 | 重点关注 |
| :--- | :--- | :--- |
| [📖 后端总览与清单](./backend/index.md) | 后端规范入口与开发前检查清单 | 模块归属、实现入口 |
| [📹 媒体管线](./backend/media-pipeline.md) | RTSP 拉流、解码、RingBuffer 与 StreamHub | 零拷贝帧引用、丢帧降级、无界队列杜绝 |
| [🧠 推理后端](./backend/inference-backends.md) | RKNN、Apple Core ML、Ascend CANN 适配 | NPU 调度、内存池复用、RAII 释放 |
| [🔒 FFI 规范](./backend/ffi-guidelines.md) | C ABI、裸指针、内存对齐与 safe 封装 | `// SAFETY:` 注释、双侧内存布局断言 |
| [📦 算法 SDK 与沙箱](./backend/algo-sdk-guidelines.md) | 动态库装载、参数校验与六步自检沙箱 | 目录防穿透、子进程隔离自检、虚表版本 |
| [🚨 检测告警契约](./backend/detection-alarm-contract.md) | 检测载荷、ByteTrack、几何判定与证据三支柱 | `trackId` 连续性、5 秒冷却防抖、状态流转 |
| [⚡ 并发与异步](./backend/concurrency-guidelines.md) | Tokio 异步、专用工作线程与停机协调 | 不持锁 await、有界通道、优雅退出 |
| [💾 数据库与淘汰](./backend/database-guidelines.md) | SQLite WAL 模式、迁移与原子级联淘汰 | statvfs 水位检测、单事务图文同步清理 |
| [🌐 API 与网关](./backend/api-guidelines.md) | Axum HTTP/WS、统一信封与鉴权 | `/api/v1`、`camelCase`、统一响应结构体 |
| [⚠️ 错误处理](./backend/error-handling.md) | thiserror/anyhow 分层与降级策略 | 严禁 unwrap、错误上下文与安全屏蔽 |
| [📝 日志规范](./backend/logging-guidelines.md) | tracing 结构化日志、span 与采样 | 禁用 println/dbg、高频帧路径采样降噪 |
| [📁 目录组织](./backend/directory-structure.md) | Crates 边界划分与配置管理 | 单向依赖、配置分层 |
| [🧪 质量与测试](./backend/quality-guidelines.md) | 单元测试、集成测试与真机标记 | `#[ignore]` 硬件测试、边界验证 |

### 3. 前端工程规范 (Frontend Specs)
适用于 `web/` 的 Vite + React 19 + TypeScript 现代化控制台。

| 文档 | 说明 | 重点关注 |
| :--- | :--- | :--- |
| [📖 前端总览与清单](./frontend/index.md) | 前端规范入口与开发前检查清单 | 状态归属、播放器隔离 |
| [🎨 组件设计](./frontend/component-guidelines.md) | React 19 组件规范与播放器隔离 | 播放器不随无关状态重刷、SVG 矢量绘制 |
| [🏪 状态管理](./frontend/state-management.md) | Zustand 状态划分与 WebSocket 增量管理 | UI 状态 vs 服务端缓存、细粒度选择器 |
| [🪝 Hook 规范](./frontend/hook-guidelines.md) | 自定义 Hook 封装与订阅生命周期 | 严格清理定时器/WS、防止内存泄漏 |
| [🛡️ 类型与契约](./frontend/type-safety.md) | TypeScript 严格模式与前后端契约 | 严禁 `@ts-ignore`、毫秒整数时间戳 |
| [✨ 工业级视觉](./frontend/styling-guidelines.md) | Clean-Room 双主题、Tailwind 与 Lucide | 严禁 Emoji、统一 Lucide 矢量图标 |
| [🚨 错误处理](./frontend/error-handling.md) | ErrorBoundary、网络重试与弱网恢复 | WebSocket 指数退避、渲染错误隔离 |
| [🌐 目录与 i18n](./frontend/directory-structure.md) | Feature 目录组织与中简/中繁/英文三语 | 所有文本必须接入 i18n 语言包 |
| [🧪 质量与构建](./frontend/quality-guidelines.md) | ESLint、TypeScript 门禁与端到端测试 | 零 Warning 门禁、构建产物尺寸监控 |

---

## 🚦 质量门禁检查命令 (Quality Gate)

### 后端门禁 (Rust / Native)
```bash
cargo fmt --all
if [ -d native ]; then
  find native -type f \( -name '*.c' -o -name '*.h' \) -exec clang-format -i {} +
fi
cargo fmt --all -- --check
cargo clippy --all-targets -- -D warnings
cargo test --workspace
```

### 前端门禁 (Web Console)
```bash
cd web
pnpm format
pnpm lint          # eslint --max-warnings=0
pnpm typecheck     # tsc --noEmit
pnpm test          # vitest run
pnpm build
```
