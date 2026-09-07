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


## Session 5: 动态实时流布防工作台、证据中心与多格式算法包归档支持

**Date**: 2026-09-05
**Task**: 动态实时流布防工作台、证据中心与多格式算法包归档支持
**Branch**: `dev`

### Summary

完成 LiveRulesStudio 动态实时流矢量布防工作台与等比归一化坐标计算；交付证据中心三重视图（违规告警与高清大图灯箱、行迹抓拍瀑布流、人脸/车牌识别对账左右对比）；升级算法包支持 .zip、.tar.gz 和 .tar 归档格式及沙箱自检；完成跨层代码精简重构与全链路质量门禁。

### Git Commits

| Hash | Message |
|------|---------|
| `1a9ea15` | (see git log) |

### Status

[OK] **Completed**

## Session 6: 攻克 B 帧时序颠倒与自适应时间戳重铸优化

**Date**: 2026-09-05
**Task**: B 帧时序与 CompositionTime 自适应纠偏优化
**Branch**: `dev`

### Summary

针对真实安防摄像头开启 B 帧（High Profile / Main Profile）导致 PTS 乱序到达、浏览器画面倒退与抽搐的顽疾，在 `crates/media/src/flv.rs` 实现了 `BFrameTimeManager` 自适应时间戳重铸与 CompositionTime 自动生成引擎，实现 0 用户配置下的端到端无感自愈播放。

### Main Changes

- 在 `crates/media/src/flv.rs` 实现 `BFrameTimeManager`，支持自动帧间隔探测与 B 帧乱序回跳感知；
- 在 `FlvMuxer` 增加 `packet_to_flv_tag_with_dts_cts`，将 `CompositionTime` (CTS = PTS - DTS) 精准编码为 3 字节写入 H.264 与 H.265 (Enhanced FLV) Video Tag Body；
- 采用自适应双模机制：无 B 帧流保持 0ms 额外延迟极速直通；一旦检测到 B 帧，自适应平滑推导严格单调递增的 DTS 并注入稳定时延偏置，彻底消除负 CTS 与时间戳回跳；
- 新增单元测试 `test_bframe_time_manager_without_b_frames`、`test_bframe_time_manager_with_b_frames` 与 `test_flv_video_tag_with_composition_time`，全库 90 项单元测试及 clippy 全绿通过。

### Status

[OK] **Completed**


## Session 6: 设备管理解耦、任务全生命周期管理与矢量布防工程化重构

**Date**: 2026-09-05
**Task**: 设备管理解耦、任务全生命周期管理与矢量布防工程化重构
**Branch**: `dev`

### Summary

完成设备管理（CamerasPage）与 AI 任务编排的独立解耦；后端交付 DELETE /api/v1/tasks/{camera_id} 接口与 TaskRepo 级联清理；LiveRulesStudio 彻底模块化拆解；接入 Framer Motion 工业级物理缓动与中英繁全量国际化。

### Main Changes

- 拆分并交付独立设备管理页 CamerasPage 与 CameraModal/DeleteCameraModal 模态框
- 后端实现 DELETE /api/v1/tasks/{camera_id} 与 TaskRepo::delete_by_camera_id，打通任务全链路销毁与审计日志落库
- LiveRulesStudio 深度解耦为 AlgoSandboxDrawer、AlgoSettingsSidebar、RuleInspectorSidebar、CreateTaskModal、DeleteTaskModal 等模块
- 接入 motionTokens 物理缓动体系，全面清理 Emoji 字符，补全中英繁全量国际化
- 初始化 Linux 边缘异构 VPU 解码器扩展（Rockchip MPP / 华为昇腾 DVPP）架构设计与规划工件

### Git Commits

| Hash | Message |
|------|---------|
| `1e1983e` | (see git log) |
| `43a9889` | (see git log) |

### Testing

- [OK] Rust 后端 workspace 108 项测试全绿，Clippy 0 警告
- [OK] Web 前端 25 项测试全绿，Prettier、ESLint、TypeScript 类型检查与生产构建 100% 通过

### Status

[OK] **Completed**

### Next Steps

- 推进 Linux 异构 VPU 解码器扩展任务实施：实现 Rockchip MPP 与华为昇腾 DVPP 解码器扩展


## Session 7: 实现子码流驱动泵与专用常驻推理线程架构

**Date**: 2026-09-05
**Task**: 实现子码流驱动泵与专用常驻推理线程架构
**Branch**: `dev`

### Summary

构建专用常驻推理线程（InferenceWorker）与子码流驱动泵（SubStreamAnalysisPump），打通视频解码至规则引擎与靶向高清抓拍的全链路闭环，并实现 GOP 语义感知防花屏队列及无锁单槽 Drop-Oldest 背压防护

