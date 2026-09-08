//! 纯 Rust ByteTrack 多目标航迹跟踪器
//!
//! 1. 8 维状态卡尔曼滤波 (状态: [xc, yc, a, h, vxc, vyc, va, vh])；
//! 2. 两阶段高低分二分图匹配 (First stage: dets_high; Second stage: dets_low)；
//! 3. 航迹生命周期管理 (New -> Tracked -> Lost -> Removed)；
//! 4. 严格内存有界：失联超过 `max_time_lost` 帧自动注销。

use std::collections::VecDeque;

/// 边界框表示：[x1, y1, width, height]，左上角原点
pub type Rect = [f32; 4];

/// 计算两个 Rect 之间的 IoU
pub fn box_iou(a: &Rect, b: &Rect) -> f32 {
    let ax2 = a[0] + a[2];
    let ay2 = a[1] + a[3];
    let bx2 = b[0] + b[2];
    let by2 = b[1] + b[3];

    let inter_x1 = a[0].max(b[0]);
    let inter_y1 = a[1].max(b[1]);
    let inter_x2 = ax2.min(bx2);
    let inter_y2 = ay2.min(by2);

    let inter_w = (inter_x2 - inter_x1).max(0.0);
    let inter_h = (inter_y2 - inter_y1).max(0.0);
    let inter_area = inter_w * inter_h;

    let area_a = a[2] * a[3];
    let area_b = b[2] * b[3];
    let union_area = area_a + area_b - inter_area;

    if union_area <= 0.0 {
        0.0
    } else {
        inter_area / union_area
    }
}

/// 8 维卡尔曼滤波状态: [xc, yc, a, h, vxc, vyc, va, vh]
#[derive(Debug, Clone)]
pub struct KalmanBoxTracker {
    mean: [f32; 8],
    covariance: [f32; 64], // 8x8 对角与协方差矩阵展开
}

impl KalmanBoxTracker {
    pub fn new(rect: &Rect) -> Self {
        let xc = rect[0] + rect[2] * 0.5;
        let yc = rect[1] + rect[3] * 0.5;
        let h = rect[3].max(1.0);
        let a = rect[2] / h;

        let mut mean = [0.0; 8];
        mean[0] = xc;
        mean[1] = yc;
        mean[2] = a;
        mean[3] = h;

        let mut covariance = [0.0; 64];
        let std = [
            2.0 * 0.05 * h,
            2.0 * 0.05 * h,
            1e-2,
            2.0 * 0.05 * h,
            10.0 * 0.05 * h,
            10.0 * 0.05 * h,
            1e-5,
            10.0 * 0.05 * h,
        ];
        for i in 0..8 {
            covariance[i * 8 + i] = std[i] * std[i];
        }

        Self { mean, covariance }
    }

    /// 预测下一帧位置与协方差
    pub fn predict(&mut self) {
        // 匀速运动模型: x' = x + vx
        for i in 0..4 {
            self.mean[i] += self.mean[i + 4];
        }

        // 简化的过程噪声更新
        let h = self.mean[3].max(1.0);
        let std_pos = 0.05 * h;
        let std_vel = 0.00625 * h;
        for i in 0..4 {
            self.covariance[i * 8 + i] += std_pos * std_pos;
            self.covariance[(i + 4) * 8 + (i + 4)] += std_vel * std_vel;
        }
    }

    /// 根据观测值校准更新状态
    pub fn update(&mut self, rect: &Rect) {
        let xc = rect[0] + rect[2] * 0.5;
        let yc = rect[1] + rect[3] * 0.5;
        let h = rect[3].max(1.0);
        let a = rect[2] / h;
        let measurement = [xc, yc, a, h];

        // 简化的卡尔曼观测增益更新 (Steady-state approximation)
        for (i, &m) in measurement.iter().enumerate() {
            let p = self.covariance[i * 8 + i];
            let r = (0.1 * h).powi(2).max(1.0);
            let k = p / (p + r);
            let innovation = m - self.mean[i];
            self.mean[i] += k * innovation;
            self.mean[i + 4] += (k * 0.5) * innovation;
            self.covariance[i * 8 + i] *= 1.0 - k;
        }
    }

    /// 将当前卡尔曼状态转换为 Rect [x1, y1, w, h]
    pub fn to_rect(&self) -> Rect {
        let a = self.mean[2].max(1e-4);
        let h = self.mean[3].max(1.0);
        let w = a * h;
        let x1 = self.mean[0] - w * 0.5;
        let y1 = self.mean[1] - h * 0.5;
        [x1, y1, w, h]
    }
}

