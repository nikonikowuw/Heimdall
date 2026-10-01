# 事件驱动 NVR 录像 — 实施计划

## 实施顺序

按依赖关系从底向上，分 6 个阶段。每阶段完成后可独立验证。

---

### Phase 1: fMP4 Writer (`media` crate)

**目标**：纯 Rust fMP4 封装器，可将 H.264/H.265 EncodedPacket 序列写成可播放的 fMP4 文件。

**文件**：
- `crates/media/src/fmp4/mod.rs` — 模块入口
- `crates/media/src/fmp4/boxes.rs` — MP4 box 序列化（ftyp, moov, moof, mdat）
- `crates/media/src/fmp4/writer.rs` — FMP4Writer 状态机
- `crates/media/src/fmp4/nalu.rs` — Annex-B → AVCC 转换、SPS/PPS 提取
- `crates/media/src/fmp4/track.rs` — Track trait + VideoTrack

**验证**：
```bash
cargo nextest run -p heimdall-media --filter "fmp4"
# 单元测试：生成 fMP4 文件 → 用 ffprobe 验证 box 结构和 duration
# ffprobe -show_format -show_streams /tmp/test_output.mp4
```

**风险**：fMP4 box 字段偏移错误导致播放器无法解析。用 `mp4box -info` 和 `ffprobe` 交叉验证。

---

### Phase 2: PreCaptureRingBuffer (`media` crate)

**目标**：独立于 MainStreamRingBuffer 的压缩流滑动窗口。

**文件**：
- `crates/media/src/pre_capture_ring.rs`

**验证**：
```bash
cargo nextest run -p heimdall-media --filter "pre_capture"
# 测试：push N 包后 drain，验证首包为 keyframe、时长在配置窗口内
# 测试：clear 后 drain 返回空
# 测试：max_bytes 保护
```

---

### Phase 3: 数据库迁移 + Repository (`db` crate)

**目标**：recordings / recording_events 表、CRUD Repository。

**文件**：
- `crates/db/migrations/` — 新迁移文件
- `crates/db/src/entities/recordings.rs`
- `crates/db/src/entities/recording_events.rs`
- `crates/db/src/repo/recording_repo.rs`

**验证**：
```bash
cargo nextest run -p heimdall-db --filter "recording"
# 测试：CRUD 操作、级联删除、按 TTL 查询过期记录
```

---

### Phase 4: EventBus + RecordingWorker (`types` / `pipeline` crate)

**目标**：事件总线广播 + 录像工作线程状态机。

**文件**：
- `crates/types/src/event_bus.rs` (或 `crates/pipeline/src/event_bus.rs`)
- `crates/pipeline/src/recording/mod.rs` — 模块入口
- `crates/pipeline/src/recording/worker.rs` — RecordingWorker 线程主循环
- `crates/pipeline/src/recording/state.rs` — Idle/Recording 状态机

**依赖**：Phase 1 (fMP4Writer) + Phase 2 (PreCaptureRingBuffer) + Phase 3 (DB)

**验证**：
```bash
cargo nextest run -p heimdall-pipeline --filter "recording"
# 测试：Idle 收到事件 → 切换 Recording → post 倒计时结束 → 闭合 → 回 Idle
# 测试：Recording 中收到新事件 → 延长 post 倒计时
# 测试：SourceReset → 正确闭合
# 测试：EventBus publish/subscribe 多消费者
```

**关键实现细节**：
- `RecordingWorker::run()` 使用 `crossbeam::select!` 同时监听 mailbox_rx 和 event_rx
- 状态机用 enum `RecordingState { Idle, Recording { ... } }` 显式建模
- 文件 IO 全部在 worker 线程内执行

---

### Phase 5: ConsumerKind 扩展 + Pipeline 集成 + API (`media` / `pipeline` / `api` crate)

**目标**：串联全链路——Dispatcher 注册录像消费者、Pipeline 装配 Worker、HTTP 接口暴露。

**文件**：
- `crates/media/src/dispatcher.rs` — 新增 `ConsumerKind::Recording`
- `crates/pipeline/src/manager.rs` — 录像 Worker 生命周期管理
- `crates/api/src/routes/recordings.rs` — HTTP handlers
- `crates/api/src/dto/recording_dto.rs` — DTO 定义
- `crates/app/src/` — 装配 EventBus + RecordingWorker

**验证**：
```bash
cargo nextest run --workspace --filter "recording"
cargo clippy --all-targets -- -D warnings
# 集成测试：模拟码流 + 模拟事件 → 验证 fMP4 文件生成、DB 记录、HTTP 接口返回
```

---

### Phase 6: 前端 UI (`web`)

**目标**：录像配置 + 告警回放 + 导出。

**文件**：
- `web/src/features/cameras/components/RecordingConfigCard.tsx` — 配置卡片
- `web/src/features/alarms/components/EventRecordingTab.tsx` — 回放 tab
- `web/src/features/alarms/components/RecordingExportButton.tsx` — 导出按钮
- `web/src/lib/api/recordings.ts` — API 调用
- `web/src/types/recording.ts` — TypeScript 类型

**验证**：
```bash
cd web
pnpm lint
pnpm typecheck
pnpm test
pnpm build
```

---

## 存储淘汰集成（跨 Phase 3~5）

- Phase 3：recording_repo 提供 `find_expired(ttl_days)` 和 `delete_with_file(id)` 方法
- Phase 5：在现有淘汰定时任务中插入录像淘汰步骤，位于抓拍淘汰之后、告警证据之前

---

## 风险点与回滚

| 风险 | 影响 | 缓解 |
|------|------|------|
| fMP4 box 结构错误 | 文件不可播放 | Phase 1 用 ffprobe + 浏览器交叉验证 |
| NALU startcode 解析边界错误 | 花屏/crash | 用真实摄像头码流做 fuzz 测试 |
| 磁盘写入抖动反压 | 丢帧 | bounded mailbox 自动丢旧帧，不阻塞上游 |
| SPS/PPS 动态变化 | 新参数集不被写入 moov | 检测参数集变化 → 闭合当前文件 → 新文件新 moov |
| 极端高频事件 | 文件无限延长 | 硬上限：单文件最大时长（如 5 分钟）强制切片 |

---

## 不做的事情

- 不修改现有 `MainStreamRingBuffer` 的窗口或接口
- 不修改现有快照链路（`snapshot.rs`）
- 不实现音频 Track（仅预留接口）
- 不实现连续录像或混合录像模式
- 不实现告警上报（EventBus 预留但不接具体上报器）
