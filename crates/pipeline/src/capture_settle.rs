//! 通行抓拍结算状态机（CaptureSettle）
//!
//! 识别类算法（`AlgorithmKind::Recognition`）的抓拍语义从「进入 ROI 首帧立即抓拍」升级为
//! 「逐帧跟踪峰值 → 结算时刻一次性产出证据」。本模块只做纯同步状态推进，禁止任何 IO 与
//! `.await`；候选编码（内存驻留字节）与结算写盘由 `pump.rs` 在分析循环中执行。
//!
//! 抓拍以**人体**为主体：人脸只作为可选附加证据，不参与准入判定（见 `rules.rs`）；
//! 证据质量分统一走 [`TrackedObject::evidence_quality_score`]，无脸目标用归一化人体框面积
//! 选峰值帧，而不是回退置信度。
//!
//! 同轨去重：以 `(algorithm_id, track_id)` 为键，窗口随已落库记录数**指数退避**
//! （5s → 10s → 20s → 40s → 60s 封顶），并以 [`MAX_RECORDS_PER_TRACK`] 作为生命周期硬上限。
//! 正常通行者按基础窗口出图（通常 1～3 条），滞留者逐步稀疏；否则一个站立 10 分钟的人会
//! 刷出上百条几乎重复的记录。
//!
//! 结算触发（按优先级）：
//! 1. `TemplateMature`：包内融合模板成熟（M2 的 `face.template_mature` 握手）；
//! 2. `QualityPlateau`：峰值质量平台期（`last_improve + PLATEAU_MS` 且峰值达标）；
//! 3. `WindowExpired`：兜底窗口到期（`first_pts + SETTLE_WINDOW_MS`）；
//! 4. `TrackExited`：目标离开 ROI / 航迹消失。
//!
//! 设计依据：`docs/algo/face-best-shot-fusion-design.md` §4（M1）。

use std::collections::HashMap;
use std::sync::Arc;

use types::{BoundingBox, DetectionRule, EvidenceImageStream, TrackedObject};

use crate::rules::is_capture_triggering;

/// 结算兜底窗口（毫秒）：目标在 ROI 内停留超过该时长必然结算。
pub const SETTLE_WINDOW_MS: i64 = 1500;
/// 平台期结算的最低峰值质量。
///
/// 量纲为 [`TrackedObject::evidence_quality_score`]：人脸轨道沿用人脸质量（0.5 相当于中等
/// 成像），无脸轨道是归一化面积（0.5 相当于占半幅画面，罕见），因此无脸轨道通常不走平台期
/// 早结算，而是由窗口到期（[`SETTLE_WINDOW_MS`]）与离场结算产出记录。
pub const SETTLE_MIN_QUALITY: f32 = 0.50;
/// 峰值刷新门限：新帧质量需超过当前峰值该幅度才刷新。
pub const PEAK_DELTA: f32 = 0.05;
/// 候选留存节流（毫秒）：同一轨道两次编码留存的最小间隔。
pub const CANDIDATE_THROTTLE_MS: i64 = 200;
/// 峰值平台判定窗口（毫秒，≈8 帧 @25fps）。
pub const PLATEAU_MS: i64 = 320;
/// 同一轨道两次结算之间的基础冷却（毫秒），也是去重窗口退避的起点。
pub const CAPTURE_COOLDOWN_MS: i64 = 5000;
/// 去重窗口退避的最大左移位数：每多落一条记录窗口翻倍（5s → 10s → 20s → 40s → 80s），
/// 再由 [`DEDUP_WINDOW_MAX_MS`] 收敛到 60s 封顶。
pub const DEDUP_BACKOFF_MAX_SHIFT: u32 = 4;
/// 退避后的去重窗口上限（毫秒）：滞留者稳定在每分钟一条，不再继续稀疏。
pub const DEDUP_WINDOW_MAX_MS: i64 = 60_000;
/// 单条轨道生命周期内最多落库的记录数（硬上限，防止超长滞留者无界刷屏）。
pub const MAX_RECORDS_PER_TRACK: u32 = 32;
/// 离场宽限（毫秒）：轨道未再触发后需持续该时长才按离场结算。
///
/// 瞬时丢脸（背身、低头、短暂遮挡）会让 `is_capture_triggering` 返回 false，但它既不是
/// 「离开 ROI」也不是「轨道注销」。没有宽限就会把一次转头当成离场结算，烧掉整段冷却
/// 并只能回退低分辨率当帧证据。≈10 帧 @25fps。
pub const EXIT_GRACE_MS: i64 = 400;
/// 每路摄像头候选字节预算（内存上界）：超限拒绝新候选，结算回退当帧快照。
///
/// 候选只在结算窗口（≤1.5s）内驻留：sub 流单轨约 0.06–0.15MB、1080P 约 0.25–0.6MB，
/// 8 MiB 预算对应数十条并发待结算轨，正常场景远达不到。
pub const CANDIDATE_BUDGET_BYTES: usize = 8 * 1024 * 1024;
/// 结算后无 pending 条目的保留宽限（用于重入冷却判定）。
const ENTRY_GRACE_MS: i64 = 1000;

/// 结算状态机参数。M1 阶段使用常量默认值，后续可上实例配置。
#[derive(Debug, Clone, Copy)]
pub struct CaptureSettleConfig {
    pub settle_window_ms: i64,
    pub settle_min_quality: f32,
    pub peak_delta: f32,
    pub candidate_throttle_ms: i64,
    pub plateau_ms: i64,
    pub cooldown_ms: i64,
    /// 离场宽限：轨道未再触发后需持续该时长才按离场结算。
    pub exit_grace_ms: i64,
    /// 每路候选驻留字节预算（超限拒绝登记）。
    pub candidate_budget_bytes: usize,
}

impl Default for CaptureSettleConfig {
    fn default() -> Self {
        Self {
            settle_window_ms: SETTLE_WINDOW_MS,
            settle_min_quality: SETTLE_MIN_QUALITY,
            peak_delta: PEAK_DELTA,
            candidate_throttle_ms: CANDIDATE_THROTTLE_MS,
            plateau_ms: PLATEAU_MS,
            cooldown_ms: CAPTURE_COOLDOWN_MS,
            exit_grace_ms: EXIT_GRACE_MS,
            candidate_budget_bytes: CANDIDATE_BUDGET_BYTES,
        }
    }
}

