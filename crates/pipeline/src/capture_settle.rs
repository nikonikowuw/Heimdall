//! 通行抓拍结算状态机（CaptureSettle）
//!
//! 识别类算法（`AlgorithmKind::Recognition`）的抓拍语义从「进入 ROI 首帧立即抓拍」升级为
//! 「逐帧跟踪峰值 → 结算时刻一次性产出证据」。本模块只做纯同步状态推进，禁止任何 IO 与
//! `.await`；候选编码（内存驻留字节）与结算写盘由 `pump.rs` 在分析循环中执行。
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
pub const SETTLE_MIN_QUALITY: f32 = 0.50;
/// 峰值刷新门限：新帧质量需超过当前峰值该幅度才刷新。
pub const PEAK_DELTA: f32 = 0.05;
/// 候选留存的最低帧质量（防垃圾图）。
pub const CANDIDATE_MIN_QUALITY: f32 = 0.40;
/// 候选留存节流（毫秒）：同一轨道两次编码留存的最小间隔。
pub const CANDIDATE_THROTTLE_MS: i64 = 200;
/// 峰值平台判定窗口（毫秒，≈8 帧 @25fps）。
pub const PLATEAU_MS: i64 = 320;
/// 同一轨道两次结算之间的冷却（毫秒），与旧版抓拍防刷屏语义一致。
pub const CAPTURE_COOLDOWN_MS: i64 = 5000;
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
    pub candidate_min_quality: f32,
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
            candidate_min_quality: CANDIDATE_MIN_QUALITY,
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
    /// 人脸框（若该帧挂载人脸）
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
    /// 峰值帧特写 JPEG 字节（无有效裁剪时与全景同源）
    pub crop_jpeg: Arc<[u8]>,
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
        self.full_jpeg.len() + self.crop_jpeg.len()
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
            // 冷却未过的同轨重入：不重复挂起，也不刷新事件载体。
            // 必须在 `obj.clone()` 之前判定，否则每帧都要为冷却内的轨道白付一次克隆。
            if entry.pending.is_none()
                && entry
                    .settled_at_ms
                    .is_some_and(|settled| now_ms - settled < config.cooldown_ms)
            {
                continue;
            }
            seen.push(obj.track_id);

            let quality = frame_quality(obj);
            let template_mature = obj
                .face
                .as_ref()
                .is_some_and(|face| face.template_mature == Some(true));
            let geometry = FrameGeometry {
                bbox: obj.bbox,
                face_bbox: obj.face.as_ref().map(|face| face.bbox),
                pts_ms: now_ms,
                quality,
            };

            // 唯一无法避免的每触发帧克隆：结算发生在未来某帧，届时必须能拿到事件载体。
            entry.last_seen = Some(obj.clone());
            entry.last_seen_pts_ms = now_ms;
            entry.last_trigger_pts_ms = now_ms;

            match entry.pending.as_mut() {
                None => {
                    // 兼容退化模式 (`SETTLE_WINDOW_MS <= 0`)：不做峰值留存，进入即结算，
                    // 等价旧「首帧抓拍」语义（冷却仍然生效）。
                    if config.settle_window_ms <= 0 {
                        entry.settled_at_ms = Some(now_ms);
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
                    } else if quality >= config.candidate_min_quality {
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
                    if quality > pending.best.quality + config.peak_delta
                        && quality >= config.candidate_min_quality
                    {
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

    /// 修剪已出冷却且无 pending 的条目；**不**删除空的内层表。
    ///
    /// 保留空桶是为了让稳态下的键路径零分配：`String` 键只在首次见到该算法时分配一次，
    /// 之后每帧都是 `get_mut(&str)` 命中；桶数上限为单路并发算法实例数，天然有界。
    fn prune(&mut self, now_ms: i64) {
        let cooldown = self.config.cooldown_ms;
        for tracks in self.entries.values_mut() {
            tracks.retain(|_, entry| {
                if entry.pending.is_some() {
                    return true;
                }
                entry
                    .settled_at_ms
                    .is_some_and(|settled| now_ms - settled < cooldown + ENTRY_GRACE_MS)
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
    Some(SettleRequest {
        track_id,
        reason,
        tracked_object: last_seen,
        last_seen_pts_ms: entry.last_seen_pts_ms,
        target_in_current_frame,
        candidate: pending.candidate,
    })
}

/// 帧质量：优先人脸质量分，回退目标级质量分。
fn frame_quality(obj: &TrackedObject) -> f32 {
    obj.face
        .as_ref()
        .and_then(|face| face.quality_score)
        .or(obj.quality_score)
        .unwrap_or(0.0)
        .clamp(0.0, 1.0)
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

    /// 构造驻留 80 字节（64 全景 + 16 特写）的候选证据。
    fn candidate_evidence(pts_ms: i64, quality: f32) -> CandidateEvidence {
        CandidateEvidence {
            full_jpeg: Arc::from(vec![1_u8; 64]),
            crop_jpeg: Arc::from(vec![2_u8; 16]),
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
    fn seed_below_candidate_min_quality_does_not_retain() {
        let mut controller = CaptureSettleController::new();
        let rules = empty_rules();

        // 弱帧播种：允许挂起（作为几何兜底），但不得触发候选写盘。
        let actions = controller.observe("algo", &[face_object(1, 0.30)], &rules, 1000);
        assert!(actions.is_empty(), "低于候选门限的帧不应触发留盘");

        // 质量达标帧：触发一次留盘。
        let actions = controller.observe("algo", &[face_object(1, 0.62)], &rules, 1040);
        assert_eq!(actions.len(), 1);
        let request = retain_request(&actions[0]);
        assert_eq!(request.track_id, 1);
        assert_eq!(request.geometry.pts_ms, 1040);
        assert_eq!(request.geometry.quality, 0.62);
    }

    #[test]
    fn person_without_face_is_not_observed() {
        let mut controller = CaptureSettleController::new();
        let mut obj = face_object(1, 0.90);
        obj.face = None;
        let actions = controller.observe("algo", &[obj], &empty_rules(), 1000);
        assert!(actions.is_empty());
        assert_eq!(controller.pending_count(), 0);
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
        assert_eq!(candidate.resident_bytes(), 80);
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
    fn exit_settle_uses_last_seen_and_rearms_after_cooldown() {
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

        // 冷却期内同轨重入：不重新挂起。
        let actions = controller.observe("algo", &[face_object(11, 0.70)], &rules, 3000);
        assert!(actions.is_empty());
        assert_eq!(controller.pending_count(), 0);

        // 冷却期满重新挂起（结算在 1400，冷却至 6400）。
        let actions = controller.observe("algo", &[face_object(11, 0.70)], &rules, 7000);
        assert_eq!(actions.len(), 1, "冷却期满重入必须重新留盘");
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
        assert_eq!(controller.pending_bytes(), 80, "候选不得因瞬时丢脸被释放");

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
            candidate_budget_bytes: 100,
            ..CaptureSettleConfig::default()
        });
        let rules = empty_rules();
        controller.observe("algo", &[face_object(31, 0.80)], &rules, 1000);

        assert_eq!(
            controller.record_candidate("algo", 31, candidate_evidence(1000, 0.80)),
            RecordCandidateOutcome::Accepted
        );
        assert_eq!(controller.pending_bytes(), 80);

        // 超预算新候选：拒绝登记，既有候选与峰值几何保持不变。
        let oversized = CandidateEvidence {
            full_jpeg: Arc::from(vec![3_u8; 200]),
            ..candidate_evidence(1040, 0.90)
        };
        assert_eq!(
            controller.record_candidate("algo", 31, oversized),
            RecordCandidateOutcome::RejectedBudget
        );
        assert_eq!(controller.pending_bytes(), 80, "拒绝后既有候选保持不变");
        assert_eq!(controller.budget_rejections(), 1);

        // 同轨替换按“扣旧 + 加新”核算，不因旧候选占额而误拒。
        assert_eq!(
            controller.record_candidate("algo", 31, candidate_evidence(1040, 0.85)),
            RecordCandidateOutcome::Accepted
        );
        assert_eq!(controller.pending_bytes(), 80);
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

        // 冷却期内不重复结算
        let cooldown = controller.observe("algo", &[face_object(5, 0.90)], &rules, 1100);
        assert!(cooldown.is_empty());

        // 冷却过后重入：再次立即结算
        let after = controller.observe("algo", &[face_object(5, 0.90)], &rules, 7000);
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
