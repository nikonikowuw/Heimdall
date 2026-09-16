# 人脸识别「最优帧 + 特征融合」设计（Best-Shot Fusion & Evidence Settlement）

> **状态**：设计评审稿 (Draft v1.1)　**日期**：2026-09-15
> **实施进度**：M1 已落地（2026-09-15，含单测/集成测试与宿主接线；候选证据按 **D5** 以内存编码字节驻留）；M2 已落地（帧池融合、成熟 sidecar 与宿主握手）；M3 待开工。
> **本版本范围**：M1 结算 / M2 融合 v2 / M3 主流回溯；**不含** M4 对账策略增强（margin → 附录 C；底库渐进增强 → 附录 B）。
> **适用**：RK3568 `face_recognition` 算法包 + Heimdall 宿主（`pipeline` / `api` / `db` / `web`）
> **关联**：[EdgeFace.md](./EdgeFace.md)（模型选型与阈值建议）、[face-recognition-pipeline.md](../archive/face-recognition-pipeline.md)（包族输出契约）、[algo-sdk-guidelines.md](../nuwa/backend/algo-sdk-guidelines.md)

---

## 0. 决策记录与语义锚点

### 0.1 决策记录（Decision Log）

| # | 决策 | 结论 | 日期 |
| --- | --- | --- | --- |
| D1 | 结算延迟改变抓拍语义（记录落库延迟 ≤ `SETTLE_WINDOW_MS`） | ✅ 批准：保留 1500ms 默认，`0` 为兼容退化模式 | 2026-09-15 |
| D2 | 包内每轨提取上限 4 → 8（`KMAX=8`） | ✅ 批准：实机多路并发压测列为验收项 | 2026-09-15 |
| D3 | 底库渐进增强（gallery augmentation） | ❌ **本版本不做**（2026-09-15 决定）：设计归档至附录 B；后续版本如重启需重新评审（此前评审结论：能力在包内、策略在宿主、开关为实例参数且不做全局配置） | 2026-09-15 |
| D4 | margin 误认防控（M4） | ❌ **本版本不做**（2026-09-15 决定）：设计归档至附录 C；后续版本再评估 | 2026-09-15 |
| D5 | 峰值候选的驻留介质：盘上 `pending/` 还是内存 | ✅ 批准（2026-09-15）：候选以**内存编码字节**驻留（按路预算限流、覆盖式替换），**磁盘只承载已结算证据**；根除 `pending/` 的 TTL 清扫、启动清扫、cleaner 豁免、崩溃残留四类手续 | 2026-09-15 |
| D6 | 宿主侧候选质量准入（`CANDIDATE_MIN_QUALITY=0.40`） | ✅ 批准删除（2026-09-16，实机验证）：候选留存**不做质量下界**。该门限用算法包内部的评分尺度决定证据是否存在，尺度随检测器/参数漂移（实机换 SCRFD 包后低于 0.40 的轨道由 0.7% → 14.2%），而无候选轨道在主码流分析模式下回溯取证必然失败，直接丢整条通行记录（实测丢 24%）。识别侧的 `quality_thresholds.min_score` 仍归算法包，不参与证据判定 | 2026-09-16 |
| D7 | 无证据图的通行抓拍是否落库 | ✅ 批准**不落库**（2026-09-16）：`capture_records` 的行**本身就是证据产物**（消费者：人工复核、识别裁剪、存储统计），无图行不可复核、会污染证据表并掩盖证据缺失率，故「无图不成行」。告警侧的「事实保留」**不适用**于抓拍侧：告警事实由规则引擎独立判定、行必须留（`evidenceStatus=failed` 标注，见 `detection-alarm-contract.md`）；抓拍行没有独立的事实来源，图缺失即无可复核内容 | 2026-09-16 |

### 0.2 术语

| 术语 | 定义 |
| --- | --- |
| **峰值帧 (Peak Frame)** | 单条航迹生命周期内人脸综合质量分最高（且达标）的那一帧；需要清晰度、姿态、尺寸俱佳 |
| **融合模板 (Fused Template)** | 从航迹多个高质量帧特征合成的 512D 单位向量，用于 1:N 比对 |
| **候选证据 (Candidate Evidence)** | 峰值帧发生时立即编码并驻留内存的候选字节（全景 JPEG + 人脸裁剪 JPEG + 峰值帧几何），结算时一次性写盘为正式证据 |
| **结算 (Settle)** | 确定为一条识别记录生成证据图 + 匹配特征的时刻；此后进入抓拍冷却 |
| **识别对账 (Reconciliation)** | `recognitions` 表记录：现场人脸 vs 底库样本的一次 1:N 判定 |

### 0.3 语义锚点（跨层共识，实现不得偏离）

> `recognitions.fieldCropPath`（现场人脸图）= **峰值帧**；
> 匹配特征 = **含峰值帧在内、跨多帧合成的融合模板**（严格优于任何单帧）。

---

## 1. 背景与问题

### 1.1 现状数据流（代码级）

```
分析帧(子码流) ──推理──> 包内 best-shot 融合 ──sidecar embedding──> 宿主 tracker(粘性特征)
      │                                                                │
      │            ┌─────────── 进入 ROI 的第一帧 ───────────┐          │
      └── 快照引擎 ─> 证据图 = 该帧像素（manager.rs: trigger_snapshot_for_frame）
                       │
                       └──> PipelineCaptureEvent ──> capture_records
                                                  └> recognitions（1:N，用当时的粘性特征）
```

- 抓拍触发：`crates/pipeline/src/rules.rs::evaluate_captures`，**进入 ROI 首帧 + 5s 冷却**（`manager.rs` 传入 `cooldown_ms=5000`）；
- 快照：`crates/pipeline/src/manager.rs::trigger_snapshot_internal`，**与推理帧强绑定**（子流回退帧容差 `MAX_TARGET_FRAME_DIFF_MS = 100ms`），无历史帧回溯能力（子流路径）；
- 特征：`crates/pipeline/src/tracker.rs::TrackState` **粘性 embedding**（新特征到达前沿用旧值）；
- 匹配：`crates/api/src/capture_service.rs::try_match_and_record_recognition` 优先用 sidecar 特征，缺失时降级读裁剪 JPEG 重新 `extract_face`；
- 包内融合：`face_recognition/src/best_shot.rs::update_with_fusion`（q² 加权超球面融合 + 防漂移），**上限 4 帧、之后绝对冻结**。

### 1.2 三个根因

