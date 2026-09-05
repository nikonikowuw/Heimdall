# AGENTS.md

Heimdall 仓库级协作入口，适用于 AI 智能体与工程师。本文只保留跨层、不可妥协的规则；分层实现细节以 `.trellis/spec/` 为准。

## 规范入口

开始工作前：

1. 阅读 `.trellis/workflow.md`，确认当前 Trellis 阶段、活动任务和必须完成的步骤。
2. 修改代码前阅读 `.trellis/spec/guides/index.md`，再按目标层阅读对应 spec index 及其 `Pre-Development Checklist` 指向的文件。
3. 修改跨层数据或接口时，同时阅读跨层指南和相关 API/类型规范。
4. 发现 spec 与实际代码或硬件行为不一致时，先以可验证事实为准，记录差异；完成后通过 spec 更新流程补回约定。

权威关系如下：

- `AGENTS.md`：仓库级协作规则与不可违背的项目契约。
- `.trellis/spec/`：后端、前端和跨层的详细设计、编码及质量规范。
- `.trellis/tasks/`：当前任务的需求、设计、执行计划和验证记录。
- Trellis 托管区：仅通过 Trellis 更新机制维护，不手工修改。

## 工作方式

- **先理解再修改**：先搜索相关实现、调用方、测试和配置，明确最小行为缺口、真实归属位置、预计修改文件及明确不做的事情；不要凭假设补接口或重构邻近代码。
- **小步且可验证**：复用已有类型、解析器、错误和测试工具；新增依赖、抽象或配置必须有直接理由。每一行改动都应能对应需求或验证失败。
- **边界先定义**：跨 crate、FFI、数据库、HTTP/WebSocket 和 TypeScript 之间先写清输入、输出、错误、所有权、时间单位和校验责任，再实现。
- **保持用户改动**：开始前检查 `git status`；不覆盖、不回退、不删除未由本次任务产生的变更。除非用户明确要求，不执行破坏性 Git 操作，也不创建提交。
- **完成闭环**：实现后运行相关格式化、lint、类型检查、测试和构建；未运行或失败的检查必须在交付说明中明确列出。

## 项目契约

### 架构与依赖

- Rust workspace 是核心；职责依次收敛于 `types`、`db`、`media`、`infer`、`pipeline`、`api` 和 `app`。依赖保持单向，`media` 与 `infer` 通过 `types::FrameRef` 交汇且互不依赖。
- `pipeline` 负责管线编排、规则判定、后处理和跟踪；`api` 只处理协议适配和控制句柄；`app` 负责装配与生命周期。上层不得直接接触数据库 DSL、媒体驱动或平台 SDK。
- 平台差异只允许收敛在 `media` / `infer` 及其 FFI 实现内；上层禁止出现平台 feature 分支。`native/` 只保留必要的极薄 C/C++ 硬件垫片。
- **双流高能效分工**：系统默认仅对子码流进行常驻硬件解码与 NPU 推理；主码流仅以裸 NALU 写入内存环形缓冲区（`RingBuffer`），绝不常驻解码。仅在规则引擎触发告警时按需单帧解码主码流关键帧，产出高清全景大图与 10% 扩边特写抠图。
- 生产交付保持单二进制：前端 SPA 构建产物由 Rust 后端通过 `rust-embed` 提供；开发模式的外挂资源或 HMR 不得改变生产交付约束。

### 帧、并发与资源

- **三大路径严格区隔与零拷贝边界精确定义**：
  - 严禁笼统宣称“全链路零拷贝”。压缩输入码流存在一次网络/Host 内存向硬件解码器的 Host→Device DMA 复制（数据量极小）；
  - **常驻推理主路径 (`infer_fast_path`)**：生产媒体管线必须沿 `FrameRef` 传递平台原生 buffer（如 DMA-BUF、CVPixelBuffer 或 device memory），解码输出到推理输入严格维持纯设备侧零拷贝（VPU/DVPP -> RGA/VPC/AIPP -> RKNN/ACL），严禁在常驻推理流水线上发生任何 CPU 像素拷贝、CPU 色彩转换或 CPU 软解；
  - **低频证据生成路径 (`snapshot_readback_path`)**：作为显式特例，仅在告警触发或人工抓拍时按需单帧触发，允许将物理设备帧执行 Device-to-Host readback（如 `aclrtMemcpy(D2H)`、`mmap` cache sync）并交由 CPU 转为 RGB / JPEG 存盘；
  - **开发调试回退路径 (`debug_cpu_fallback_path`)**：仅在目标环境物理上确无硬件加速单元时作为保底，严禁伪装为硬件加速。
- 所有帧队列、事件缓冲、批处理和缓存必须有固定上限及明确丢弃/降级策略；帧路径禁止无界 channel。优先丢弃旧帧，不能用阻塞发送反压硬件解码。
- 任何平台 SDK、FFI 或超过约 1 ms 的 CPU 密集工作都不得直接运行在 Tokio worker 中。使用启动时确定数量的专用线程和有界通道；模型/硬件上下文应在线程内常驻。
- 不持锁执行 IO、FFI 或 `.await`；跨线程转移帧所有权，不复制帧。文件、fd、buffer pool 租约和模型句柄必须有 RAII 生命周期。
- **存储保护与原子级联淘汰**：写盘严禁无界累积。禁止使用高开销的 `du` 递归扫描磁盘，统一基于 `statvfs` 系统调用实时监控高低水位。存储超限时优先级联淘汰无告警的普通抓拍，严格在单一 SQLite 事务内同步销毁物理文件与 DB 记录（“图在案在，图销案销”），杜绝孤儿文件与死记录。

### API 与数据契约