/// 一帧的几何与质量快照。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FrameGeometry {
    /// 目标整体框（归一化 [0,1]）
    pub bbox: BoundingBox,
    /// 人脸特写裁剪目标（嵌套人脸框；纯人脸包则为检测框本身），`None` = 无人脸特写
    pub face_bbox: Option<BoundingBox>,
    /// 该帧的检测流 PTS（毫秒）
    pub pts_ms: i64,
    /// 该帧的人脸质量分
    pub quality: f32,
}

/// 已编码的峰值候选证据（内存驻留；结算时一次性写入正式证据目录）。
///
/// 只保一份字节、随轨道条目释放：没有盘上中间态，因此不需要 TTL 清扫、孤儿对账或
/// `storage_cleaner` 豁免；超预算时由调用方回退当帧快照。
#[derive(Debug, Clone, PartialEq)]
pub struct CandidateEvidence {
    /// 峰值帧全景 JPEG 字节
    pub full_jpeg: Arc<[u8]>,
    /// 峰值帧**人脸特写** JPEG 字节；`None` = 该帧未挂载人脸（背身/低头）。
    pub crop_jpeg: Option<Arc<[u8]>>,
    /// 峰值帧**人体特写** JPEG 字节（人工复查看衣着的主体证据）；`None` = 裁剪不可得。
    pub body_crop_jpeg: Option<Arc<[u8]>>,
    /// 图像宽高（结算写盘时的 `SnapshotResult` 元数据；届时帧已不在）
    pub width: u32,
    pub height: u32,
    /// 该候选图对应的峰值帧几何（INV-3：事件 bbox 必须与此图同帧）
    pub geometry: FrameGeometry,
    /// 编码时刻该帧所属码流（冻结于编码时，而不是结算时重新采样）。
    pub stream: EvidenceImageStream,
}

impl CandidateEvidence {
    /// 驻留字节数（预算核算口径）。
    pub fn resident_bytes(&self) -> usize {
        self.full_jpeg.len()
            + self.crop_jpeg.as_ref().map_or(0, |b| b.len())
            + self.body_crop_jpeg.as_ref().map_or(0, |b| b.len())
    }
}

/// 结算原因。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SettleReason {
    /// 包内融合模板成熟（M2 启用）
    TemplateMature,
    /// 峰值质量平台期
    QualityPlateau,
    /// 兜底窗口到期
    WindowExpired,
    /// 目标离开 ROI / 航迹消失
    TrackExited,
}

impl SettleReason {
    /// 稳定字符串标识（用于日志与遥测）。
    pub fn as_str(self) -> &'static str {
        match self {
            SettleReason::TemplateMature => "template_mature",
            SettleReason::QualityPlateau => "quality_plateau",
            SettleReason::WindowExpired => "window_expired",
            SettleReason::TrackExited => "track_exited",
        }
    }
}

/// 结算请求：由控制器产出、`pump.rs` 执行证据生成与事件发射。
#[derive(Debug, Clone)]
pub struct SettleRequest {
    pub track_id: u64,
    pub reason: SettleReason,
    /// 最近一帧的完整对象（携带最新 sticky embedding / face，供识别对账）。
    pub tracked_object: TrackedObject,
    /// 目标最近一次处于 ROI 内的帧 PTS（离场结算时用于回溯证据帧）。
    pub last_seen_pts_ms: i64,
    /// 结算帧目标是否仍在 ROI 内（决定是否走同帧快照路径）。
    pub target_in_current_frame: bool,
    /// 峰值候选证据（存在则结算时优先写入正式证据目录）。
    pub candidate: Option<CandidateEvidence>,
}

/// 候选留存请求：`pump.rs` 复用当帧 `analyzed_frame` 编码为内存候选。
#[derive(Debug, Clone, Copy)]
pub struct CandidateRetainRequest {
    pub track_id: u64,
    /// 当帧几何（`pts_ms` 必须与 `analyzed_frame.timestamp` 一致）。
    pub geometry: FrameGeometry,
}

/// 控制器输出的抓拍动作。
#[derive(Debug, Clone)]
pub enum CaptureAction {
    /// 留存当前帧为峰值候选（覆盖旧候选）。
    RetainCandidate(CandidateRetainRequest),
    /// 结算：生成正式证据并发射抓拍事件。
    Settle(Box<SettleRequest>),
}

/// 候选登记结论。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecordCandidateOutcome {
    /// 已登记为新候选（旧候选已被替换并释放）。
    Accepted,
    /// 对应轨道已结算或已清理：本次编码产物由调用方丢弃。
    Dismissed,
    /// 每路候选字节预算已满：拒绝登记，既有候选与峰值几何保持不变。
    RejectedBudget,
}

#[derive(Debug)]
struct PendingCapture {
    first_pts_ms: i64,
    best: FrameGeometry,
    last_improve_pts_ms: i64,
    candidate: Option<CandidateEvidence>,
    last_candidate_attempt_ms: i64,
}

#[derive(Debug, Default)]
struct TrackEntry {
    pending: Option<PendingCapture>,
    settled_at_ms: Option<i64>,
    last_seen: Option<TrackedObject>,
    last_seen_pts_ms: i64,
    /// 最近一次实际触发的帧时标；用于离场宽限判定。
    last_trigger_pts_ms: i64,
    /// 本轨道生命周期内已落库的记录数；驱动去重窗口退避与硬上限。
    records_written: u32,
}

/// 每路摄像头 × 每个算法的结算控制器（位于 `CameraPipelineContext`）。
///
/// 外层键为 `algorithm_id`，内层键为 `track_id`：内层表按算法分桶后，扫描离场只能
/// 命中同算法航迹，不需要把全路 `(String, u64)` 键逐个构造成对再逐帧比对算法名前缀。
/// 条目只在轨道处于 ROI 内或冷却保留期内占用，由冷却宽限自动修剪，内存有界。
#[derive(Debug)]
pub struct CaptureSettleController {
    config: CaptureSettleConfig,
    entries: HashMap<String, HashMap<u64, TrackEntry>>,
    /// 因超预算被拒绝登记的候选数（遥测口径）。
    budget_rejections: u64,
    /// 可复用暂存：本帧已触发的轨道集合（仅活在一个算法分桶内）。
    scratch_seen: Vec<u64>,
    /// 可复用暂存：待离场结算的轨道（避免每帧新建 Vec）。
    scratch_exited: Vec<u64>,
}

impl Default for CaptureSettleController {
    fn default() -> Self {
        Self::new()
    }
}

impl CaptureSettleController {
    pub fn new() -> Self {
        Self::with_config(CaptureSettleConfig::default())
    }

