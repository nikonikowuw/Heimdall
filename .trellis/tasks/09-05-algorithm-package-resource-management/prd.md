# PRD: 算法包管理系统级资源接入 (Algorithm Package Resource Management)

## Goal

将算法包（Algorithm Package）正式接入为系统的一等公民（First-Class System Resource），对齐 `/Users/niko/dev/go/argus` 的成熟工业级标准设计：
1. 建立以本地 SQLite 数据库为唯一事实源、磁盘管理二进制编译资产的双级实体模型（`algorithms` + `algorithm_versions`）；
2. 引入 `algorithm_instances` 独立多实例表，支持单摄像头并发挂载多算法实例；
3. 支持系统启动时多平台架构内置算法的动态自愈扫描（Reconciliation & Boot Seeding）；
4. 提供规范的 `/api/v1/algorithms` RESTful CRUD 接口与单进程优雅热重载（零停机、不断流）；
5. 在前端 Web 控制台开辟一级「算法仓库」界面，将任务布防工作室彻底解耦为轻量消费方。

---

## Background & Problem Statement

当前系统的算法包管理仅停留在物理目录（`algo-packages/{platform}/{id}`）和内存注册（`infer::AlgoRegistry`），存在以下缺陷：
- **无状态持久化**：数据库中没有任何算法与版本表，无法支持启用/禁用、多版本并存和历史回滚；
- **任务绑定缺失**：`analysis_tasks` 表未包含 `algorithm_id`，无法记录摄像头跑的是哪个模型，无法多模型并发；
- **缺乏使用中防误删保护**：无法感知某个算法是否正在被任务调用；
- **前端缺少一级入口**：算法上传与校验逻辑挤在布防工作室的抽屉里，无法全局掌控边缘盒子上的算力与算法资产。

---

## Detailed Requirements

### R1. 数据模型与持久化体系 (Data Model & Schema)
- **R1.1 新增 `algorithms` 算法主表**：记录算法元数据（`id`, `algorithm_id`, `name`, `algorithm_type`, `alarm_type_id`, `active_version`, `description`, `is_builtin`, `created_at`, `updated_at`）。
- **R1.2 新增 `algorithm_versions` 版本资产表**：记录版本明细（`id`, `algorithm_id`, `version`, `platform_id`, `min_adapter_version`, `package_root`, `fps_tiers`, `config_schema`, `manifest_raw`, `package_size_bytes`, `is_active`, `is_builtin`, `created_at`, `updated_at`），建立 `(algorithm_id, version, platform_id)` 唯一索引。
- **R1.3 新增 `algorithm_instances` 算法实例表**：挂载在 `camera_id` 下（`instance_id`, `camera_id`, `algorithm_id`, `analysis_fps`, `params_json`, `rules_json`, `motion_gate_json`, `enabled`, `actual_status`, `status_message`），支持单摄像头并发挂载多个独立算法。
- **R1.4 物理硬删除与事务原子级联**：
  - 卸载算法版本时，严格校验 `is_builtin`（如果是内置算法则直接拒绝）以及是否有活跃实例在用（`CountActiveInstances == 0`，否则拒绝并提示 `ALGO_IN_USE`）；
  - 校验通过后，在单一事务内删除 DB 记录并同步删除磁盘 `var/packages/{algorithm_id}/{version}` 目录，杜绝孤儿文件。

### R2. 冷启动自举与多平台自愈 (Boot Reconciliation & Seeding)
- **R2.1 启动自动探测**：系统启动时读取宿主硬件架构（macOS CoreML / Linux RKNN / Ascend CANN），扫描内置预置目录（`algo-packages/{platform}/` 或预装目录）；
- **R2.2 自动入库与沙箱自检**：若 DB 中缺少对应记录，自动运行 7 步安全沙箱物理自检，入库并标记为 `is_builtin = true, is_active = true`；
- **R2.3 DB 驱动运行时装载**：以 DB 中所有 `is_active = true` 的版本为事实源，通过 `AlgoPackage::load_and_verify` 加载进内存 `AlgoRegistry`；
- **R2.4 损坏容错**：若物理文件损坏或丢失，标记为 `corrupted` 并输出告警，不导致主进程崩溃。

