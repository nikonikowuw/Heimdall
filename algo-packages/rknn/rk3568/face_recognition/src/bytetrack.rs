//! 纯 Rust 工业级 ByteTrack 多目标航迹跟踪器 (Scale-Invariant ByteTrack)
//!
//! 核心机制与数学保证：
//! 1. 8 维状态标准线性卡尔曼滤波 (状态: [xc, yc, a, h, vxc, vyc, va, vh])；
//! 2. 尺度无关性 (Scale-Invariance)：完美适配 [0.0, 1.0] 归一化坐标系及绝对像素坐标系，
//!    彻底消除绝对阈值截断带来的尺度崩塌；
//! 3. 序列化卡尔曼观测更新 (Sequential Kalman Update)：解耦观测噪声计算，数值无奇异，
//!    实现完全的位置与速度全协方差自然修正；
//! 4. 基于 Kuhn-Munkres (KM / 匈牙利算法) 的全局最优二分图匹配，杜绝贪婪抢注导致的连环 ID Switch；
//! 5. 两阶段高低分关联 (Stage 1: dets_high; Stage 2: dets_low 挽救遮挡目标)；
//! 6. 航迹两帧确认缓冲 (Tentative Confirmation)，彻底过滤单帧虚警与幽灵 ID；
//! 7. 严格内存有界：失联超过 `max_time_lost` 帧自动注销。

use std::collections::VecDeque;

/// 边界框表示：[x1, y1, width, height]，左上角原点
pub type Rect = [f32; 4];

/// 计算两个 Rect 之间的 IoU (交并比)
pub fn box_iou(a: &Rect, b: &Rect) -> f32 {
    let ax2 = a[0] + a[2];
    let ay2 = a[1] + a[3];
    let bx2 = b[0] + b[2];
    let by2 = b[1] + b[3];

    let inter_x1 = a[0].max(b[0]);
    let inter_y1 = a[1].max(b[1]);
    let inter_x2 = ax2.min(bx2);
    let inter_y2 = ay2.min(by2);

    if inter_x2 <= inter_x1 || inter_y2 <= inter_y1 {
        return 0.0;
    }

    let inter_w = inter_x2 - inter_x1;
    let inter_h = inter_y2 - inter_y1;
    let inter_area = inter_w * inter_h;

    let area_a = (a[2] * a[3]).max(0.0);
    let area_b = (b[2] * b[3]).max(0.0);
    let union_area = area_a + area_b - inter_area;

    if union_area <= 1e-9 {
        0.0
    } else {
        inter_area / union_area
    }
}

/// 8 维状态卡尔曼滤波器: [xc, yc, a, h, vxc, vyc, va, vh]
/// 具有全尺度自适应性，支持归一化空间 [0.0, 1.0] 与像素物理空间
#[derive(Debug, Clone)]
pub struct KalmanBoxTracker {
    mean: [f32; 8],
    covariance: [f32; 64], // 8x8 全协方差矩阵 (行优先展平)
}

impl KalmanBoxTracker {
    /// 基于初始矩形框初始化滤波器
    pub fn new(rect: &Rect) -> Self {
        let h = rect[3].max(1e-4);
        let a = (rect[2] / h).max(1e-4);
        let xc = rect[0] + rect[2] * 0.5;
        let yc = rect[1] + rect[3] * 0.5;

        let mut mean = [0.0f32; 8];
        mean[0] = xc;
        mean[1] = yc;
        mean[2] = a;
        mean[3] = h;

        let mut covariance = [0.0f32; 64];
        // 初始不确定度：位置与高度正比于目标自身尺度 h，避免硬编码像素常数
        let std_pos = 2.0 * 0.05 * h;
        let std_a = 1e-2;
        let std_h = 2.0 * 0.05 * h;
        let std_v_pos = 10.0 * 0.05 * h;
        let std_v_a = 1e-5;
        let std_v_h = 10.0 * 0.05 * h;

        let stds = [
            std_pos, std_pos, std_a, std_h, std_v_pos, std_v_pos, std_v_a, std_v_h,
        ];
        for (i, &std) in stds.iter().enumerate() {
            covariance[i * 9] = std * std;
        }

        Self { mean, covariance }
    }

