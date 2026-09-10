# 技术方案设计：规则告警与抓拍证据三支柱落库及 WebSocket 实时广播 (design.md)

## 1. Context & Goals (背景与目标)

### 1.1 问题与现状剖析
系统在 `crates/pipeline/src/pump.rs` 中已具备了完备的几何规则判定（`RuleEvaluator`）与双流快照机制（`trigger_snapshot`）：
1. **数据库落库缺失**：
   - 规则判定命中后，虽然触发了快照生成（`var/data/evidence/`），并且构建了 `PipelineAlarmEvent`，发布至内存广播通道与有界环形补偿缓冲（`pending_alarm_events`）；
   - 但后端没有任何常驻服务或消费者订阅该事件或执行 DB 落库，`db::AlarmRepo::insert` 和 `db::CaptureRepo::insert` 从未被调用；
   - 导致 SQLite 数据库中的 `alarm_records`、`capture_records` 始终为空，告警中心与证据列表无法查看历史真实数据。
2. **WebSocket 实时告警广播未点火**：
   - 前端 `LivePage.tsx` 已准备好 `WS_TOPICS.ALARM_TRIGGERED` ("alarm.triggered") 的监听、告警音效播放与右下角切图悬浮卡片展示；
   - 但后端的 `AppState.event_broadcaster` 从未发送过该 Topic 的事件，导致前端实时中心处于完全静默状态。
3. **证据三支柱流转闭环**：
   - 工业级安防监控遵循“违规告警 (Alarms)”、“行迹抓拍 (Captures)”与“识别对账 (Recognitions)”三支柱证据体系；
   - 当规则告警触发且快照成功生成时，应保证告警记录与抓拍记录原子/成对生成，且与磁盘文件建立严格关联。

### 1.2 核心设计目标
1. **分析热路径零阻塞**：
   - 推理驱动泵（`pump.rs`）运行在核心帧分析循环中，严禁在热路径上执行 SQLite 同步写盘或阻塞式数据库 IO；
   - 沿用现有的 `pipeline.publish_analysis_event(PipelineAnalysisEvent::Alarm)` 异步解耦设计，告警事件即刻推入通道与内存补偿队列。
2. **独立持久化与广播 Worker (`AlarmDispatchService`)**：
   - 在 `crates/api` 中实现 `AlarmDispatchService`，遵循与 `CameraProbeService` 一致的清晰架构风格；
   - 在 `crates/app/src/main.rs` 中装配并启动常驻后台 Worker；
   - 支持冷启动/滞后补偿：在启动时及通道滞后时自动调用 `pipeline.drain_pending_alarm_events`，杜绝告警漏单。
3. **双表双写 (Alarm & Capture)**：
   - 告警事实优先持久化至 `alarm_records`；
   - 行迹抓拍凭证持久化至 `capture_records`，完善证据三支柱；
   - 无论快照成功与否（`EvidenceStatus::Ready` vs `Failed`），告警事实均必须落库（快照失败仅将图片相对路径置空，保留告警违规记录）。
4. **规范 WebSocket 广播**：
   - 持久化入库成功后，立即向 `state.event_broadcaster` 发送 `topic: "alarm.triggered"`；
   - 广播载荷包含数据库主键 `id`、全局一致的 `eventId`、摄像头信息、规则类型、严重级别、特写图与大图相对路径及 UTC 毫秒时间戳；
   - 严格匹配前端 `LivePage` 的类型约定，实现毫秒级音效提示与卡片弹出。
5. **多端状态实时核验**：
   - 现有 `PUT /api/v1/alarms/{id}/status` 接口已支持状态流转，广播 `alarm.status_changed`；
   - 验证前端核验后悬浮卡片实时收起、告警中心状态同步更新。

---

## 2. Architecture & Data Flow (架构分层与数据流)

```text
┌────────────────────────────────────────────────────────────────────────┐
│                        crates/pipeline (Pump)                          │
│  - 规则引擎判定命中 TriggeredAlarm                                       │
│  - 触发主码流按需靶向快照 trigger_snapshot                               │
│  - 组装 PipelineAlarmEvent (含 UUID event_id, bbox, snapshot 相对路径)    │
│  - publish_analysis_event: 发送至 broadcast channel + pending 队列    │
└───────────────────────────────────┬────────────────────────────────────┘
                                    │ PipelineAnalysisEvent::Alarm
                                    ▼
┌────────────────────────────────────────────────────────────────────────┐
│                  crates/api (AlarmDispatchService)                     │
│  - subscribe_analysis_events() 持续接收告警事件                         │
│  - drain_pending_alarm_events() 冷启动与 Lagged 补偿                    │
│  - 查找 Camera 基础元数据 (名称映射)                                      │
│  - 组装并写入 db::AlarmRepo::insert(&db, alarm_active)                 │
│  - 组装并写入 db::CaptureRepo::insert(&db, capture_active)             │
│  - 向 state.event_broadcaster 发送 TOPIC_ALARM_TRIGGERED 广播事件        │
└───────────────┬────────────────────────────────────────┬───────────────┘
                │ SQLite 本地事务持久化                     │ WebSocket JSON
                ▼                                        ▼
┌───────────────────────────────┐        ┌───────────────────────────────┐
│           crates/db           │        │         Web Frontend          │
│  - alarm_records              │        │  - LivePage (悬浮卡片+音效)   │
│  - capture_records            │        │  - AlarmsPage (证据核验与大图)│
└───────────────────────────────┘        └───────────────────────────────┘
```