| # | 根因 | 证据 |
| --- | --- | --- |
| R1 | **结算时机错位**：识别记录诞生于 ROI 首帧，此时融合模板通常只有 1 帧素材，甚至为空（→ JPEG 二次提取降级） | `rules.rs::evaluate_captures` + `tracker.rs` 粘性语义 |
| R2 | **模板 4 帧绝对冻结**：`fused_count >= MAX_FUSED_FRAMES` 的分支排在 ΔQ 之前，后续更好的帧永远进不来 | `best_shot.rs::should_update_best_shot_with_delta` 门控顺序 |
| R3 | **首帧无条件播种 + 门限过宽**：`min_face_size=30`、`yaw≤45°`、`quality min_score=0.3`，30px 侧脸即可成为初值；无重播种/遗忘 | `config.rs` 默认值 + `plugin.rs::best_shot_sidecar` 首帧分支 |

对照文档建议（`EdgeFace.md §4.1`）：识别可用人脸宜 ≥80×80px、yaw ≤30°；当前门控远宽于建议。

### 1.3 为什么"融合了还是错"

- 融合只影响**探针特征**，不影响证据图；
- 融合是"越融越好"的过程，而记录在融合开始前就结算了；
- 4 帧冻结使"后续红利"对任何后续记录也不生效；
- 底库侧**没有进化通道**：录入照单帧特征入库——**本版本接受现状**（增强方案见附录 B，本期不做）。

---

## 2. 目标与非目标

**目标**

1. 识别对账的现场人脸图 = 峰值帧（子流保底 / 主流高分辨率增强）；
2. 识别对账的匹配特征 = 成熟融合模板（含峰值帧，且允许后续更优帧重播种）；
3. 全链路可观测：每条记录可追溯 `imageSource / fusedCount / templateQuality`。
   M1 阶段这三项落在**日志与既有字段**上而不改 schema：`fusedCount/templateQuality` 在包内
   随 sidecar 上行，宿主在结算完成的 `tracing::info!` 上记录（见 §7.2）；`imageSource` 属于 M3。

**非目标**

- 不改变告警（`alarm_records`）链路；
- 不引入落库后回填/升级记录；
- 不引入跨轨融合、底库均值合成、每帧全量特征传输；
- **不引入底库渐进增强**（auto 模板写入，归档附录 B）与 **margin 降级改判**（归档附录 C）；
- 不改变 `/api/v1` 根信封与既有字段语义。

---

## 3. 总体设计

### 3.1 架构分层

```
┌─ L1 包内 FusionEngine v2（M2）──────────────────────────────┐
│  种子门控 → 帧池(≤KMAX) → Top-K 冗余剔除 → q²加权球面融合    │
│  → 成熟 FSM → sidecar: embedding/fused_count/template_quality/ │
│                        template_mature（可选字段，向后兼容）    │
└───────────────────────────┬─────────────────────────────────┘
                            │ 每帧载荷
┌─ L2 宿主 CaptureSettle（M1）─▼──────────────────────────────┐
│  ROI 命中 → pending 挂起 → 逐帧峰值跟踪 → 候选编码驻留内存       │
│  结算: mature ∥ 质量平台 ∥ 1.5s 兜底 ∥ 离场                   │
│  证据: 内存候选一次性写盘 或 主流按峰值 PTS 回溯(M3)             │
└───────────────────────────┬─────────────────────────────────┘
                            │ capture 事件（图=峰值帧，特征=模板）
┌─ L3 识别对账 ────────────────▼──────────────────────────────┐
│  成熟模板 1:N → recognitions 落库                            │
└─────────────────────────────────────────────────────────────┘
（M4 对账策略增强：margin 与底库渐进增强均已移出本版本，归档附录 C/B）
```

### 3.2 目标时序

```
t0       人脸入 ROI ─→ pending 挂起
t0~t*    包内: 种子门控→提取→重算模板→发射 sidecar（仅变化帧）
         宿主: 质量提升帧 → 候选编码覆盖驻留内存（节流 ≥200ms/轨，零落盘）
t*       模板成熟翻转（或质量平台/兜底）→ 结算
         → 证据 = 候选帧（M3: 尝试主流同 PTS 回溯）; 特征 = 成熟模板
         → capture_records + recognitions 落库 → WS 推送
t*+5s    冷却期满，可开启新一轮 pending（独立峰值，互不污染）
```

### 3.3 跨层不变量

| 不变量 | 说明 |
| --- | --- |
| INV-1 | 图、人脸框、匹配特征必须属于同一 `(camera, algorithm, track)` |
| INV-2 | 落库记录必须带图（无图不落库，保持现状） |
| INV-3 | 事件中的 `bbox / face.bbox` 必须与所存图像同帧（候选路径下替换为峰值帧几何） |
| INV-4 | 融合模板只由本轨帧构成；防漂移拒绝帧不得进入模板 |
| INV-5 | 候选不产生盘上中间态与孤儿文件：编码字节随轨道条目替换/释放，崩溃即清零（D5） |

---

## 4. M1：宿主结算与峰值帧留存（CaptureSettle）

> **独立可交付**：不依赖包内改动、不依赖双流、不改 DB schema。
> 交付后：记录图 = 峰值帧（子流分辨率），匹配特征 = 结算时的融合模板（受益于推迟结算，上限受 R2 约束直到 M2）。

### 4.1 状态机

新增模块 `crates/pipeline/src/capture_settle.rs`：

```rust
/// 每个 (algorithm_id, track_id) 一份（实现见 capture_settle.rs）
struct PendingCapture {
    first_pts_ms: i64,              // 挂起时刻（进入 ROI 首帧）
    best: FrameGeometry,            // 峰值帧几何（质量最高）
    last_improve_pts_ms: i64,       // 峰值最后一次刷新的时刻
    candidate: Option<CandidateEvidence>, // 内存驻留的候选（编码字节 + 峰值帧几何）
    last_candidate_attempt_ms: i64, // 候选编码节流基准（仅在真正产出动作时更新）
}

struct TrackEntry { pending: Option<PendingCapture>, settled_at_ms: Option<i64>,
                    last_seen: Option<TrackedObject>, last_seen_pts_ms: i64 } // 冷却基准 + 离场取帧

struct FrameGeometry { quality: f32, bbox: BoundingBox, face_bbox: Option<BoundingBox>, pts_ms: i64 }
struct CandidateEvidence { full_jpeg: Arc<[u8]>, crop_jpeg: Arc<[u8]>, width: u32, height: u32,
                          geometry: FrameGeometry } // INV-3 几何随图同帧
```

**每帧推进（锁内纯计算）**：

1. 命中 ROI（复用 `rules.rs` 的 mask/ROI/fullscreen 逻辑）且冷却已过 → 创建 pending；
2. 首帧（尚无 pending）**无条件**产出一次 `RetainCandidate`；此后取 `q = 当帧 face.quality_score`，
   若 `q > best.quality + PEAK_DELTA(0.05)`：刷新 `best` 并产出 `RetainCandidate` 动作
   （节流：距上次编码 ≥ `CANDIDATE_THROTTLE_MS(200)`，首帧同样占用节流锚点）；
   - **候选留存无质量下界**（D6）：候选的作用是「让每次结算都有一张同刻证据」，不是筛选好图；
     宿主按分数设门，等于用包内评分尺度决定证据是否存在；
