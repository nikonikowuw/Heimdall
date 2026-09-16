//! 基于 ByteTrack 航迹生命周期的最佳人脸帧池与时域特征融合状态机。
//!
//! 每条内部航迹只保留有限数量的 embedding 样本，不保留原始像素：
//! - 质量门控后按种子、质量提升和最小采样间隔决定是否提取；
//! - 新特征先经过防漂移校验，再进入最多 `KMAX` 帧的有限池；
//! - 每次入池都按质量排序、去冗余并对 Top-K 样本重新计算模板；
//! - 模板成熟只发射一次显式信号，供宿主 CaptureSettle 提前结算。
//!
//! 池淘汰、融合数学与成熟门限本身见 [`crate::template_pool`]；本模块只管
//! 「何时采样」与「采样结果如何更新航迹状态」。

use std::collections::HashMap;

use crate::quality::FaceQuality;
use crate::template_pool::{BestShotRecord, FrameSample, MaturityFlip};

/// 池内最多保留的高质量 embedding 帧数（KMAX）。
pub const MAX_FUSED_FRAMES: usize = 8;
/// 每次融合最多选取的非冗余帧数（Top-K）。
pub const TOP_K_FUSED_FRAMES: usize = 4;
/// 首次提取与补采样的最低质量门限。
pub const MIN_FUSION_QUALITY_SCORE: f32 = 0.50;
/// 新质量超过当前池内峰值该幅度时允许追质量提取。
pub const DEFAULT_QUALITY_UPGRADE_DELTA: f32 = 0.08;
/// 两次特征提取之间的最小帧间隔。
pub const MIN_FUSION_FRAME_INTERVAL: usize = 6;
/// 新特征相对当前模板的最低余弦相似度。
pub const DRIFT_REJECTION_SIMILARITY: f32 = 0.55;
/// 选样时相对任一已选帧达到该相似度即视为冗余。
pub const REDUNDANCY_SIMILARITY: f32 = 0.85;
/// 模板成熟所需的最少池内样本数。
pub const MIN_MATURE_POOL_SIZE: usize = 3;
/// 连续无质量提升达到该帧数后允许模板成熟。
pub const PLATEAU_FRAMES: usize = 10;
/// 峰值样本人脸短边达到该像素数后允许模板成熟。
pub const SIZE_TARGET_PIXELS: u32 = 140;

/// 一次特征更新后对插件发射层可见的状态。
#[derive(Debug, Clone, PartialEq)]
pub struct FusionUpdate {
    /// 当前融合模板；只有 `template_changed` 时才需要编码发射。
    pub template: [f32; 512],
    /// 实际参与 Top-K 融合的非冗余样本数。
    pub fused_count: usize,
    /// 参与融合样本的质量加权均值。
    pub template_quality: f32,
    /// 本次更新是否改变了融合模板。
    pub template_changed: bool,
    /// 本次是否首次翻转为成熟。
    pub template_mature: bool,
}

/// 航迹最佳人脸帧池状态机。
#[derive(Debug, Default)]
pub struct BestShotManager {
    records: HashMap<u64, BestShotRecord>,
}

impl BestShotManager {
    pub fn new() -> Self {
        Self {
            records: HashMap::new(),
        }
    }

    /// 查询某条航迹当前的最佳抓拍记录。
    pub fn get(&self, track_id: u64) -> Option<&BestShotRecord> {
        self.records.get(&track_id)
    }

    /// 判定当前帧人脸是否应该触发特征提取与融合。
    ///
    /// 池为空时先执行 `SEED_MIN(0.50)` 门控；已有样本时，质量提升、补采样间隔、
    /// 失败退避和成熟状态共同决定是否进入 NPU。模板成熟后停止继续提取，保证单轨发射
    /// 次数与池上限有界。
    pub fn should_update_best_shot(
        &self,
        track_id: u64,
        new_quality: &FaceQuality,
        current_frame_id: usize,
    ) -> bool {
        self.should_update_best_shot_with_delta(
            track_id,
            new_quality,
            DEFAULT_QUALITY_UPGRADE_DELTA,
            current_frame_id,
        )
    }

    /// 带自定义质量增量阈值的采样判定。
    pub fn should_update_best_shot_with_delta(
        &self,
        track_id: u64,
        new_quality: &FaceQuality,
        delta: f32,
        current_frame_id: usize,
    ) -> bool {
        let Some(previous) = self.records.get(&track_id) else {
            return new_quality.score >= MIN_FUSION_QUALITY_SCORE;
        };

        if previous.template_mature || current_frame_id < previous.retry_after_frame_id {
            return false;
        }

        if previous.pool_is_empty() {
            return new_quality.score >= MIN_FUSION_QUALITY_SCORE;
        }

        new_quality.score > previous.best_pool_quality() + delta
            || (previous.pool_len() < MAX_FUSED_FRAMES
                && current_frame_id.saturating_sub(previous.last_extract_frame_id)
                    >= MIN_FUSION_FRAME_INTERVAL
                && new_quality.score >= MIN_FUSION_QUALITY_SCORE)
    }