/// 航迹活跃状态
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrackStatus {
    New,
    Tracked,
    Lost,
    Removed,
}

/// 单个检测输入
#[derive(Debug, Clone, Copy)]
pub struct TrackDetection {
    pub bbox: Rect,
    pub score: f32,
    pub class_id: usize,
}

/// 单条目标航迹 (Single Tracklet)
#[derive(Debug, Clone)]
pub struct STrack {
    pub track_id: u64,
    pub kalman: KalmanBoxTracker,
    pub bbox: Rect,
    pub score: f32,
    pub status: TrackStatus,
    pub is_activated: bool,
    pub frame_id: usize,
    pub tracklet_len: usize,
    pub time_since_update: usize,
    /// 历史轨迹点集合 (底边中心点，最大保留 30 点)
    pub trajectory: VecDeque<[f32; 2]>,
}

impl STrack {
    pub fn new(det: &TrackDetection, frame_id: usize) -> Self {
        let mut trajectory = VecDeque::with_capacity(32);
        let bottom_center = [det.bbox[0] + det.bbox[2] * 0.5, det.bbox[1] + det.bbox[3]];
        trajectory.push_back(bottom_center);

        Self {
            track_id: 0,
            kalman: KalmanBoxTracker::new(&det.bbox),
            bbox: det.bbox,
            score: det.score,
            status: TrackStatus::New,
            is_activated: false,
            frame_id,
            tracklet_len: 0,
            time_since_update: 0,
            trajectory,
        }
    }

    pub fn activate(&mut self, track_id: u64, frame_id: usize) {
        self.track_id = track_id;
        self.status = TrackStatus::Tracked;
        self.is_activated = true;
        self.frame_id = frame_id;
        self.tracklet_len = 1;
        self.time_since_update = 0;
    }

    pub fn re_activate(&mut self, det: &TrackDetection, frame_id: usize) {
        self.kalman.update(&det.bbox);
        self.bbox = det.bbox;
        self.score = det.score;
        self.status = TrackStatus::Tracked;
        self.is_activated = true;
        self.frame_id = frame_id;
        self.tracklet_len += 1;
        self.time_since_update = 0;

        let bottom_center = [det.bbox[0] + det.bbox[2] * 0.5, det.bbox[1] + det.bbox[3]];
        self.trajectory.push_back(bottom_center);
        if self.trajectory.len() > 30 {
            self.trajectory.pop_front();
        }
    }

    pub fn update(&mut self, det: &TrackDetection, frame_id: usize) {
        self.kalman.update(&det.bbox);
        self.bbox = det.bbox;
        self.score = det.score;
        self.status = TrackStatus::Tracked;
        self.is_activated = true;
        self.frame_id = frame_id;
        self.tracklet_len += 1;
        self.time_since_update = 0;

        let bottom_center = [det.bbox[0] + det.bbox[2] * 0.5, det.bbox[1] + det.bbox[3]];
        self.trajectory.push_back(bottom_center);
        if self.trajectory.len() > 30 {
            self.trajectory.pop_front();
        }
    }

    pub fn mark_lost(&mut self) {
        self.status = TrackStatus::Lost;
    }

    pub fn mark_removed(&mut self) {
        self.status = TrackStatus::Removed;
    }
}

/// 纯 Rust ByteTrack 跟踪器配置
#[derive(Debug, Clone)]
pub struct ByteTrackConfig {
    /// 第一阶段高置信度阈值 (默认 0.50)
    pub track_thresh: f32,
    /// 新建航迹激活门槛 (默认 0.60)
    pub high_thresh: f32,
    /// 匹配 IoU 阈值 (默认 0.50)
    pub match_thresh: f32,
    /// 目标丢失最大容忍帧数 (默认 30 帧，约 1~1.5 秒)
    pub max_time_lost: usize,
}

impl Default for ByteTrackConfig {
    fn default() -> Self {
        Self {
            track_thresh: 0.50,
            high_thresh: 0.60,
            match_thresh: 0.50,
            max_time_lost: 30,
        }
    }
}

/// 纯 Rust ByteTrack 多目标航迹跟踪器
#[derive(Debug)]
pub struct ByteTracker {
    config: ByteTrackConfig,
    frame_id: usize,
    next_id: u64,
    tracked_stracks: Vec<STrack>,
    lost_stracks: Vec<STrack>,
    removed_stracks: Vec<STrack>,
}