### R3. RESTful API 契约与优雅热重载 (API & Hot-Reload)
- **R3.1 算法仓库 API**：
  - `GET /api/v1/algorithms`：支持分页、`keyword`、`algorithmType`、`isBuiltin` 过滤；
  - `GET /api/v1/algorithms/{id}`：返回单算法详情及关联版本树；
  - `GET /api/v1/algorithms/{id}/versions`：返回全部版本；
  - `POST /api/v1/algorithms/upload`：接收 `.tar.gz`/`.zip`/`.tar` 归档包，执行 7 步物理沙箱自检，落盘至 `var/packages/{algo_id}/{version}`，入库并激活；
  - `PUT /api/v1/algorithms/{id}/versions/{version}/activate`：切换当前生效版本，触发运行时热重载；
  - `DELETE /api/v1/algorithms/{id}/versions/{version}`：卸载版本，内置或使用中强阻断。
- **R3.2 实例管理 API**：
  - `GET /api/v1/tasks/instances?cameraId=...`：查询摄像头绑定的实例列表；
  - `POST /api/v1/tasks/instances`：创建算法实例；
  - `PUT /api/v1/tasks/instances/{instanceId}`：更新实例配置；
  - `PUT /api/v1/tasks/instances/{instanceId}/enabled`：启停实例；
  - `DELETE /api/v1/tasks/instances/{instanceId}`：删除实例。
- **R3.3 单进程优雅热重载**：在 DB 事务更新后，通知 `PipelineManager` 在两帧推理间隙原子切换 `AlgoInstance` 动态库句柄，视频流不断流、解码器不重启。

### R4. Web UI 独立「算法仓库」与布防解耦 (Web UI)
- **R4.1 一级导航增加「算法仓库」**：左侧工具栏新增 `NavTab = 'algorithms'`（Lucide `Cpu` 图标）；
- **R4.2 算法仓库全量看板**：
  - 顶部指标卡：算法总数、活跃版本数、内置/自定义模型统计、当前平台硬件架构；
  - 算法卡片矩阵：展示名称、`algorithm_id`、版本标签、支持平台、描述、查看版本/参数 Schema 按钮；
  - 版本管理抽屉（VersionsDrawer）：展示版本列表、FPS 档位、一键设为激活（Activate）、安全卸载（Uninstall）；
  - 上传弹窗（UploadModal）：拖拽上传归档包，流式展示 7 步沙箱验证步骤；
  - 参数 Schema 模态框（SchemaModal）：实时预览 `config.schema.json`；
- **R4.3 任务布防工作室解耦**：移除原先嵌在布防工作室里的全局上传与检验，仅作为轻量消费者选择算法实例。

---

## Acceptance Criteria

- [ ] SQLite 迁移脚本创建 `algorithms`、`algorithm_versions` 与 `algorithm_instances` 表无误；
- [ ] 启动自愈机制成功扫描并注册内置 `general_detection`，DB 记录 `is_builtin = 1, is_active = 1`；
- [ ] 算法上传接口支持 `.tar.gz` 与 `.zip`，沙箱 7 步自检通过后正确解压至 `var/packages/{algo_id}/{version}` 并落库；
- [ ] 正在被启用的算法实例所引用的版本，执行删除时返回 `ALGO_IN_USE` 且文件不被误删；
- [ ] 内置算法版本执行删除时返回 `BUILTIN_ALGO_PROTECTED` 并被拒绝；
- [ ] 激活新版本时，数据库状态与内存 `AlgoRegistry` 保持原子同步，运行中的 pipeline 优雅热替换；
- [ ] Web 控制台成功渲染一级「算法仓库」页面，具备指标统计、卡片列表、多版本抽屉、上传弹窗与 Schema 预览；
- [ ] 任务与布防工作室可选择并绑定已激活算法实例；
- [ ] 验证门禁全绿（`cargo fmt`, `cargo clippy`, `cargo test`, `pnpm typecheck`, `pnpm build`）。