    /// 更新某条航迹并返回模板变化/成熟翻转元数据。
    ///
    /// 采样失败时返回未变化的快照，调用方据此跳过发射。
    #[allow(clippy::too_many_arguments)]
    pub fn update_with_fusion_result(
        &mut self,
        track_id: u64,
        bbox: [f32; 4],
        landmarks: [[f32; 2]; 5],
        score: f32,
        quality: FaceQuality,
        new_embedding: &[f32; 512],
        frame_id: usize,
    ) -> FusionUpdate {
        let record = self.records.entry(track_id).or_insert_with(|| {
            BestShotRecord::uninitialized(bbox, landmarks, score, quality, frame_id)
        });

        if new_embedding.iter().any(|value| !value.is_finite()) {
            record.mark_extraction_failure(frame_id);
            return unchanged_update(record, MaturityFlip::None);
        }
        let norm_sq: f32 = new_embedding.iter().map(|value| value * value).sum();
        if !norm_sq.is_finite() || norm_sq <= 1e-12 {
            record.mark_extraction_failure(frame_id);
            return unchanged_update(record, MaturityFlip::None);
        }

        // 只有已有有效模板时才做防漂移判定；首个样本负责建立模板基准。
        if !record.pool_is_empty() {
            let similarity = crate::cosine_similarity(new_embedding, record.template());
            if similarity < DRIFT_REJECTION_SIMILARITY {
                tracing::warn!(
                    track_id,
                    similarity,
                    threshold = DRIFT_REJECTION_SIMILARITY,
                    "特征融合防漂移校验拦截：新特征与当前模板余弦相似度过低，拒绝进入帧池"
                );
                record.mark_extraction_failure(frame_id);
                return unchanged_update(record, MaturityFlip::None);
            }
        }

        let sample = FrameSample {
            embedding: *new_embedding,
            quality,
            score,
            bbox,
            landmarks,
            frame_id,
        };
        // 顺序固定：先吸收样本（内含失败/成功状态机），再判定成熟，最后组装快照。
        let template_changed = record.absorb_sample(sample, frame_id);
        let maturity = record.refresh_maturity(frame_id);
        FusionUpdate {
            template: *record.template(),
            fused_count: record.fused_count,
            template_quality: record.template_quality,
            template_changed,
            template_mature: maturity.flipped(),
        }
    }

    /// 在没有新 embedding 的帧上推进成熟 FSM；成熟首次翻转时返回一次握手信号。
    pub fn maturity_signal(&mut self, track_id: u64, frame_id: usize) -> Option<FusionUpdate> {
        let record = self.records.get_mut(&track_id)?;
        if record.pool_is_empty() || record.template_mature {
            return None;
        }
        match record.refresh_maturity(frame_id) {
            MaturityFlip::FirstTime => Some(unchanged_update(record, MaturityFlip::FirstTime)),
            MaturityFlip::None => None,
        }
    }

    /// 记录一次没有得到新 embedding 的 best-shot 尝试。
    ///
    /// 保留已有帧池与模板，避免暂时性的读回或模型错误导致每帧重复执行重型路径。
    pub fn record_attempt_without_embedding(
        &mut self,
        track_id: u64,
        bbox: [f32; 4],
        landmarks: [[f32; 2]; 5],
        score: f32,
        quality: FaceQuality,
        frame_id: usize,
    ) {
        self.records
            .entry(track_id)
            .and_modify(|record| {
                record.mark_extraction_failure(frame_id);
                if record.pool_is_empty() && quality.score > record.quality.score {
                    record.bbox = bbox;
                    record.landmarks = landmarks;
                    record.score = score;
                    record.quality = quality;
                }
            })
            .or_insert_with(|| {
                let mut record =
                    BestShotRecord::uninitialized(bbox, landmarks, score, quality, frame_id);
                record.mark_extraction_failure(frame_id);
                record
            });
    }

    /// 删除已经进入 `Removed` 状态的航迹对应记录；`Lost` 航迹必须保留。
    pub fn remove_tracks(&mut self, removed_track_ids: &[u64]) {
        for track_id in removed_track_ids {
            self.records.remove(track_id);
        }
    }

    /// 清空所有状态。
    pub fn clear(&mut self) {
        self.records.clear();
    }
}

