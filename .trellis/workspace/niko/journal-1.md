# Journal - niko (Part 1)

> AI development session journal
> Started: 2026-08-30

---



## Session 1: User Authentication and Password Management

**Date**: 2026-09-04
**Task**: User Authentication and Password Management
**Branch**: `feat/users`

### Summary

实现单管理员双轨安全初始化(OOBE向导与环境变量)、PBKDF2加盐密码哈希、JWT与毫秒级撤销时间戳、HTTP RFC 9110标准服务端i18n响应中间件、前端开箱向导、登录页面与修改密码模态框，并更新Trellis规范文档。

### Git Commits

| Hash | Message |
|------|---------|
| `afcbcd5` | (see git log) |
| `12a8699` | (see git log) |
| `a2c6507` | (see git log) |
| `bc1b718` | (see git log) |

### Status

[OK] **Completed**


## Session 2: Implement Camera Ingestion, Enhanced FLV Streaming, and MSE Live Console

**Date**: 2026-09-04
**Task**: Implement Camera Ingestion, Enhanced FLV Streaming, and MSE Live Console
**Branch**: `feat/users`

### Summary

Completed 09-04-camera-live-preview: implemented pure Rust RTSP ingestor with Exp-Golomb SPS parsing & RFC 7798 H.265/RFC 6184 H.264 depacketization, StreamHub multiplexing & keyframe cache, Enhanced FLV (hvc1) & HTTP/WS-FLV streaming endpoints, dual-track 3-state health probing, multi-vendor sub-stream rule deduction, Smart Hero + Bento Rail console with MSE hardware-decoded LivePlayer and 60fps Canvas 2D overlays, and aligned full codebase with two-axis review and quality gates.

### Git Commits

| Hash | Message |
|------|---------|
| `6f5071a` | (see git log) |
| `6f9fe10` | (see git log) |

### Status

[OK] **Completed**


## Session 3: PRD Grilling 深度对齐与任务分层编排

**Date**: 2026-09-04
**Task**: PRD Grilling 深度对齐与任务分层编排
**Branch**: `dev`

### Summary

通过 Grilling 会话完成 PRD 全面审查与决策收敛，重写 prd/prd-v1.0.md 并创建 Master 主任务与 4 个子任务

### Main Changes

- 全面重写 prd/prd-v1.0.md，锁定 Enhanced FLV、C ABI 算法包生态、两级感知/规则解耦、双流高清抓拍与证据三支柱
- 创建 Trellis Master 任务 09-04-ai-pipeline-evidence-system 及 4 个子任务并补齐 PRD 验收标准

### Git Commits

| Hash | Message |
|------|---------|
| `986379a` | (see git log) |

### Status

[OK] **Completed**

### Next Steps

- 启动子任务 09-04-c-abi-algo-sandbox 进行 C ABI 虚表映射与沙箱加载器开发



## Session 4: 接入 Retina 工业级 RTSP 接入内核

**Date**: 2026-09-05
**Task**: 接入 Retina 工业级 RTSP 接入内核
**Branch**: `dev`

### Summary

引入纯 Rust 工业级 RTSP 客户端库 retina (v0.4.20)，封装 RetinaIngestor 并打通 StreamHub，实现对真实安防摄像头非标 SDP 容错、TCP/UDP 传输策略支持、Annex B 零拷贝直通与逆向锚点脏 URL 清洗

### Main Changes

- 在 crates/media/Cargo.toml 引入 retina = 0.4.20 与 futures = 0.3
- 实现 crates/media/src/retina_ingest.rs，包含 URL 凭据解耦、TransportPolicy 映射、H.264/H.265 自动感知、Annex B (FrameFormat::SIMPLE) 零拷贝封装、单调时间戳看门狗与 Tokio 异步取消支持
- 在 crates/media/src/stream_hub.rs 将拉流内核挂载为 RetinaIngestor
- 重构 crates/media/src/probe.rs，将 StreamProber 底层统一迁移至 retina::client::Session::describe，彻底移除手写 TCP/Digest 握手逻辑
- 实现工业级逆向锚点 RTSP URL 解析与清洗器 parse_and_clean_rtsp_url，攻克密码包含保留字符 (@, :, #, ?) 导致 Url::parse 崩溃截断的问题
- 在 sanitize_rtsp_url_and_credentials、mask_rtsp_url 和 canonicalize_rtsp_url 中全链路接入逆向锚点清洗

### Git Commits

| Hash | Message |
|------|---------|
| `0021c9a` | (see git log) |
| `27fb8e1` | (see git log) |

### Testing

- [OK] 新增针对 URL 凭证清洗、特殊保留字符密码提取与传输策略映射的单元测试
- [OK] 全库 92 项单元测试及 clippy 门禁 100% 通过

### Status

[OK] **Completed**
