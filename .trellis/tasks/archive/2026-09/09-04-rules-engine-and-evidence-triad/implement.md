# Subtask 3 实施计划: 感知解耦、统一规则引擎与证据三支柱落库

## 目标与范围
实现 Engine 预裁剪与坐标变换、ByteTrack 航迹管理、几何规则引擎与 5 秒冷却、证据三支柱数据库迁移与仓储、以及 statvfs 原子级淘汰保护。

## 步骤拆解

### Step 1: Pre-crop ROI 映射与 ByteTrack 航迹跟踪器实现
- [x] 在 `crates/pipeline/src/roi.rs` 实现局部特写区域坐标仿射变换
- [x] 在 `crates/pipeline/src/tracker.rs` 实现多目标 IoU 航迹关联算法，维护稳定 `track_id` 与轨迹序列

### Step 2: 几何规则引擎与防刷屏冷却调度器
- [x] 在 `crates/pipeline/src/rules.rs` 实现 `RuleEvaluator`：
  - 遮罩 Mask 过滤
  - 多边形入侵 ROI 判定
  - 单向/双向绊线越界 Line 判定
  - 5 秒防重复告警冷却 (Cooldown Map)

### Step 3: 数据库 V2 迁移与证据三支柱仓储层
- [x] 新增 `crates/db/src/migration/migrations/V2__evidence_triad_and_galleries.sql`
- [x] 新增 SeaORM 实体与仓储：
  - `capture_records` (行迹抓拍)
  - `alarm_records` (扩展违规告警字段)
  - `recognition_records` (识别对账)
  - `galleries` (底库名单与特征向量)

### Step 4: 统一存储池与 statvfs 加权原子淘汰引擎
- [x] 在 `crates/pipeline/src/storage_cleaner.rs` 实现基于 `statvfs` 的磁盘水位检测器
- [x] 实现普通抓拍优先淘汰与“图在案在，图销案销”的原子清理事务

### Step 5: 集成测试与全链路闭环验证
- [x] 编写 `crates/pipeline/tests/rules_engine_tests.rs`：测试越界判定、5 秒防抖冷却、证据落库与模拟磁盘淘汰
- [x] 运行全工作区测试与代码规范门禁