3. 结算判定（按优先级，任一满足即产出 `Settle` 动作）：
   - ① `face.template_mature == true`（M2 生效；M1 阶段该字段缺省不触发）；
   - ② 峰值平台：`now - last_improve_pts_ms ≥ PLATEAU_MS(320ms≈8帧@25fps)` 且 `best.quality ≥ SETTLE_MIN_QUALITY(0.50)`；
   - ③ 兜底：`now - first_pts_ms ≥ SETTLE_WINDOW_MS(1500)`；
   - ④ 目标离场 / 轨道注销：有候选则结算，无候选按 `DISCARD_WITHOUT_CANDIDATE` 策略处理（默认：用当帧兜底，避免漏记对账）；
4. 结算后清除 pending 且 `cooldown_armed_at_ms = now`。

### 4.2 职责划分（锁内计算 vs 异步 IO）

| 环节 | 归属 | 说明 |
| --- | --- | --- |
| pending 状态推进、结算判定 | `manager.rs` 持 tracker 锁期间（`CaptureSettleController`） | 纯同步计算，禁止 IO/await |
| `RetainCandidate` 编码 | `pump.rs`（分析循环） | 复用当帧 `analyzed_frame`，同帧裁剪，走快照编码线程；产物仅驻留内存 |
| `Settle` 证据生成 | `pump.rs` | 内存候选一次性写盘（失败回退当帧快照；M3 增加回溯分支） |
| 事件发射 | `pump.rs` | `PipelineCaptureEvent` 结构不变（M1） |

`AnalysisOutcome`（`manager.rs`）与 `rules.rs` 的关系调整：

- 识别类算法（`AlgorithmKind::is_recognition()`）不再调用 `evaluate_captures` 直接产事件，改走 `CaptureSettleController::observe(...) -> Vec<CaptureAction>`；
- 检测类算法（告警）保持 `evaluate` 不变；
- `CaptureAction::{RetainCandidate{..}, Settle{..}}` 由 `pump.rs` 消费；
- **测试影响**：`rules.rs::evaluate_captures` 相关单测迁移/重写为结算控制器用例（首帧不再直接抓拍）。

### 4.3 候选证据生命周期（D5：内存驻留，磁盘只承载已结算证据）

- **编码**：`RetainCandidate` 仅调用 `SnapshotEngine::encode_candidate_async` 得到编码字节（全景 + 裁剪），**不产生任何盘上文件**；设备侧编码链路不变（RK：DMA-BUF → RGA/MPP JPEGE，只有 bitstream 回读）；
- **驻留**：编码字节登记进 `CaptureSettleController`；同一轨道新候选直接替换旧候选（旧字节随替换释放，驻留恒为单份）；
- **预算**：每路驻留总量受 `CANDIDATE_BUDGET_BYTES(8 MiB)` 约束；超限拒绝新候选（保留既有候选），拒绝次数计入遥测（`CaptureSettleController::budget_rejections` 与 `PumpMetrics::candidate_budget_rejections`），该轨结算时回退当帧快照；
- **优先级**：候选编码与正式证据虽共用同一条专用编码线程，但走**两条独立有界队列**（证据 8 / 候选 4），线程先排空证据队列；候选突发最多拒绝候选自身，永远不会把告警证据顶出队列；
- **结算**：`Settle` 携带候选字节，由 `write_candidate_evidence` 一次性原子写入 `{evidence_base}/{camera_id}/`（分配新 `image_id/crop_image_id`；第二份写失败回滚第一份；沿用存储断路器）；该写入作为 `SnapshotTask::WriteCandidate` 排入**证据队列**，与编码共用同一专用线程，不经 `spawn_blocking`；
- **来源标记**：候选登记时冻结当时的码流来源（子码流/主码流），`CandidateEvidence.stream` 与结算产出的 `SnapshotResult.image_stream` 取该冻结值，而不是结算时刻重新采样的模式；`SnapshotResult.image_source` 同时标明本条证据来自峰值候选帧还是靶向快拍帧（`types::EvidenceImageSource`）。
- **丢弃**：结算前轨道注销 / 清轨 / 算法超时 → 内存条目直接释放，无文件需要回收；
- **崩溃语义**：进程退出即丢失全部未结算候选（天生无孤儿），因此**无 `pending/` 目录、无 TTL、无启动清扫、无 `storage_cleaner` 豁免**。

### 4.4 图-框一致性（INV-3）

结算采用候选帧时，`PipelineCaptureEvent.tracked_object` 的 `bbox` 与 `face.bbox` 必须替换为**峰值帧几何**（`PeakFrame`），否则前端叠加框与图像错位、`field_bbox_json` 失真。当帧快照路径（无候选）保持当帧几何。

候选登记时同时冻结当时帧几何（`CandidateEvidence.geometry`）；结算时事件几何取该冻结值，
不重新采样当帧。

**可审计**：`SnapshotResult.frame_pts_ms` 带回证据图实际所用帧的 PTS。审计**只针对无候选的
回溯/回退取证路径**：该路径下证据帧与事件几何都对齐 `last_seen_pts_ms`，子码流回退帧同轴，
不等则输出 `warn`（记录保留，但降级可见）。两种惰形不进比对：

- 候选路径的 `frame_pts_ms` 是**峰值帧**时标，与 `last_seen_pts_ms` 本就不同，且事件几何已同步
  替换为峰值帧几何，属于设计预期；
- 主码流取证帧位于另一条 PTS 轴（见 `media::StreamClockAnchor`），跨轴比较无意义。

### 4.5 参数（阶段值，M1 用常量；后续可上配置）

| 参数 | 默认 | 说明 |
| --- | --- | --- |
| `SETTLE_WINDOW_MS` | 1500 | 结算兜底窗口 |
| `SETTLE_MIN_QUALITY` | 0.50 | 平台期结算的最低峰值质量 |
| `PEAK_DELTA` | 0.05 | 峰值刷新门限 |
| `CANDIDATE_THROTTLE_MS` | 200 | 候选编码节流 |
| `PLATEAU_MS` | 320 | 峰值平台判定（≈8 帧@25fps） |
| `CAPTURE_COOLDOWN_MS` | 5000 | 同一轨道两次结算之间的冷却（防刷屏） |
| `EXIT_GRACE_MS` | 400 | 离场宽限：轨道未再触发后需持续该时长才按离场结算（≈10 帧@25fps） |
| `CANDIDATE_BUDGET_BYTES` | 8 MiB | 每路候选驻留字节预算（超限拒绝新候选） |