    /// 预测下一帧状态与全协方差
    ///
    /// 状态转移模型:
    /// F = [ I_4  I_4 ]
    ///     [  0   I_4 ]
    /// 协方差演化: P' = F * P * F^T + Q
    pub fn predict(&mut self) {
        // 1. 均值线性外推: x' = x + v
        for i in 0..4 {
            self.mean[i] += self.mean[i + 4];
        }

        // 2. 协方差演化: P' = F * P * F^T
        // 分块推导：
        // P'00 = P00 + P01 + P10 + P11
        // P'01 = P01 + P11
        // P'10 = P10 + P11
        // P'11 = P11
        let mut p_next = [0.0f32; 64];
        for i in 0..4 {
            for j in 0..4 {
                let p_pp = self.covariance[i * 8 + j];
                let p_pv = self.covariance[i * 8 + (j + 4)];
                let p_vp = self.covariance[(i + 4) * 8 + j];
                let p_vv = self.covariance[(i + 4) * 8 + (j + 4)];

                let pv_vv = p_pv + p_vv;
                p_next[i * 8 + j] = p_pp + p_vp + pv_vv;
                p_next[i * 8 + (j + 4)] = pv_vv;
                p_next[(i + 4) * 8 + j] = p_vp + p_vv;
                p_next[(i + 4) * 8 + (j + 4)] = p_vv;
            }
        }

        // 3. 注入尺度自适应过程噪声 Q
        let h = self.mean[3].max(1e-4);
        let q_pos = 0.05 * h;
        let q_a = 1e-2;
        let q_h = 0.05 * h;
        let q_v_pos = 0.00625 * h;
        let q_v_a = 1e-5;
        let q_v_h = 0.00625 * h;

        let q_stds = [q_pos, q_pos, q_a, q_h, q_v_pos, q_v_pos, q_v_a, q_v_h];
        for (k, &std) in q_stds.iter().enumerate() {
            p_next[k * 9] += std * std;
        }

        self.covariance = p_next;
    }

    /// 根据观测值执行状态更新 (基于序列卡尔曼更新，数值零奇异且自然更新全协方差)
    pub fn update(&mut self, rect: &Rect) {
        let h = rect[3].max(1e-4);
        let a = (rect[2] / h).max(1e-4);
        let xc = rect[0] + rect[2] * 0.5;
        let yc = rect[1] + rect[3] * 0.5;
        let measurement = [xc, yc, a, h];

        // 尺度自适应测量噪声标准差
        let r_pos = 0.05 * h;
        let r_a = 1e-2;
        let r_h = 0.05 * h;
        let r_vars = [r_pos * r_pos, r_pos * r_pos, r_a * r_a, r_h * r_h];

        // 序列卡尔曼观测更新 (Sequential Kalman Update)
        // 每个标量观测分量独立更新，等价于多维联合更新，彻底消除矩阵求逆奇点
        for m in 0..4 {
            let z_m = measurement[m];
            let x_m = self.mean[m];
            let innovation = z_m - x_m;

            // 新息方差 S = P_mm + R_m
            let p_mm = self.covariance[m * 8 + m];
            let s = p_mm + r_vars[m];
            if s <= 1e-12 {
                continue;
            }
            let inv_s = 1.0 / s;

            // 卡尔曼增益向量 K (8维)
            let mut k_gain = [0.0f32; 8];
            for (j, kj) in k_gain.iter_mut().enumerate() {
                *kj = self.covariance[j * 8 + m] * inv_s;
            }

            // 状态更新: mean = mean + K * innovation
            for (j, &kj) in k_gain.iter().enumerate() {
                self.mean[j] += kj * innovation;
            }

            // 协方差更新: P = P - K * P_m*
            let mut p_row_m = [0.0f32; 8];
            for (col, item) in p_row_m.iter_mut().enumerate() {
                *item = self.covariance[m * 8 + col];
            }

            for (j, &kj) in k_gain.iter().enumerate() {
                for (col, &p_mc) in p_row_m.iter().enumerate() {
                    self.covariance[j * 8 + col] -= kj * p_mc;
                }
            }

            // 数值对称性保护
            for j in 0..8 {
                for col in (j + 1)..8 {
                    let avg = 0.5 * (self.covariance[j * 8 + col] + self.covariance[col * 8 + j]);
                    self.covariance[j * 8 + col] = avg;
                    self.covariance[col * 8 + j] = avg;
                }
            }
        }
    }

