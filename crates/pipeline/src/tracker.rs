//! 纯 CPU ByteTrack 多目标航迹跟踪器。
//!
//! 跟踪器只处理检测元数据，不触碰视频像素或硬件 buffer。实现包含：
//! - 8 维尺度自适应 Kalman 预测；
//! - 高分检测优先、低分检测补救的两阶段关联；
//! - CPU Hungarian 全局分配，避免贪心匹配造成的 ID 抢占；
//! - Tracked/Lost/Removed 生命周期与按源帧 PTS 的超时清理；
//! - 有界轨迹历史和告警冷却状态。

use std::collections::{HashMap, HashSet};

use types::{BoundingBox, Detection, DetectionRuleRole, FaceDetail, FaceEmbedding, TrackedObject};

const DEFAULT_FRAME_INTERVAL_MS: i64 = 100;
const DEFAULT_MAX_LOST_MS: i64 = 3_000;
const DEFAULT_LOW_SCORE: f32 = 0.10;
const DEFAULT_TRACK_SCORE: f32 = 0.45;
const DEFAULT_NEW_TRACK_SCORE: f32 = 0.45;
const DEFAULT_HIGH_MATCH_IOU: f32 = 0.20;
const DEFAULT_LOW_MATCH_IOU: f32 = 0.30;
const DEFAULT_MAX_TRAJECTORY_LEN: usize = 30;
const MAX_TRACKING_DETECTIONS: usize = 256;
const MAX_TRACKS: usize = 512;
const EPSILON: f32 = 1e-6;

/// 纯 CPU ByteTrack 配置。
#[derive(Debug, Clone, Copy)]
pub struct ByteTrackConfig {
    /// 低于此分数的候选框不进入跟踪器。
    pub low_score: f32,
    /// 高分/低分检测分界线。
    pub track_score: f32,
    /// 只有达到此分数的未匹配检测才能创建新航迹。
    pub new_track_score: f32,
    /// 第一阶段高分检测关联所需的最小 IoU。
    pub high_match_iou: f32,
    /// 第二阶段低分检测补救所需的最小 IoU。
    pub low_match_iou: f32,
    /// 航迹从 Lost 转 Removed 的最大失联时长。
    pub max_lost_ms: i64,
    /// 无 PTS 调用时使用的默认帧间隔。
    pub default_frame_interval_ms: i64,
    /// 底边中心轨迹最大保留点数。
    pub max_trajectory_len: usize,
}

impl Default for ByteTrackConfig {
    fn default() -> Self {
        Self {
            low_score: DEFAULT_LOW_SCORE,
            track_score: DEFAULT_TRACK_SCORE,
            new_track_score: DEFAULT_NEW_TRACK_SCORE,
            high_match_iou: DEFAULT_HIGH_MATCH_IOU,
            low_match_iou: DEFAULT_LOW_MATCH_IOU,
            max_lost_ms: DEFAULT_MAX_LOST_MS,
            default_frame_interval_ms: DEFAULT_FRAME_INTERVAL_MS,
            max_trajectory_len: DEFAULT_MAX_TRAJECTORY_LEN,
        }
    }
}

/// 单次更新的处理结果，用于区分已应用、迟到和被拒绝的推理结果。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrackUpdateStatus {
    /// 结果已推进 Kalman、关联和规则状态。
    Applied,
    /// PTS 不晚于上一帧，结果被忽略。
    Stale,
    /// 结果违反跟踪输入上限或检测契约，状态保持不变。
    Rejected,
}

/// ByteTrack 一次更新的结果。
#[derive(Debug, Clone)]
pub struct TrackUpdateResult {
    pub status: TrackUpdateStatus,
    pub objects: Vec<TrackedObject>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TrackStatus {
    Tracked,
    Lost,
    Removed,
}

/// 8 维状态：[中心 x、中心 y、宽高比、高度、对应四维速度]。
#[derive(Debug, Clone)]
struct KalmanBoxFilter {
    mean: [f32; 8],
    covariance: [f32; 64],
}

impl KalmanBoxFilter {
    fn new(bbox: BoundingBox) -> Self {
        let [cx, cy, aspect, height] = measurement_from_bbox(&bbox);
        let height = height.max(1e-4);
        let pos_std = 0.05 * height;
        let vel_std = 10.0 * pos_std;
        let pos_var = pos_std * pos_std;
        let vel_var = vel_std * vel_std;

        let mut covariance = [0.0; 64];
        covariance[0] = pos_var;
        covariance[9] = pos_var;
        covariance[18] = 1e-4;
        covariance[27] = pos_var;
        covariance[36] = vel_var;
        covariance[45] = vel_var;
        covariance[54] = 1e-4;
        covariance[63] = vel_var;

        Self {
            mean: [cx, cy, aspect, height, 0.0, 0.0, 0.0, 0.0],
            covariance,
        }
    }