    pub fn with_config(config: CaptureSettleConfig) -> Self {
        Self {
            config,
            entries: HashMap::new(),
            budget_rejections: 0,
            scratch_seen: Vec::new(),
            scratch_exited: Vec::new(),
        }
    }

    pub fn config(&self) -> CaptureSettleConfig {
        self.config
    }

    /// 当前处于挂起（未结算）状态的轨道数。
    pub fn pending_count(&self) -> usize {
        self.entries
            .values()
            .flat_map(|tracks| tracks.values())
            .filter(|entry| entry.pending.is_some())
            .count()
    }

    /// 每帧推进（须在持有 tracker 锁期间调用，纯同步计算）。
    ///
    /// `tracked_objects` 为该算法当帧活跃航迹；`rules` 为任务级几何规则。
    pub fn observe(
        &mut self,
        algorithm_id: &str,
        tracked_objects: &[TrackedObject],
        rules: &[DetectionRule],
        now_ms: i64,
    ) -> Vec<CaptureAction> {
        let config = self.config;
        let mut actions = Vec::new();
        // 只取本算法的航迹分桶：内层表键只有 `track_id`，不再逐帧构造 `(String, u64)` 键。
        // 先在借用内层表之前取出暂存缓冲（否则 `scratch_exited` 与 `tracks` 无法同时可变借用）。
        let mut seen = std::mem::take(&mut self.scratch_seen);
        let mut exited = std::mem::take(&mut self.scratch_exited);
        seen.clear();
        exited.clear();

        let tracks = match self.entries.get_mut(algorithm_id) {
            Some(tracks) => tracks,
            // 首次见到该算法：此时才分配内层表；命中路径不做任何分配。
            None => self.entries.entry(algorithm_id.to_owned()).or_default(),
        };

        for obj in tracked_objects {
            if !is_capture_triggering(rules, obj) {
                continue;
            }

            let entry = tracks.entry(obj.track_id).or_default();
            entry.last_seen_pts_ms = now_ms;

            // 去重窗口随已落库记录数指数退避：正常通行者按基础窗口出图，滞留者逐步稀疏。
            // 达到生命周期硬上限后不再登记新挂起。
            // 必须在 `obj.clone()` 之前判定，否则每帧都要为窗口内的轨道白付一次克隆。
            if entry.pending.is_none() {
                if entry.records_written >= MAX_RECORDS_PER_TRACK {
                    continue;
                }
                let in_dedup_window = entry.settled_at_ms.is_some_and(|settled| {
                    now_ms - settled < dedup_window_ms(config.cooldown_ms, entry.records_written)
                });
                if in_dedup_window {
                    continue;
                }
            }
            seen.push(obj.track_id);

            let quality = frame_quality(obj);
            let template_mature = obj
                .face
                .as_ref()
                .is_some_and(|face| face.template_mature == Some(true));
            let geometry = FrameGeometry {
                bbox: obj.bbox,
                face_bbox: obj.face_crop_target(),
                pts_ms: now_ms,
                quality,
            };

            // 唯一无法避免的每触发帧克隆：结算发生在未来某帧，届时必须能拿到事件载体。
            entry.last_seen = Some(obj.clone());
            entry.last_trigger_pts_ms = now_ms;

            match entry.pending.as_mut() {
                None => {
                    // 兼容退化模式 (`SETTLE_WINDOW_MS <= 0`)：不做峰值留存，进入即结算，
                    // 等价旧「首帧抓拍」语义（冷却仍然生效）。
                    if config.settle_window_ms <= 0 {
                        entry.settled_at_ms = Some(now_ms);
                        // 退化模式同样计入已落库条数：否则退避窗口与生命周期上限会双双失效。
                        entry.records_written = entry.records_written.saturating_add(1);
                        if let Some(tracked_object) = entry.last_seen.take() {
                            actions.push(CaptureAction::Settle(Box::new(SettleRequest {
                                track_id: obj.track_id,
                                reason: SettleReason::WindowExpired,
                                tracked_object,
                                last_seen_pts_ms: now_ms,
                                target_in_current_frame: true,
                                candidate: None,
                            })));
                        }
                        continue;
                    }
                    let pending = PendingCapture {
                        first_pts_ms: now_ms,
                        best: geometry,
                        last_improve_pts_ms: now_ms,
                        candidate: None,
                        last_candidate_attempt_ms: i64::MIN / 2,
                    };
                    entry.pending = Some(pending);
                    if template_mature {
                        if let Some(request) = settle_pending(
                            entry,
                            obj.track_id,
                            SettleReason::TemplateMature,
                            now_ms,
                            /* target_in_current_frame */ true,
                        ) {
                            actions.push(CaptureAction::Settle(Box::new(request)));
                        }
                    } else {
                        // 无质量准入下界：候选的作用是「让每次结算都有一张同刻证据」，
                        // 而不是筛好图。宿主若按分数设门，等于用算法包内部的评分尺度
                        // 决定证据是否存在（尺度随检测器与参数变化），必然漏记对账。
                        if let Some(pending) = entry.pending.as_mut() {
                            pending.last_candidate_attempt_ms = now_ms;
                        }
                        actions.push(CaptureAction::RetainCandidate(CandidateRetainRequest {
                            track_id: obj.track_id,
                            geometry,
                        }));
                    }
                }
                Some(pending) => {
                    if quality > pending.best.quality + config.peak_delta {
                        pending.best = geometry;
                        pending.last_improve_pts_ms = now_ms;
                        if now_ms - pending.last_candidate_attempt_ms
                            >= config.candidate_throttle_ms
                        {
                            pending.last_candidate_attempt_ms = now_ms;
                            actions.push(CaptureAction::RetainCandidate(CandidateRetainRequest {
                                track_id: obj.track_id,
                                geometry,
                            }));
                        }
                    }

                    let settle_reason = if template_mature {
                        Some(SettleReason::TemplateMature)
                    } else if now_ms - pending.last_improve_pts_ms >= config.plateau_ms
                        && pending.best.quality >= config.settle_min_quality
                    {
                        Some(SettleReason::QualityPlateau)
                    } else if now_ms - pending.first_pts_ms >= config.settle_window_ms {
                        Some(SettleReason::WindowExpired)
                    } else {
                        None
                    };

                    if let Some(reason) = settle_reason {
                        if let Some(request) = settle_pending(
                            entry,
                            obj.track_id,
                            reason,
                            now_ms,
                            /* target_in_current_frame */ true,
                        ) {
                            actions.push(CaptureAction::Settle(Box::new(request)));
                        }
                    }
                }
            }
        }

        // 离场结算：本算法下仍有 pending、但本帧未触发且已超出离场宽限的轨道。
        //
        // 宽限不可省：`is_capture_triggering` 为 false 既可能是“目标离开 ROI”，
        // 也可能只是一两帧背身/低头导致人脸丢失。后者若立即按离场结算，会烧掉
        // 整段冷却并把最优证据从峰值候选降级为当帧回退图。
        for (&track_id, entry) in tracks.iter() {
            if entry.pending.is_none() {
                continue;
            }
            if seen.contains(&track_id) {
                continue;
            }
            if now_ms.saturating_sub(entry.last_trigger_pts_ms) < config.exit_grace_ms {
                continue;
            }
            exited.push(track_id);
        }
        for track_id in exited.drain(..) {
            if let Some(entry) = tracks.get_mut(&track_id) {
                if let Some(request) = settle_pending(
                    entry,
                    track_id,
                    SettleReason::TrackExited,
                    now_ms,
                    /* target_in_current_frame */ false,
                ) {
                    actions.push(CaptureAction::Settle(Box::new(request)));
                }
            }
        }

        // 归还暂存缓冲，保持预分配容量；`tracks` 借用已在上方结束。
        self.scratch_seen = seen;
        self.scratch_exited = exited;

        self.prune(now_ms);
        actions
    }

