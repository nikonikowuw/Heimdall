# 执行计划：规则告警与抓拍证据三支柱落库及 WebSocket 实时广播 (implement.md)

## 1. 任务目标与检查清单

- [x] **Step 1: 实现后台告警持久化与广播服务 (`crates/api/src/alarm_service.rs`)**
  - [x] 定义 `AlarmDispatchService` 结构体及 `from_state` 构造方法；
  - [x] 实现 `process_alarm_event`：将 `PipelineAlarmEvent` 原子持久化至 `alarm_records` 与 `capture_records`；
  - [x] 实现摄像头名称缓存/查询解析，填充广播载荷；
  - [x] 发送 `TOPIC_ALARM_TRIGGERED` ("alarm.triggered") WebSocket 事件；
  - [x] 实现启动时与滞后 (`Lagged`) 时的 `drain_and_persist_pending` 补偿逻辑；
  - [x] 启动常驻后台 Worker，响应 `shutdown_tx` 信号优雅退出。
- [x] **Step 2: 导出服务并在 App 主入口点火装配 (`crates/api/src/lib.rs`, `crates/app/src/main.rs`)**
  - [x] 在 `crates/api/src/lib.rs` 中导出 `AlarmDispatchService`；
  - [x] 在 `crates/app/src/main.rs` 中初始化并调用 `start_worker`。
- [x] **Step 3: 自动化集成测试与异常边界覆盖 (`crates/api/tests/alarm_persistence_broadcast_tests.rs`)**
  - [x] 编写测试：触发管线告警事件 -> 验证 `alarm_records` 与 `capture_records` 落库成功；
  - [x] 编写测试：订阅 WebSocket 广播通道 -> 验证收到 `alarm.triggered` 且字段完整符合契约；
  - [x] 编写测试：模拟快照生成失败场景 -> 验证告警事实仍能正常持久化落库；
  - [x] 编写测试：验证 `drain_pending_alarm_events` 补偿积压事件入库；
  - [x] 编写测试：验证状态变更接口 `PUT /api/v1/alarms/{id}/status` 正常广播 `alarm.status_changed`。
- [x] **Step 4: 前端契约与类型对齐校验 (`web/src/features/live/LivePage.tsx`, `web/src/types/index.ts`)**
  - [x] 确认 `WS_TOPICS.ALARM_TRIGGERED` ("alarm.triggered") 与后端常量 100% 对齐；
  - [x] 确认 `LivePage.tsx` 中告警切图路径加载、声音提示与卡片渲染逻辑；
  - [x] 运行前端 `pnpm typecheck` 保证无类型断层。
- [x] **Step 5: 验证门禁与代码质量检查**
  - [x] `cargo fmt --all -- --check`
  - [x] `cargo clippy --all-targets -- -D warnings`
  - [x] `cargo test -p api --test alarm_persistence_broadcast_tests`
  - [x] `cargo test --workspace`
  - [x] `pnpm lint`, `pnpm typecheck`, `pnpm test` in `web/`

---

## 2. 验证命令指南

```bash
# 1. 运行告警持久化与广播集成测试
cargo test -p api --test alarm_persistence_broadcast_tests -- --nocapture

# 2. 全工作区测试
cargo test --workspace

# 3. Rust 代码质量与 Lint 检查
cargo fmt --all -- --check
cargo clippy --all-targets -- -D warnings

# 4. 前端类型与单元测试检查
cd web && pnpm typecheck && pnpm test && pnpm lint
```