    fn predict(&mut self, dt_frames: f32) {
        let dt = dt_frames.clamp(0.25, 10.0);
        let dt_squared = dt * dt;
        for index in 0..4 {
            self.mean[index] += self.mean[index + 4] * dt;
        }

        let mut next = [0.0; 64];
        for row in 0..4 {
            for col in 0..4 {
                let pp = self.covariance[row * 8 + col];
                let pv = self.covariance[row * 8 + col + 4];
                let vp = self.covariance[(row + 4) * 8 + col];
                let vv = self.covariance[(row + 4) * 8 + col + 4];

                next[row * 8 + col] = pp + dt * (pv + vp) + dt_squared * vv;
                next[row * 8 + col + 4] = pv + dt * vv;
                next[(row + 4) * 8 + col] = vp + dt * vv;
                next[(row + 4) * 8 + col + 4] = vv;
            }
        }

        let height = self.mean[3].abs().max(1e-4);
        let process_pos_std = 0.05 * height * dt;
        let process_vel_std = 0.00625 * height * dt;
        let pos_var = process_pos_std * process_pos_std;
        let vel_var = process_vel_std * process_vel_std;
        let dt_var = (1e-2 * dt) * (1e-2 * dt);

        next[0] += pos_var;
        next[9] += pos_var;
        next[18] += dt_var;
        next[27] += pos_var;
        next[36] += vel_var;
        next[45] += vel_var;
        next[54] += dt_var;
        next[63] += vel_var;
        self.covariance = next;
    }

    fn update(&mut self, bbox: BoundingBox) {
        let measurement = measurement_from_bbox(&bbox);
        let height = measurement[3].max(1e-4);
        let pos_std = 0.05 * height;
        let position_var = pos_std * pos_std;
        let measurement_vars = [position_var, position_var, 1e-4, position_var];

        // 按观测分量顺序执行标量 Kalman 更新，避免在热路径中做矩阵求逆。
        for m_idx in 0..4 {
            let innovation = measurement[m_idx] - self.mean[m_idx];
            let innovation_var = self.covariance[m_idx * 8 + m_idx] + measurement_vars[m_idx];
            if !innovation_var.is_finite() || innovation_var <= 1e-12 {
                continue;
            }
            let inverse = 1.0 / innovation_var;
            let mut gain = [0.0; 8];
            for (row, g) in gain.iter_mut().enumerate() {
                *g = self.covariance[row * 8 + m_idx] * inverse;
                self.mean[row] += *g * innovation;
            }

            let row_offset = m_idx * 8;
            let mut observed_row = [0.0; 8];
            observed_row.copy_from_slice(&self.covariance[row_offset..row_offset + 8]);

            for (row, &g) in gain.iter().enumerate() {
                let cov_row = row * 8;
                for (col, &obs) in observed_row.iter().enumerate() {
                    self.covariance[cov_row + col] -= g * obs;
                }
            }
        }

        for row in 0..8 {
            for column in (row + 1)..8 {
                let value =
                    0.5 * (self.covariance[row * 8 + column] + self.covariance[column * 8 + row]);
                self.covariance[row * 8 + column] = value;
                self.covariance[column * 8 + row] = value;
            }
        }
    }

    fn bbox(&self) -> BoundingBox {
        let height = self.mean[3].abs().max(1e-4);
        let width = (self.mean[2].abs() * height).max(1e-4);
        let half_w = width * 0.5;
        let half_h = height * 0.5;
        BoundingBox::new(
            self.mean[0] - half_w,
            self.mean[1] - half_h,
            self.mean[0] + half_w,
            self.mean[1] + half_h,
        )
    }
}

#[derive(Debug, Clone)]
struct TrackState {
    track_id: u64,
    class_id: usize,
    label: String,
    confidence: f32,
    quality_score: Option<f32>,
    embedding: Option<FaceEmbedding>,
    bbox: BoundingBox,
    predicted_bbox: BoundingBox,
    face: Option<FaceDetail>,
    kalman: KalmanBoxFilter,
    trajectory: Vec<(f64, f64)>,
    status: TrackStatus,
    lost_for_ms: i64,
}

impl TrackState {
    fn new(
        track_id: u64,
        detection: Detection,
        embedding: Option<FaceEmbedding>,
        max_trajectory_len: usize,
    ) -> Self {
        let bbox = sanitize_bbox(detection.bbox);
        let kalman = KalmanBoxFilter::new(bbox);
        let mut track = Self {
            track_id,
            class_id: detection.class_id,
            label: detection.label,
            confidence: detection.confidence,
            quality_score: detection.quality_score,
            embedding,
            bbox,
            predicted_bbox: bbox,
            face: detection.face,
            kalman,
            trajectory: Vec::with_capacity(max_trajectory_len.min(32)),
            status: TrackStatus::Tracked,
            lost_for_ms: 0,
        };
        track.attach_embedding_to_face();
        track.push_trajectory(max_trajectory_len);
        track
    }