    /// 登记峰值候选（内存驻留）。
    ///
    /// 预算按“扣旧 + 加新”核算：超限拒绝新候选，既有候选与峰值几何保持不变，
    /// 结算时回退当帧快照。
    pub fn record_candidate(
        &mut self,
        algorithm_id: &str,
        track_id: u64,
        evidence: CandidateEvidence,
    ) -> RecordCandidateOutcome {
        let current_bytes = self.pending_bytes();
        // 先按 `&str` 查外层桶，避免每次登记都克隆一份 `algorithm_id` 键。
        let Some(entry) = self
            .entries
            .get_mut(algorithm_id)
            .and_then(|tracks| tracks.get_mut(&track_id))
        else {
            return RecordCandidateOutcome::Dismissed;
        };
        let Some(pending) = entry.pending.as_mut() else {
            return RecordCandidateOutcome::Dismissed;
        };
        let replaced_bytes = pending
            .candidate
            .as_ref()
            .map_or(0, CandidateEvidence::resident_bytes);
        let requested = current_bytes
            .saturating_sub(replaced_bytes)
            .saturating_add(evidence.resident_bytes());
        if requested > self.config.candidate_budget_bytes {
            self.budget_rejections = self.budget_rejections.saturating_add(1);
            return RecordCandidateOutcome::RejectedBudget;
        }
        pending.candidate = Some(evidence);
        RecordCandidateOutcome::Accepted
    }

    /// 当前驻留的候选字节总量。
    pub fn pending_bytes(&self) -> usize {
        self.entries
            .values()
            .flat_map(|tracks| tracks.values())
            .filter_map(|entry| entry.pending.as_ref())
            .filter_map(|pending| pending.candidate.as_ref())
            .map(CandidateEvidence::resident_bytes)
            .sum()
    }

    /// 因超预算被拒绝登记的候选数。
    pub fn budget_rejections(&self) -> u64 {
        self.budget_rejections
    }

    /// 清理全部状态（含内存候选）。
    pub fn clear(&mut self) {
        self.entries.clear();
    }

    /// 清理指定算法的全部状态（含内存候选）。
    pub fn clear_algorithm(&mut self, algorithm_id: &str) {
        self.entries.remove(algorithm_id);
    }

    /// 修剪已出去重窗口且无 pending 的条目；**不**删除空的内层表。
    ///
    /// 保留空桶是为了让稳态下的键路径零分配：`String` 键只在首次见到该算法时分配一次，
    /// 之后每帧都是 `get_mut(&str)` 命中；桶数上限为单路并发算法实例数，天然有界。
    ///
    /// 窗口判定必须与 `observe` 共用退避函数：若仍按基础冷却修剪，滞留者的条目会在退避
    /// 窗口中途被丢弃，`records_written` 跟着归零，退避与上限就双双失效了。
    fn prune(&mut self, now_ms: i64) {
        let cooldown = self.config.cooldown_ms;
        for tracks in self.entries.values_mut() {
            tracks.retain(|_, entry| {
                if entry.pending.is_some() {
                    return true;
                }
                // 若目标仍在画面中（最近仍在被观测到），绝不能修剪，
                // 否则会重置退避窗口与 records_written 上限。
                if now_ms.saturating_sub(entry.last_seen_pts_ms) < ENTRY_GRACE_MS {
                    return true;
                }
                entry.settled_at_ms.is_some_and(|settled| {
                    now_ms - settled
                        < dedup_window_ms(cooldown, entry.records_written) + ENTRY_GRACE_MS
                })
            });
        }
    }
}

/// 结算一条 pending：清空挂起状态、进入冷却，并构建结算请求。
///
/// 失败路径（`last_seen` 缺失）在正常流程中不可达；一旦发生，必须把已拿走的 pending
/// 与刚设置的冷却一并回滚，否则该轨道既拿不回候选证据、又要等满冷却才能重新挂起。
fn settle_pending(
    entry: &mut TrackEntry,
    track_id: u64,
    reason: SettleReason,
    now_ms: i64,
    target_in_current_frame: bool,
) -> Option<SettleRequest> {
    // 先取走唯一可能失败的部分，再做任何状态变更。
    let last_seen = entry.last_seen.take()?;
    let pending = entry.pending.take();
    entry.settled_at_ms = Some(now_ms);
    let pending = pending?;
    // 只统计真正产出的结算：退避窗口与生命周期上限都消费这个计数。
    entry.records_written = entry.records_written.saturating_add(1);
    Some(SettleRequest {
        track_id,
        reason,
        tracked_object: last_seen,
        last_seen_pts_ms: entry.last_seen_pts_ms,
        target_in_current_frame,
        candidate: pending.candidate,
    })
}

/// 同轨去重窗口（毫秒）：随已落库记录数指数退避，封顶 [`DEDUP_WINDOW_MAX_MS`]。
///
/// `observe` 与 `prune` 必须共用本函数，否则计数会在窗口中途被修剪重置。
fn dedup_window_ms(cooldown_ms: i64, records_written: u32) -> i64 {
    let shift = records_written.min(DEDUP_BACKOFF_MAX_SHIFT);
    (cooldown_ms << shift).min(DEDUP_WINDOW_MAX_MS)
}