/// 构造“模板未变化”的状态快照。
///
/// 入参是 [`MaturityFlip`] 而不是 `bool`：调用点写 `MaturityFlip::None` / `FirstTime`
/// 自解释，避免出现 `unchanged_update(record, true)` 这种读不出含义的布尔实参。
fn unchanged_update(record: &BestShotRecord, maturity: MaturityFlip) -> FusionUpdate {
    FusionUpdate {
        template: *record.template(),
        fused_count: record.fused_count,
        template_quality: record.template_quality,
        template_changed: false,
        template_mature: maturity.flipped(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn quality(score: f32, face_size: u32) -> FaceQuality {
        FaceQuality {
            score,
            blur: 0.2,
            yaw: 3.0,
            pitch: 2.0,
            face_size,
        }
    }

    fn embedding(axis: usize) -> [f32; 512] {
        let mut value = [0.0f32; 512];
        value[axis] = 1.0;
        value
    }

    fn similar_embedding(axis: usize) -> [f32; 512] {
        let mut value = [0.0f32; 512];
        value[0] = 0.7;
        value[axis] = (1.0 - 0.7f32 * 0.7).sqrt();
        value
    }

    fn add(
        manager: &mut BestShotManager,
        track_id: u64,
        score: f32,
        face_size: u32,
        vector: &[f32; 512],
        frame_id: usize,
    ) -> FusionUpdate {
        manager.update_with_fusion_result(
            track_id,
            [0.1, 0.1, 0.3, 0.3],
            [[0.0; 2]; 5],
            0.9,
            quality(score, face_size),
            vector,
            frame_id,
        )
    }

    #[test]
    fn seed_gate_rejects_weak_frame_before_npu_extraction() {
        let manager = BestShotManager::new();
        assert!(!manager.should_update_best_shot(7, &quality(0.49, 80), 1));
        assert!(manager.should_update_best_shot(7, &quality(0.50, 80), 1));
    }

    #[test]
    fn failed_extraction_enters_exponential_backoff() {
        let mut manager = BestShotManager::new();
        let q = quality(0.60, 80);
        manager.record_attempt_without_embedding(7, [0.1; 4], [[0.0; 2]; 5], 0.9, q, 1);
        assert!(!manager.should_update_best_shot(7, &q, 1));
        assert!(!manager.should_update_best_shot(7, &q, 6));
        assert!(manager.should_update_best_shot(7, &q, 7));

        manager.record_attempt_without_embedding(7, [0.1; 4], [[0.0; 2]; 5], 0.9, q, 7);
        assert!(!manager.should_update_best_shot(7, &q, 18));
        assert!(manager.should_update_best_shot(7, &q, 19));

        manager.record_attempt_without_embedding(7, [0.1; 4], [[0.0; 2]; 5], 0.9, q, 19);
        assert!(!manager.should_update_best_shot(7, &q, 42));
        assert!(manager.should_update_best_shot(7, &q, 43));

        manager.record_attempt_without_embedding(7, [0.1; 4], [[0.0; 2]; 5], 0.9, q, 43);
        assert!(!manager.should_update_best_shot(7, &q, 90));
        assert!(manager.should_update_best_shot(7, &q, 91));
    }

    #[test]
    fn pool_is_bounded_and_evicts_lowest_quality_sample() {
        let mut manager = BestShotManager::new();
        let vector = embedding(0);
        for index in 0..MAX_FUSED_FRAMES {
            add(
                &mut manager,
                1,
                0.50 + index as f32 * 0.03,
                80,
                &vector,
                index + 1,
            );
        }
        let record = manager.get(1).expect("record should exist");
        assert_eq!(record.pool_len(), MAX_FUSED_FRAMES);
        assert_eq!(record.quality.score, 0.71);
        assert!(record.template_mature, "池满后必须翻转成熟状态");
        assert!(!manager.should_update_best_shot(1, &quality(0.99, 120), 20));

        let update = add(&mut manager, 1, 0.99, 120, &vector, 20);
        let record = manager.get(1).expect("record should exist");
        assert_eq!(record.pool_len(), MAX_FUSED_FRAMES);
        assert_eq!(record.quality.score, 0.99);
        assert!(update.template_changed);
    }

    #[test]
    fn redundant_samples_are_removed_before_top_k_fusion() {
        let mut manager = BestShotManager::new();
        let repeated = embedding(0);
        let distinct = similar_embedding(1);
        add(&mut manager, 2, 0.60, 80, &repeated, 1);
        add(&mut manager, 2, 0.70, 80, &repeated, 2);
        add(&mut manager, 2, 0.80, 80, &repeated, 3);
        add(&mut manager, 2, 0.90, 80, &repeated, 4);
        let update = add(&mut manager, 2, 0.70, 80, &distinct, 7);

        let record = manager.get(2).expect("record should exist");
        assert_eq!(record.pool_len(), 5);
        assert_eq!(record.fused_count, 2, "同向重复帧不得占用 Top-K");
        assert!(update.template_quality > 0.0);
    }

    #[test]
    fn quality_squared_weight_favors_high_quality_embedding() {
        let mut manager = BestShotManager::new();
        let first = embedding(0);
        let second = [0.8, 0.6].into_iter().chain([0.0; 510]).collect::<Vec<_>>();
        let second: [f32; 512] = second.try_into().expect("512 dimensions");
        add(&mut manager, 3, 0.50, 80, &first, 1);
        let update = add(&mut manager, 3, 1.0, 100, &second, 7);

        let similarity_to_first = crate::cosine_similarity(&update.template, &first);
        let similarity_to_second = crate::cosine_similarity(&update.template, &second);
        assert!(similarity_to_second > similarity_to_first);
        assert!(
            (update
                .template
                .iter()
                .map(|value| value * value)
                .sum::<f32>()
                .sqrt()
                - 1.0)
                .abs()
                < 1e-5
        );
    }

    #[test]
    fn mature_signal_flips_after_three_samples_and_plateau() {
        let mut manager = BestShotManager::new();
        let first = embedding(0);
        let second = similar_embedding(1);
        let third = similar_embedding(2);
        add(&mut manager, 4, 0.60, 80, &first, 1);
        add(&mut manager, 4, 0.65, 80, &second, 7);
        let update = add(&mut manager, 4, 0.70, 80, &third, 13);
        assert!(!update.template_mature);

        let mature = manager
            .maturity_signal(4, 23)
            .expect("平台期应触发成熟握手");
        assert!(mature.template_mature);
        assert!(!mature.template_changed);
        assert!(manager.maturity_signal(4, 24).is_none());
        assert!(manager.get(4).expect("record should exist").template_mature);
    }

    #[test]
    fn large_peak_face_matures_template_without_waiting_for_plateau() {
        let mut manager = BestShotManager::new();
        let first = embedding(0);
        let second = similar_embedding(1);
        let third = similar_embedding(2);
        add(&mut manager, 5, 0.60, 80, &first, 1);
        add(&mut manager, 5, 0.65, 100, &second, 7);
        let update = add(&mut manager, 5, 0.70, SIZE_TARGET_PIXELS, &third, 13);
        assert!(update.template_mature);
        assert!(manager.get(5).expect("record should exist").template_mature);
    }

    #[test]
    fn late_large_quality_frame_can_reseed_and_replace_peak() {
        let mut manager = BestShotManager::new();
        let first = embedding(0);
        let second = similar_embedding(1);
        let third = similar_embedding(2);
        add(&mut manager, 6, 0.50, 60, &first, 1);
        add(&mut manager, 6, 0.60, 80, &second, 7);
        assert!(manager.should_update_best_shot(6, &quality(0.90, 160), 13));
        add(&mut manager, 6, 0.90, 160, &third, 13);
        let record = manager.get(6).expect("record should exist");
        assert_eq!(record.quality.score, 0.90);
        assert_eq!(record.best_face_size(), Some(160));
    }

    #[test]
    fn drift_rejection_enters_backoff_without_pool_pollution() {
        let mut manager = BestShotManager::new();
        let stable = embedding(0);
        let drift = embedding(100);
        add(&mut manager, 7, 0.60, 80, &stable, 1);
        assert!(manager.should_update_best_shot(7, &quality(0.95, 120), 7));
        let update = add(&mut manager, 7, 0.95, 120, &drift, 7);
        assert!(!update.template_changed);
        let record = manager.get(7).expect("record should exist");
        assert_eq!(record.pool_len(), 1);
        assert!(!manager.should_update_best_shot(7, &quality(0.95, 120), 8));
        assert!(!manager.should_update_best_shot(7, &quality(0.95, 120), 12));
        assert!(manager.should_update_best_shot(7, &quality(0.95, 120), 13));
    }

    #[test]
    fn non_finite_and_degenerate_embeddings_are_rejected() {
        let mut manager = BestShotManager::new();
        let mut nan_vector = embedding(0);
        nan_vector[3] = f32::NAN;
        let update = add(&mut manager, 8, 0.80, 80, &nan_vector, 1);
        assert!(!update.template_changed);
        assert_eq!(
            manager.get(8).expect("record should exist").pool_len(),
            0,
            "非有限向量不得污染帧池"
        );

        let update = add(&mut manager, 8, 0.80, 80, &[0.0f32; 512], 5);
        assert!(!update.template_changed);
        assert_eq!(manager.get(8).expect("record should exist").pool_len(), 0);

        let update = add(&mut manager, 8, 0.80, 80, &embedding(0), 9);
        assert!(update.template_changed);
        assert_eq!(manager.get(8).expect("record should exist").fused_count, 1);
    }
}