    fn update(
        &mut self,
        detection: &Detection,
        embedding: Option<&FaceEmbedding>,
        max_trajectory_len: usize,
    ) {
        let bbox = sanitize_bbox(detection.bbox);
        self.kalman.update(bbox);
        self.bbox = bbox;
        self.predicted_bbox = sanitize_bbox(self.kalman.bbox());
        self.confidence = detection.confidence;
        self.quality_score = detection.quality_score;
        self.class_id = detection.class_id;
        self.label.clone_from(&detection.label);
        self.face.clone_from(&detection.face);
        if let Some(face_emb) = self.face.as_ref().and_then(|f| f.embedding.as_ref()) {
            self.embedding = Some(face_emb.clone());
        } else if let Some(emb) = embedding {
            self.embedding = Some((*emb).clone());
        }
        self.attach_embedding_to_face();
        self.status = TrackStatus::Tracked;
        self.lost_for_ms = 0;
        self.push_trajectory(max_trajectory_len);
    }

    fn mark_lost(&mut self, elapsed_ms: i64) {
        self.status = TrackStatus::Lost;
        self.lost_for_ms = self.lost_for_ms.saturating_add(elapsed_ms.max(0));
    }

    fn attach_embedding_to_face(&mut self) {
        if let Some(face) = &mut self.face {
            if face.embedding.is_none() {
                face.embedding = self.embedding.clone();
            }
        }
    }

    fn push_trajectory(&mut self, max_trajectory_len: usize) {
        if max_trajectory_len == 0 {
            return;
        }
        self.trajectory.push(self.bbox.bottom_center());
        if self.trajectory.len() > max_trajectory_len {
            self.trajectory.remove(0);
        }
    }

    fn to_public(&self) -> TrackedObject {
        TrackedObject {
            track_id: self.track_id,
            class_id: self.class_id,
            label: self.label.clone(),
            confidence: self.confidence,
            quality_score: self.quality_score,
            bbox: sanitize_bbox(self.bbox),
            face: self.face.clone(),
            embedding: self.embedding.clone(),
            trajectory: self.trajectory.clone(),
        }
    }
}

/// 规则报警防刷屏冷却目标键。
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum CooldownTarget {
    Rule(usize, u8),
    Named(String),
}

/// 纯 CPU ByteTrack 跟踪器。
#[derive(Debug)]
pub struct ByteTrack {
    config: ByteTrackConfig,
    next_track_id: u64,
    tracks: Vec<TrackState>,
    last_timestamp_ms: Option<i64>,
    alarm_cooldowns: HashMap<(u64, CooldownTarget), i64>,
}

impl Default for ByteTrack {
    fn default() -> Self {
        Self::new()
    }
}

impl ByteTrack {
    pub fn new() -> Self {
        Self::with_config(ByteTrackConfig::default())
    }

    pub fn with_config(mut config: ByteTrackConfig) -> Self {
        config.low_score = config.low_score.clamp(0.0, 1.0);
        config.track_score = config.track_score.clamp(config.low_score, 1.0);
        config.new_track_score = config.new_track_score.clamp(config.track_score, 1.0);
        config.high_match_iou = config.high_match_iou.clamp(0.0, 1.0);
        config.low_match_iou = config.low_match_iou.clamp(0.0, 1.0);
        config.max_lost_ms = config.max_lost_ms.max(0);
        config.default_frame_interval_ms = config.default_frame_interval_ms.max(1);

        Self {
            config,
            next_track_id: 1,
            tracks: Vec::new(),
            last_timestamp_ms: None,
            alarm_cooldowns: HashMap::new(),
        }
    }

    /// 清除当前会话的航迹、PTS 和冷却状态，但保留递增 ID，避免同一上下文内复用旧 ID。
    pub fn clear(&mut self) {
        self.tracks.clear();
        self.last_timestamp_ms = None;
        self.alarm_cooldowns.clear();
    }

    /// 在没有成功推理结果时，按源帧 PTS 清理已经超时的航迹。
    pub fn expire_if_stale_at(&mut self, timestamp_ms: i64) -> bool {
        let Some(previous) = self.last_timestamp_ms else {
            return false;
        };
        if self.tracks.is_empty()
            || timestamp_ms <= previous
            || timestamp_ms.saturating_sub(previous) <= self.config.max_lost_ms
        {
            return false;
        }

        self.clear();
        true
    }

    /// 输入检测列表，使用默认帧间隔。兼容规则测试和离线调用。
    pub fn update(&mut self, detections: Vec<Detection>) -> Vec<TrackedObject> {
        let embeddings = vec![None; detections.len()];
        self.update_with_embeddings(detections, embeddings)
    }

    /// 更新航迹并转移与检测索引对应的低频 embedding sidecar。
    pub fn update_with_embeddings(
        &mut self,
        detections: Vec<Detection>,
        embeddings: Vec<Option<FaceEmbedding>>,
    ) -> Vec<TrackedObject> {
        self.update_internal(
            detections,
            embeddings,
            self.config.default_frame_interval_ms,
        )
    }