/// 帧质量：宿主统一证据质量分（人脸质量 → 目标级质量 → 归一化人体框面积）。
fn frame_quality(obj: &TrackedObject) -> f32 {
    obj.evidence_quality_score()
}

#[cfg(test)]
mod tests {
    use super::*;
    use types::{FaceDetail, TrackedObject};

    fn face_object(track_id: u64, quality: f32) -> TrackedObject {
        TrackedObject {
            track_id,
            class_id: 0,
            label: "person".to_string(),
            confidence: 0.95,
            quality_score: Some(quality),
            bbox: BoundingBox::new(0.4, 0.3, 0.6, 0.6),
            face: Some(FaceDetail {
                bbox: BoundingBox::new(0.45, 0.35, 0.55, 0.5),
                confidence: 0.95,
                quality_score: Some(quality),
                fused_count: None,
                template_quality: None,
                template_mature: None,
                embedding: None,
            }),
            embedding: None,
            trajectory: vec![(0.5, 0.6)],
        }
    }

    /// 无脸人体目标：`quality_score` / `face` 均为空，质量分只能来自归一化面积。
    fn faceless_object(track_id: u64, bbox: BoundingBox) -> TrackedObject {
        TrackedObject {
            track_id,
            class_id: 0,
            label: "person".to_string(),
            confidence: 0.95,
            quality_score: None,
            bbox,
            face: None,
            embedding: None,
            trajectory: vec![(f64::from(bbox.x1), f64::from(bbox.y2))],
        }
    }

    fn empty_rules() -> Vec<DetectionRule> {
        Vec::new()
    }

    fn retain_request(action: &CaptureAction) -> &CandidateRetainRequest {
        match action {
            CaptureAction::RetainCandidate(request) => request,
            other => panic!("expected RetainCandidate, got {other:?}"),
        }
    }

    fn settle_request(action: &CaptureAction) -> &SettleRequest {
        match action {
            CaptureAction::Settle(request) => request,
            other => panic!("expected Settle, got {other:?}"),
        }
    }

    /// 构造驻留 112 字节（64 全景 + 16 人脸特写 + 32 人体特写）的候选证据。
    fn candidate_evidence(pts_ms: i64, quality: f32) -> CandidateEvidence {
        CandidateEvidence {
            full_jpeg: Arc::from(vec![1_u8; 64]),
            crop_jpeg: Some(Arc::from(vec![2_u8; 16])),
            body_crop_jpeg: Some(Arc::from(vec![3_u8; 32])),
            width: 640,
            height: 360,
            geometry: FrameGeometry {
                bbox: BoundingBox::new(0.4, 0.3, 0.6, 0.6),
                face_bbox: Some(BoundingBox::new(0.45, 0.35, 0.55, 0.5)),
                pts_ms,
                quality,
            },
            stream: EvidenceImageStream::Sub,
        }
    }

    #[test]
    fn low_quality_seed_still_retains_candidate() {
        let mut controller = CaptureSettleController::new();
        let rules = empty_rules();

        // 证据存在性优先于画面质量：宿主不做候选质量准入，弱帧同样必须留盘。
        // 否则该轨结算时无候选，只能回溯取证——主码流分析模式必然失败，整条通行记录丢失。
        let actions = controller.observe("algo", &[face_object(1, 0.30)], &rules, 1000);
        assert_eq!(actions.len(), 1, "低质量首帧同样必须留存候选");
        let request = retain_request(&actions[0]);
        assert_eq!(request.track_id, 1);
        assert_eq!(request.geometry.pts_ms, 1000);
        assert_eq!(request.geometry.quality, 0.30);

        // 首帧留存同样占用节流锚点：40ms 内的提升被抑制（峰值照常刷新，但不重复写盘）。
        let actions = controller.observe("algo", &[face_object(1, 0.62)], &rules, 1040);
        assert!(actions.is_empty(), "节流窗口内的提升不重复写盘");

        // 超过节流间隔后的提升重新放行。
        let actions = controller.observe("algo", &[face_object(1, 0.70)], &rules, 1300);
        assert_eq!(actions.len(), 1);
        assert_eq!(retain_request(&actions[0]).geometry.quality, 0.70);
    }

    #[test]
    fn low_quality_track_refreshes_candidate_on_peak_improvement() {
        let mut controller = CaptureSettleController::new();
        let rules = empty_rules();

        let actions = controller.observe("algo", &[face_object(2, 0.12)], &rules, 1000);
        assert_eq!(retain_request(&actions[0]).geometry.quality, 0.12);

        // 提升幅度不足 PEAK_DELTA(0.05)：不刷新峰值。
        let actions = controller.observe("algo", &[face_object(2, 0.15)], &rules, 1300);
        assert!(actions.is_empty(), "提升不足 PEAK_DELTA 不得刷新峰值");

        // 低质量区间内的有效提升：仍必须刷新峰值并留存（无质量准入下界）。
        let actions = controller.observe("algo", &[face_object(2, 0.30)], &rules, 1400);
        assert_eq!(actions.len(), 1);
        assert_eq!(retain_request(&actions[0]).geometry.quality, 0.30);
    }

    #[test]
    fn person_without_face_is_observed_and_settles() {
        // 抓拍以人体为主体：背身/低头（无 face）必须照样登记候选并在窗口到期时结算。
        let mut controller = CaptureSettleController::new();
        let obj = faceless_object(1, BoundingBox::new(0.4, 0.3, 0.6, 0.6));

        let actions = controller.observe("algo", std::slice::from_ref(&obj), &empty_rules(), 1000);
        assert_eq!(actions.len(), 1, "无脸目标必须登记峰值候选");
        let geometry = retain_request(&actions[0]).geometry;
        assert_eq!(geometry.face_bbox, None);
        assert!(
            (geometry.quality - 0.06).abs() < 1e-5,
            "无脸目标的质量分必须是归一化人体框面积"
        );
        assert_eq!(controller.pending_count(), 1);

        // 平台期门槛按面积量纲（0.06 < 0.5）不成立，因此走窗口到期结算。
        let actions = controller.observe("algo", &[obj], &empty_rules(), 2500);
        assert_eq!(actions.len(), 1, "窗口到期必须结算");
        match &actions[0] {
            CaptureAction::Settle(request) => {
                assert_eq!(request.reason, SettleReason::WindowExpired);
                assert!(request.candidate.is_none(), "本用例未登记候选字节");
            }
            other => panic!("期望结算动作，实际为 {other:?}"),
        }
    }

