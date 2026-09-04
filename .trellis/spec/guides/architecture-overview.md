# Argus 架构概览

> 所有 spec 文件的锚点。写任何代码前先读这一份，确认你要动的东西属于哪一层。

> ⚠️ **状态：立项约定（尚未经代码验证）**
> Argus 仓库当前无产品代码。本文件是基于既定技术栈的前瞻性约定，而非从现有代码提取的事实。
> 首批代码落地后，必须回填真实文件路径与示例，并删除本提示。

---

## 系统定位

Argus 是**面向边缘计算场景的高性能 AI 视频分析与管理系统**。接入多路摄像头，在设备本地完成硬解码、目标检测与事件判定，实现高并发拉流、毫秒级本地告警、快照与抓拍证据落盘，并通过现代化轻量 Web 控制台进行实时监控、算法布防与配置管理。

关键含义（会渗透到每一条规范里）：

- **Rust-Native 纯粹现代管线架构（Rust as the Engine）**：
  - **Rust 掌控全链路流媒体与推理中枢**：拉流解复用、运动检测门控、ROI/Mask/Line 规则引擎、NPU 推理调度、NMS 后处理、ByteTrack 多目标跟踪、业务持久化与 WebSocket 广播全部在 Rust 内以类型安全、无锁并发的方式实现。彻底摆脱传统 C++ 重型流媒体引擎的虚表、内存泄漏与复杂构建包袱。
  - **开源生态优先（Open-Source First）**：
    - 拉流与解码：基于纯 Rust 异步 RTSP 库（如 `retina`）或 `ffmpeg-next`；
    - 图像缩放与 SIMD：基于高性能向量化 `fast_image_resize` 与 `imageproc`；
    - 跨平台推理引擎：跨平台回退使用 **`ort`** (ONNX Runtime)、苹果生态使用 **`coreml-rs`** / `objc2`、瑞芯微 (RKNN) 与华为昇腾 (AscendCL) 采用专用微型硬件胶合垫片（`native/`）对接底层驱动。
  - **极薄平台硬件垫片（Minimal Platform Shims）**：C/C++ 严格退守到底层微型胶合垫片（百行级别驱动包装），仅在需要穿透平台底层专有零拷贝总线（如 Rockchip 的 `MPP -> RGA -> RKNN` DMA-BUF 共享）时作为极薄 FFI 存在，上层完全面向统一的 Rust Trait 编程。
- **极致的单二进制自包含交付（All-in-One Binary）**：
  - 前端 React SPA 产物通过 `rust-embed` 直接编译进二进制，整个系统运行时只需一个可执行文件 `argus`，单工具链 `cargo build` 极速交付。
- **算力和内存是硬约束**，不是优化项。设计时先问"这一路 1080p@15fps 要占多少内存"。
- **帧数据不能拷贝**。解码输出到推理输入之间必须走平台原生的零拷贝路径（通过 `FrameRef` 持有 DMA-BUF fd 或 `CVPixelBuffer` 原生句柄）。
- **设备无人值守长期运行**。内存泄漏、句柄泄漏、存储写满都是致命故障，Rust 所有权机制与 RAII 从根源杜绝悬垂指针与资源泄露。

---

## 技术栈

