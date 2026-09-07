# Heimdall

> **工业级边缘端一体化 AI 视频分析与证据闭环系统**  
> *Industrial Edge AI Video Analytics & Evidence Loop System*

[![Rust](https://img.shields.io/badge/Rust-1.80%2B-orange.svg?logo=rust)](https://www.rust-lang.org/)
[![React](https://img.shields.io/badge/React-19.0%2B-61DAFB.svg?logo=react)](https://react.dev/)
[![License](https://img.shields.io/badge/License-MIT%20OR%20Apache--2.0-blue.svg)](LICENSE)
[![Platform](https://img.shields.io/badge/Platform-macOS%20%7C%20Linux%20(x86__64%20%2F%20aarch64)-black.svg)](https://github.com/)

---

## 📖 系统概览

**Heimdall**专为边缘嵌入式算力设备与无人值守监控场景打造，是一套基于纯 Rust 核心构建、集成现代化前端控制台的**单二进制自包含（All-in-One Binary）**智能视频分析系统。

系统针对边缘端计算资源与 DDR 内存总线受限的特点进行了全链路深度优化，集成了**工业级 RTSP 拉流与子码流智能推导**、**Enhanced FLV / MSE 低延迟实时流分发**、**双流环形高清证据抓拍**、**C ABI 算法包物理沙箱与热插拔**、**统一空间几何规则引擎**以及**动态矢量布防工作台**，实现了从视频接入、按需低功耗推理到原子级证据闭环交付的完整链路。

```
┌──────────────────────────────────────────────────────────────────────────────────┐
│                        Heimdall (Rust All-in-One Binary)                         │
│                                                                                  │
│  ┌────────────────────────┐  ┌────────────────────────────────────────────────┐  │
│  │  Embedded Web Console  │  │          High-Performance Rust Core            │  │
│  │  React 19 + TypeScript │  │  - Axum HTTP & Enhanced FLV (HTTP/WS) Stream   │  │
│  │  Tailwind CSS v4       │  │  - StreamHub Multiplexing (GOP Ring Buffer)    │  │
│  │  Live Rules Studio SVG │  │  - Pipeline: ByteTrack + Geometry Rules Engine │  │
│  │  rust-embed (Single)   │  │  - C ABI Dynamic Algo-Packages Sandbox Host    │  │
│  │                        │  │  - SQLite (WAL Mode) + Statvfs Atomic Eviction │  │
│  └────────────────────────┘  └────────────────────────────────────────────────┘  │
│                                                                                  │
│  ┌────────────────────────────────────────────────────────────────────────────┐  │
│  │  Hardware Acceleration Layer (Zero-Copy FrameRef: CVPixelBuffer / DMA-BUF) │  │
│  │  - macOS Apple Silicon: VideoToolbox + Core ML / ANE                       │  │
│  │  - Rockchip Linux: MPP Decode + RGA 2D Crop + RKNN NPU                     │  │
│  │  - Huawei Ascend Linux: DVPP VDEC + VPC 2D Crop + CANN ACL                 │  │
│  └────────────────────────────────────────────────────────────────────────────┘  │
└──────────────────────────────────────────────────────────────────────────────────┘
```

---

## ⚡ 核心架构与技术亮点

### 1. 边缘端高能效解码与主/子双流环形抓拍 (Dual-Stream Pipeline)
- **子码流低功耗常驻推理**：默认仅对低分辨率子码流（640×360 / D1）进行硬件解码并送入 NPU / ANE 进行实时分析与目标跟踪，整机解码负载降低 70% 以上。
- **主码流内存零解码环形缓冲**：主码流（1080P / 4K）以裸 NALU 形式写入内存环形缓冲区（`RingBuffer`），无需常驻解码。
- **告警瞬间按需解码关键帧**：当规则引擎触发越界或入侵告警时，精准定位主码流对应时标的 GOP 关键帧进行即时单帧硬解，产出 4K 超清证据大图与 10% 扩边特写抠图（Crop Image），兼顾超低功耗与高清证据存证。

### 2. C ABI 算法包沙箱与多格式热插拔 (Algorithm Sandbox Hub)
- **标准 C ABI 接口解耦**：算法包遵循稳定 C ABI 虚拟函数表抽象，算法开发者只需交付极轻量感知推理动态库（`.dylib` / `.so`），无需绑定庞大系统依赖。
- **七步物理隔离沙箱自检**：算法包安装时严格执行目录防穿透、SHA-256 安全指纹、平台拓扑匹配、Schema 参数校验、隔离子进程自测、C ABI 导出符号核验及真机推理内存复核。
- **多归档格式自适应**：原生支持 `.tar.gz` (`.tgz`)、`.tar` 与 `.zip` 算法包上传与自动魔数识别，严格防御 Tar/Zip-Slip 路径逃逸漏洞。

### 3. 统一空间几何规则与防抖跟踪引擎 (Rules & Tracker Engine)
- **空间几何判定**：支持多边形感兴趣区（ROI 区域入侵）、屏蔽遮罩（Mask 区域过滤）与折线（Line 跨越绊线，支持双向及单向 $A \to B$ / $B \to A$ 方向判定）。
- **轻量级航迹跟踪与防抖**：基于 IOU 目标空间连续性跟踪，为运动目标分配全局唯一 `trackId`，内置状态机与 5 秒冷却周期，彻底消除同目标频繁抖动重复报案。
- **运动门控（Motion Gate）**：支持像素级运动门控检测，无目标静止时自动挂起解码与推理，显著延长边缘设备电池与寿命。

### 4. 证据中心三支柱与物理水位原子淘汰 (Evidence Center & Atomic Eviction)
- **业务证据三支柱**：
  - 📸 **行迹抓拍（Captures）**：高频全天候运动目标抓拍瀑布流，记录 `trackId` 与质量评分；
  - 🚨 **违规告警（Alarms）**：越界与入侵违法行为存证，全景大图 + 目标特写 + 状态追踪（待处理/已核验）；
  - 👤 **识别对账（Recognitions）**：「现场抓拍特写 | 相似度匹配 | 底库登记照」左右直观对账排版。
- **原子级级联淘汰保护**：摒弃低效的 `du` 递归扫描，基于 Linux `statvfs` 内核系统调用实时监测磁盘高低水位。存储超限时，优先淘汰无告警的普通抓拍，严格在单一 SQLite 事务内同步销毁物理文件与数据库行（“图在案在，图销案销”）。

### 5. 动态实时流布防工作台与单二进制交付 (Live Rules Studio & WebUI)
- **动态矢量布防工作台**：告别传统静态死图标注，工作台在子码流实时流画面上覆盖透明 SVG 交互层，支持边看实时视频边绘制 ROI/Mask/Line，支持顶点磁吸、拖拽与方向调整，坐标自动无损归一化至 `[0.0, 1.0]` 相对坐标系。
- **纯正 Industrial 工业级设计**：无杂乱霓虹或 Emoji 装饰，采用 Clean-Room 亮色与 Dark Industrial 深色双主题，全量中简、中繁与英文 i18n 国际化。
- **All-in-One 单二进制交付**：前端构建产物由 Rust 后端通过 `rust-embed` 编译内嵌，单命令交付运行，单端口（默认 8080）开箱即用。

---

## 🧱 项目模块与目录结构

```
Heimdall/
├── crates/                         # Rust Workspace 核心组件库
│   ├── types/                      # 基础共享类型 (FrameRef, 空间规则, 编解码协议)
│   ├── db/                         # SQLite 存储层 (SeaORM + Refinery 增量迁移)
│   ├── media/                      # 媒体接入与流媒体网关 (RTSP Ingest, StreamHub, FLV)
│   ├── infer/                      # 推理引擎抽象与 C ABI 算法沙箱热加载
│   ├── pipeline/                   # 分析管线编排 (几何规则, ByteTrack, 双流抓拍, 淘汰机制)
│   ├── api/                        # Axum HTTP RESTful & WebSocket 接口服务与静态资源内嵌
│   └── app/                        # 宿主装配入口 (单二进制构建目标: heimdall)
├── web/                            # 现代化边缘监控前端控制台 (React 19 + TypeScript + Vite)
│   ├── src/
│   │   ├── features/
│   │   │   ├── live/               # 实时视频预览监控 (Hero+Rail, Bento, 告警浮层提示)
│   │   │   ├── alarms/             # 证据中心三重视图 (告警卡片/表格, 抓拍瀑布流, 识别对账)
│   │   │   └── tasks/              # 任务配置与 LiveRulesStudio 动态实时流布防工作台
│   │   ├── lib/api.ts              # 类型安全的通用 API 交互客户端
│   │   └── i18n/                   # 国际化语言包 (zh-CN, zh-TW, en)
├── algo-packages/                  # 本地各硬件平台算法包资产库 ({platform}/{algo_id})
└── docs/                           # 架构评估与技术规格文档
```

---

## 🚀 快速上手

### 环境准备

- **Rust**: 1.80+ (推荐安装最新稳定版 `rustup update stable`)
- **Node.js**: 20+ 及 **pnpm** (用于前端编译)
- **操作系统**: macOS (Apple Silicon M系列) 或 Linux (x86_64 / aarch64 ARM)

### 1. 源码编译 (单二进制打包)

```bash
# 1. 编译前端工程生成静态产物
cd web
pnpm install
pnpm build
cd ..

# 2. 编译 Rust 单二进制程序 (内嵌 web/dist)
cargo build --release --bin Heimdall

# 编译产物位于: target/release/Heimdall
```

### 2. 启动运行

```bash
# 启动服务 (默认监听 0.0.0.0:8080)
./target/release/Heimdall

# 或者通过 cargo 直接运行
cargo run --bin Heimdall
```

服务启动后，在浏览器访问控制台：
```text
http://localhost:8080
```
- **初始安装向导**：初次运行将引导设置系统管理员账号及初始凭据。
- **API 接口前缀**：`/api/v1`

---

## 🧪 验证门禁与质量检查

根据项目规范，提交代码前需运行质量门禁：

### 后端门禁 (Rust)
```bash
cargo fmt --all -- --check
cargo clippy --all-targets -- -D warnings
cargo test --workspace
```

### 前端门禁 (Web)
```bash
cd web
pnpm format
pnpm lint          # eslint --max-warnings=0
pnpm typecheck     # tsc --noEmit
pnpm test          # vitest run
pnpm build
```

---

## 🔌 API 契约与开发规范

- **通信协议**：RESTful API 统一使用 `/api/v1` 前缀；实时消息通过 WebSocket `/api/v1/ws/events` 传输。
- **根信封格式**：
  ```json
  {
    "code": 0,
    "message": "success",
    "data": { ... },
    "timestamp": 1725510000000
  }
  ```
- **时间规范**：绝对时间戳统一为 13 位 UTC Unix 毫秒整数（Rust `i64`，TypeScript `number`）；时长类字段带有 `Ms` 后缀。
- **空间规则归一化**：所有几何点坐标在前后端传递中均保持 `x, y ∈ [0.0, 1.0]`。

---

## 📄 许可证

本项目基于 [MIT](LICENSE) OR [Apache-2.0](LICENSE) 协议开源。