    #[test]
    fn faceless_person_peak_is_the_largest_body_box() {
        // 面积即"离镜头多近"：朝镜头走来时更大的框必须刷新峰值（复审看外观要用最清楚那帧）。
        let mut controller = CaptureSettleController::new();
        let near = faceless_object(4, BoundingBox::new(0.3, 0.2, 0.7, 0.8));
        let far = faceless_object(4, BoundingBox::new(0.45, 0.4, 0.55, 0.55));

        let actions = controller.observe("algo", &[far], &empty_rules(), 1000);
        assert!((retain_request(&actions[0]).geometry.quality - 0.015).abs() < 1e-5);

        // 远景 → 近景：面积提升必须刷新峰值。
        let actions = controller.observe("algo", &[near], &empty_rules(), 1200);
        assert_eq!(actions.len(), 1, "面积提升必须刷新峰值候选");
        assert!((retain_request(&actions[0]).geometry.quality - 0.24).abs() < 1e-5);
    }

    #[test]
    fn dedup_window_backs_off_with_settled_records() {
        // 滞留者封顶机制：每落一条记录，去重窗口翻倍（5s → 10s → 20s ...），
        // 否则一个站立 10 分钟的人会刷出上百条几乎重复的记录。
        assert_eq!(dedup_window_ms(CAPTURE_COOLDOWN_MS, 0), 5_000);
        assert_eq!(dedup_window_ms(CAPTURE_COOLDOWN_MS, 1), 10_000);
        assert_eq!(dedup_window_ms(CAPTURE_COOLDOWN_MS, 2), 20_000);
        assert_eq!(dedup_window_ms(CAPTURE_COOLDOWN_MS, 3), 40_000);
        assert_eq!(dedup_window_ms(CAPTURE_COOLDOWN_MS, 4), DEDUP_WINDOW_MAX_MS);
        assert_eq!(
            dedup_window_ms(CAPTURE_COOLDOWN_MS, 99),
            DEDUP_WINDOW_MAX_MS
        );

        let mut controller = CaptureSettleController::new();
        let obj = face_object(6, 0.80);
        // 第一次结算后窗口翻倍到 10s：5s 处不得重新挂起，10s 处才放行。
        controller.observe("algo", std::slice::from_ref(&obj), &empty_rules(), 1000);
        controller.observe("algo", &[], &empty_rules(), 2500);
        let actions = controller.observe("algo", std::slice::from_ref(&obj), &empty_rules(), 7500);
        assert!(actions.is_empty(), "退避窗口内不得重新挂起");
        assert_eq!(controller.pending_count(), 0);
        let actions = controller.observe("algo", &[obj], &empty_rules(), 12500);
        assert!(
            matches!(actions.first(), Some(CaptureAction::RetainCandidate(_))),
            "退避窗口届满必须重新登记候选"
        );
    }

    #[test]
    fn track_record_cap_stops_further_settles() {
        // 硬上限兜底：超过上限后即使退避窗口届满也不再产出新记录。
        let mut controller = CaptureSettleController::new();
        let obj = face_object(8, 0.80);
        controller.observe("algo", std::slice::from_ref(&obj), &empty_rules(), 1000);
        {
            let entry = controller
                .entries
                .get_mut("algo")
                .and_then(|tracks| tracks.get_mut(&8))
                .expect("轨道条目已登记");
            entry.records_written = MAX_RECORDS_PER_TRACK;
        }
        // 已挂起的结算照常收尾（本条即上限边界上的最后一条记录）。
        let actions = controller.observe("algo", &[], &empty_rules(), 3000);
        assert_eq!(actions.len(), 1, "已挂起的结算必须收尾，不得漏证据");
        assert_eq!(
            settle_request(&actions[0]).reason,
            SettleReason::TrackExited
        );

        // 之后即使远超退避窗口上限，只要目标还在持续出现，条目绝不能被 prune 丢弃重置
        let actions = controller.observe("algo", &[obj], &empty_rules(), 300_000);
        assert!(actions.is_empty(), "达到生命周期上限后不得再登记新挂起");
        assert_eq!(controller.pending_count(), 0);
        assert_eq!(
            controller
                .entries
                .get("algo")
                .and_then(|tracks| tracks.get(&8))
                .map(|e| e.records_written),
            Some(MAX_RECORDS_PER_TRACK + 1),
            "持续活跃的目标条目必须保留其 records_written 计数，不得被 prune 破坏重置为 0"
        );
    }

    #[test]
    fn peak_throttle_suppresses_rapid_retains() {
        let mut controller = CaptureSettleController::new();
        let rules = empty_rules();

        let actions = controller.observe("algo", &[face_object(1, 0.50)], &rules, 1000);
        assert_eq!(actions.len(), 1, "首帧达标必须留盘");

        // 40ms 内的提升被节流（<200ms）。
        let actions = controller.observe("algo", &[face_object(1, 0.70)], &rules, 1040);
        assert!(actions.is_empty(), "节流窗口内的提升不重复写盘");

        // 超过节流间隔后的下一次提升重新放行。
        let actions = controller.observe("algo", &[face_object(1, 0.80)], &rules, 1240);
        assert_eq!(actions.len(), 1, "超过节流间隔的提升必须留盘");
        assert_eq!(retain_request(&actions[0]).geometry.quality, 0.80);
    }

    #[test]
    fn plateau_settle_takes_peak_geometry() {
        let mut controller = CaptureSettleController::new();
        let rules = empty_rules();

        // 序列 [0.45, 0.70, 0.85, 0.80, 0.75] → 平台期结算取 0.85 帧几何。
        controller.observe("algo", &[face_object(7, 0.45)], &rules, 1000);
        controller.observe("algo", &[face_object(7, 0.70)], &rules, 1040);
        controller.observe("algo", &[face_object(7, 0.85)], &rules, 1080);
        controller.observe("algo", &[face_object(7, 0.80)], &rules, 1120);
        let actions = controller.observe("algo", &[face_object(7, 0.75)], &rules, 1400);

        assert_eq!(actions.len(), 1);
        let settle = settle_request(&actions[0]);
        assert_eq!(settle.reason, SettleReason::QualityPlateau);
        assert_eq!(settle.track_id, 7);
        assert_eq!(
            settle.tracked_object.track_id, 7,
            "事件载体必须是最近一帧对象"
        );
        assert_eq!(
            settle.tracked_object.bbox,
            BoundingBox::new(0.4, 0.3, 0.6, 0.6)
        );
        assert_eq!(settle.last_seen_pts_ms, 1400);
        assert!(settle.target_in_current_frame);
    }