| 层 | 选型 | 说明 |
|----|------|------|
| 核心主程序与管线 | Rust（2021 edition） | 业务编排、拉流调度、规则引擎、后处理、Web 服务 |
| RTSP 接入与解码 | `retina` / `ffmpeg-next` | 异步 RTSP 拉流解复用，平台硬件加速解码接入 |
| 图像预处理与 SIMD | `fast_image_resize` | AVX2 / NEON 向量化加速，用于小图门控与格式缩放 |
| 推理后端 | `ort` / `coreml-rs` / `native/` 平台硬件胶合 | 成熟开源库结合专用极薄 C 垫片，按平台 feature 编译切换 |
| 目标跟踪与规则判定 | 纯 Rust 实现 | ByteTrack 多目标跟踪，ROI/Mask/Line 向量几何算法 |
| 平台硬件极薄胶合 | 微型 C/C++ 垫片（`native/`） | 仅用于 DMA-BUF / 硬件解码专有总线穿透 |
| HTTP + WebSocket | Axum + Tokio + Tower | RESTful 控制 API 与实时高频事件推送 |
| 静态资源内嵌 | `rust-embed` | React 构建产物打包进二进制，实现单文件部署 |
| 数据库 | SQLite + SeaORM | WAL 模式高并发读写，批量提交保护 eMMC |
| 日志 / 可观测 | tracing + tracing-subscriber | 结构化日志，与帧率解耦 |
| 错误处理 | thiserror（库层） / anyhow（入口） | 强类型领域错误枚举 |
| 前端 UI | Vite + React + TypeScript + Tailwind CSS v4 | 双主题组件化控制台（Clean-Room / Dark Industrial），i18n 三语 |
| 前端状态 | Zustand | 细粒度选择器订阅，杜绝多路视频下级联重渲染 |
| 前端可视化 | Three.js (拓扑背景) + Canvas 2D (检测框叠加) | 线框几何体惯性视差 + 毫秒级 Bounding Box 绘制 |
| 前端字体 | Unbounded (Display) + Space Grotesk (Tech) + Inter (Body) + Space Mono (Data) | 四套字体各司其职，等宽数字零抖动 |
| 前端 i18n | react-i18next | zh-CN / zh-TW / en 三语，翻译按语言懒加载 |

---

## 目标平台矩阵

| 平台 | 推理 | 解码 | 预处理 | 零拷贝载体 |
|------|------|------|--------|-----------|
| Apple Silicon (M 系列) | Core ML / ANE | VideoToolbox | vImage / Metal | `CVPixelBuffer` + `IOSurface` |
| Huawei Ascend (Atlas) | AscendCL + OM 模型 | DVPP | AIPP / DVPP | DVPP device buffer |
| Rockchip (RK3588 / RK3568 / RK3576) | RKNN Runtime | MPP | RGA | DMA-BUF fd |
| 开发机回退 | CPU backend | 软解 | CPU | 普通堆内存 |

**规则**：任何跨平台差异都收敛在 `infer` 和 `media` 两个 crate 内部，通过 trait + feature gate 隔离。上层 crate（`pipeline`、`api`）代码中**不允许出现 `#[cfg(feature = "backend-rknn")]` 之类的平台分支**。

平台细节不写在 spec 里 —— 分别由 `rknn-pro`、`ascend-pro` 技能与 Apple 官方 Core ML / VideoToolbox 文档承载。spec 只规定边界在哪、抽象长什么样。

---

## 仓库布局

Monorepo，Cargo workspace 为主体核心，外加极薄硬件垫片、模型定义与 React 前端：

```
argus/
├── Cargo.toml                 # Cargo workspace root，统一锁定生态依赖版本
├── crates/
│   ├── types/                 # 领域核心类型（Camera、Task、Detection、FrameRef）、通用枚举
│   ├── db/                    # SeaORM entity + Refinery SQL migration（SQLite WAL 存储）
│   ├── media/                 # 视频接入 (retina/ffmpeg)、硬件解码抽象与池化帧缓冲
│   ├── infer/                 # 推理后端 trait (ort, coreml-rs, native 平台硬件后端)
│   ├── pipeline/              # 纯 Rust 业务管线：抽帧 → 门控 → ROI规则 → 推理调度 → NMS/ByteTrack
│   ├── api/                   # Axum router / handler / DTO / WebSocket / rust-embed 静态前端
│   └── app/                   # 单二进制主入口：统一启动、配置热重载、信号优雅关停（最终输出二进制 argus）
├── native/                    # 极薄平台硬件胶合（仅在开源 crate 无法覆盖硬件零拷贝时使用）
│   ├── rknn/                  # MPP / RGA / RKNN DMA-BUF 微型胶合
│   └── ascend/                # DVPP / AIPP 微型胶合
├── models/                    # 模型 manifest.toml 元数据与转换脚本
├── web/                       # Vite + React 前端工程（构建产物内嵌进二进制）
└── .trellis/                  # 本规范目录
```

---

## crate 依赖方向

依赖是清晰严格的**单向拓扑**，绝不允许出现反向依赖或环：

