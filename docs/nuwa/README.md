# Nuwa (女娲) 工程规范与架构体系

> **Heimdall 系统的技术规范、设计哲学与工程实现守则**  
> *Engineering Specifications & Architectural Standards for Heimdall*

---

## 🏛️ 设计哲学与不可妥协原则

Nuwa 规范体系为 Heimdall 提供统一的工程准绳。系统定位于工业级边缘视频分析与证据闭环，所有模块开发均需遵守以下不可妥协原则：

1. **码流分析模式用户可控与高能效分工（Analysis Stream Selection Policy）**：
   - 将分析码流选择权完全交予用户，支持三种模式：
     - **主码流高清分析 (`main`)**：常驻硬件解码主码流，将原生 `FrameRef` 直接下发至算法包；算法包自行选择预处理尺寸、裁切和色彩格式，快照直接复用设备帧零解码。若未来需要宿主统一预处理，必须先通过算法能力协商得到该实例的格式/尺寸约束，不能使用全局固定尺寸；
     - **子码流低能耗分析 (`sub`)**：常驻硬解低分辨率子码流，主码流仅以裸 NALU 写入 RingBuffer，告警触发时按需单帧解码，节约 VPU 算力以支持超高并发路数；
     - **智能自适应模式 (`auto`)**：优先探活子码流，若无子流或不可达则自适应降级使用主码流常驻分析；
   - 底层无论单流还是双流，均由 `StreamHub` 自动复用底层单条物理 TCP 连接，杜绝重复拉流带宽。
2. **严格区隔三大路径与纯设备侧零拷贝**：
   - **常驻推理主路径 (`infer_fast_path`)**：保持设备原生 buffer（DMA-BUF / CVPixelBuffer / device memory），严禁在主路径上发生 CPU 像素拷贝、色彩转换或 CPU 软解；
   - **低频证据生成路径 (`snapshot_readback_path`)**：显式特例，仅在抓拍/告警触发时允许 D2H readback 并由 CPU 转存 JPEG；
   - **开发调试回退路径 (`debug_cpu_fallback_path`)**：仅无硬件加速单元时作为保底，严禁伪装为硬件加速。
3. **单二进制自包含交付（Single All-in-One Binary）**：
   - Rust 后端通过 `rust-embed` 将 Web 控制台完全内嵌，单端口开箱即用。
4. **空间坐标归一化与动态布防**：
   - 几何规则（ROI / Mask / Line）坐标必须严格归一化为 `[0.0, 1.0]` 区间；
   - 前端布防必须在动态实时流上叠加矢量交互层绘制，严禁静态死图标注。
5. **存储安全与原子级联淘汰**：
   - 基于 `statvfs` 内核调用监控物理高低水位，杜绝高开销 `du` 扫描；
   - 淘汰时优先清除无告警普通抓拍，单一 SQLite 事务内同步销毁物理文件与 DB 记录（“图在案在，图销案销”）。
6. **算法沙箱与防御性安全**：
   - 算法包遵循稳定 C ABI 虚表规范；
   - 上传解压严格防御 Tar/Zip-Slip 路径逃逸漏洞；六步物理沙箱自检通过后方可加载。

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