    #[test]
    fn template_mature_settles_before_quality_plateau() {
        let mut controller = CaptureSettleController::new();
        let rules = empty_rules();

        let actions = controller.observe("algo", &[face_object(12, 0.80)], &rules, 1000);
        assert_eq!(actions.len(), 1);
        assert_eq!(
            controller.record_candidate("algo", 12, candidate_evidence(1000, 0.80)),
            RecordCandidateOutcome::Accepted
        );

        let mut mature_object = face_object(12, 0.80);
        mature_object
            .face
            .as_mut()
            .expect("测试对象必须带人脸")
            .template_mature = Some(true);
        let actions = controller.observe("algo", &[mature_object], &rules, 1100);
        assert_eq!(actions.len(), 1);
        let settle = settle_request(&actions[0]);
        assert_eq!(settle.reason, SettleReason::TemplateMature);
        assert!(settle.candidate.is_some(), "成熟握手应优先携带已有候选");
    }

    #[test]
    fn peak_geometry_is_captured_via_candidate_geometry() {
        let mut controller = CaptureSettleController::new();
        let rules = empty_rules();

        // 0.85 峰值帧编码留存 → 后续帧不再刷新峰值。
        let actions = controller.observe("algo", &[face_object(3, 0.85)], &rules, 1000);
        assert_eq!(actions.len(), 1);
        assert_eq!(
            controller.record_candidate("algo", 3, candidate_evidence(1000, 0.85)),
            RecordCandidateOutcome::Accepted
        );

        // 弱帧拖到平台期结算：候选证据必须原样携带峰值帧几何（INV-3）。
        let actions = controller.observe("algo", &[face_object(3, 0.60)], &rules, 1400);
        let settle = settle_request(&actions[0]);
        assert_eq!(settle.reason, SettleReason::QualityPlateau);
        let candidate = settle.candidate.as_ref().expect("结算必须携带候选");
        assert_eq!(candidate.geometry.pts_ms, 1000);
        assert_eq!(candidate.geometry.quality, 0.85);
        assert_eq!(candidate.resident_bytes(), 112);
    }

    #[test]
    fn window_settle_when_quality_never_reaches_plateau_gate() {
        let mut controller = CaptureSettleController::new();
        let rules = empty_rules();

        controller.observe("algo", &[face_object(9, 0.45)], &rules, 1000);
        controller.observe("algo", &[face_object(9, 0.45)], &rules, 1500);
        let actions = controller.observe("algo", &[face_object(9, 0.45)], &rules, 2500);

        assert_eq!(actions.len(), 1);
        let settle = settle_request(&actions[0]);
        assert_eq!(settle.reason, SettleReason::WindowExpired);
        assert_eq!(settle.last_seen_pts_ms, 2500);
    }

    #[test]
    fn exit_settle_uses_last_seen_and_rearms_after_backoff_window() {
        let mut controller = CaptureSettleController::new();
        let rules = empty_rules();

        controller.observe("algo", &[face_object(11, 0.70)], &rules, 1000);
        // 目标离开 ROI（当帧无对象）：超出离场宽限后才结算，事件载体为最后可见帧对象。
        let grace = controller.observe("algo", &[], &rules, 1040);
        assert!(grace.is_empty(), "宽限窗口内不得结算：离场尚未被确认");
        let actions = controller.observe("algo", &[], &rules, 1000 + EXIT_GRACE_MS);
        assert_eq!(actions.len(), 1);
        let settle = settle_request(&actions[0]);
        assert_eq!(settle.reason, SettleReason::TrackExited);
        assert_eq!(settle.last_seen_pts_ms, 1000);
        assert!(!settle.target_in_current_frame);
        assert_eq!(settle.tracked_object.track_id, 11);

        // 退避窗口内同轨重入：不重新挂起（本次结算已使窗口从 5s 翻倍到 10s）。
        let actions = controller.observe("algo", &[face_object(11, 0.70)], &rules, 3000);
        assert!(actions.is_empty());
        assert_eq!(controller.pending_count(), 0);
        let actions = controller.observe("algo", &[face_object(11, 0.70)], &rules, 7000);
        assert!(actions.is_empty(), "退避窗口内不得重新挂起");

        // 退避窗口届满重新挂起（结算在 1400，窗口 10s 至 11400）。
        let actions = controller.observe("algo", &[face_object(11, 0.70)], &rules, 11_400);
        assert_eq!(actions.len(), 1, "退避窗口届满重入必须重新留盘");
    }

    #[test]
    fn transient_face_loss_does_not_burn_cooldown() {
        // 回归：背身/低头导致一两帧丢脸时，`is_capture_triggering` 回复 false，
        // 但这不等于“离开 ROI”。旧逻辑当帧即离场结算，烧掉 5s 冷却且把证据从
        // 峰值候选降级为当帧回退图。
        let mut controller = CaptureSettleController::new();
        let rules = empty_rules();

        controller.observe("algo", &[face_object(41, 0.80)], &rules, 1000);
        assert_eq!(
            controller.record_candidate("algo", 41, candidate_evidence(1000, 0.80)),
            RecordCandidateOutcome::Accepted
        );

        // 丢脸两帧（无 face）：未超宽限，不得结算，候选必须还在。
        for pts in [1040, 1080] {
            let mut faceless = face_object(41, 0.80);
            faceless.face = None;
            let actions = controller.observe("algo", &[faceless], &rules, pts);
            assert!(actions.is_empty(), "瞬时丢脸不得结算 (pts={pts})");
        }
        assert_eq!(controller.pending_count(), 1);
        assert_eq!(controller.pending_bytes(), 112, "候选不得因瞬时丢脸被释放");

        // 人脸恢复：轨道仍处 pending，平台期后结算仍携带峰值候选。
        let actions = controller.observe("algo", &[face_object(41, 0.78)], &rules, 1400);
        assert_eq!(actions.len(), 1);
        let settle = settle_request(&actions[0]);
        assert_eq!(settle.reason, SettleReason::QualityPlateau);
        assert_eq!(
            settle
                .candidate
                .as_ref()
                .expect("恢复后结算必须仍携带峰值候选")
                .geometry
                .quality,
            0.80
        );
    }