```
app
 │
 ├──> api ──> pipeline ──┬──> media ──┐
 │     │                 └──> infer ──┤
 │     └──> db ───────────────────────┤
 │                                    │
 └────────────────────────────────────┴──> types
```

规则：

- `types` 是叶子：定义纯领域类型与 `FrameRef` 抽象，不依赖 tokio、axum、sea-orm、平台 SDK。
- `media` 与 `infer` **互不依赖**。两者的交汇点是 `types` 里的 `FrameRef`。
- `pipeline` 负责核心编排，是唯一的推理调用方与规则判定方，产出业务事件。
- `api` 仅依赖 `pipeline` 暴露的控制句柄与 `db`，不碰任何底层媒体/推理逻辑。
- 只有 `app` 知道如何具体组装（选哪个推理后端、哪个解码器）。

---

## 运行时数据流（纯 Rust-Native 全链路）

```
Web 客户端 (React)
   │
   ├── ① 任务下发 / 规则配置 (REST API) ──> [api] ──> [pipeline]
   │
摄像头 (RTSP)
   │
   ▼
[media] 解码线程（每路独立 OS 线程，调用硬解驱动或开源封装）
   │  产出 FrameRef（持有 DMA-BUF / CVPixelBuffer 句柄，零 CPU 拷贝）
   │  通过 crossbeam 有界通道移交（满则丢弃最旧帧）
   ▼
[pipeline] 抽帧 / SIMD 运动门控（过滤 90% 静止帧）
   │  未命中运动直接释放 FrameRef 归还缓冲池
   ▼
[infer] 推理 Worker（数量 = NPU core 数，调用 ort / coreml-rs / native 后端）
   │  产出原始张量或 Detection 列表
   ▼
[pipeline] 后处理管线
   │  ├─ 统一 NMS 抑制
   │  ├─ ROI 区域 / Mask 遮罩 / Line 跨线判定 (纯 Rust 向量运算)
   │  └─ ByteTrack 多目标跟踪 (分配稳定 TrackId)
   │
   ┌───────────────────────────┴───────────────────────────┐
   ▼                                                       ▼
[db] 批量写入 SQLite WAL 数据库                 [api] 实时广播 (WebSocket)
                                                           │
   ◄── ② 实时事件推送与 Canvas 2D 毫秒对齐渲染 ───────────────┘
```

---

## 各层规范入口

| 你要做的事 | 读哪份 |
|-----------|--------|
| 新建 crate / 决定代码放哪 | [backend/directory-structure.md](../backend/directory-structure.md) |
| 定义或传播错误 | [backend/error-handling.md](../backend/error-handling.md) |
| 打日志 / 加指标 | [backend/logging-guidelines.md](../backend/logging-guidelines.md) |
| 加表 / 写查询 / 迁移 | [backend/database-guidelines.md](../backend/database-guidelines.md) |
| 加 HTTP 或 WebSocket 接口 | [backend/api-guidelines.md](../backend/api-guidelines.md) |
| 决定 async 还是线程 | [backend/concurrency-guidelines.md](../backend/concurrency-guidelines.md) |
| 加 / 改推理后端 | [backend/inference-backends.md](../backend/inference-backends.md) |
| 动解码、零拷贝、预处理 | [backend/media-pipeline.md](../backend/media-pipeline.md) |
| 写 Rust ↔ C++ 绑定 | [backend/ffi-guidelines.md](../backend/ffi-guidelines.md) |
| 写前端组件 / 页面 | [frontend/index.md](../frontend/index.md) |
| 判断资源开销是否可接受 | [edge-constraints-guide.md](./edge-constraints-guide.md) |

---

## 待验证事项

本文件基于立项决策，以下需在首批代码落地后确认并回填：

- [ ] crate 划分是否成立（`pipeline` 是否会过厚，需不需要拆出 `tracker`）
- [ ] Rust ↔ C++ 绑定选型：`cxx` / `bindgen` / 手写 `extern "C"`（见 `ffi-guidelines.md`）
- [ ] 前端静态资源交付方式：`rust-embed` 内嵌单二进制 vs `ServeDir` 外挂目录
- [ ] 三平台是否需要各自独立的构建产物，还是一个二进制多 feature