> `EXIT_GRACE_MS` 不可省略：`is_capture_triggering` 为 `false` 既可能是「离开 ROI」，
> 也可能只是背身/低头导致一两帧丢脸。若当帧即按离场结算，会烧掉整段冷却，并把最优证据
> 从峰值候选降级为当帧回退图。宽限以**最后一次真正触发**的帧时标为基准。
>
> `CANDIDATE_MIN_QUALITY`（原 0.40）已按 D6 删除：候选留存不做质量准入，首帧即留一张同刻候选。
> 分数低的图仍带 `quality_score` 落库供消费方筛选，但**不允许**因分数低而让记录消失。

### 4.6 文件级改动清单

| 文件 | 改动 |
| --- | --- |
| `crates/pipeline/src/capture_settle.rs` | 新增：状态机 + 单测 |
| `crates/pipeline/src/manager.rs` | `CameraPipelineContext` 挂载 `capture_settle: Mutex<CaptureSettleController>`；`AnalysisOutcome.captures` → `capture_actions: Vec<CaptureAction>`；新增 `retain_capture_candidate`（仅编码驻留）/ `write_capture_candidate`（结算写盘）；清轨/算法超时同步释放内存候选（无文件回收） |
| `crates/pipeline/src/pump.rs` | `execute_capture_actions`：`RetainCandidate`（当帧同帧编码驻留内存，软失败）与 `Settle`（优先把内存候选一次性写盘，回退当帧/`last_seen_pts` 快照）；事件发射含 INV-3 几何替换 |
| `crates/pipeline/src/snapshot.rs` | `SnapshotTask::{Evidence, EncodeCandidate, WriteCandidate}` + `encode_candidate_async`（只编码不落盘）/ `write_candidate_async`（结算写盘作业）；证据与候选分两条有界队列并由同一专用线程串行消费；`SnapshotResult.frame_pts_ms`（INV-3 审计）；`encode_candidate` / `write_candidate_evidence`（原子写 + 双份回滚 + 断路器） |
| `crates/pipeline/src/rules.rs` | `evaluate_captures` 移除，新增 `is_capture_triggering`（Mask/ROI/全屏 + 无脸 person 跳过）供控制器复用；单测迁移 |
| `crates/pipeline/src/lib.rs` | 导出 `capture_settle` 模块与 `CaptureAction/CaptureSettleController/CandidateEvidence` 等；`test_support`（`#[cfg(test)]`）收纳跨模块共享的测试替身 |

### 4.7 验收标准与测试

**单测**（`capture_settle.rs`）

- 峰值刷新门限 / 节流 / 覆盖写；
- 平台期结算：质量序列 `[0.45, 0.70, 0.85, 0.80, 0.75 → 平台]` → 结算取 `pts(0.85)`；
- 兜底结算：质量始终 `< SETTLE_MIN` → 1.5s 后按候选/当帧结算；
- 离场结算 / 无候选策略；
- 冷却从结算时刻重新计时。

**集成**（pipeline 层）

- 模拟帧序列驱动 `process_detections_*`（生产入口），断言：候选动作几何与当帧检测同源；瞬时丢脸不结算；平台期结算；候选编码驻留/覆盖/结算写盘/释放；事件 bbox 与候选帧一致；

> 冷启动清扫**不适用**：D5 决策后候选只以编码字节驻留内存，不存在 `pending/` 盘上中间态，
> 因此没有可清扫的孤儿文件（见 §4.3「崩溃语义」）。此项已作废。

**实机**

- 抽 50 条记录：记录图人脸像素尺寸/清晰度 vs 同轨峰值帧（人工抽检）；
- 记录延迟分布：结算延迟 ≤1.5s（`captured_at - first_pts`）。

**落地情况（2026-09-15，含 D5 内存化重构）**

- 状态机单测：`capture_settle.rs` 覆盖种子门控/节流/平台期取峰值几何/窗口兜底/离场结算+冷却/覆盖与丢弃/字节预算拒绝/算法隔离/清轨释放内存；
- 集成测试：`manager::tests::capture_candidate_encodes_in_memory_and_writes_once_at_settle`（留存零盘上产物 / 覆盖写单份 / 结算唯一一次写盘 / 清轨释放）、`manager::tests::recognition_settle_actions_flow_through_process_detections`（生产入口 `process_detections_*` 产出留存与结算动作、几何同帧、瞬时丢脸不结算）、`pump::tests::settle_action_writes_memory_candidate_and_aligns_event_geometry`（INV-3 事件几何对齐峰值帧）、`pump::tests::settle_action_without_candidate_falls_back_to_frame_snapshot`（无候选回退）；
- 回归单测：`capture_settle.rs::transient_face_loss_does_not_burn_cooldown`、`capture_settle.rs::exit_grace_is_measured_from_last_trigger_not_last_seen_at`；
- 破坏性变异验证：去掉 INV-3 几何替换 / 去掉清轨候选释放 / 放开字节预算 / 结算跳过写盘 → 对应测试必须失败（均已实测）。

**加固验证（本轮修复，均已实测）**

| 变异 | 必须失败的测试 |
| --- | --- |
| 候选作业改回与证据共用同一条队列 | `snapshot::tests::evidence_lane_is_not_starved_by_candidate_flood`（6/6 失败） |
| `EXIT_GRACE_MS` 置 0（取消离场宽限） | `capture_settle::tests::transient_face_loss_does_not_burn_cooldown`、`exit_grace_is_measured_from_last_trigger_not_last_seen_at`、`manager::tests::recognition_settle_actions_flow_through_process_detections`（3/3 失败） |
| 恢复宿主侧 `fused_count` 区间校验 | `infer::package::tests::test_parse_fusion_sidecar_fields_and_camel_aliases` |
| 停止跳帧沿用 `fused_count`/`template_quality` | `tracker::tests::template_metadata_follows_track_but_maturity_handshake_does_not`（落库字段退化为 NULL） |
| 让 `template_mature` 跳帧粘滞 | 同上（成熟握手变成常驻状态，轨道首帧即结算，峰值留存窗口失效） |
| 主码流靶向帧写回真实轴 PTS | `snapshot::tests::test_snapshot_saves_both_full_and_crop_files_transactionally`（断言 `comparable_frame_pts_ms()==0`） |
| 落库丢失 `fused_count`/`template_quality` | `api` 集成：`test_recognition_capture_event_persistence_without_alarm`（断言 `cap.fused_count == Some(4)`） |
| DTO 把未标注记录暴露成 `0`/空串（而不是 `null`） | `api` 集成：`test_captures_list_exposes_evidence_origin_and_template_metadata` |
| 前端无条件渲染来源徽标 | `web` 单测：`utils.test.ts::deriveEvidenceOriginBadges`（4/4 失败） |
| V14 去掉历史行回填 | `db` 集成：`migration_tests::test_v14_migration_backfills_evidence_origin_without_guessing_unknowns` |

**结算取证降级链（2026-09-16，P0 兑现在 `17cc7df`，D6 准入下界移除在后续变更，均已实测）**