impl ByteTracker {
    pub fn new(config: ByteTrackConfig) -> Self {
        Self {
            config,
            frame_id: 0,
            next_id: 1,
            tracked_stracks: Vec::new(),
            lost_stracks: Vec::new(),
            removed_stracks: Vec::new(),
        }
    }

    /// 重置跟踪器状态
    pub fn reset(&mut self) {
        self.frame_id = 0;
        self.next_id = 1;
        self.tracked_stracks.clear();
        self.lost_stracks.clear();
        self.removed_stracks.clear();
    }

    /// 执行一帧跟踪更新，返回当前处于 Tracked 状态的活跃航迹列表
    pub fn update(&mut self, detections: &[TrackDetection]) -> Vec<STrack> {
        self.frame_id += 1;

        // 1. 卡尔曼滤波预测所有活跃与丢失中的航迹
        for track in self.tracked_stracks.iter_mut() {
            track.kalman.predict();
            track.bbox = track.kalman.to_rect();
        }
        for track in self.lost_stracks.iter_mut() {
            track.kalman.predict();
            track.bbox = track.kalman.to_rect();
        }

        // 2. 将输入检测按置信度切分为高分 (dets_high) 与低分 (dets_low)
        let mut dets_high = Vec::new();
        let mut dets_low = Vec::new();

        for det in detections {
            if det.score >= self.config.track_thresh {
                dets_high.push(*det);
            } else if det.score >= 0.1 {
                dets_low.push(*det);
            }
        }

        // 3. 第一阶段：高分检测与当前活跃航迹匹配
        let (matched_th, unmatched_t1, unmatched_dh) =
            greedy_iou_match(&self.tracked_stracks, &dets_high, self.config.match_thresh);

        for (t_idx, d_idx) in matched_th {
            self.tracked_stracks[t_idx].update(&dets_high[d_idx], self.frame_id);
        }

        // 4. 第二阶段：未匹配的高分航迹与低分检测匹配 (找回被遮挡/光照变暗的人体)
        let mut unconfirmed_tracks = Vec::new();
        let mut tracked_for_low = Vec::new();
        for &t_idx in &unmatched_t1 {
            if self.tracked_stracks[t_idx].is_activated {
                tracked_for_low.push(t_idx);
            } else {
                unconfirmed_tracks.push(t_idx);
            }
        }

        let candidates_for_low: Vec<STrack> = tracked_for_low
            .iter()
            .map(|&idx| self.tracked_stracks[idx].clone())
            .collect();

        let (matched_tl, unmatched_t2, _) = greedy_iou_match(&candidates_for_low, &dets_low, 0.45);

        for (c_idx, d_idx) in matched_tl {
            let orig_idx = tracked_for_low[c_idx];
            self.tracked_stracks[orig_idx].update(&dets_low[d_idx], self.frame_id);
        }

        // 5. 将二次失配的航迹标记为 Lost
        let mut newly_lost = Vec::new();
        for &c_idx in &unmatched_t2 {
            let orig_idx = tracked_for_low[c_idx];
            let track = &mut self.tracked_stracks[orig_idx];
            track.mark_lost();
            newly_lost.push(track.clone());
        }

        // 6. 第三阶段：剩余未匹配的高分检测与丢失的 lost_stracks 匹配 (重连 Re-ID)
        let remaining_dets: Vec<TrackDetection> = unmatched_dh
            .into_iter()
            .map(|d_idx| dets_high[d_idx])
            .collect();

        let (matched_lost, _, unmatched_dh2) =
            greedy_iou_match(&self.lost_stracks, &remaining_dets, 0.50);

        let mut reactivated_tracks = Vec::new();
        let mut reactivated_lost_indices = Vec::new();
        for (l_idx, d_idx) in matched_lost {
            let mut track = self.lost_stracks[l_idx].clone();
            track.re_activate(&remaining_dets[d_idx], self.frame_id);
            reactivated_tracks.push(track);
            reactivated_lost_indices.push(l_idx);
        }

        // 从 lost_stracks 移除已复活的
        self.lost_stracks = self
            .lost_stracks
            .iter()
            .enumerate()
            .filter(|(idx, _)| !reactivated_lost_indices.contains(idx))
            .map(|(_, t)| t.clone())
            .collect();

        // 7. 处理全新出现的高分检测：初始化新航迹
        let mut new_stracks = Vec::new();
        for d_idx in unmatched_dh2 {
            let det = &remaining_dets[d_idx];
            if det.score >= self.config.high_thresh {
                let mut track = STrack::new(det, self.frame_id);
                let id = self.next_id;
                self.next_id += 1;
                track.activate(id, self.frame_id);
                new_stracks.push(track);
            }
        }

        // 8. 整合活跃航迹集
        let mut updated_tracked = Vec::new();
        for track in self.tracked_stracks.drain(..) {
            if track.status == TrackStatus::Tracked {
                updated_tracked.push(track);
            }
        }
        updated_tracked.extend(reactivated_tracks);
        updated_tracked.extend(new_stracks);
        self.tracked_stracks = updated_tracked;

        // 9. 更新与清理 lost_stracks
        self.lost_stracks.extend(newly_lost);
        let max_lost = self.config.max_time_lost;
        let cur_frame = self.frame_id;
        self.lost_stracks.retain_mut(|track| {
            if cur_frame - track.frame_id > max_lost {
                track.mark_removed();
                false
            } else {
                true
            }
        });

        // 仅返回当前处于 Tracked 状态的活跃航迹
        self.tracked_stracks
            .iter()
            .filter(|t| t.is_activated)
            .cloned()
            .collect()
    }
}

