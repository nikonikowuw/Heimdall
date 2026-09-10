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


## Session 13: 工业级网络服务配置

**Date**: 2026-09-07
**Task**: 工业级网络服务配置
**Branch**: `dev`

### Summary

构建工业级边缘网络服务架构：实现防失联看门狗（Commit-Confirm 60s 倒计时回滚）、掉电安全原子快照冷启动自愈、RFC 5227 地址冲突检测（ARP Probe）、多网口策略路由与自动跃点分配；前端支持试用倒计时横幅、冲突拦截弹窗与多网卡路由管理，全量门禁通过。

### Main Changes

- 实现 Commit-Confirm 事务与独立 60s 看门狗，超时未确认自动执行物理级原子回滚与冷启动自愈
- 实现基于 Raw Socket 的 RFC 5227 地址冲突检测（ARP Probe）及变更后的 Gratuitous ARP 广播
- 支持多网卡策略路由与度量值动态指派（管理网口 100，从属网口 500），杜绝网关冲突与路由漂移
- 模块化重构 NetworkService 并新增 ICMP Ping / DNS 诊断端点与 51000 系列系统错误码多语言国际化
- 前端构建试用倒计时横幅、地址冲突告警拦截 Modal 与网卡度量值配置，完整覆盖全生命周期交互

### Git Commits

| Hash | Message |
|------|---------|
| `7a38510` | (see git log) |

### Testing

- [OK] cargo test --workspace & cargo clippy --all-targets -- -D warnings & cargo fmt --all -- --check 通过
- [OK] web pnpm format / lint / typecheck / test / build 全量门禁通过

### Status

[OK] **Completed**


## Session 14: 实现 Rockchip RGA 硬件加速驱动与 CvEngine 并完成代码精简

**Date**: 2026-09-07
**Task**: 实现 Rockchip RGA 硬件加速驱动与 CvEngine 并完成代码精简
**Branch**: `dev`

### Summary

基于工业级 IoC 与有界缓冲池模式在 algo-sdk 中实现 Rockchip RGA 硬件加速预处理驱动与 CvEngine，支持动态加载 librga、Linux DMA-BUF 显存池复用与 DMA32/RGA 硬件约束防御，并完成了代码精简重构与单元测试验证。

### Git Commits

| Hash | Message |
|------|---------|
| `0d8e96d` | (see git log) |

### Status

[OK] **Completed**


## Session 15: RK3576 RKNN 通用目标检测算法包开发与硬件压测

**Date**: 2026-09-07
**Task**: RK3576 RKNN 通用目标检测算法包开发与硬件压测
**Branch**: `dev`

### Summary

完成基于 RK3576 平台的通用目标检测算法包（YOLOv8n）开发，打通 RGA 硬件预处理与 RKNN 双核 NPU 并行推理，实现 C ABI 插件封装与全流程硬件压测验证。

### Git Commits

| Hash | Message |
|------|---------|
| `9b92700` | (see git log) |

### Status

[OK] **Completed**


## Session 16: macOS arm64 人脸识别算法包 (EdgeFace) 实现与代码精简

**Date**: 2026-09-08
**Task**: macOS arm64 人脸识别算法包 (EdgeFace) 实现与代码精简
**Branch**: `dev`

### Summary

实现基于 Apple Silicon CoreML/ANE 的人脸检测 (YOLOv5n-face) 与特征提取 (EdgeFace-S) 算法包，支持 112x112 ArcFace 五点仿射对齐、质量门控、独立 C ABI 单帧特征提取符号与 run_local 本地评测工具；完成全模块代码结构精简与门禁验证

### Git Commits

| Hash | Message |
|------|---------|
| `0df3c1e` | (see git log) |

### Status

[OK] **Completed**


## Session 17: 优化系统概述模块 - 工业级硬件监控仪表盘

**Date**: 2026-09-08
**Task**: 优化系统概述模块 - 工业级硬件监控仪表盘
**Branch**: `dev`

### Summary

完成系统概述模块工业级升级与代码审查修复：重构多核CPU/内存/磁盘/网络/温度指标采集层并移至api crate消除架构反向依赖；对接RKNN与Ascend多核NPU监控并修复利用率计算与双重读取；修复macOS Mach内存泄漏与前端轮询竞态；前端新增Top 5进程排行与网络实时走势曲线，全链路门禁测试全绿并通过。

### Git Commits

| Hash | Message |
|------|---------|
| `23b7ce5` | (see git log) |

### Status

[OK] **Completed**


## Session 18: API 层架构重构：AppState 瘦身 + 路由文件拆分

**Date**: 2026-09-08
**Task**: API 层架构重构：AppState 瘦身 + 路由文件拆分
**Branch**: `dev`

### Summary