    /// 将当前卡尔曼滤波中心状态转换为边界框 Rect [x1, y1, w, h]
    pub fn to_rect(&self) -> Rect {
        let a = self.mean[2].max(1e-4);
        let h = self.mean[3].max(1e-4);
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
        self.bbox = self.kalman.to_rect();
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
        self.bbox = self.kalman.to_rect();
        self.score = det.score;
        self.status = TrackStatus::Tracked;
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
    /// 是否对新目标启用 2 帧确认机制 (默认 true，有效杜绝单帧假阳性虚警)
    pub confirm_new_tracks: bool,
}

impl Default for ByteTrackConfig {
    fn default() -> Self {
        Self {
            track_thresh: 0.50,
            high_thresh: 0.60,
            match_thresh: 0.50,
            max_time_lost: 30,
            confirm_new_tracks: true,
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
    recently_removed: Vec<u64>,
}

impl ByteTracker {
    pub fn new(config: ByteTrackConfig) -> Self {
        Self {
            config,
            frame_id: 0,
            next_id: 1,
            tracked_stracks: Vec::new(),
            lost_stracks: Vec::new(),
            recently_removed: Vec::new(),
        }
    }

    /// 重置跟踪器状态
    pub fn reset(&mut self) {
        self.frame_id = 0;
        self.next_id = 1;
        self.tracked_stracks.clear();
        self.lost_stracks.clear();
        self.recently_removed.clear();
    }

    /// 返回本次 update 刚刚进入 Removed 的 track ID。
    /// Lost 航迹不在这里返回，调用方必须继续保留其时域状态。
    pub fn recently_removed_track_ids(&self) -> &[u64] {
        &self.recently_removed
    }

    /// 热更新建轨与关联门限。
    ///
    /// 建轨门槛与检测阈值存在耦合关系（见 `plugin::FaceRecognizer::init`），
    /// 宿主通过 `instance_update_config` 调整检测阈值时必须同步刷新，
    /// 否则会出现“检测通过但无法建轨”的状态不一致。
    pub fn apply_config(&mut self, config: ByteTrackConfig) {
        self.config = config;
    }

    /// 执行一帧跟踪更新，返回当前处于 Tracked 状态的活跃航迹列表
    pub fn update(&mut self, detections: &[TrackDetection]) -> Vec<STrack> {
        self.frame_id += 1;
        self.recently_removed.clear();

        // 1. 卡尔曼滤波时间更新：预测所有活跃与暂失航迹在当前时刻的位置
        for track in self.tracked_stracks.iter_mut() {
            track.kalman.predict();
            track.bbox = track.kalman.to_rect();
        }
        for track in self.lost_stracks.iter_mut() {
            track.kalman.predict();
            track.bbox = track.kalman.to_rect();
        }

        // 2. 将输入检测按置信度切分为高分 (dets_high) 与低分 (dets_low)
        let mut dets_high = Vec::with_capacity(detections.len());
        let mut dets_low = Vec::with_capacity(detections.len() / 2);

        for det in detections {
            if det.score >= self.config.track_thresh {
                dets_high.push(*det);
            } else if det.score >= 0.10 {
                dets_low.push(*det);
            }
        }

        // 3. 第一阶段：高分检测与当前跟踪中航迹匹配 (基于 KM 全局最优二分图匹配)
        let (matched_th, unmatched_t1, unmatched_dh) =
            kuhn_munkres_match(&self.tracked_stracks, &dets_high, self.config.match_thresh);

        for (t_idx, d_idx) in matched_th {
            let track = &mut self.tracked_stracks[t_idx];
            track.update(&dets_high[d_idx], self.frame_id);
            if !track.is_activated {
                // 待确认航迹在连续第 2 帧成功匹配高分检测，正式确认激活！
                track.is_activated = true;
            }
        }

        // 4. 第二阶段：未匹配的活跃航迹与低分检测匹配 (ByteTrack 核心：挽救被遮挡/模糊目标)
        let mut tracked_for_low = Vec::new();
        for &t_idx in &unmatched_t1 {
            if self.tracked_stracks[t_idx].is_activated {
                tracked_for_low.push(t_idx);
            } else {
                self.tracked_stracks[t_idx].mark_removed();
                self.recently_removed
                    .push(self.tracked_stracks[t_idx].track_id);
            }
        }

        // 低分匹配门限适度放宽至 0.40（直接索引闭包，杜绝在热路径对 STrack 做全量堆内存克隆）
        let (matched_tl, unmatched_t2, _) = kuhn_munkres_match_cost(
            tracked_for_low.len(),
            dets_low.len(),
            |c, d| {
                box_iou(
                    &self.tracked_stracks[tracked_for_low[c]].bbox,
                    &dets_low[d].bbox,
                )
            },
            0.40,
        );

        for (c_idx, d_idx) in matched_tl {
            let orig_idx = tracked_for_low[c_idx];
            self.tracked_stracks[orig_idx].update(&dets_low[d_idx], self.frame_id);
        }

        // 5. 将二次失配的成熟航迹标记为 Lost
        let mut newly_lost = Vec::with_capacity(unmatched_t2.len());
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

        let (matched_lost, _, unmatched_dh2) = kuhn_munkres_match(
            &self.lost_stracks,
            &remaining_dets,
            self.config.match_thresh,
        );

        let mut reactivated_tracks = Vec::new();
        let mut reactivated_lost_indices = Vec::new();
        for (l_idx, d_idx) in matched_lost {
            let mut track = self.lost_stracks[l_idx].clone();
            track.re_activate(&remaining_dets[d_idx], self.frame_id);
            reactivated_tracks.push(track);
            reactivated_lost_indices.push(l_idx);
        }

        // 从 lost_stracks 移除已重连复活的航迹 (降序 swap_remove 避免索引偏移与多余克隆)
        reactivated_lost_indices.sort_unstable_by(|a, b| b.cmp(a));
        for idx in reactivated_lost_indices {
            self.lost_stracks.swap_remove(idx);
        }

        // 7. 第四阶段：处理全新出现的高分检测：初始化新航迹
        let mut new_stracks = Vec::new();
        for d_idx in unmatched_dh2 {
            let det = &remaining_dets[d_idx];
            if det.score >= self.config.high_thresh {
                let mut track = STrack::new(det, self.frame_id);
                let id = self.next_id;
                self.next_id += 1;

                if self.frame_id == 1 || !self.config.confirm_new_tracks {
                    // 冷启动第 1 帧或关闭确认门控时，直接激活上线
                    track.activate(id, self.frame_id);
                } else {
                    // 后续帧默认进入未确认状态，分配 ID 但 is_activated 保持 false，待第 2 帧确认
                    track.track_id = id;
                    track.status = TrackStatus::New;
                    track.is_activated = false;
                    track.frame_id = self.frame_id;
                    track.tracklet_len = 1;
                }
                new_stracks.push(track);
            }
        }

        // 8. 整合存活航迹
        self.tracked_stracks
            .retain(|track| track.status == TrackStatus::Tracked);
        self.tracked_stracks.extend(reactivated_tracks);
        self.tracked_stracks.extend(new_stracks);

        // 9. 更新与清理超时 lost_stracks
        self.lost_stracks.extend(newly_lost);
        let max_lost = self.config.max_time_lost;
        let cur_frame = self.frame_id;
        let recently_removed = &mut self.recently_removed;
        self.lost_stracks.retain_mut(|track| {
            if cur_frame.saturating_sub(track.frame_id) > max_lost {
                track.mark_removed();
                recently_removed.push(track.track_id);
                false
            } else {
                true
            }
        });

        // 10. 输出：仅对外发布已确认激活 (is_activated == true) 且活跃的航迹
        self.tracked_stracks
            .iter()
            .filter(|t| t.is_activated && t.status == TrackStatus::Tracked)
            .cloned()
            .collect()
    }
}

/// 基于 Kuhn-Munkres (KM / 匈牙利算法) 的全局最优二分图匹配器
///
/// 相较于简单的贪婪排序匹配，KM 算法严格求解全局权值和最大的匹配，
/// 彻底避免交叉遮挡场景下局部最优抢占导致的连环 ID 交换。
pub fn kuhn_munkres_match(
    tracks: &[STrack],
    dets: &[TrackDetection],
    threshold: f32,
) -> (Vec<(usize, usize)>, Vec<usize>, Vec<usize>) {
    kuhn_munkres_match_cost(
        tracks.len(),
        dets.len(),
        |t, d| box_iou(&tracks[t].bbox, &dets[d].bbox),
        threshold,
    )
}

/// 基于自定义代价评估函数的 Kuhn-Munkres 全局最优匹配（零数据结构克隆抽象）
pub fn kuhn_munkres_match_cost<F: Fn(usize, usize) -> f32>(
    n: usize,
    m: usize,
    cost_fn: F,
    threshold: f32,
) -> (Vec<(usize, usize)>, Vec<usize>, Vec<usize>) {
    if n == 0 {
        return (Vec::new(), Vec::new(), (0..m).collect());
    }
    if m == 0 {
        return (Vec::new(), (0..n).collect(), Vec::new());
    }

    let dim = n.max(m);
    // 构造 dim x dim 展平连续权重矩阵 (以 10000 放大为整数以杜绝浮点精度丢失)
    let mut weight = vec![0i64; dim * dim];

    for t_idx in 0..n {
        let row_offset = t_idx * dim;
        for d_idx in 0..m {
            let iou = cost_fn(t_idx, d_idx);
            if iou >= threshold {
                weight[row_offset + d_idx] = (iou * 10000.0) as i64;
            }
        }
    }

    // 匹配顶标与双向匹配映射数组
    let mut lx = vec![0i64; dim];
    let mut ly = vec![0i64; dim];
    let mut match_x: Vec<Option<usize>> = vec![None; dim]; // match_x[i] = Some(j) 表示左部 i 匹配右部 j
    let mut match_y: Vec<Option<usize>> = vec![None; dim]; // match_y[j] = Some(i) 表示右部 j 匹配左部 i

    // 初始化左顶标为行最大值
    for (i, target) in lx.iter_mut().enumerate() {
        let row_offset = i * dim;
        *target = weight[row_offset..row_offset + dim]
            .iter()
            .copied()
            .max()
            .unwrap_or(0);
    }

    // 预分配增广路搜索辅助缓冲区，循环复用避免每轮重新申请堆内存
    let mut slack = vec![i64::MAX; dim];
    let mut slack_x = vec![0usize; dim];
    let mut prev = vec![None; dim];
    let mut vis_x = vec![false; dim];
    let mut vis_y = vec![false; dim];
    let mut queue = VecDeque::with_capacity(dim);

    // 为每个左部节点寻找增广路 (带 slack 优化的 O(V^3) 实现)
    for root in 0..dim {
        slack_x.fill(root);
        prev.fill(None);
        vis_x.fill(false);
        vis_y.fill(false);
        queue.clear();
        queue.push_back(root);
        vis_x[root] = true;

        let root_offset = root * dim;
        for j in 0..dim {
            slack[j] = lx[root] + ly[j] - weight[root_offset + j];
        }

        let mut matched_y_idx = None;

        'augment: loop {
            while let Some(u) = queue.pop_front() {
                let u_offset = u * dim;
                for v in 0..dim {
                    if !vis_y[v] {
                        let delta = lx[u] + ly[v] - weight[u_offset + v];
                        if delta == 0 {
                            vis_y[v] = true;
                            prev[v] = Some(u);
                            if let Some(next_u) = match_y[v] {
                                vis_x[next_u] = true;
                                queue.push_back(next_u);
                            } else {
                                matched_y_idx = Some(v);
                                break 'augment;
                            }
                        } else if delta < slack[v] {
                            slack[v] = delta;
                            slack_x[v] = u;
                        }
                    }
                }
            }

            // 计算最小松弛量
            let mut delta = i64::MAX;
            for j in 0..dim {
                if !vis_y[j] && slack[j] < delta {
                    delta = slack[j];
                }
            }

            if delta == i64::MAX || delta == 0 {
                break;
            }

            // 修改顶标
            for i in 0..dim {
                if vis_x[i] {
                    lx[i] -= delta;
                }
            }
            for j in 0..dim {
                if vis_y[j] {
                    ly[j] += delta;
                } else {
                    slack[j] -= delta;
                }
            }

            // 检查新的相等子图边
            for j in 0..dim {
                if !vis_y[j] && slack[j] == 0 {
                    vis_y[j] = true;
                    prev[j] = Some(slack_x[j]);
                    if let Some(next_u) = match_y[j] {
                        vis_x[next_u] = true;
                        queue.push_back(next_u);
                    } else {
                        matched_y_idx = Some(j);
                        break 'augment;
                    }
                }
            }
        }

        // 沿增广路径反向更新匹配 (利用 match_x 沿真实交替路回溯)
        if let Some(mut curr_v) = matched_y_idx {
            while let Some(u) = prev[curr_v] {
                let next_v = match_x[u];
                match_y[curr_v] = Some(u);
                match_x[u] = Some(curr_v);
                if let Some(nv) = next_v {
                    curr_v = nv;
                } else {
                    break;
                }
            }
        }
    }

    // 提取有效匹配
    let mut matches = Vec::with_capacity(n.min(m));
    let mut matched_tracks = vec![false; n];
    let mut matched_dets = vec![false; m];

    for (t_idx, maybe_d) in match_x.into_iter().take(n).enumerate() {
        if let Some(d_idx) = maybe_d {
            if d_idx < m && weight[t_idx * dim + d_idx] > 0 {
                matches.push((t_idx, d_idx));
                matched_tracks[t_idx] = true;
                matched_dets[d_idx] = true;
            }
        }
    }

    let unmatched_tracks: Vec<usize> = matched_tracks
        .into_iter()
        .enumerate()
        .filter_map(|(t_idx, matched)| (!matched).then_some(t_idx))
        .collect();

    let unmatched_dets: Vec<usize> = matched_dets
        .into_iter()
        .enumerate()
        .filter_map(|(d_idx, matched)| (!matched).then_some(d_idx))
        .collect();

    (matches, unmatched_tracks, unmatched_dets)
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
    fn test_kalman_predict_and_update_pixel_scale() {
        let rect = [100.0, 100.0, 50.0, 100.0];
        let mut tracker = KalmanBoxTracker::new(&rect);
        tracker.predict();
        let pred = tracker.to_rect();
        assert!((pred[0] - 100.0).abs() < 1.0);
        assert!((pred[1] - 100.0).abs() < 1.0);

        let next_meas = [105.0, 102.0, 50.0, 100.0];
        tracker.update(&next_meas);
        let updated = tracker.to_rect();
        assert!(updated[0] > 100.0 && updated[0] <= 105.0);
    }

    #[test]
    fn test_kalman_predict_and_update_normalized_scale() {
        // 测试在 [0.0, 1.0] 归一化尺度下的卡尔曼滤波数学稳定性
        let rect = [0.20, 0.15, 0.10, 0.30];
        let mut tracker = KalmanBoxTracker::new(&rect);
        tracker.predict();
        let pred = tracker.to_rect();

        // 验证高度绝不会被强行撑大至 1.0
        assert!((pred[3] - 0.30).abs() < 0.02, "高度必须维持尺度不变性");
        assert!((pred[2] - 0.10).abs() < 0.01, "宽度必须维持尺度不变性");

        let next_meas = [0.205, 0.152, 0.10, 0.30];
        tracker.update(&next_meas);
        let updated = tracker.to_rect();
        assert!(updated[0] > 0.20 && updated[0] <= 0.205);
        assert!((updated[3] - 0.30).abs() < 0.02);
    }

    #[test]
    fn test_bytetrack_tracking_continuity_normalized() {
        // 核心验证：在归一化浮点坐标下，连续移动的目标 ID 必须完美保持，杜绝跳变
        let mut tracker = ByteTracker::new(ByteTrackConfig::default());

        // 帧 1: 出现一个人体检测
        let det1 = vec![TrackDetection {
            bbox: [0.30, 0.20, 0.12, 0.38],
            score: 0.90,
            class_id: 0,
        }];
        let active1 = tracker.update(&det1);
        assert_eq!(active1.len(), 1, "第 1 帧冷启动必须激活航迹");
        let id1 = active1[0].track_id;

        // 帧 2: 平移微移
        let det2 = vec![TrackDetection {
            bbox: [0.305, 0.202, 0.12, 0.38],
            score: 0.88,
            class_id: 0,
        }];
        let active2 = tracker.update(&det2);
        assert_eq!(active2.len(), 1);
        assert_eq!(active2[0].track_id, id1, "TrackId 必须连续稳定保持");

        // 帧 3: 连续移动
        let det3 = vec![TrackDetection {
            bbox: [0.310, 0.205, 0.12, 0.38],
            score: 0.85,
            class_id: 0,
        }];
        let active3 = tracker.update(&det3);
        assert_eq!(active3.len(), 1);
        assert_eq!(active3[0].track_id, id1, "TrackId 必须跨帧稳定");
    }

    #[test]
    fn test_bytetrack_low_score_rescue() {
        // 验证 ByteTrack 核心优势：第二阶段低分框成功挽救遮挡目标
        let mut tracker = ByteTracker::new(ByteTrackConfig::default());

        // 帧 1
        let det1 = vec![TrackDetection {
            bbox: [0.2, 0.2, 0.1, 0.3],
            score: 0.9,
            class_id: 0,
        }];
        let active1 = tracker.update(&det1);
        let id1 = active1[0].track_id;

        // 帧 2: 被遮挡，置信度骤降为 0.25 (低于 track_thresh 0.50，但高于 0.10)
        let det2 = vec![TrackDetection {
            bbox: [0.205, 0.202, 0.1, 0.3],
            score: 0.25,
            class_id: 0,
        }];
        let active2 = tracker.update(&det2);
        assert_eq!(active2.len(), 1, "低分框必须成功挽救被遮挡目标");
        assert_eq!(active2[0].track_id, id1, "ID 必须保持不变");
    }

    #[test]
    fn test_false_positive_rejection() {
        // 验证两帧确认机制：第 2 帧及以后的单帧误检不会对外产生虚警 ID
        let mut tracker = ByteTracker::new(ByteTrackConfig::default());

        // 帧 1: 正常目标
        let det1 = vec![TrackDetection {
            bbox: [0.1, 0.1, 0.1, 0.3],
            score: 0.9,
            class_id: 0,
        }];
        let _ = tracker.update(&det1);

        // 帧 2: 目标继续存在，同时右侧突现一个单帧误检 (例如椅子误检)
        let det2 = vec![
            TrackDetection {
                bbox: [0.105, 0.102, 0.1, 0.3],
                score: 0.9,
                class_id: 0,
            },
            TrackDetection {
                bbox: [0.7, 0.7, 0.1, 0.1],
                score: 0.65,
                class_id: 0,
            },
        ];
        let active2 = tracker.update(&det2);
        // 单帧误检在第 2 帧处于未确认状态 (is_activated == false)，不应输出
        assert_eq!(active2.len(), 1, "新目标必须经过确认方可对外激活");

        // 帧 3: 误检消失
        let det3 = vec![TrackDetection {
            bbox: [0.110, 0.104, 0.1, 0.3],
            score: 0.9,
            class_id: 0,
        }];
        let active3 = tracker.update(&det3);
        assert_eq!(active3.len(), 1);
        assert_eq!(active3[0].track_id, active2[0].track_id);
    }

    #[test]
    fn test_kuhn_munkres_global_optimal_assignment() {
        let d_init0 = TrackDetection {
            bbox: [0.0, 0.0, 1.0, 1.0],
            score: 0.9,
            class_id: 0,
        };
        let d_init1 = TrackDetection {
            bbox: [0.0, 0.0, 1.0, 1.0],
            score: 0.9,
            class_id: 0,
        };
        let mut t0 = STrack::new(&d_init0, 1);
        t0.activate(1, 1);
        let mut t1 = STrack::new(&d_init1, 1);
        t1.activate(2, 1);

        let d0 = TrackDetection {
            bbox: [0.0, 0.0, 1.0, 1.0],
            score: 0.9,
            class_id: 0,
        };
        let d1 = TrackDetection {
            bbox: [0.2, 0.0, 1.0, 1.0],
            score: 0.9,
            class_id: 0,
        };

        let tracks = vec![t0, t1];
        let dets = vec![d0, d1];

        let (matches, unmatched_t, unmatched_d) = kuhn_munkres_match(&tracks, &dets, 0.30);
        assert_eq!(matches.len(), 2, "KM 算法必须找到两对完整匹配");
        assert!(unmatched_t.is_empty(), "两路航迹均应被分配");
        assert!(unmatched_d.is_empty(), "两个检测均应被分配");

        let mut matched_tracks_set = std::collections::HashSet::new();
        let mut matched_dets_set = std::collections::HashSet::new();
        for (t_idx, d_idx) in matches {
            assert!(
                matched_tracks_set.insert(t_idx),
                "同一 Track 严禁匹配多个 Detection！"
            );
            assert!(
                matched_dets_set.insert(d_idx),
                "同一 Detection 严禁匹配多个 Track！"
            );
        }
    }

    /// 板端实测的人脸分数上限：`yolov8n-face-640x384_rk3568_mixed_face.rknn`
    /// 的 score 分支量化区间被钳到 0.5，任何人脸都不可能得到更高的分数。
    const SATURATED_FACE_SCORE: f32 = 0.5;

    /// 派生门限（high_thresh == track_thresh == 检测阈值）必须能让饱和分数建轨；
    /// 否则 face_track_id 恒为空，best-shot 与 EdgeFace 提取链路整体失效。
    #[test]
    fn saturated_face_score_creates_track_with_detection_derived_gate() {
        let mut tracker = ByteTracker::new(ByteTrackConfig {
            high_thresh: SATURATED_FACE_SCORE,
            track_thresh: SATURATED_FACE_SCORE,
            confirm_new_tracks: false,
            ..Default::default()
        });
        let detections = vec![TrackDetection {
            bbox: [0.22, 0.21, 0.27, 0.30],
            score: SATURATED_FACE_SCORE,
            class_id: 0,
        }];

        let active = tracker.update(&detections);
        assert_eq!(
            active.len(),
            1,
            "饱和分数 {SATURATED_FACE_SCORE} 必须能建立并激活人脸航迹"
        );

        // 跨帧持续命中必须保持同一 ID，保证 best-shot 融合的身份键稳定。
        let track_id = active[0].track_id;
        let next = vec![TrackDetection {
            bbox: [0.225, 0.211, 0.27, 0.30],
            score: SATURATED_FACE_SCORE,
            class_id: 0,
        }];
        let active_next = tracker.update(&next);
        assert_eq!(active_next.len(), 1);
        assert_eq!(active_next[0].track_id, track_id);
    }

    /// 记录导致本次修复的缺陷：默认建轨门槛 0.60 高于模型可达上限 0.5，航迹永不创建。
    #[test]
    fn default_high_thresh_cannot_track_saturated_face_score() {
        let mut tracker = ByteTracker::new(ByteTrackConfig {
            confirm_new_tracks: false,
            ..Default::default()
        });
        assert!(
            ByteTrackConfig::default().high_thresh > SATURATED_FACE_SCORE,
            "默认建轨门槛必须仍然高于模型可达上限，否则此回归测试失去意义"
        );
        let detections = vec![TrackDetection {
            bbox: [0.22, 0.21, 0.27, 0.30],
            score: SATURATED_FACE_SCORE,
            class_id: 0,
        }];
        assert!(
            tracker.update(&detections).is_empty(),
            "默认 0.60 门槛下饱和分数不应建轨"
        );
    }
}