/// 贪婪 IoU 匹配二分图匹配器
fn greedy_iou_match(
    tracks: &[STrack],
    dets: &[TrackDetection],
    threshold: f32,
) -> (Vec<(usize, usize)>, Vec<usize>, Vec<usize>) {
    if tracks.is_empty() {
        let unmatched_dets = (0..dets.len()).collect();
        return (Vec::new(), Vec::new(), unmatched_dets);
    }
    if dets.is_empty() {
        let unmatched_tracks = (0..tracks.len()).collect();
        return (Vec::new(), unmatched_tracks, Vec::new());
    }

    let mut matches = Vec::new();
    let mut pairs = Vec::with_capacity(tracks.len() * dets.len());

    for (t_idx, track) in tracks.iter().enumerate() {
        for (d_idx, det) in dets.iter().enumerate() {
            let iou = box_iou(&track.bbox, &det.bbox);
            if iou >= threshold {
                pairs.push((t_idx, d_idx, iou));
            }
        }
    }

    pairs.sort_by(|a, b| b.2.partial_cmp(&a.2).unwrap_or(std::cmp::Ordering::Equal));

    let mut used_t = vec![false; tracks.len()];
    let mut used_d = vec![false; dets.len()];

    for (t_idx, d_idx, _) in pairs {
        if !used_t[t_idx] && !used_d[d_idx] {
            used_t[t_idx] = true;
            used_d[d_idx] = true;
            matches.push((t_idx, d_idx));
        }
    }

    let mut unmatched_t = Vec::new();
    for (t_idx, &used) in used_t.iter().enumerate() {
        if !used {
            unmatched_t.push(t_idx);
        }
    }

    let mut unmatched_d = Vec::new();
    for (d_idx, &used) in used_d.iter().enumerate() {
        if !used {
            unmatched_d.push(d_idx);
        }
    }

    (matches, unmatched_t, unmatched_d)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_box_iou_computation() {
        let a = [0.0, 0.0, 10.0, 10.0];
        let b = [5.0, 0.0, 10.0, 10.0];
        let iou = box_iou(&a, &b);
        assert!((iou - 50.0 / 150.0).abs() < 1e-4);
    }

    #[test]
    fn test_kalman_predict_and_update() {
        let rect = [100.0, 100.0, 50.0, 100.0];
        let mut tracker = KalmanBoxTracker::new(&rect);
        tracker.predict();
        let pred = tracker.to_rect();
        assert!((pred[0] - 100.0).abs() < 1.0);
        assert!((pred[1] - 100.0).abs() < 1.0);

        let next_meas = [105.0, 102.0, 50.0, 100.0];
        tracker.update(&next_meas);
        let updated = tracker.to_rect();
        assert!(updated[0] > 100.0);
    }

    #[test]
    fn test_bytetrack_tracking_continuity() {
        let mut tracker = ByteTracker::new(ByteTrackConfig::default());

        // 帧 1: 出现一个人体检测
        let det1 = vec![TrackDetection {
            bbox: [100.0, 100.0, 50.0, 150.0],
            score: 0.9,
            class_id: 0,
        }];
        let active1 = tracker.update(&det1);
        assert_eq!(active1.len(), 1);
        let id1 = active1[0].track_id;

        // 帧 2: 平移微移
        let det2 = vec![TrackDetection {
            bbox: [105.0, 102.0, 50.0, 150.0],
            score: 0.88,
            class_id: 0,
        }];
        let active2 = tracker.update(&det2);
        assert_eq!(active2.len(), 1);
        assert_eq!(active2[0].track_id, id1, "TrackId 必须连续稳定保持");
    }
}