- P0（`pump.rs::execute_capture_actions` 的 `Settle` 分支）：回溯取证未命中时不再报错放弃，降级为当帧快照并 warn（`回索取证未命中，降级为当帧快照（图与事件几何可能不同刻，记录保留）`），落实 §6.2 回退链「候选缺失 → 当帧快照」与 §4.1 第 ④ 条的「默认：用当帧兜底，避免漏记对账」。实机（RK3568 / camera 0aec）：修复前 16:23–16:50 丢 215 条记录，修复后 0 条且 55 次降级全部落库。
- D6：删除宿主侧候选质量准入，首帧即留候选 ⇒ 主码流分析模式下正常轨道不再进入回溯分支。
- 新增测试：`pump::tests::exit_settle_without_candidate_degrades_to_current_frame_in_main_stream_mode`（主码流分析模式离场结算必须降级取证，断言 `image_stream=Main`、`frame_pts_ms` 与图落盘）、`capture_settle::tests::low_quality_seed_still_retains_candidate`（首帧无条件留存 + 节流锚点）、`capture_settle::tests::low_quality_track_refreshes_candidate_on_peak_improvement`（低质量区间仍刷新峰值）、`manager::tests::low_quality_recognition_frame_still_produces_candidate`（生产入口低质量首帧仍产出候选动作）。
- 变异验证：恢复 `CANDIDATE_MIN_QUALITY=0.40` 准入（种子 + 峰值刷新两处）→ 上述三条低质量测试 3/3 失败；移除 P0 兜底 → 离场结算降级测试失败。
- 无图不落库（D7，2026-09-16 复核确认）：`api::capture_service::event_to_active_model` 在 `snapshot == None` 时继续跳过落库（「无图不成行」），与告警侧 `alarm_service.rs` 的既定约定一致（告警行可无图存活，抓拍行必须与快照成对落库）。P0 + D6 之后无图路径已是病态兜底：候选必留 + 回溯未命中降级当帧，只有快照引擎整体失败才会走到这里；失败可见性由 WARN `通行抓拍快照未就绪或捕获失败，跳过无图抓拍记录落库`（可按路/分钟计数）承担，不靠造无图行。
- 候选预算余量（D6 带来的唯一风险项，实机实测）：候选全景图体积中位 257KB（1080p 主码流，n=300）→ 8 MiB/路 可容 **~32 条**并发待结算轨；而近 24h 结算并发峰值为 **7 条/2s 窗口**（``≤ ~6`` 轨同时 pending）≈ 1.5 MB，余量约 5 倍。因此保留 `CANDIDATE_BUDGET_BYTES` 与「超限拒绝新候选 + 回退链兜底」策略不变（非残留，是轻内存上界的既定设计）；稳态人群密集场景若实测 `candidate_budget_rejections` 抬头，再立专项改「淘汰最低质量」。

---

## 5. M2：包内融合引擎 v2（FusionEngine）

> 依赖 M1 的结算通道；交付后解除 R2/R3。
> 模块：`face_recognition/src/best_shot.rs`（保留文件名，语义升级为模板引擎）。

### 5.1 帧池与数据结构

```rust
struct FrameSample {
    embedding: [f32; 512],
    quality: FaceQuality,
    score: f32,
    bbox: [f32; 4],
    landmarks: [[f32; 2]; 5],
    frame_id: usize,
}

struct TrackTemplate {
    pool: Vec<FrameSample>,       // 上限 KMAX(8)
    template: [f32; 512],         // 每次入池后重算
    template_quality: f32,        // 参与融合帧的质量加权均值
    last_improve_frame_id: usize,
    best_frame: Option<FrameSample>, // 图像峰值（证据/可视化）
    retry_after_frame_id: usize,
    failed_attempts: u8,
    mature_emitted: bool,
}
```

### 5.2 采样策略（替代现 `should_update_best_shot_with_delta`）

| 条件 | 动作 |
| --- | --- |
| 池空 && `q < SEED_MIN(0.50)` | 不提取（零成本等待，R3 修复） |
| 池空 && 达标 | 提取 → 入池 |
| `q > best_pool_quality + ΔQ(0.08)` | 提取（追质量） |
| 池未满 && 距上次提取 ≥ `MIN_FUSION_FRAME_INTERVAL(6)` && `q ≥ MIN_FUSION_QUALITY(0.50)` | 提取（补采样） |
| `pool.len() ≥ KMAX` && 无提升 | 停止提取 |
| 提取失败 / 防漂移拒绝 | 软失败退避（6→12→24→48 帧，保留现实现） |

### 5.3 防漂移（保留并参数化）

`cos(新帧, 当前模板) ≥ DRIFT_MIN(0.55)`，拒绝帧不进模板、计软失败。`DRIFT_MIN` 列为实机标定项（见 §10.2）。

### 5.4 融合重算（每次入池后执行，CPU 标量运算，512D×≤8）

```
1. 池按 quality.score 降序
2. 贪心选样：与已选任一帧 cos ≥ REDUNDANCY_SIM(0.85) 视为冗余跳过；取 Top-K(4)
3. 加权平均：w = q_c²，q_c = clamp(q, 0.1, 1.0)
4. L2 归一化 → 新模板；template_quality = Σ(q_i·w_i)/Σw_i（参与帧）
```

**保证**：峰值帧是"质量提升帧"，必被 ΔQ 触发入池，因此**图上那张脸必然参与模板合成**。

### 5.5 成熟 FSM 与发射契约

```
mature = pooled_count ≥ 3
      && ( pool.len() ≥ KMAX
        || frame_id - last_improve_frame_id ≥ PLATEAU_FRAMES(10)
        || best_frame.face_size ≥ SIZE_TARGET(140px) )
```

发射规则：

- 模板变化帧：携带 `embedding` + `fused_count` + `template_quality`；
- 成熟首次翻转帧：即使模板未变，也补发一次（带 `template_mature: true`），作为宿主结算的**显式握手信号**；
- 每条轨道总发射次数有界（≤ KMAX + 1）。

### 5.6 sidecar 契约（向后兼容）

`fused_count` 的取值上界由包内 `MAX_FUSED_FRAMES` 决定，宿主不得假定或校验该区间：
宿主只能透传与记录。否则包内把 `KMAX` 从 8 提到 16 时，老宿主的区间校验会把整帧
检测全部丢弃（而不是仅忽略一个诊断字段）。

```json
"face": {
  "bbox": [0.41, 0.30, 0.52, 0.48],
  "confidence": 0.93,
  "quality_score": 0.81,
  "embedding": "base64...",
  "fused_count": 4,
  "template_quality": 0.81,
  "template_mature": true
}
```