    /// 使用源帧 PTS 更新航迹，真实帧间隔用于 Kalman 预测和 Lost 超时。
    pub fn update_with_embeddings_at(
        &mut self,
        detections: Vec<Detection>,
        embeddings: Vec<Option<FaceEmbedding>>,
        timestamp_ms: i64,
    ) -> Vec<TrackedObject> {
        self.update_with_embeddings_at_result(detections, embeddings, timestamp_ms)
            .objects
    }

    /// 使用源帧 PTS 更新航迹并返回结果状态。
    pub fn update_with_embeddings_at_result(
        &mut self,
        detections: Vec<Detection>,
        embeddings: Vec<Option<FaceEmbedding>>,
        timestamp_ms: i64,
    ) -> TrackUpdateResult {
        let elapsed_ms = match self.last_timestamp_ms {
            None => self.config.default_frame_interval_ms,
            Some(previous) if timestamp_ms > previous => timestamp_ms - previous,
            Some(_) => {
                // 迟到帧不能回写已经推进的航迹，避免 PTS 倒退污染速度与冷却状态。
                return TrackUpdateResult {
                    status: TrackUpdateStatus::Stale,
                    objects: self.active_objects(),
                };
            }
        };
        let result = self.update_internal_result(detections, embeddings, elapsed_ms);
        if result.status == TrackUpdateStatus::Applied {
            self.last_timestamp_ms = Some(timestamp_ms);
        }
        result
    }

    fn update_internal(
        &mut self,
        detections: Vec<Detection>,
        embeddings: Vec<Option<FaceEmbedding>>,
        elapsed_ms: i64,
    ) -> Vec<TrackedObject> {
        self.update_internal_result(detections, embeddings, elapsed_ms)
            .objects
    }

    fn update_internal_result(
        &mut self,
        detections: Vec<Detection>,
        mut embeddings: Vec<Option<FaceEmbedding>>,
        elapsed_ms: i64,
    ) -> TrackUpdateResult {
        if detections.len() > MAX_TRACKING_DETECTIONS || !detections.iter().all(is_valid_detection)
        {
            return TrackUpdateResult {
                status: TrackUpdateStatus::Rejected,
                objects: self.active_objects(),
            };
        }

        embeddings.resize(detections.len(), None);

        let elapsed_ms = elapsed_ms.max(1);
        let dt_frames = elapsed_ms as f32 / self.config.default_frame_interval_ms as f32;
        for track in &mut self.tracks {
            if track.status == TrackStatus::Removed {
                continue;
            }
            track.kalman.predict(dt_frames);
            track.predicted_bbox = sanitize_bbox(track.kalman.bbox());
            if track.status == TrackStatus::Lost {
                track.lost_for_ms = track.lost_for_ms.saturating_add(elapsed_ms);
            }
        }

        let mut high_indices = Vec::with_capacity(detections.len());
        let mut low_indices = Vec::with_capacity(detections.len());
        for (index, detection) in detections.iter().enumerate() {
            if detection.confidence >= self.config.track_score {
                high_indices.push(index);
            } else if detection.confidence >= self.config.low_score {
                low_indices.push(index);
            }
        }

        // 第一阶段：Tracked + Lost 航迹与高分检测全局匹配。
        let pool_indices: Vec<usize> = self
            .tracks
            .iter()
            .enumerate()
            .filter_map(|(index, track)| {
                matches!(track.status, TrackStatus::Tracked | TrackStatus::Lost).then_some(index)
            })
            .collect();
        let (high_matches, unmatched_pool, unmatched_high) = assign_tracks(
            &self.tracks,
            &pool_indices,
            &detections,
            &high_indices,
            self.config.high_match_iou,
        );

        for (track_index, detection_index) in high_matches {
            self.tracks[track_index].update(
                &detections[detection_index],
                embeddings[detection_index].as_ref(),
                self.config.max_trajectory_len,
            );
        }

        // 第二阶段：只有未匹配的活跃航迹可以使用低分检测补救，低分检测不能创建新航迹。
        let low_track_indices: Vec<usize> = unmatched_pool
            .into_iter()
            .filter(|&index| self.tracks[index].status == TrackStatus::Tracked)
            .collect();
        let (low_matches, unmatched_low_tracks, _) = assign_tracks(
            &self.tracks,
            &low_track_indices,
            &detections,
            &low_indices,
            self.config.low_match_iou,
        );
        for (track_index, detection_index) in low_matches {
            self.tracks[track_index].update(
                &detections[detection_index],
                embeddings[detection_index].as_ref(),
                self.config.max_trajectory_len,
            );
        }
        for track_index in unmatched_low_tracks {
            self.tracks[track_index].mark_lost(elapsed_ms);
        }

        // 先淘汰已超时 Lost 航迹，为新目标释放固定容量。
        self.tracks.retain(|track| {
            !(track.status == TrackStatus::Removed
                || (track.status == TrackStatus::Lost
                    && track.lost_for_ms > self.config.max_lost_ms))
        });

        // 剩余高分检测初始化新航迹。低分检测永远不能直接产生业务 TrackId。
        for detection_index in unmatched_high {
            let detection = &detections[detection_index];
            if detection.confidence < self.config.new_track_score {
                continue;
            }
            if self.tracks.len() >= MAX_TRACKS {
                break;
            }
            let track_id = self.next_track_id;
            self.next_track_id = self.next_track_id.saturating_add(1).max(1);
            self.tracks.push(TrackState::new(
                track_id,
                detection.clone(),
                embeddings[detection_index].clone(),
                self.config.max_trajectory_len,
            ));
        }

        if !self.alarm_cooldowns.is_empty() {
            if self.tracks.len() <= 16 {
                self.alarm_cooldowns.retain(|(track_id, _), _| {
                    self.tracks.iter().any(|track| track.track_id == *track_id)
                });
            } else {
                let active_ids: HashSet<u64> =
                    self.tracks.iter().map(|track| track.track_id).collect();
                self.alarm_cooldowns
                    .retain(|(track_id, _), _| active_ids.contains(track_id));
            }
        }

        TrackUpdateResult {
            status: TrackUpdateStatus::Applied,
            objects: self.active_objects(),
        }
    }