### Git Commits

| Hash | Message |
|------|---------|
| `19a1e45` | (see git log) |

### Status

[OK] **Completed**


## Session 8: 算法包管理系统级资源接入与控制台落地

**Date**: 2026-09-05
**Task**: 算法包管理系统级资源接入与控制台落地
**Branch**: `dev`

### Summary

实现算法包与算法实例数据库持久化 Schema (V4)、启动自愈扫描与装载、子码流推理 Worker 零中断原子热重载、/api/v1/algorithms 与 /api/v1/tasks/instances RESTful API 端点、一级算法仓库 Web 界面（指标卡、卡片矩阵、版本抽屉、Schema 预览与 7 步沙箱自检上传），完成全栈质量门禁与修复验证。

### Git Commits

| Hash | Message |
|------|---------|
| `8d76782` | (see git log) |

### Status

[OK] **Completed**


## Session 9: 实现纯 Rust 算法包开发套件 algo-sdk 与硬件预处理测试脚手架

**Date**: 2026-09-06
**Task**: 实现纯 Rust 算法包开发套件 algo-sdk 与硬件预处理测试脚手架
**Branch**: `dev`

### Summary

完成 crates/algo-sdk 轻量独立套件开发，导出 export_algo! 宏及 C ABI 虚拟方法表；实现 SafeFrame 零拷贝内存布局与 Stride 校验；封装 AppleCvEngine / CpuCvEngine 硬件预处理及 Letterbox 坐标反算；提供高效 NMS、零分配 ResultEmitter 及多核心轮询调度；构建 MockFrameBuilder 测试脚手架并通过全套单元与集成测试。

### Git Commits

| Hash | Message |
|------|---------|
| `2a771ab` | (see git log) |

### Status

[OK] **Completed**


## Session 10: Fix configurable algorithm package upload limits

**Date**: 2026-09-06
**Task**: Fix configurable algorithm package upload limits
**Branch**: `dev`

### Summary

Fixed configurable algorithm package upload limits end to end: centralized and validated MB configuration with both environment variable forms, scoped large request bodies to upload routes, streamed multipart data to temporary files with bounded processing concurrency and cancellation-safe cleanup, improved frontend plain-text upload error handling, and added regression coverage. Verified Rust workspace tests, Clippy, formatting, Web lint, typecheck, tests, and build.

### Git Commits

| Hash | Message |
|------|---------|
| `21b1f8eecb7537b920e1b15a1087f8e353dd2aa2` | (see git log) |

### Status

[OK] **Completed**


## Session 11: 实现系统设置模块与全功能控制台

**Date**: 2026-09-06
**Task**: 实现系统设置模块与全功能控制台
**Branch**: `dev`

### Summary

完成系统设置模块全链路研发：实现系统概览（POSIX statvfs、macOS Mach CPU ticks与运行时长）、网络网卡枚举与IP配置、多级存储保留策略与热更新配置、主机对时NTP管理与安全手动调时；前端交付包含概览仪表盘、网络设置、存储配额及账号安全在内的完整设置页，通过三语国际化与全量门禁。

### Git Commits

| Hash | Message |
|------|---------|
| `7f6fc10` | (see git log) |

### Status

[OK] **Completed**


## Session 12: Axum 审计日志中间件统一拦截改造

**Date**: 2026-09-07
**Task**: Axum 审计日志中间件统一拦截改造
**Branch**: `dev`

### Summary

统一将写操作审计日志下沉至 Axum 中间件层（AuditLogLayer / AuditLogService），自动提取真实客户端 IP、推导审计模块与动作、安全截断请求载荷；移除各路由手工侵入式埋点；前端操作日志页面接入真实接口与分页，全量门禁通过。

### Main Changes

- 实现 AuditLogLayer / AuditLogService 统一拦截 POST/PUT/PATCH/DELETE 等受保护写操作并异步记录审计日志
- 精准提取 X-Forwarded-For、X-Real-IP 及 SocketAddr 客户端 IP，并按路径前缀推导模块与动作
- 安全捕获并截断请求载荷（上限 2048 字符），自动旁路 multipart 二进制大文件流
- 清理各领域路由手工 OplogRepo 调用，并在 AuthUser 提取器中支持 Extensions 注入优化
- 前端操作日志页面接入 live API，支持模块筛选、偏移分页、日志去重与国际化错误降级

### Git Commits

| Hash | Message |
|------|---------|
| `55c5b48` | (see git log) |

### Testing

- [OK] cargo test --workspace & cargo clippy --all-targets -- -D warnings & cargo fmt --all -- --check 通过
- [OK] web pnpm format / lint / typecheck / test / build 全量门禁通过

### Status

[OK] **Completed**