- 算法包内部 JSON 使用既有 snake_case 约定；宿主 `RawFaceDetail` 同时接受 camelCase 别名，未知字段天然忽略；
- 包内：`postprocess::FaceDetailObject` 增加 `fused_count/template_quality/template_mature`（`Option`，仅相关帧携带）；
- 宿主：`crates/infer/src/package.rs::RawFaceDetail` 增加 `#[serde(default)]` 可选字段（老格式缺省为 `None`，**老包/老宿主零破坏**）；
- `crates/types`：`FaceDetail` 增加对应字段（领域类型以 camelCase 序列化，`TrackDto` 继续只暴露低频无关字段）。

### 5.7 参数与配置暴露（可选，建议 M2 后独立小步）

通过实例参数暴露（**包内消费**，适用三级优先级：宿主显式 > 包 `.env` > 默认；需同步 `config.schema.json` + 控制台表单 + `schema_exposes_every_runtime_field` 测试）：
`best_shot_seed_min_quality / best_shot_max_pool / best_shot_drift_similarity / best_shot_top_k`。

### 5.8 文件级改动清单

| 文件 | 改动 |
| --- | --- |
| `face_recognition/src/best_shot.rs` | 帧池/重算/成熟 FSM 重写 + 单测 |
| `face_recognition/src/plugin.rs` | 采样触发与发射逻辑接入；首帧门控 |
| `face_recognition/src/postprocess.rs` | `FaceDetailObject` 扩展字段 |
| `crates/pipeline/src/capture_settle.rs` | 消费 `template_mature` 成熟握手，优先以 `SettleReason::TemplateMature` 结算 |
| `crates/infer/src/package.rs` | 新字段解析 + 旧格式/别名兼容测试 |
| `crates/types/src/detection.rs` | `FaceDetail` 字段扩展与 sidecar 保留 |

### 5.9 验收与测试

- 单测：种子门控（弱帧不播种）、池淘汰、冗余剔除、Top-K 融合数学（含 `q²` 权重偏向高分帧）、成熟翻转、重播种（后期大脸进入并提升模板）、防漂移拒绝与退避；
- 兼容测试：**旧格式 JSON（无新字段）解析通过**、snake_case 实际 sidecar 与 camelCase 别名均可解析、新字段缺省路径不 panic；
- 回归：RK3568 算法包 lib 测试 **57 passed**；`types + infer` 测试 **66 passed**；算法包 workspace `cargo clippy -- -D warnings` 已通过；
- pipeline：成熟握手优先结算与现有 M1 回归均通过；当前全量 **123 passed / 1 pre-existing failed**（coordinator 生命周期测试，与本次无关）；
- 实机：NPU 提取次数/轨 ≤ KMAX；融合模板与单帧逐帧对比（同人不同帧相似度方差收窄）。

---

## 6. M3：主流高分辨率回溯（Retro Main-Stream Fetch）

> 双流部署增强；单流/未标定自动回退 M1 内存候选（不失败）。

### 6.1 调用路径（复用既有能力）

结算时：

```
peak.pts_ms(子流轴) ──resolve_main_axis_pts──> 主码流轴 PTS
    └─ trigger_snapshot(camera_id, peak_pts_ms, Some(peak.face_bbox), None)
       └─ ring_buffer.get_gop_for_timestamp(main_axis_pts) → 按需单帧硬解
```

- `crates/pipeline/src/manager.rs::trigger_snapshot` 已支持"无 analyzed_frame + 目标 PTS"的取证路径（`snapshot.rs::decode_target_frame_with_config` 支持环内任意历史 PTS 的 GOP 强解）；
- 主码流环窗口：`RingBufferConfig::default()` = 3500ms / 150 包，覆盖结算延迟（≤1.5s）；
- 跨流时标需 `stream_clock_anchors` 已标定，否则返回 `None` → 回退候选。

### 6.2 回退链（自外向内）

```
主流回溯成功 → 高分辨率峰值帧（最佳）
  ↳ 未标定 / GOP 超预算 / 配额满载 / 解码失败
     → M1 内存候选（子流峰值帧，结算时写盘）
        ↳ 候选缺失 → 当帧快照
```

### 6.3 遥测

- 日志/WS 载荷的 `imageSource` **已落地**（V14 列 + WS 增量字段）：当前取值为 `peak_candidate` / `targeted`，M3 引入主流回溯后新增 `main_stream_replay` 即可，不需再改表结构；
- 指标：`settle_total / settle_retro_hit / settle_retro_fallback`（`PipelineManager` 或 pump 指标集）；
- `capture_records.image_pts_ms` 已经能区分「证据帧与事件同刻」与「错刻取证」，M3 落地后需保证主流回溯命中时写入主流轴换算后的可比值。

> `image_pts_ms` 与 `image_stream` 现已落库，M3 的验收不再需要人工抽检图分辨率——
> 可直接按 `image_stream='sub'` 的比例监控主流回溯命中率。

### 6.4 文件级改动清单 + 验收

| 文件 | 改动 |
| --- | --- |
| `crates/pipeline/src/pump.rs` | `Settle` 分支优先尝试回溯，失败回退候选/当帧 |
| `crates/pipeline/src/manager.rs` | 复用 `trigger_snapshot`；补充遥测计数 |
| `crates/types/src/evidence.rs` | `EvidenceImageSource` 增加 `MainStreamReplay` 取值（旧消费者解析未知取值降级为未标注，不会崩） |

验收：双流实机记录图分辨率提升（人脸像素 ≥ 主流裁剪预期）；断锚点/单流场景回退率 100% 且无失败记录。

---

## 7. 契约汇总

### 7.1 sidecar payload（M2）

| 字段 | 类型 | 出现时机 |
| --- | --- | --- |
| `embedding` | base64(2048B LE) | 模板变化帧；成熟翻转且模板未变化时可不携带 |
| `fused_count` | u32 | 模板变化帧或成熟翻转帧。**宿主不校验其上界**：`KMAX` 是包内不变量（§5.1），宿主硬编码区间会阻死包内提高池上限的演进；宿主只透传并记日志 |
| `template_quality` | f32 | 模板变化帧或成熟翻转帧 |
| `template_mature` | bool | 仅成熟首次翻转帧为 `true`（本版本消费方：M1 结算触发①） |

### 7.2 事件/记录

| 载体 | 增量 |
| --- | --- |
| `PipelineCaptureEvent` | `snapshot.image_source` / `snapshot.image_stream`（**已落地**，随 `SnapshotResult` 携带，不再另设事件字段）；M3 只需新增枚举取值 |
| `capture_records` / `recognitions` | **已落地**（V14）：`image_source`、`image_stream`、`image_pts_ms`、`fused_count`、`template_quality`。峰值帧直接体现为 `image_rel_path/crop_image_rel_path`；`field_bbox_json` 与图同帧 |
| WS 载荷 | `recognition.matched` 增量携带 `imageSource` / `imageStream` / `fusedCount` / `templateQuality` |

字段语义：