完成 API 层架构重构。将 CameraProbeService 从 AppState 解耦为独立后台服务并提供静态广播方法；sync_auth_state 下沉至 routes::auth 使 AppState 退化为纯同步数据结构；algo.rs 585行拆分为 algo/{archive, dto, lifecycle, upload_service} 四模块（路由降至 117 行）；system.rs 拆为 system/{mod, overview, network, storage, time} 子路由（mod.rs 仅 24 行）；evidence 图片服务补齐 camera_id/client_ip 访问日志；WebSocket 广播 Lagged/Closed 错误区分处理。全量门禁通过：cargo fmt、clippy、277 个测试全绿。

### Git Commits

| Hash | Message |
|------|---------|
| `5cff35b` | (see git log) |

### Status

[OK] **Completed**


## Session 19: RK3576 人脸识别算法包实现

**Date**: 2026-09-09
**Task**: RK3576 人脸识别算法包实现
**Branch**: `dev`

### Summary

实现 RK3576 人脸识别算法包，包括 YOLOv8n-face 检测和 EdgeFace-xs 嵌入模型，简化 C ABI 接口，重新设计 worker 架构并增强检测稳定性

### Git Commits

| Hash | Message |
|------|---------|
| `0f69feb` | (see git log) |
| `3ad2ac3` | (see git log) |

### Status

[OK] **Completed**


## Session 20: Implement task algorithm binding contract and instance consistency

**Date**: 2026-09-09
**Task**: Implement task algorithm binding contract and instance consistency
**Branch**: `dev`

### Summary

Defined explicit i32 mapping for TaskStatus and added algorithm binding fields to AnalysisTask. Added V5 migration and implemented atomic transactional sync between analysis_tasks and algorithm_instances in TaskRepo. Updated API DTOs and handlers with validation. Updated backend database guidelines spec.

### Git Commits

| Hash | Message |
|------|---------|
| `62eb169` | (see git log) |
| `d0bae9b` | (see git log) |

### Status

[OK] **Completed**


## Session 21: 分析任务运行时协调与双流资源生命周期实现及质量复核

**Date**: 2026-09-09
**Task**: 分析任务运行时协调与双流资源生命周期实现及质量复核
**Branch**: `dev`

### Summary

完成 TaskRuntimeCoordinator 摄像机级运行时编排、双流 RingBuffer 与 Pump 生命周期治理、StreamHub AI 保活及阻塞式硬件解码器析构保护，并通过全量门禁与 Code Review 验证

### Main Changes

- 实现 TaskRuntimeCoordinator，统一双流 RingBuffer、分析泵、解码器与 InferenceWorker 启动、停止与回滚
- 引入 BlockingDecoder 保证硬件解码器析构脱离 Tokio Worker 并在专用 blocking 池执行
- 解决 PipelineManager 内部锁层级死锁，并增加广播 Lagged 后的关键帧门控防止残缺 GOP
- 改进 StreamHub AI 会话保持计数，区隔人工使能与 Pump 引用

### Git Commits

| Hash | Message |
|------|---------|
| `d0eb1c7` | (see git log) |
| `2932302` | (see git log) |
| `5a0d4c7` | (see git log) |

### Testing

- [OK] cargo test -p pipeline (33 unit + 12 coordinator + 7 snapshot + 1 rules + 3 pump e2e 全部通过)
- [OK] cargo test --workspace 全部通过
- [OK] cargo clippy -p pipeline --all-targets -- -D warnings 零告警通过
- [OK] cargo fmt --all -- --check 格式检查通过

### Status

[OK] **Completed**

### Next Steps

- 推进 09-08-task-api-pipeline-orchestration 任务编排与状态同步


## Session 22: 实现任务API启停编排与状态双写同步

**Date**: 2026-09-10
**Task**: 实现任务API启停编排与状态双写同步
**Branch**: `dev`

### Summary

接入 TaskRuntimeService 协调层，实现任务启用/停用/配置重载、摄像头与任务级联析构编排、默认算法按需解析及状态持久化双写与回滚机制，补充幂等性与优雅停机覆盖。

### Git Commits

| Hash | Message |
|------|---------|
| `453d895` | (see git log) |
| `d7a3207` | (see git log) |

### Status

[OK] **Completed**


## Session 23: 多算法绑定、驱动泵并发调度、冷启动恢复与定向热替换

**Date**: 2026-09-10
**Task**: 多算法绑定、驱动泵并发调度、冷启动恢复与定向热替换
**Branch**: `dev`

### Summary

完成任务多算法实例绑定与 V6 数据库迁移，实现单摄像头多算法并发调度与目标跟踪隔离，重构 Drop-Oldest 丢帧指标并修复无限递归；完善冷启动任务恢复与实例生命周期管线联动；前端支持任务创建算法选择与卡片运行态回显；通过双轴审查修复 10 项缺陷，工作区通过全部验证门禁。

### Git Commits

| Hash | Message |
|------|---------|
| `81214ac` | (see git log) |

### Status

[OK] **Completed**
