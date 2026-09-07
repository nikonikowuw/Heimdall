# Technical Design: 算法包管理系统级资源接入 (Algorithm Package Resource Management)

## 1. 系统架构拓扑 (System Topology)

```
┌─────────────────────────────────────────────────────────────────────────────┐
│                             Web 前端 (React 18 + TS + Tailwind)             │
│  ┌─────────────────────────────────┐   ┌──────────────────────────────────┐ │
│  │ 独立一级导航:「算法仓库」        │   │ 「任务与布防工作室」             │ │
│  │ - 算力/算法资产概览看板         │   │ - 挂载/选择 algorithm_instance    │ │
│  │ - 上传归档包 + 7步沙箱校验      │   │ - 几何规则 (ROI/Mask/Line)       │ │
│  │ - 版本抽屉/热激活/安全卸载/Schema│  │ - 门控阈值与模型专属参数配置     │ │
│  └────────────────┬────────────────┘   └──────────────────┬───────────────┘ │
└───────────────────┼───────────────────────────────────────┼─────────────────┘
                    │ HTTP /api/v1/algorithms               │ HTTP /api/v1/tasks/instances
                    ▼                                       ▼
┌─────────────────────────────────────────────────────────────────────────────┐
│                            crates/api (Axum 控制层)                         │
│  - 算法包生命周期路由: list, get, list_versions, upload, activate, uninstall│
│  - 算法实例路由: list_instances, create, update, set_enabled, delete       │
│  - 操作日志: 统一写入 db::OplogRepo                                         │
└───────────────────┬───────────────────────────────────────┬─────────────────┘
                    │                                       │
                    ▼                                       ▼
┌───────────────────────────────────────┐  ┌──────────────────────────────────┐
│      crates/db (SeaORM + SQLite)      │  │ crates/pipeline (分析管线编排)   │
│  - algorithms (主表)                  │  │  - CameraPipelineContext         │
│  - algorithm_versions (版本/平台资产) │  │  - process_detections -> tracker │
│  - algorithm_instances (相机多实例)   │  │  - 帧间安全点平滑热重载实例句柄  │
│  - 物理清理: 事务内原子清理 DB 与磁盘 │  │    (无需重启推流与解码器)        │
└───────────────────┬───────────────────┘  └────────────────▲─────────────────┘
                    │ 启动自愈扫描                          │ 获取已加载 AlgoPackage
                    ▼                                       │
┌───────────────────────────────────────────────────────────┴─────────────────┐
│                     crates/infer (推理运行时与安全沙箱)                     │
│  - Boot Reconciliation: 启动检测本机平台架构，自动扫描预置包自举入库         │
│  - AlgoRegistry: 内存常驻已激活版本句柄 (RwLock<HashMap<String, Arc>>)      │
│  - AlgoSandbox: 7步安全物理沙箱自检 (路径防穿透/SHA256/ABI符号/前向推理自测) │
│  - 规范存储路径: var/packages/{algorithm_id}/{version}                      │
└─────────────────────────────────────────────────────────────────────────────┘
```

---

## 2. 数据库建模 (Database Schema: V4 Migration)

采用 SQLite 原生 DDL（通过 Refinery 统一版本控制，由 SeaORM 生成 ActiveModel）：

### 2.1 算法主表 `algorithms`
```sql
CREATE TABLE IF NOT EXISTS algorithms (
    id             INTEGER PRIMARY KEY AUTOINCREMENT,
    algorithm_id   TEXT NOT NULL UNIQUE,
    name           TEXT NOT NULL,
    algorithm_type TEXT NOT NULL,
    alarm_type_id  TEXT NOT NULL DEFAULT '',
    active_version TEXT NOT NULL DEFAULT '',
    description    TEXT NOT NULL DEFAULT '',
    is_builtin     INTEGER NOT NULL DEFAULT 0,
    created_at     DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_at     DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP
);
CREATE INDEX IF NOT EXISTS idx_algorithms_type ON algorithms(algorithm_type);
```

### 2.2 算法版本表 `algorithm_versions`
```sql
CREATE TABLE IF NOT EXISTS algorithm_versions (
    id                  INTEGER PRIMARY KEY AUTOINCREMENT,
    algorithm_id        TEXT NOT NULL,
    version             TEXT NOT NULL,
    platform_id         TEXT NOT NULL,
    min_adapter_version TEXT NOT NULL DEFAULT '',
    package_root        TEXT NOT NULL DEFAULT '',
    fps_tiers           TEXT NOT NULL DEFAULT '[]',
    config_schema       TEXT NOT NULL DEFAULT '{}',
    manifest_raw        TEXT NOT NULL DEFAULT '{}',
    package_size_bytes  INTEGER NOT NULL DEFAULT 0,
    is_active           INTEGER NOT NULL DEFAULT 0,
    is_builtin          INTEGER NOT NULL DEFAULT 0,
    created_at          DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_at          DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP,
    UNIQUE(algorithm_id, version, platform_id)
);
CREATE INDEX IF NOT EXISTS idx_algo_versions_algo_id ON algorithm_versions(algorithm_id);
```

