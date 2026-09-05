# Subtask 3 技术设计: 感知解耦、统一规则引擎与证据三支柱落库

## 1. 架构总览

```
Sub-stream Frame (NV12)
          │
          ├── [Pre-crop ROI 硬件裁剪] (可选局部特写)
          │        │
          │        ▼
          │   [C ABI 算法包推理] (输入局部帧 -> 输出局部 Vec<Detection>)
          │        │
          │   [线性仿射坐标还原] (局部 [0,1] -> 全景 [0,1] 坐标系)
          ▼        ▼
[ByteTrack 纯 Rust 航迹关联跟踪器]
          │
          ▼ 连续 TrackID + 历史轨迹向量 (Trajectory)
[统一空间几何规则引擎 (RuleEvaluator)]
          │
          ├── Mask 区域遮罩 (静默丢弃)
          ├── ROI 多边形入侵 (底部中心点判定)
          ├── Line 绊线越界 (跨立实验 + 向量叉积方向判定)
          └── 5 秒防重复报警冷却 (Cooldown Map)
          │
          ▼ 触发告警
[靶向快拍双切片生成] (调用 Subtask 2 SnapshotEngine)
          │
          ▼ 落地数据库 (SQLite WAL)
[证据三支柱与底库]
  ├── capture_records (行迹抓拍: 全景+特写+质量分)
  ├── alarm_records (违规告警: 全景+特写+规则类型+严重等级)
  ├── recognition_records (识别对账: 现场特写+底库照+相似度)
  └── galleries (人员/车辆底库名单: 512维 FP32 Blob + 登记照)
          ▲
          │ 定时 5 分钟巡检 (statvfs 磁盘剩余 < 15%)
[原子级加权淘汰引擎 (Atomic Eviction Engine)]
  └── 优先清理普通抓拍，保全告警大图；物理删除与 SQL 删除事务联动，图在案在，图销案销
```

## 2. 详细模块设计

### 2.1 Pre-crop ROI 与坐标线性还原
算法包只接收 `[0.0, 1.0]` 归一化输入并输出归一化 Detection。
Engine 在配置有局部特写区域 `ROI = [rx1, ry1, rx2, ry2]` 时：
- 将局部检测框 $(cx_1, cy_1, cx_2, cy_2)$ 还原为全景大图坐标：
  - $X_1 = rx_1 + cx_1 \cdot (rx_2 - rx_1)$
  - $Y_1 = ry_1 + cy_1 \cdot (ry_2 - ry_1)$
  - $X_2 = rx_1 + cx_2 \cdot (rx_2 - rx_1)$
  - $Y_2 = ry_1 + cy_2 \cdot (ry_2 - ry_1)$

### 2.2 纯 Rust ByteTrack 航迹关联
- 基于 IoU 贪婪二分匹配，维护具有历史轨迹的 `TrackedObject`。
- 轨迹保留最近 30 个底中心采样点，用于绊线跨越判定。
- 丢失目标超过 30 帧后平滑注销，杜绝内存泄漏。

### 2.3 几何规则引擎与 5 秒防刷屏冷却
- `RuleEvaluator`:
  - 判定入侵：目标底中心点落在多边形内部；
  - 判定越界：目标最新轨迹线段与绊线线段求交，且叉积符合设定方向（单向/双向）；
  - 防重复：每个 `track_id` 对同一 `rule_id` 在 5000ms 内仅触发一次。

### 2.4 数据库 Schema 迁移 (V2)
- 创建 `capture_records`、`recognition_records`、`galleries`；
- 扩展 `alarm_records`（增加特写图、规则类型、严重程度字段）。

### 2.5 磁盘水位保底与加权原子清理 (statvfs)
- POSIX `libc::statvfs` 检测物理可用磁盘比例；
- 当 `avail / total < 0.15` 时触发批量清理；
- 加权优先级：普通抓拍记录（capture_records）最老数据优先淘汰，保留 alarm_records；
- “图在案在，图销案销”：同步执行 `fs::remove_file` 与数据库行删除。
