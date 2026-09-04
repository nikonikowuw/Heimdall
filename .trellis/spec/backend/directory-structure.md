# 后端目录结构

> Rust workspace 与 C++ 侧的代码归属规则。新建文件前先确认它该落在哪个 crate。

> ⚠️ **状态：立项约定（尚未经代码验证）**
> Argus 仓库当前无产品代码。首批 crate 落地后需回填真实路径与模块示例，并删除本提示。

---

## Workspace 布局

```
argus/
├── Cargo.toml              # [workspace] + [workspace.dependencies] 统一版本
├── rustfmt.toml
├── clippy.toml
├── config.toml             # 应用配置（默认值，入 git）
├── config.example.toml     # 配置模板（入 git，带注释说明）
├── .env.example            # 环境变量模板（入 git）
├── .env                    # 本地环境变量（不入 git）
├── .gitignore
├── crates/
│   ├── types/              # 领域核心类型（Camera、Task、FrameRef 等）、错误基类型
│   ├── db/                 # SeaORM entity、Refinery SQL migration、repository 函数（SQLite WAL）
│   ├── media/              # RTSP 拉流接入 (retina/ffmpeg)、平台硬件解码与池化帧缓冲
│   ├── infer/              # 推理后端 trait 抽象 (ort, coreml-rs, native 平台硬件后端)
│   ├── pipeline/           # 纯 Rust 核心管线：抽帧 → 门控 → ROI规则 → 推理调度 → NMS/ByteTrack
│   ├── api/                # Axum router、handler、DTO、WebSocket、rust-embed 内嵌前端
│   └── app/                # 单二进制主入口：统一装配启动、配置加载、优雅退出（二进制名 argus）
├── native/                 # 极薄底层硬件垫片（仅在开源 crate 无法直接覆盖专有驱动时启用）
│   ├── rknn/               # MPP / RGA / RKNN DMA-BUF 极薄胶合（< 300 行 C）
│   └── ascend/             # DVPP / AIPP 极薄胶合
├── models/                 # 模型 manifest.toml 与转换脚本
└── web/                    # Vite + React 前端工程（构建产物内嵌进 Rust 二进制）
```