| 列 | 取值 | 说明 |
| --- | --- | --- |
| `image_source` | `peak_candidate` / `targeted` | 峰值候选帧 / 靶向快拍帧；V14 迁移把历史行回填为 `targeted`（该机制与该迁移同批引入） |
| `image_stream` | `main` / `sub` | 主码流高分辨率帧 / 子码流帧；历史行无等价来源，保持空串 = 未标注 |
| `image_pts_ms` | 13 位 UTC 毫秒 / 0 | **仅在与检测轴同轴时写入**；主码流靶向帧位于另一条时钟轴（§4.4），写 0 表示不可比，不用跨轴值冒充已知时序 |
| `fused_count` / `template_quality` | 可空 | 匹配所用融合模板的元数据；旧包与无 sidecar 帧为 NULL |

`fused_count` / `template_quality` 之所以从「只记日志」改为落库：结算日志只能对单次运行事后追溯，
而核查一条历史记录时无法重建「当时是几帧融合、模板质量多少」。
两者经 `tracker.rs::attach_embedding_to_face` 与模板向量同步跳帧沿用（描述同一个模板），
因此绝大多数记录都能给出真实值而不是一片 NULL。

`template_mature` **不落库**：它是成熟翻转的一次性握手信号（§5.5），不是状态；
存下来只会得到「NULL 究竟是未成熟还是未在翻转帧结算」的歧义。

### 7.3 存储目录

| 目录 | 用途 | 生命周期 |
| --- | --- | --- |
| （无候选目录） | 候选证据驻留内存（D5） | 编码字节随轨道条目替换/释放；崩溃即清零 |
| `{camera}/`（既有正式证据布局） | 正式证据（含结算写下的峰值帧） | 现有淘汰策略（`storage_cleaner` 对账池） |

### 7.4 参数清单（跨层）

| 参数 | 层 | 默认 | 暴露方式 |
| --- | --- | --- | --- |
| `SETTLE_WINDOW_MS` / `SETTLE_MIN_QUALITY` / `PEAK_DELTA` 等 | pipeline | 见 §4.5 | M1 常量 → 后续配置 |
| `SEED_MIN` / `KMAX` / `TOP_K` / `DRIFT_MIN` / `ΔQ` | 包内 | 见 §5 | M2 常量 → 实例参数（可选） |
| `best_shot_seed_min_quality` 等 M2 调优参数 | 包内消费 | 见 §5 | 包 `.env` / 宿主显式参数（三级优先级） |
| 融合路径开发期 A/B（可选） | 包内消费 | — | 包 `.env`，标定后移除 |

---

## 8. 降级矩阵

| 故障 | 行为 |
| --- | --- |
| 候选编码失败 / 超预算（D5） | 软失败；该轨无候选，结算回退当帧/环内快照 |
| 候选编码队列满（低优先级通道） | 丢弃该次候选编码；告警证据队列不受影响 |
| 结算写盘失败（断路器/磁盘错误） | 事件照发但 `snapshot: None`（无图不落库，保持现状） |
| 主流回溯失败/未标定/GOP 超预算 | 回退候选图 |
| 轨道极速消失（无候选） | 默认当帧兜底（不丢对账）；可配置严格模式丢弃 |
| 包内提取失败/防漂移拒绝 | 软失败退避；模板不被污染 |
| `templateMature` 永不触发 | 平台期/兜底窗口仍能结算（M1 兜底） |
| 无图 | 不落库（保持现状） |

---

## 9. 明确否决的方案

| 方案 | 否决原因 |
| --- | --- |
| 落库后 UPDATE 升级记录（图/相似度） | 时序脆弱、幂等复杂、WS 二次广播混乱 |
| 破坏性增量融合 + 4 帧绝对冻结 | 峰值进不来、初值永留（现状病灶 R2） |
| 每帧发射 embedding | 带宽 × NPU 白耗 |
| 跨轨融合 | 身份混血 |
| 底库多照片均值合成 | 特征互相抵消；应为多模板 + Max（若未来做底库增强，此约束仍适用，见附录 B） |
| 全帧（原始像素 / DMA-BUF）内存常驻至结算 | 占用解码缓冲池租约，饿死解码器；且需全帧 D2H，把低频证据 readback 升格为中频（D5 仅驻留编码字节） |
| 包内直接决定抓拍时机 | 抓拍/快照归宿主，包内只提供成熟信号 |

---

## 10. 测试与标定计划

### 10.1 测试矩阵

| 层 | 类型 | 用例 | 状态 |
| --- | --- | --- | --- |
| pipeline | 单测 | 结算状态机（§4.7） | 已落地 |
| pipeline | 集成 | 候选内存驻留生命周期、结算写盘、事件几何一致性 | 已落地 |
| package | 单测 | 融合数学/池淘汰/成熟/兼容（§5.9） | M2 |
| infer | 单测 | 新字段解析 + 旧格式兼容 | M2 |
| 实机 | 端到端 | 记录图=峰值帧（抽检）、延迟分布、回溯命中率、NPU 负载 | M2/M3 后 |

### 10.2 阈值标定（M2+M3 上线后）

1. 收集相似度分布：

```sql
SELECT status, similarity FROM recognitions
WHERE recognized_at >= ? ORDER BY similarity DESC;
```

2. 人工抽检 top-N 分布，确定：
   - `confirm/review`（融合模板分布通常比单帧更集中）；
   - 包内 `DRIFT_MIN`（同人跨姿态帧相似度下界分位）。
3. 标定结果回写本文件与 `EdgeFace.md` 附录。

---

## 11. 里程碑与依赖

```text
M1 (pipeline) ──> M2 (package+infer)
       └────────> M3 (pipeline, 依赖 M1 的候选 PTS)
```

| 里程碑 | 依赖 | 交付物 | 状态 | 风险 |
| --- | --- | --- | --- | --- |
| M1 | 无 | 结算状态机 + 峰值留存 | **已落地（2026-09-15）** | 抓拍语义变化（延迟 ≤1.5s，**D1 已批准**）；rules.rs 测试重写 |
| M2 | M1 | 融合引擎 v2 + 成熟信号 + 宿主 sidecar 解析/结算握手 | **已落地（本轮）** | NPU 每轨提取上限 KMAX=8（**D2 已批准**）；实机多路并发压测与阈值标定待执行 |
| M3 | M1 | 主流回溯增强 | 待开工（图源标识 `image_source`/`image_stream`/`image_pts_ms` 与融合元数据已提前落地，见 §7.2） | 依赖跨流 anchors 标定；配额竞争 |

---

## 12. 风险与开放问题