    fn active_objects(&self) -> Vec<TrackedObject> {
        let mut objects: Vec<TrackedObject> = self
            .tracks
            .iter()
            .filter(|track| track.status == TrackStatus::Tracked)
            .map(TrackState::to_public)
            .collect();
        objects.sort_unstable_by_key(|object| object.track_id);
        objects
    }

    /// 基于规则索引与角色执行防刷屏判定。
    pub fn check_and_mark_rule(
        &mut self,
        track_id: u64,
        rule_idx: usize,
        role: DetectionRuleRole,
        now_ms: i64,
        cooldown_ms: i64,
    ) -> bool {
        let role_code = match role {
            DetectionRuleRole::Roi => 1,
            DetectionRuleRole::Line => 2,
            DetectionRuleRole::Mask => 3,
        };
        self.check_and_mark_cooldown(
            track_id,
            CooldownTarget::Rule(rule_idx, role_code),
            now_ms,
            cooldown_ms,
        )
    }

    /// 检查指定 track_id 是否可以在给定命名规则下触发报警。
    pub fn check_and_mark_alarm(
        &mut self,
        track_id: u64,
        rule_id: &str,
        now_ms: i64,
        cooldown_ms: i64,
    ) -> bool {
        self.check_and_mark_cooldown(
            track_id,
            CooldownTarget::Named(rule_id.to_string()),
            now_ms,
            cooldown_ms,
        )
    }

