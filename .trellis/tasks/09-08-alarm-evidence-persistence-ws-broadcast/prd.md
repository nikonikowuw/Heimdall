# PRD: 规则告警与抓拍证据三支柱落库及WebSocket实时广播 (Alarm Evidence Persistence & Real-time WS Broadcast)

## 1. Goal

打通系统告警产生、快照存储、数据库落库、WebSocket 实时广播与前端响应消费的全链路闭环。消除“规则触发有日志有快照，但数据库无记录、实时中心无提示、告警中心查不到”的核心断点，实现工业级证据三支柱数据闭环与高时效性报警联动。

---

## 2. Background & Problem Statement

经系统性代码审查，目前存在严重的告警数据流转断层：
1. **数据库落库断开**：
   - `crates/pipeline/src/pump.rs` 中，规则引擎判定产生 `alarms` 后，仅调用了 `pipeline_mgr.trigger_snapshot` 将高清大图和扩边特写图写入磁盘文件系统（`var/data/evidence/`）；
   - 代码中**完全没有调用 `AlarmRepo::insert` 或 `CaptureRepo::insert`**；
   - 导致 SQLite 数据库中的 `alarm_records`、`capture_records` 表始终为空，前端访问告警中心（`/api/v1/alarms`）时无法获取任何真实告警。
2. **WebSocket 告警广播缺失**：
   - 前端 `web/src/features/live/LivePage.tsx` 已实现完整的 WebSocket 告警监听逻辑（`WS_TOPICS.ALARM_TRIGGERED`），包含提示音播放（`playAlarmChime`）和右下角活跃告警悬浮卡片；
   - 但后端 `event_broadcaster` **从未定义或发送过 `topic: "alarm_triggered"` 广播**，实时监控前端处于“静音且无感知”状态。
3. **告警状态流转前后端未实时双向同步**：
   - 用户在告警中心核验告警（标记为已处理）时，应通过 `alarm_status_changed` 广播通知实时中心即时收起悬浮告警卡片。

---

## 3. Detailed Requirements

### R1. 管线告警事件分发器 (`crates/pipeline`, `crates/api`)
- **R1.1 定义异步告警发布抽象 (`AlarmEventSink` / Channel)**：
  - 在 `PipelineManager` 中注入一个有界告警事件通道（`mpsc::channel<AlarmDispatchEvent>(256)`）；
  - 当子码流分析泵在 `pump.rs` 中检测到规则触发（`TriggeredAlarm`）且主码流靶向快照成功保存后，立即构造完整的 `AlarmDispatchEvent` 发送至通道，严禁在分析热路径上同步等待数据库磁盘 IO。
- **R1.2 告警异步持久化服务 (`crates/api/src/alarm_service.rs` 或 Worker)**：
  - 后台专用消费 Worker 接收 `AlarmDispatchEvent`：
    1. **落库违规告警**：组装 `db::entity::alarm::ActiveModel`，调用 `db::AlarmRepo::insert`，落库字段包含：
       - `event_id`: 唯一 UUID；
       - `camera_id`: 触发摄像头；
       - `rule_type`: 规则类型（`roi` / `line`）；
       - `rule_name`: 规则名称；
       - `target_label`: 违规目标类别（`person` / `car` 等）；
       - `severity`: 严重级别（`critical` / `warning` / `info`）；
       - `image_rel_path`: 主码流高清大图相对路径；
       - `crop_image_rel_path`: 10% 扩边特写切图相对路径；
       - `status`: 初始为 `"pending"`；
       - `occurred_at`: 违规触发毫秒时间戳。
    2. **落库行迹抓拍 (三支柱联动)**：同时调用 `db::CaptureRepo::insert`，建立抓拍凭证；
    3. **广播实时事件**：持久化成功后，调用 `state.event_broadcaster.send(...)` 广播实时告警。

### R2. WebSocket 实时告警广播格式规范 (`crates/api/src/routes/ws.rs`)
- **R2.1 规范广播载荷 (`alarm_triggered`)**：
  - WebSocket 广播 Topic 统一为 `"alarm_triggered"`；
  - Payload 格式严格对齐前端 `LivePage` 类型约定：
    ```json
    {
      "topic": "alarm_triggered",
      "payload": {
        "id": 1024,
        "eventId": "550e8400-e29b-41d4-a716-446655440000",
        "cameraId": "CAM-01",
        "cameraName": "东大门通道",
        "targetLabel": "person",
        "ruleType": "intrusion",
        "ruleName": "限制区域入侵",
        "severity": "critical",
        "cropImageRelPath": "evidence/2026-09-08/crop_550e...jpg",
        "imageRelPath": "evidence/2026-09-08/full_550e...jpg",
        "occurredAt": 1757300000000
      },
      "timestamp": 1757300000000
    }
    ```

### R3. 前端告警中心与实时联动验证 (`web/src/features/alarms/`, `web/src/features/live/`)
- **R3.1 实时中心即时响应**：
  - `LivePage.tsx` 接收到 `alarm_triggered` 后，立即触发报警提示音，并渲染告警卡片（展示特写缩略图与规则信息）；
  - 支持点击卡片跳转到对应摄像头的实时画面或告警详情。
- **R3.2 告警中心真实呈现与核验**：
  - `AlarmsPage.tsx` 刷新后展示数据库真实落库的告警，支持按摄像头、时间段与处理状态分页筛选；
  - 缩略图与高清全景大图可通过 `/api/v1/evidence/{path}` 正常加载预览；
  - 用户点击“标记已处理”时，调用 `PATCH/PUT /api/v1/alarms/{id}/status`，更新数据库状态并通过 `alarm_status_changed` 广播多端同步。

---

## 4. Key Files & Architecture Touchpoints

- `crates/pipeline/src/manager.rs` & `pump.rs`：触发快照后投递告警事件
- `crates/api/src/state.rs` & `crates/api/src/routes/alarm.rs`：告警异步消费 Worker 与广播
- `crates/db/src/repository/alarm.rs` & `capture.rs`：证据持久化落库
- `web/src/features/live/LivePage.tsx`：告警音效与悬浮卡片联动
- `web/src/features/alarms/AlarmsPage.tsx`：告警与抓拍记录查询展示及核验

---

## 5. Acceptance Criteria

1. **落库与文件原子性验证**：
   - 目标违规触发告警后，SQLite `alarm_records` 与 `capture_records` 表均新增记录，且对应的全景图与特写图文件在物理磁盘真实存在。
2. **WebSocket 毫秒级广播**：
   - 告警触发后 100ms 内，所有连接的 WebSocket 客户端均收到 `topic: "alarm_triggered"` 广播消息。
3. **前端音画同步弹窗**：
   - 前端 Live 页面在收到广播瞬间发出蜂鸣提示音，并在页面浮现带有切图的告警卡片。
4. **告警中心闭环核验**：
   - 在告警中心页面能查到该记录并流畅查看大图；点击处理后，状态更新并在全局广播清除。
5. **门禁检查**：
   - `cargo clippy --all-targets -- -D warnings` 无警告通过；
   - 编写告警触发 -> 异步落库 -> WS 广播的端到端集成测试。