    #[test]
    fn exit_grace_is_measured_from_last_trigger_not_last_seen_at() {
        // 宽限必须基于“最后一次真正触发”的时标：若从挂起首帧计时，
        // 持续触发的长驻目标会提前满足宽限，在下一帧丢触发时立即误结算。
        // 用长结算窗口 + 低质量帧把轨道停在 pending，以便单独观察离场宽限。
        let mut controller = CaptureSettleController::with_config(CaptureSettleConfig {
            settle_window_ms: 10_000,
            ..CaptureSettleConfig::default()
        });
        let rules = empty_rules();

        controller.observe("algo", &[face_object(51, 0.45)], &rules, 1000);
        // 持续触发到 3000（质量未达平台期门槛，轨道保持 pending）。
        controller.observe("algo", &[face_object(51, 0.45)], &rules, 3000);
        assert_eq!(controller.pending_count(), 1);
        // 刚丢触发 100ms → 不结算。若宽限错误地从首帧计时，此处会提前结算。
        assert!(controller.observe("algo", &[], &rules, 3100).is_empty());
        // 超过宽限 → 以 3000 为最后可见时标结算。
        let actions = controller.observe("algo", &[], &rules, 3000 + EXIT_GRACE_MS);
        assert_eq!(actions.len(), 1);
        assert_eq!(settle_request(&actions[0]).last_seen_pts_ms, 3000);
    }

    #[test]
    fn record_candidate_replace_and_dismiss() {
        let mut controller = CaptureSettleController::new();
        let rules = empty_rules();
        controller.observe("algo", &[face_object(5, 0.80)], &rules, 1000);

        assert_eq!(
            controller.record_candidate("algo", 5, candidate_evidence(1000, 0.80)),
            RecordCandidateOutcome::Accepted
        );
        // 覆盖写：新候选替换旧候选，驻留始终为单份，不随时间累积。
        let replacement = candidate_evidence(1040, 0.85);
        let replacement_bytes = replacement.resident_bytes();
        assert_eq!(
            controller.record_candidate("algo", 5, replacement),
            RecordCandidateOutcome::Accepted
        );
        assert_eq!(controller.pending_bytes(), replacement_bytes);

        // 结算后再登记：必须被拒绝（编码产物由调用方丢弃）。
        let actions = controller.observe("algo", &[face_object(5, 0.81)], &rules, 1400);
        assert!(matches!(actions[0], CaptureAction::Settle(_)));
        assert_eq!(
            controller.record_candidate("algo", 5, candidate_evidence(1400, 0.81)),
            RecordCandidateOutcome::Dismissed
        );
    }

    #[test]
    fn budget_rejection_keeps_existing_candidate() {
        let mut controller = CaptureSettleController::with_config(CaptureSettleConfig {
            candidate_budget_bytes: 112,
            ..CaptureSettleConfig::default()
        });
        let rules = empty_rules();
        controller.observe("algo", &[face_object(31, 0.80)], &rules, 1000);

        assert_eq!(
            controller.record_candidate("algo", 31, candidate_evidence(1000, 0.80)),
            RecordCandidateOutcome::Accepted
        );
        assert_eq!(controller.pending_bytes(), 112);

        // 超预算新候选：拒绝登记，既有候选与峰值几何保持不变。
        let oversized = CandidateEvidence {
            full_jpeg: Arc::from(vec![3_u8; 200]),
            ..candidate_evidence(1040, 0.90)
        };
        assert_eq!(
            controller.record_candidate("algo", 31, oversized),
            RecordCandidateOutcome::RejectedBudget
        );
        assert_eq!(controller.pending_bytes(), 112, "拒绝后既有候选保持不变");
        assert_eq!(controller.budget_rejections(), 1);

        // 同轨替换按“扣旧 + 加新”核算，不因旧候选占额而误拒。
        assert_eq!(
            controller.record_candidate("algo", 31, candidate_evidence(1040, 0.85)),
            RecordCandidateOutcome::Accepted
        );
        assert_eq!(controller.pending_bytes(), 112);
    }

    #[test]
    fn clear_releases_pending_candidates() {
        let mut controller = CaptureSettleController::new();
        let rules = empty_rules();
        controller.observe("algo", &[face_object(21, 0.90)], &rules, 1000);
        assert_eq!(
            controller.record_candidate("algo", 21, candidate_evidence(1000, 0.90)),
            RecordCandidateOutcome::Accepted
        );
        assert!(controller.pending_bytes() > 0);

        controller.clear();
        assert_eq!(controller.pending_count(), 0);
        assert_eq!(controller.pending_bytes(), 0, "清轨必须同步释放内存候选");
    }

    #[test]
    fn zero_window_degrades_to_immediate_settle_without_candidate() {
        let mut controller = CaptureSettleController::with_config(CaptureSettleConfig {
            settle_window_ms: 0,
            ..CaptureSettleConfig::default()
        });
        let rules = empty_rules();

        // 进入即结算，不产出候选留存动作
        let actions = controller.observe("algo", &[face_object(5, 0.90)], &rules, 1000);
        assert_eq!(actions.len(), 1);
        let settle = settle_request(&actions[0]);
        assert_eq!(settle.track_id, 5);
        assert_eq!(settle.reason, SettleReason::WindowExpired);
        assert!(settle.candidate.is_none());
        assert!(settle.target_in_current_frame);

        // 去重窗口内不重复结算
        let cooldown = controller.observe("algo", &[face_object(5, 0.90)], &rules, 1100);
        assert!(cooldown.is_empty());

        // 首条记录使窗口退避到 10s：基础冷却（5s）过后仍然不得重复结算
        let backed_off = controller.observe("algo", &[face_object(5, 0.90)], &rules, 7000);
        assert!(backed_off.is_empty(), "退避窗口必须比基础冷却更宽");

        // 退避窗口过后重入：再次立即结算
        let after = controller.observe("algo", &[face_object(5, 0.90)], &rules, 11_000);
        assert_eq!(after.len(), 1);
        assert!(matches!(after[0], CaptureAction::Settle(_)));
    }

    #[test]
    fn algorithms_are_isolated() {
        let mut controller = CaptureSettleController::new();
        let rules = empty_rules();
        controller.observe("algo_a", &[face_object(1, 0.80)], &rules, 1000);

        // 另一算法的空帧不得触发 algo_a 的离场结算。
        let actions = controller.observe("algo_b", &[], &rules, 1040);
        assert!(actions.is_empty());
        assert_eq!(controller.pending_count(), 1);

        controller.clear_algorithm("algo_b");
        assert_eq!(controller.pending_count(), 1);
        controller.clear_algorithm("algo_a");
        assert_eq!(controller.pending_count(), 0);
        assert_eq!(controller.pending_bytes(), 0, "清算法必须同步释放内存候选");
    }
}