依赖方向见 [guides/architecture-overview.md](../guides/architecture-overview.md#crate-依赖方向)。**新建 crate 前先确认不会引入环。**

---

## 各 crate 的职责边界

| crate | 装什么 | 明确不装什么 |
|-------|--------|-------------|
| `types` | 纯净领域类型（`Camera`、`Task`、`Detection`、`FrameRef`）、配置结构、通用枚举 | 任何 IO、tokio、axum、sea-orm、平台 SDK |
| `db` | SeaORM entity、Refinery SQL migration、repository 查询与批量提交函数 | 业务判定逻辑、HTTP 相关类型、直接网络请求 |
| `media` | RTSP 接入、解码器 trait 与硬件实现、帧缓冲池（Buffer Pool）管理 | 推理逻辑、业务规则、事件判定 |
| `infer` | `InferenceBackend` trait 抽象，集成 `ort`、`coreml-rs` 及 `native/` 平台硬件推理胶合 | 视频拉流、业务规则、事件生成 |
| `pipeline` | 编排调度：抽帧、SIMD 运动门控、ROI/Mask/Line 规则判定、NMS、ByteTrack 跟踪、事件产出 | HTTP 路由、直接 SQL 查询 |
| `api` | Axum router、handler、DTO 转换、轻量设备鉴权 (JWT/API-Key)、WebSocket 广播、`rust-embed` 前端 SPA 静态资源 | 图像计算、直接硬件驱动 |
| `app` | `main.rs`、单二进制入口、配置加载、依赖装配、信号处理（SIGTERM/SIGINT）、优雅退出 | 任何可复用业务逻辑 |

**判断规则**：如果一段逻辑需要被两个 crate 用到，它属于 `types`；如果它只是把别人拼起来，它属于 `app`。

---

## crate 内部模块布局

统一采用这个形状：

```
crates/infer/
├── Cargo.toml
├── src/
│   ├── lib.rs          # 只做 pub mod / pub use，不写实现
│   ├── error.rs        # 本 crate 的错误枚举（thiserror）
│   ├── backend.rs      # trait 定义
│   ├── backends/
│   │   ├── mod.rs
│   │   ├── rknn.rs     # #[cfg(feature = "backend-rknn")]
│   │   ├── ascend.rs
│   │   ├── coreml.rs
│   │   └── cpu.rs
│   └── postprocess/
│       ├── mod.rs
│       └── nms.rs
└── tests/              # 集成测试
```

约定：

- **`lib.rs` 不写实现**，只做模块声明和 re-export。
- **每个 crate 一个 `error.rs`**，定义本 crate 的错误枚举，见 [error-handling.md](./error-handling.md)。
- **不使用 `mod.rs` 之外的目录级实现文件混放**：要么 `foo.rs`，要么 `foo/mod.rs` + 子模块，一个 crate 内保持一致。
- **单文件超过 500 行就拆**，按职责拆不按行数硬切。

---

## 命名约定

| 对象 | 规则 | 示例 |
|------|------|------|
| crate | 简短单数名词，**无冗余项目前缀** | `types`、`media`、`infer`、`pipeline`、`db`、`api`、`app` |
| 模块 / 文件 | snake_case，名词 | `frame_buffer.rs` |
| trait | 大驼峰，能力名词或 `-able` | `InferenceBackend`、`Decodable` |
| trait 的平台实现 | `<平台><Trait 主干>` | `RknnBackend`、`CoreMlBackend` |
| 错误枚举 | `<域>Error` | `InferError`、`MediaError` |
| feature flag | `backend-<平台>` / `decoder-<平台>` | `backend-rknn` |

**避免**：`utils`、`common`、`helpers`、`misc` 这类无边界模块名。它们会变成垃圾堆。按职责命名，例如 `geometry.rs`、`time_window.rs`。

---

## 极薄底层硬件垫片布局（`native/`）

```
native/
├── rknn/
│   ├── include/argus_rknn.h  # 极薄 C 接口头文件（声明 DMA-BUF 零拷贝直通）
│   └── src/session.c         # < 300 行，直接调用 rknn_init / rknn_inputs_set
└── ascend/
    ├── include/argus_ascend.h
    └── src/session.c
```

约定：

- **拒绝重型 C++ 框架**：`native/` 仅承载必须直接穿透 Linux 内核或硬件私有驱动（如 DRM DMA-BUF、MPP 显存映射、昇腾内存池）的代码，不做业务状态管理，代码量以百行为限。
- 能用成熟开源 Rust crate（如 `coreml-rs`、`ort`）直接完成的，一律不写 native 垫片；专有硬件 SDK（MPP/RGA/RKNN、DVPP/AscendCL）统一通过 `native/` 极薄 C 垫片接入。
- 所有导出符号保持 `extern "C"`，严格遵守 C ABI 规范，见 [ffi-guidelines.md](./ffi-guidelines.md)。

---

## 测试文件放哪

| 测试类型 | 位置 | 说明 |
|---------|------|------|
| 单元测试 | 被测文件底部 `#[cfg(test)] mod tests` | 测私有逻辑 |
| 集成测试 | `crates/<crate>/tests/*.rs` | 只测 public API |
| 需要真机 NPU 的测试 | `#[ignore]` 标注 + 单独 feature | 开发机 `cargo test` 默认跳过 |
| 测试数据 | `crates/<crate>/tests/fixtures/` | 小样本，大模型文件不入 git |

**规则**：任何需要真实 NPU/摄像头的测试必须 `#[ignore]`，保证 `cargo test` 在开发机上能全绿通过。

---

## 待验证事项

- [ ] 跨平台推理运行时（`ort`、`coreml-rs`）的构建依赖与交叉编译环境
- [ ] 极薄硬件垫片（`native/`）与平台硬件库（`rknnrt`、`mpp`、`asyn_rga`）的静态/动态链接配置
- [ ] 前端构建产物内嵌（`rust-embed`）在开发模式（热重载外挂代理）与生产模式（单二进制内嵌）的无缝切换