    fn check_and_mark_cooldown(
        &mut self,
        track_id: u64,
        target: CooldownTarget,
        now_ms: i64,
        cooldown_ms: i64,
    ) -> bool {
        use std::collections::hash_map::Entry;

        match self.alarm_cooldowns.entry((track_id, target)) {
            Entry::Occupied(mut entry) => {
                let allowed = now_ms.saturating_sub(*entry.get()) >= cooldown_ms;
                if allowed {
                    entry.insert(now_ms);
                }
                allowed
            }
            Entry::Vacant(entry) => {
                entry.insert(now_ms);
                true
            }
        }
    }
}

fn is_valid_detection(detection: &Detection) -> bool {
    (0.0..=1.0).contains(&detection.confidence)
        && !detection.label.trim().is_empty()
        && is_valid_bbox(&detection.bbox)
}

fn is_valid_bbox(bbox: &BoundingBox) -> bool {
    (0.0..=1.0).contains(&bbox.x1)
        && (0.0..=1.0).contains(&bbox.y1)
        && (0.0..=1.0).contains(&bbox.x2)
        && (0.0..=1.0).contains(&bbox.y2)
        && bbox.x2 > bbox.x1
        && bbox.y2 > bbox.y1
}

fn sanitize_bbox(bbox: BoundingBox) -> BoundingBox {
    let x1 = bbox.x1.clamp(0.0, 1.0);
    let y1 = bbox.y1.clamp(0.0, 1.0);
    let x2 = bbox.x2.clamp(x1, 1.0);
    let y2 = bbox.y2.clamp(y1, 1.0);
    BoundingBox::new(x1, y1, x2, y2)
}

fn measurement_from_bbox(bbox: &BoundingBox) -> [f32; 4] {
    let width = (bbox.x2 - bbox.x1).max(1e-4);
    let height = (bbox.y2 - bbox.y1).max(1e-4);
    [
        (bbox.x1 + bbox.x2) * 0.5,
        (bbox.y1 + bbox.y2) * 0.5,
        (width / height).max(1e-4),
        height,
    ]
}

fn compute_iou(a: &BoundingBox, b: &BoundingBox) -> f32 {
    let inter_w = a.x2.min(b.x2) - a.x1.max(b.x1);
    if inter_w <= 0.0 {
        return 0.0;
    }
    let inter_h = a.y2.min(b.y2) - a.y1.max(b.y1);
    if inter_h <= 0.0 {
        return 0.0;
    }
    let inter_area = inter_w * inter_h;
    let area_a = (a.x2 - a.x1).max(0.0) * (a.y2 - a.y1).max(0.0);
    let area_b = (b.x2 - b.x1).max(0.0) * (b.y2 - b.y1).max(0.0);
    let union_area = area_a + area_b - inter_area;
    if union_area <= 0.0 {
        0.0
    } else {
        (inter_area / union_area).clamp(0.0, 1.0)
    }
}

fn assign_tracks(
    tracks: &[TrackState],
    track_indices: &[usize],
    detections: &[Detection],
    detection_indices: &[usize],
    min_iou: f32,
) -> (Vec<(usize, usize)>, Vec<usize>, Vec<usize>) {
    if track_indices.is_empty() {
        return (Vec::new(), Vec::new(), detection_indices.to_vec());
    }
    if detection_indices.is_empty() {
        return (Vec::new(), track_indices.to_vec(), Vec::new());
    }

    let rows = track_indices.len();
    let cols = detection_indices.len();
    let min_iou = min_iou.clamp(EPSILON, 1.0);
    let min_cost = 1.0 - min_iou;
    // 2.0 is intentionally above the maximum valid cost (1.0), so forbidden
    // class/IoU pairs can never be accepted even when min_iou is configured as 0.
    let mut costs = vec![2.0f32; rows * cols];

    for (r, &track_index) in track_indices.iter().enumerate() {
        let track = &tracks[track_index];
        let row_offset = r * cols;
        for (c, &detection_index) in detection_indices.iter().enumerate() {
            let detection = &detections[detection_index];
            if track.class_id != detection.class_id {
                continue;
            }
            let iou = compute_iou(&track.predicted_bbox, &detection.bbox);
            if iou > EPSILON && iou + EPSILON >= min_iou {
                costs[row_offset + c] = 1.0 - iou;
            }
        }
    }

    let assignment = hungarian_assignment(&costs, rows, cols);
    let mut matched_tracks = vec![false; rows];
    let mut matched_detections = vec![false; cols];
    let mut matches = Vec::with_capacity(rows.min(cols));

    for (r, assigned_column) in assignment.into_iter().enumerate() {
        let Some(c) = assigned_column else {
            continue;
        };
        let cost = costs[r * cols + c];
        if cost <= min_cost + EPSILON {
            let track_index = track_indices[r];
            let detection_index = detection_indices[c];
            matches.push((track_index, detection_index));
            matched_tracks[r] = true;
            matched_detections[c] = true;
        }
    }

    let unmatched_tracks = track_indices
        .iter()
        .enumerate()
        .filter_map(|(r, &index)| (!matched_tracks[r]).then_some(index))
        .collect();
    let unmatched_detections = detection_indices
        .iter()
        .enumerate()
        .filter_map(|(c, &index)| (!matched_detections[c]).then_some(index))
        .collect();
    (matches, unmatched_tracks, unmatched_detections)
}

/// 最小代价 Hungarian 分配（连续一维平铺矩阵，支持任意行列长宽比）。
fn hungarian_assignment(costs: &[f32], rows: usize, columns: usize) -> Vec<Option<usize>> {
    if rows == 0 || columns == 0 {
        return vec![None; rows];
    }
    if rows <= columns {
        return hungarian_solve_core(rows, columns, |r, c| costs[r * columns + c]);
    }

    // 当 rows > columns 时，逻辑转置以满足匈牙利算法 rows <= columns 约束，无需物理拷贝转置矩阵。
    let transposed_assignment = hungarian_solve_core(columns, rows, |c, r| costs[r * columns + c]);
    let mut assignment = vec![None; rows];
    for (column, row) in transposed_assignment.into_iter().enumerate() {
        if let Some(row) = row {
            assignment[row] = Some(column);
        }
    }
    assignment
}

fn hungarian_solve_core<F>(rows: usize, columns: usize, cost_at: F) -> Vec<Option<usize>>
where
    F: Fn(usize, usize) -> f32,
{
    let mut u = vec![0.0f32; rows + 1];
    let mut v = vec![0.0f32; columns + 1];
    let mut matched_column = vec![0usize; columns + 1];
    let mut previous_column = vec![0usize; columns + 1];
    let mut minimum = vec![f32::INFINITY; columns + 1];
    let mut used = vec![false; columns + 1];

    for row in 1..=rows {
        matched_column[0] = row;
        let mut column0 = 0usize;
        minimum.fill(f32::INFINITY);
        used.fill(false);

        loop {
            used[column0] = true;
            let row0 = matched_column[column0];
            let mut delta = f32::INFINITY;
            let mut column1 = 0usize;
            for column in 1..=columns {
                if used[column] {
                    continue;
                }
                let current = cost_at(row0 - 1, column - 1) - u[row0] - v[column];
                if current < minimum[column] {
                    minimum[column] = current;
                    previous_column[column] = column0;
                }
                if minimum[column] < delta {
                    delta = minimum[column];
                    column1 = column;
                }
            }
            if !delta.is_finite() {
                break;
            }
            for column in 0..=columns {
                if used[column] {
                    u[matched_column[column]] += delta;
                    v[column] -= delta;
                } else {
                    minimum[column] -= delta;
                }
            }
            column0 = column1;
            if matched_column[column0] == 0 {
                break;
            }
        }

        loop {
            let previous = previous_column[column0];
            matched_column[column0] = matched_column[previous];
            column0 = previous;
            if column0 == 0 {
                break;
            }
        }
    }

    let mut assignment = vec![None; rows];
    for (column, &row) in matched_column.iter().enumerate().skip(1).take(columns) {
        if row > 0 {
            assignment[row - 1] = Some(column - 1);
        }
    }
    assignment
}

#[cfg(test)]
mod tests {
    use super::*;

