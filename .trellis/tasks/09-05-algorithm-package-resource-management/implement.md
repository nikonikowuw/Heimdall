# Implementation Plan: 算法包管理系统级资源接入 (Algorithm Package Resource Management)

## User Review Required

> [!IMPORTANT]
> - 本计划引入 SQLite 迁移 `V4__add_algorithms_and_instances.sql`，创建 `algorithms`、`algorithm_versions` 与 `algorithm_instances` 表；
> - 启动时会自动执行目录自愈对齐（Reconciliation），为已有本地内置算法包建立数据库记录；
> - Web 控制台新增一级导航「算法仓库」，全面支持算法卡片浏览、多版本管理、拖拽上传与 7 步沙箱自检。

---

## Proposed Changes

### Phase 1: 数据库迁移与数据访问层 (`crates/db`)
1. **新建迁移脚本 `crates/db/src/migration/migrations/V4__add_algorithms_and_instances.sql`**：
   - 创建 `algorithms`、`algorithm_versions`、`algorithm_instances` 表及索引。
2. **定义 SeaORM Entity 模型**：
   - `crates/db/src/entity/algorithm.rs`
   - `crates/db/src/entity/algorithm_version.rs`
   - `crates/db/src/entity/algorithm_instance.rs`
   - 在 `crates/db/src/entity/mod.rs` 中导出。
3. **实现仓储层 Repository**：
   - `crates/db/src/repository/algorithm.rs`：提供算法/版本的分页列表、单体详情、基于 `(algorithm_id, version, platform_id)` 的 Upsert、版本激活状态切换、防卸载检查（`count_active_instances`）及物理级联删除；
   - `crates/db/src/repository/algorithm_instance.rs`：提供基于 `camera_id` 的实例列表、增删改查及启停状态更新；
   - 在 `crates/db/src/repository/mod.rs` 与 `crates/db/src/lib.rs` 中暴露接口并编写单测。

### Phase 2: 推理层自愈对齐与运行时集成 (`crates/infer` & `crates/app`)
1. **新增启动自愈模块 `crates/infer/src/reconcile.rs`**：
   - 探测 `current_platform_id()`；
   - 扫描 `algo-packages/{platform}/` 与 `var/packages/`；
   - 对未入库算法执行沙箱自检，自动在 `algorithms` 与 `algorithm_versions` 中落库（标记 `is_builtin = true, is_active = true`）；
   - 从 DB 读取所有 `is_active = true` 的有效版本，加载至 `AlgoRegistry`；物理缺失文件标损不 Panic。
2. **更新 `crates/app/src/main.rs`**：
   - 在服务启动流程中，传入 `db_conn` 调用自愈对齐逻辑；
   - 保证冷启动或重置数据库后自动开箱就绪。

### Phase 3: RESTful API 与单进程优雅热重载 (`crates/api`)
1. **重构算法路由 `crates/api/src/routes/algo.rs`**：
   - 实现 `GET /api/v1/algorithms`（支持分页、搜索、分类过滤）；
   - 实现 `GET /api/v1/algorithms/{id}` 与 `GET /api/v1/algorithms/{id}/versions`；
   - 实现 `POST /api/v1/algorithms/upload`（流式归档解压、沙箱自检、落盘至 `var/packages`、落库并设为活跃）；
   - 实现 `PUT /api/v1/algorithms/{id}/versions/{version}/activate`（DB 事务切换 + 内部事件通知 `PipelineManager` 热重载）；
   - 实现 `DELETE /api/v1/algorithms/{id}/versions/{version}`（校验 `is_builtin` 与 `count_active_instances`，通过后事务删除行并清理物理目录）。
2. **实现实例路由 `crates/api/src/routes/task.rs`**：
   - 提供 `/api/v1/tasks/instances` 系列端点；
   - 关联操作审计日志 `db::OplogRepo`。
3. **路由注册与别名**：
   - 在 `crates/api/src/routes/mod.rs` 中注册 `/algorithms`（同时保留 `/algo` 别名兼容）。

### Phase 4: 前端 Web 控制台与解耦 (`web/`)
1. **类型定义与 API 客户端**：
   - 在 `web/src/types/index.ts` 扩展 `AlgorithmItem`、`AlgorithmVersionItem`、`AlgorithmInstanceDto`；
   - 在 `web/src/lib/api.ts` 新增 `algorithmApi` 与 `instanceApi`。
2. **构建算法仓库页面 `web/src/features/algorithms/`**：
   - `AlgorithmsPage.tsx`：主页面框架与状态管理；
   - `components/AlgoStatsHeader.tsx`：顶部四宫格统计卡；
   - `components/AlgoFilterBar.tsx`：搜索过滤栏；
   - `components/AlgoCardGrid.tsx` & `AlgoCard.tsx`：卡片网格；
   - `components/VersionsDrawer.tsx`：版本管理抽屉；
   - `components/UploadModal.tsx`：流式 7 步沙箱上传弹窗；
   - `components/SchemaModal.tsx`：参数 Schema 预览模态框。
3. **主导航与 i18n 接入**：
   - `web/src/app/layout.tsx`：增加 `algorithms` 标签（Lucide `Cpu` 图标）；
   - 补充 `web/src/i18n/locales/zh-CN/` 与 `en-US/` 对应多语言词条。
4. **布防工作室纯粹化**：
   - 调整 `web/src/features/tasks/components/LiveRulesStudio.tsx`，仅消费已激活算法实例。

---

## Verification Plan

### Automated Tests
1. **后端门禁**：
   - 运行数据库迁移与仓储单测：`cargo test -p db`
   - 运行推理层沙箱与自愈单测：`cargo test -p infer`
   - 运行 API 端点集成测试：`cargo test -p api`
   - 全工作区静态检查与构建：
     ```bash
     cargo fmt --all -- --check
     cargo clippy --all-targets -- -D warnings
     cargo test --workspace
     ```
2. **前端门禁**：
   - 前端代码格式、Lint、类型与构建检查：
     ```bash
     cd web
     pnpm format
     pnpm lint
     pnpm typecheck
     pnpm test
     pnpm build
     ```

### Manual Verification
- 访问 Web 控制台点击左侧「算法仓库」图标，验证算法卡片与指标展示；
- 点击「上传算法包」，上传 `.tar.gz` 格式包，验证 7 步沙箱自测步骤逐项点亮并自动刷新列表；
- 打开版本抽屉，测试版本激活与回滚；
- 尝试删除内置算法，验证弹出保护拦截提示；
- 在布防工作室中切换算法模型，验证参数联动与保存。