### 2.3 算法实例表 `algorithm_instances`
```sql
CREATE TABLE IF NOT EXISTS algorithm_instances (
    id             INTEGER PRIMARY KEY AUTOINCREMENT,
    instance_id    TEXT NOT NULL UNIQUE,
    camera_id      TEXT NOT NULL,
    algorithm_id   TEXT NOT NULL,
    analysis_fps   INTEGER NOT NULL DEFAULT 0,
    params_json    TEXT NOT NULL DEFAULT '{}',
    rules_json     TEXT NOT NULL DEFAULT '[]',
    motion_gate_json TEXT NOT NULL DEFAULT '{}',
    enabled        INTEGER NOT NULL DEFAULT 0,
    actual_status  INTEGER NOT NULL DEFAULT 0,
    status_message TEXT NOT NULL DEFAULT '',
    created_at     DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_at     DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP,
    FOREIGN KEY(camera_id) REFERENCES cameras(camera_id) ON DELETE CASCADE
);
CREATE INDEX IF NOT EXISTS idx_algo_instances_camera ON algorithm_instances(camera_id);
CREATE INDEX IF NOT EXISTS idx_algo_instances_algo ON algorithm_instances(algorithm_id);
```

---

## 3. 核心机制设计 (Core Mechanisms)

### 3.1 启动自愈对齐 (Boot Reconciliation Flow)
```
[Heimdall 启动]
      │
      ▼
读取当前芯片架构 platform = current_platform_id() (如 macos-arm64-coreml, rknn-rk3588)
      │
      ▼
扫描预装包根目录: algo-packages/{platform}/ 与 var/packages/
      │
      ├─► 遍历发现的每一个有效算法包:
      │       │
      │       ├─ 检查 DB 是否已存在记录 (algorithm_id, version, platform_id)
      │       │     ├─ 若无:
      │       │     │    1. 执行 AlgoSandbox::validate_package(path, false)
      │       │     │    2. 搬迁或登记 package_root
      │       │     │    3. 插入 algorithms 表 (若首次出现)
      │       │     │    4. 插入 algorithm_versions 表 (is_builtin = 1, is_active = 1)
      │       │     │    5. 更新 algorithms.active_version = version
      │       │     └─ 若有: 跳过入库
      │
      ▼
从 DB 读取所有 is_active = 1 的算法版本:
      │
      ├─► 检查 package_root 目录是否存在:
      │       ├─ 存在: 通过 AlgoPackage::load_and_verify 加载进 AlgoRegistry 内存映射
      │       └─ 物理缺失: 打印 Warning 并标记运行时状态为 Degraded (不 Panic)
      │
      ▼
完成冷启动，就绪提供推理与调度服务
```

### 3.2 运行时零停机优雅热重载 (Runtime In-Process Hot-Swap)
1. 用户发起 `PUT /api/v1/algorithms/{id}/versions/{version}/activate`；
2. 数据库事务：
   - 将原版本 `is_active` 置为 0，目标版本 `is_active` 置为 1；
   - 将 `algorithms.active_version` 更新为目标 `version`；
3. 动态库预加载：
   - 检查目标版本是否已在内存中，若无则调用 `AlgoPackage::load_and_verify` 加载；
   - 更新 `state.algo_registry` 对应映射；
4. 管线原子换入：
   - 通知 `PipelineManager`：遍历所有挂载了该 `algorithm_id` 的 `CameraPipelineContext`；
   - 在前后两帧推理的间隙，利用 Rust 所有权释放旧 `AlgoInstance`（触发 RAII C ABI `instance_destroy`），并以新包创建新实例；
   - RTSP 主/子码流拉流持续、硬件解码器不重启、推流不闪断，实现毫秒级零中断热更新。

### 3.3 物理卸载与原子级联淘汰 (Physical Deletion & Cleanup)
1. 用户发起 `DELETE /api/v1/algorithms/{id}/versions/{version}`；
2. 安全自检三道门禁：
   - **门禁 1（内置保护）**：`version.is_builtin == true` -> 拦截，返回 `BUILTIN_ALGO_PROTECTED`；
   - **门禁 2（活跃占用保护）**：查询 `SELECT COUNT(*) FROM algorithm_instances WHERE algorithm_id = ? AND enabled = 1` -> 若大于 0，拦截返回 `ALGO_IN_USE`；
   - **门禁 3（当前激活保护）**：若该版本是当前的 `active_version`，且该算法还有其他版本，必须先激活另一版本或先停用。
3. 执行删除：
   - 在 SQLite 事务中删除 `algorithm_versions` 该行；
   - 若该算法下已无任何其他版本，同时删除 `algorithms` 主表记录；
   - 事务提交后，调用 `std::fs::remove_dir_all(package_root)` 原子清除磁盘二进制目录，彻底规避孤儿死文件。

---

## 4. 前端组件与交互架构 (Web UI Architecture)

```
web/src/
├── app/layout.tsx                  # 新增 NavTab 'algorithms' (Cpu 图标)
├── features/algorithms/
│   ├── AlgorithmsPage.tsx          # 算法仓库主页面
│   ├── components/
│   │   ├── AlgoStatsHeader.tsx     # 顶部统计指标卡 (总算法数、活跃版本数、平台架构、内置/自定义)
│   │   ├── AlgoFilterBar.tsx       # 搜索过滤栏 (keyword, algorithmType, isBuiltin)
│   │   ├── AlgoCardGrid.tsx        # 算法卡片矩阵网格
│   │   ├── AlgoCard.tsx            # 单算法卡片 (信息、活跃版本、平台标签、快捷操作)
│   │   ├── VersionsDrawer.tsx      # 版本管理抽屉 (版本列表、FPS档位、激活/回滚、安全卸载)
│   │   ├── UploadModal.tsx         # 归档包拖拽上传弹窗 (流式展示 7 步沙箱自检结果)
│   │   └── SchemaModal.tsx         # 参数配置 Schema 预览模态框
│   └── hooks/
│       └── useAlgorithms.ts        # TanStack Query 算法包数据流管理
└── features/tasks/
    └── components/
        └── LiveRulesStudio.tsx     # 消费算法实例，解耦出纯净的几何规则绘制与布防
```