    fn detection(x1: f32, y1: f32, x2: f32, y2: f32, confidence: f32) -> Detection {
        Detection {
            class_id: 0,
            label: "person".to_string(),
            confidence,
            quality_score: None,
            bbox: BoundingBox::new(x1, y1, x2, y2),
            face: None,
        }
    }

    #[test]
    fn test_tracker_continuous_tracking() {
        let mut tracker = ByteTrack::new();
        let first = tracker.update(vec![detection(0.1, 0.1, 0.2, 0.2, 0.9)]);
        let track_id = first[0].track_id;
        let second = tracker.update(vec![detection(0.12, 0.12, 0.22, 0.22, 0.91)]);
        assert_eq!(second[0].track_id, track_id);
        assert_eq!(second[0].trajectory.len(), 2);
    }

    #[test]
    fn test_low_score_detection_rescues_existing_track() {
        let mut tracker = ByteTrack::new();
        let first = tracker.update(vec![detection(0.1, 0.1, 0.2, 0.3, 0.9)]);
        let track_id = first[0].track_id;
        let rescued = tracker.update(vec![detection(0.12, 0.1, 0.22, 0.3, 0.20)]);
        assert_eq!(rescued.len(), 1);
        assert_eq!(rescued[0].track_id, track_id);

        let new_low = tracker.update(vec![detection(0.7, 0.7, 0.8, 0.9, 0.20)]);
        assert!(new_low.is_empty(), "低分检测不能直接创建新航迹");
    }

    #[test]
    fn test_high_speed_motion_and_short_loss_keep_id() {
        let mut tracker = ByteTrack::new();
        let first = tracker.update(vec![detection(0.10, 0.10, 0.18, 0.18, 0.95)]);
        let track_id = first[0].track_id;
        let second = tracker.update(vec![detection(0.12, 0.10, 0.20, 0.18, 0.93)]);
        assert_eq!(second[0].track_id, track_id);
        assert!(tracker.update(Vec::new()).is_empty());
        let recovered = tracker.update(vec![detection(0.155, 0.10, 0.235, 0.18, 0.91)]);
        assert_eq!(recovered[0].track_id, track_id);
    }

    #[test]
    fn test_timestamp_gap_is_used_for_lost_expiry() {
        let mut tracker = ByteTrack::with_config(ByteTrackConfig {
            max_lost_ms: 500,
            ..ByteTrackConfig::default()
        });
        let first = tracker.update_with_embeddings_at(
            vec![detection(0.1, 0.1, 0.2, 0.3, 0.95)],
            vec![None],
            1_000,
        );
        let track_id = first[0].track_id;
        assert!(tracker
            .update_with_embeddings_at(Vec::new(), Vec::new(), 1_200)
            .is_empty());
        assert!(tracker
            .update_with_embeddings_at(Vec::new(), Vec::new(), 1_800)
            .is_empty());
        let new_track = tracker.update_with_embeddings_at(
            vec![detection(0.1, 0.1, 0.2, 0.3, 0.95)],
            vec![None],
            1_900,
        );
        assert_ne!(new_track[0].track_id, track_id);
    }

    #[test]
    fn test_stale_timestamp_does_not_rewind_tracker() {
        let mut tracker = ByteTrack::new();
        let first = tracker.update_with_embeddings_at(
            vec![detection(0.1, 0.1, 0.2, 0.3, 0.95)],
            vec![None],
            1_000,
        );
        let track_id = first[0].track_id;
        let stale = tracker.update_with_embeddings_at(
            vec![detection(0.8, 0.8, 0.9, 0.9, 0.95)],
            vec![None],
            900,
        );
        assert_eq!(stale[0].track_id, track_id);
        assert!((stale[0].bbox.x1 - 0.1).abs() < 1e-6);

        let result = tracker.update_with_embeddings_at_result(
            vec![detection(0.8, 0.8, 0.9, 0.9, 0.95)],
            vec![None],
            900,
        );
        assert_eq!(result.status, TrackUpdateStatus::Stale);
    }