---

## 3. Detailed Component Design (详细组件设计)

### 3.1 告警持久化服务 `AlarmDispatchService`
- **位置**：`crates/api/src/alarm_service.rs`，在 `crates/api/src/lib.rs` 中导出。
- **结构体**：
  ```rust
  #[derive(Debug, Clone)]
  pub struct AlarmDispatchService {
      pub db: sea_orm::DatabaseConnection,
      pub pipeline: Arc<pipeline::PipelineManager>,
      pub event_broadcaster: tokio::sync::broadcast::Sender<WsBroadcastEvent>,
      pub shutdown_tx: tokio::sync::broadcast::Sender<()>,
  }
  ```
- **核心逻辑**：
  - `from_state(state: &AppState) -> Self`
  - `start_worker(self: Arc<Self>) -> tokio::task::JoinHandle<()>`:
    - 启动前先执行 `drain_and_persist_pending()` 处理启动前积压的告警；
    - 循环 `select!`:
      - `shutdown_rx.recv()`: 退出 Worker；
      - `evt = rx.recv()`:
        - `Ok(PipelineAnalysisEvent::Alarm(alarm_evt))`: 处理持久化与广播；
        - `Err(RecvError::Lagged(n))`: 记录告警 Lagged，调用 `drain_and_persist_pending()` 补偿；
        - `Err(RecvError::Closed)`: 退出。
- **落库细节**：
  - `AlarmRecord`:
    - `event_id`: `alarm_evt.event_id`
    - `camera_id`: `alarm_evt.camera_id`
    - `alarm_type_id`: `format!("rule_{}", alarm_evt.alarm.rule_index)`
    - `occurred_at`: `DateTime::from_timestamp_millis(alarm_evt.timestamp)`
    - `target_label`: `alarm_evt.alarm.tracked_object.label`
    - `confidence`: `alarm_evt.alarm.tracked_object.confidence`
    - `track_id`: `alarm_evt.alarm.tracked_object.track_id as i64`
    - `bbox_json`: `serde_json::to_string(&alarm_evt.alarm.tracked_object.bbox)`
    - `image_id`: 快照中的 `image_id`，若无快照则为 `""`
    - `image_rel_path`: 快照中的 `image_rel_path`，若无快照则为 `""`
    - `crop_image_id`: 快照中的 `crop_image_id`，若无快照则为 `""`
    - `crop_image_rel_path`: 快照中的 `crop_image_rel_path`，若无快照则为 `""`
    - `rule_type`: match `alarm_evt.alarm.role` (`Roi` -> `"roi"`, `Line` -> `"line"`, `Mask` -> `"mask"`)
    - `severity`: `"warning"` 或 `"critical"`（默认入侵/越界均为 `"warning"`）
    - `status`: `"unprocessed"`
    - `handled_at`: `None`
    - `created_at`: `Utc::now()`
  - `CaptureRecord`:
    - `capture_id`: `uuid::Uuid::new_v4().to_string()`
    - 字段同上映射，`quality_score: 1.0`，`captured_at: occurred_at`。

### 3.2 WebSocket 广播载荷
- Topic: `types::TOPIC_ALARM_TRIGGERED` ("alarm.triggered")
- Payload:
  ```json
  {
    "id": 1,
    "eventId": "c73a8867-b5bb-41db-bc4a-9ef854e4c278",
    "cameraId": "camera-main-1",
    "cameraName": "大门入口",
    "targetLabel": "person",
    "ruleType": "roi",
    "ruleName": "区域入侵 #1",
    "severity": "warning",
    "cropImageRelPath": "camera-main-1/2026-09-08/crop_1741100000000.jpg",
    "imageRelPath": "camera-main-1/2026-09-08/full_1741100000000.jpg",
    "occurredAt": 1741100000000
  }
  ```

### 3.3 生命周期装配
- 在 `crates/app/src/main.rs` 中：
  ```rust
  let alarm_svc = Arc::new(api::AlarmDispatchService::from_state(&state));
  alarm_svc.start_worker();
  tracing::info!("后台告警异步持久化与实时广播工作线程已启动");
  ```

---

## 4. Error Handling & Robustness (异常防护)

1. **数据库写入失败降级**：
   - 若 `AlarmRepo::insert` 遇到 DB 错误（如数据库锁死），使用 `tracing::error!` 记录并保留重试/指标，绝不引起主进程 Panic。
2. **广播 Lagged 零丢单保证**：
   - 依赖 `PipelineManager.pending_alarm_events` 内存环形队列（容量 1024）；
   - 当消费者因高负载被判定为 `Lagged` 时，自动调用 `drain_pending_alarm_events` 将滞留项无损同步入库。
3. **快照失败时的告警保护**：
   - 快照硬解码可能因解码器超时失败（`evidence_status == Failed`）；
   - 此时 `snapshot == None`，系统仍然持久化告警与抓拍记录，图片路径留空，保留违规事实证据。