- HTTP API 统一使用 `/api/v1`、JSON 和 `camelCase`；响应根信封固定为 `{ "code": 0, "message": "success", "data": T, "timestamp": ms }`，错误时 `data` 为 `null`。未经明确变更不得调整字段或根信封。
- 系统内部所有绝对时间戳均为 13 位 UTC Unix 毫秒整数；Rust 使用 `i64`，TypeScript 使用 `number`。相对时长必须带 `Ms` 后缀，如 `timeoutMs`、`latencyMs`。
- WebSocket 消息、视频 PTS 和检测结果使用同一帧时间基准；边界处统一解码和校验，消费方复用共享类型守卫/归一化逻辑，不在渲染代码中私自重定义 payload。
- **空间规则坐标归一化**：所有几何规则（多边形 ROI 区域入侵、Mask 屏蔽遮罩、折线 Line 越界绊线）的顶点坐标，在前后端传输与落库时必须严格归一化为 `[0.0, 1.0]` 浮点区间；布防规则配置必须在子码流动态实时流画面上叠加矢量交互层绘制，严禁静态死图标注。
- **业务证据三支柱与状态追踪**：业务数据严格划分为违规告警（Alarms，支持待处理/已核验状态流转）、行迹抓拍（Captures）与识别对账（Recognitions）。证据图片服务接口必须验证路径规范化（`canonicalize`），杜绝路径穿越。
- handler 只负责提取参数、调用领域服务/仓储和映射 DTO；业务判定、SQL 复杂查询、硬件调用和阻塞工作放在对应层。

### FFI 与安全边界

- `unsafe`、裸指针和平台 C 类型只允许集中在 FFI/sys 边界；每个 `unsafe` 块及 `unsafe impl` 都必须有准确的 `// SAFETY:` 说明，裸指针不得逃逸到安全层。
- C ABI 只暴露稳定的 `extern "C"`、不透明句柄和固定布局 POD；C++ 异常或 Rust panic 不得跨 FFI 边界传播。创建与销毁必须成对，并由 RAII 保证错误路径释放。
- FFI 结构体的 `size_of`、`align_of` 和关键偏移必须有双侧断言；DMA-BUF、stride、cache sync 和平台对齐约束按对应 media/FFI spec 执行。
- **算法包沙箱与归档安全**：算法包统一遵循标准 C ABI 虚拟函数表；上传解压原生支持 `.tar.gz` (`.tgz`)、`.tar` 与 `.zip` 格式自动识别，解压过程必须强校验剔除绝对路径与包含 `..` 的路径组件（严防 Tar/Zip-Slip 漏洞），经七步沙箱物理自检通过后方可加载至运行时。

### 前端运行时

- Zustand 只保存客户端 UI 状态及 WebSocket 连接状态/有界增量缓冲；普通服务端资源通过专用数据获取 hook 管理。选择器保持细粒度，返回对象/数组时使用 `useShallow`。
- 视频播放器必须与非相关状态隔离并保持稳定 props；高频检测框和实时流数据使用 `useRef`、`requestAnimationFrame`、Worker 或 Canvas，不用 React state 驱动高频 DOM 重排。
- **低噪监控体验与视觉规范**：日常监控无违规时不推高频噪点框，仅在告警触发时通过稀疏 WebSocket 事件弹出轻量卡片和声音提示。生产界面严格遵循工业级设计准则，**严禁使用 Emoji 表情符号**，所有图标统一采用 Lucide 矢量图标，所有可见文本必须接入 i18n 国际化。
- 事件缓冲有界，WebSocket 全局复用并指数退避重连；样式使用主题 token/CSS 变量，不硬编码颜色或提交调试日志。

## 验证门禁

只运行与本次变更相关的层；提交前在具备对应工程时完成完整门禁。格式化必须先于检查：

### Rust / Native

```bash
cargo fmt --all
if [ -d native ]; then
  find native -type f \( -name '*.c' -o -name '*.h' \) -exec clang-format -i {} +
fi
cargo fmt --all -- --check
cargo clippy --all-targets -- -D warnings
cargo test --workspace
```

有对应平台 SDK 的环境再额外运行 `cargo clippy --all-targets --all-features -- -D warnings` 及平台测试；硬件依赖测试必须显式标记为 `#[ignore]`，开发机 `cargo test` 仍须全绿。

### Web

```bash
cd web
pnpm format
pnpm lint          # eslint --max-warnings=0
pnpm typecheck     # tsc --noEmit
pnpm test          # vitest run
pnpm build
```

测试应优先覆盖纯逻辑、边界转换、资源清理和用户可观察行为；修复 bug 时先建立可复现的失败测试。禁止提交 `dbg!`、`println!`、`console.log`、`todo!()`、`@ts-ignore` 或被关闭的 lint 规则。

<!-- TRELLIS:START -->
# Trellis Instructions

These instructions are for AI assistants working in this project.

This project is managed by Trellis. The working knowledge you need lives under `.trellis/`:

- `.trellis/workflow.md` — development phases, when to create tasks, skill routing
- `.trellis/spec/` — package- and layer-scoped coding guidelines (read before writing code in a given layer)
- `.trellis/workspace/` — per-developer journals and session traces
- `.trellis/tasks/` — active and archived tasks (PRDs, research, jsonl context)

If a Trellis command is available on your platform (e.g. `/trellis:finish-work`, `/trellis:continue`), prefer it over manual steps. Not every platform exposes every command.

If you're using Codex or another agent-capable tool, additional project-scoped helpers may live in:

- `.agents/skills/` — reusable Trellis skills
- `.codex/agents/` — optional custom subagents

Managed by Trellis. Edits outside this block are preserved; edits inside may be overwritten by a future `trellis update`.

<!-- TRELLIS:END -->