    #[test]
    fn test_rejected_detection_batch_keeps_tracker_state() {
        let mut tracker = ByteTrack::new();
        let first = tracker.update_with_embeddings_at(
            vec![detection(0.1, 0.1, 0.2, 0.3, 0.95)],
            vec![None],
            1_000,
        );
        let track_id = first[0].track_id;
        let oversized = (0..=MAX_TRACKING_DETECTIONS)
            .map(|_| detection(0.1, 0.1, 0.2, 0.3, 0.95))
            .collect();
        let result = tracker.update_with_embeddings_at_result(oversized, Vec::new(), 1_100);
        assert_eq!(result.status, TrackUpdateStatus::Rejected);
        assert_eq!(result.objects[0].track_id, track_id);

        let invalid = tracker.update_with_embeddings_at_result(
            vec![detection(-0.1, 0.1, 0.2, 0.3, 0.95)],
            vec![None],
            1_100,
        );
        assert_eq!(invalid.status, TrackUpdateStatus::Rejected);
        assert_eq!(invalid.objects[0].track_id, track_id);
    }

    #[test]
    fn test_expire_if_stale_clears_without_reusing_track_id() {
        let mut tracker = ByteTrack::with_config(ByteTrackConfig {
            max_lost_ms: 500,
            ..ByteTrackConfig::default()
        });
        let first = tracker.update_with_embeddings_at(
            vec![detection(0.1, 0.1, 0.2, 0.3, 0.95)],
            vec![None],
            1_000,
        );
        let first_id = first[0].track_id;
        assert!(!tracker.expire_if_stale_at(1_500));
        assert!(tracker.expire_if_stale_at(1_501));

        let next = tracker.update_with_embeddings_at(
            vec![detection(0.1, 0.1, 0.2, 0.3, 0.95)],
            vec![None],
            1_600,
        );
        assert_ne!(next[0].track_id, first_id);
    }

    #[test]
    fn test_zero_iou_threshold_does_not_match_disjoint_boxes() {
        let mut tracker = ByteTrack::with_config(ByteTrackConfig {
            high_match_iou: 0.0,
            low_match_iou: 0.0,
            ..ByteTrackConfig::default()
        });
        let first = tracker.update(vec![detection(0.1, 0.1, 0.2, 0.2, 0.95)]);
        let second = tracker.update(vec![detection(0.7, 0.7, 0.8, 0.8, 0.95)]);
        assert_ne!(first[0].track_id, second[0].track_id);
    }

    #[test]
    fn test_embedding_sidecar_follows_track() {
        let mut tracker = ByteTrack::new();
        let embedding = Box::new([0.125; 512]);
        let detection = Detection {
            class_id: 0,
            label: "face".to_string(),
            confidence: 0.95,
            quality_score: Some(0.9),
            bbox: BoundingBox::new(0.2, 0.2, 0.4, 0.5),
            face: None,
        };
        let first =
            tracker.update_with_embeddings(vec![detection.clone()], vec![Some(embedding.clone())]);
        assert_eq!(first[0].embedding.as_deref(), Some(embedding.as_ref()));
        let second = tracker.update_with_embeddings(vec![detection], vec![None]);
        assert_eq!(second[0].embedding.as_deref(), Some(embedding.as_ref()));
    }

    #[test]
    fn test_alarm_cooldown_memory_reclamation() {
        let mut tracker = ByteTrack::new();
        let objects = tracker.update(vec![detection(0.3, 0.3, 0.5, 0.5, 0.95)]);
        let track_id = objects[0].track_id;
        assert!(tracker.check_and_mark_rule(track_id, 0, DetectionRuleRole::Roi, 1_000, 5_000));
        for _ in 0..31 {
            let _ = tracker.update(Vec::new());
        }
        assert!(tracker.alarm_cooldowns.is_empty());
    }

    #[test]
    fn test_hungarian_assignment_prefers_global_solution() {
        let costs = [0.10, 0.20, 0.11, 1.0];
        let assignment = hungarian_assignment(&costs, 2, 2);
        assert_eq!(assignment, vec![Some(1), Some(0)]);
    }

    #[test]
    fn test_hungarian_assignment_rectangular_transposed() {
        // 3 航迹，2 检测 (rows > columns)
        let costs = [
            0.10, 0.90, // row 0
            0.90, 0.20, // row 1
            0.15, 0.80, // row 2
        ];
        let assignment = hungarian_assignment(&costs, 3, 2);
        assert_eq!(assignment, vec![Some(0), Some(1), None]);

        // 2 航迹，3 检测 (rows < columns)
        let costs_wide = [
            0.10, 0.90, 0.80, // row 0
            0.90, 0.20, 0.80, // row 1
        ];
        let assignment_wide = hungarian_assignment(&costs_wide, 2, 3);
        assert_eq!(assignment_wide, vec![Some(0), Some(1)]);
    }
}