| # | 风险/问题 | 建议 |
| --- | --- | --- |
| 1 | 结算延迟对实时大屏/工单体验的影响 | **D1 已批准**（记录延迟 ≤1.5s）；`SETTLE_WINDOW_MS=0` 为兼容退化模式 |
| 2 | 快速通过目标（步速快/遮挡多）永远达不到平台期 | 兜底窗口内取最佳可得帧；严格模式按部署场景选择 |
| 3 | RK3568 多路并发下融合提取次数上升 | **D2 已批准 KMAX=8**；实机压测（N 路 25fps × KMAX）为验收项，必要时下调 |
| 4 | eMMC 写放大 | 候选零落盘（D5），每条记录仅结算时写 2 张图；实测写入量 |
| 5 | 底库 auto 模板的长期漂移 | **底库增强已移出本版本**（附录 B）；未来启用时：容量上限 + 质量淘汰 + 90 天过期评估 |
| 6 | 阈值随模型/量化版本变化 | 标定流程固化为发布门禁 |
| 7 | 崩溃丢失未结算候选 | 内存态天生无孤儿（D5）；进程重启后新轨迹重新结算，仅损失重启瞬间的在途记录 |
| 8 | 候选内存占用（编码字节） | 每路预算 `CANDIDATE_BUDGET_BYTES(8 MiB)` + 覆盖式替换单份驻留；超限拒绝并计数 |

---

## 附录 A：现状代码索引（实现时对照）

| 关注点 | 位置 |
| --- | --- |
| 抓拍触发 | `crates/pipeline/src/rules.rs::evaluate_captures`（`manager.rs` 传 `cooldown_ms=5000`） |
| 快照同帧约束 | `crates/pipeline/src/manager.rs::trigger_snapshot_internal`、`snapshot.rs::MAX_TARGET_FRAME_DIFF_MS=100` |
| 主流环 | `crates/media/src/ring_buffer.rs`（3500ms/150 包）、`snapshot.rs::decode_target_frame_with_config` |
| 粘性特征 | `crates/pipeline/src/tracker.rs::TrackState::update/attach_embedding_to_face` |
| 1:N 与阈值 | `crates/api/src/capture_service.rs::resolve_recognition_thresholds/try_match_and_record_recognition` |
| 底库索引 | `crates/api/src/gallery_index.rs`（Max 聚合 `search_top_k`） |
| 底库录入 | `crates/api/src/personnel_service.rs::extract_face_pipeline`（质量 ≥0.50） |
| 包内融合 | `face_recognition/src/best_shot.rs`、`plugin.rs::best_shot_sidecar` |
| 包内质量 | `face_recognition/src/quality.rs`、`config.rs`（默认门限） |
| 离线提取 | `face_recognition/src/lib.rs::extract_face_impl`（注册检测器 + 对齐 + FP16 EdgeFace） |
| 证据目录保护 | `crates/pipeline/src/storage_cleaner/mod.rs::GALLERIES_DIR_NAME` |

---

## 附录 B：底库渐进增强（本版本不做，归档）

> **2026-09-15 决定：本版本不实现**。设计保留备查；未来重启需重新评审。

**动机**：底库样本仅来自录入照（单帧特征），现场光照/角度/相机分布下同一人相似度上不去 → 误拒与低置信偏高。

**机制概要**：

- 触发：识别 `Confirmed` ∧ `templateMature` ∧ `templateQuality ≥ AUGMENT_MIN_QUALITY(0.60)` ∧ margin 通过（附录 C 判据，若启用）；
- 防污染：自去重（与本人已有模板 `cos ≥ 0.98` → 跳过）；跨主体（与其他主体任一模板 `cos ≥ AMBIGUOUS_SIM`（标定，初值 0.55）→ 跳过）；`PendingReview` 永不入库；
- 写入：`gallery_faces` 新增行——向量 = 融合模板（2048B LE）、`source='auto'`、`quality_score = templateQuality`、`detection_score = 0`、`aligned_rel_path` 留空；照片 = 峰值帧裁剪图复制到 `galleries/{subject_id}/auto_{face_id}.jpg`（受保护目录）；
- 检索：`gallery_index::search_top_k` Max 聚合天然支持，无需改动；`personnel_reextract` 跳过 `source=auto`；
- 容量：每主体 ≤ `max_auto_templates(5)`；淘汰顺序：最低 `quality_score` → 最旧 `created_at`；删文件 + 删行同一事务（图在案在，图销案销）。

**开关模型（已评审结论）**：能力在包内（sidecar 元数据）；策略在宿主；开关 = 包 schema 声明的实例参数 `gallery_augment_enabled`（宿主消费、默认 off，与 `similarity_threshold` 同款）；**不做全局 env 开关**；包 `.env` 仅承载包内消费的开发期 A/B。

**未来实施改动清单**：

| 层 | 改动 |
| --- | --- |
| `crates/db/src/migration/migrations/V14__gallery_auto_templates.sql` | `ALTER TABLE gallery_faces ADD COLUMN source TEXT NOT NULL DEFAULT 'manual';` + `idx_gallery_faces_source` |
| `crates/db` / `crates/types` | `gallery_face.rs` 增加 `source` 字段；`GalleryFaceDto.source` |
| `crates/api/src/gallery_augment.rs` | 新增：增强服务（异步、单飞、失败静默） |
| `crates/api` / `web` | 人脸样本列表带 `source` + 徽标（人工/自动）/删除 + i18n（注意 web 工作区在途改动） |

**标定项**（未来启用前）：`AMBIGUOUS_SIM`、`AUGMENT_MIN_QUALITY`、`MAX_AUTO_TEMPLATES`。

**风险（未来启用时）**：误识放大（默认 off + 防污染 + 可删除）；模板长期漂移（容量淘汰 + 90 天过期评估）；内存 +2KB/条（千人 ≈ +10MB）。

**重启条件**：现场误拒主因确认为“底库覆盖不足”，且 M1–M3 已稳定运行、实机标定完成。

---

## 附录 C：margin 误认防控（本版本不做，归档）

> **2026-09-15 决定：本版本不实现**。设计保留备查。

**动机**：Top1 与 Top2 相似度接近（相似人群 / 底库覆盖不足）时，静默 `Confirmed` 存在误认风险；margin 判据强制降级为 `PendingReview`。

**机制**：

```
if top1.similarity - top2.similarity < MARGIN_MIN(0.05):
    status = PendingReview   // 强制降级，禁止静默 Confirmed
```

- `MARGIN_MIN` 列为标定项（与 confirm/review 共享外部分布）；
- 无 top2（单候选）时不做 margin 判定；
- 只降级、不写入、不影响底库检索与记录内容；
- 参数暴露：包 `config.schema.json` 声明 `margin_min`（注明“由宿主消费”）+ 宿主从 `params_json` 读取（与 `similarity_threshold` 同款），默认 `0.05`。

**未来实施改动**：`crates/api/src/capture_service.rs::try_match_and_record_recognition` 判定段；API 测试（双候选 `top1-top2 < margin` → `PendingReview`；`≥ margin` → `Confirmed`；单候选不受影响）；回归 `personnel_api_tests` / `evidence_api_tests` 全绿。

**重启条件**：现场出现“相似主体静默误确认”为主诉问题时。
