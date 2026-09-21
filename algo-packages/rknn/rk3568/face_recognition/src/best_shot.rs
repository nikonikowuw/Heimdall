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
use crate::template_pool::{BestShotRecord, FrameSample};

/// 池内最多保留的高质量 embedding 帧数（KMAX）。
pub const MAX_FUSED_FRAMES: usize = 8;
/// 每次融合最多选取的非冗余帧数（Top-K）。
pub const TOP_K_FUSED_FRAMES: usize = 4;
/// 首次提取与补采样的最低质量门限。
pub const MIN_FUSION_QUALITY_SCORE: f32 = 0.50;
/// 实例配置 `fusion_min_quality_score` 的缺省值（见 [`MIN_FUSION_QUALITY_SCORE`]）。
pub const DEFAULT_FUSION_MIN_QUALITY_SCORE: f32 = MIN_FUSION_QUALITY_SCORE;
/// 新质量超过当前池内峰值该幅度时允许追质量提取。
pub const DEFAULT_QUALITY_UPGRADE_DELTA: f32 = 0.08;
/// 两次特征提取之间的最小帧间隔。
pub const MIN_FUSION_FRAME_INTERVAL: usize = 4;
/// 新特征相对当前模板的最低余弦相似度。
pub const DRIFT_REJECTION_SIMILARITY: f32 = 0.50;
/// 选样时相对任一已选帧达到该相似度即视为冗余。
pub const REDUNDANCY_SIMILARITY: f32 = 0.95;
/// 模板成熟所需的最少池内样本数。
pub const MIN_MATURE_POOL_SIZE: usize = 3;
/// 连续无质量提升达到该帧数后允许模板成熟。
pub const PLATEAU_FRAMES: usize = 10;
/// 人脸尺寸达到该阈值时允许提前成熟（免受连续无提升帧数限制）。
pub const SIZE_TARGET_PIXELS: u32 = 140;

/// 模板重播种触发门限：当新样本质量比池内最佳高出该值时，允许清空旧弱池。
pub const RESEED_QUALITY_DELTA: f32 = 0.12;
/// 重播种时的余弦相似度下限，防止彻底异人漂移引发误重置。
pub const RESEED_SIMILARITY_FLOOR: f32 = 0.25;

/// 融合更新结果
#[derive(Debug, Clone)]
pub struct FusionUpdate {
    pub template: [f32; 512],
    pub template_quality: f32,
    pub fused_count: usize,
    pub template_mature: bool,
    pub template_changed: bool,
}

/// 航迹状态更新
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrackUpdateStatus {
    Updated,
    Removed,
}

/// 基于 ByteTrack 航迹生命周期维护最佳帧与融合特征
#[derive(Debug, Default)]
pub struct BestShotManager {
    records: HashMap<u64, BestShotRecord>,
}

impl BestShotManager {
    pub fn new() -> Self {
        Self::default()
    }

    /// 判定是否应为当前帧触发 NPU 人脸特征提取。
    ///
    /// `min_extract_quality` 是「值得花一次 EdgeFace 前向」的质量下限，由实例配置
    /// `fusion_min_quality_score` 提供（默认 [`MIN_FUSION_QUALITY_SCORE`]）。
    /// 现场放宽 `quality_min_score` 必须同步放宽该门限，否则落入两者之间的人脸会
    /// 通过检测与准入审核、却永远拿不到特征向量。
    pub fn should_update_best_shot(
        &self,
        track_id: u64,
        quality: &FaceQuality,
        min_extract_quality: f32,
        frame_id: usize,
    ) -> bool {
        self.should_update_best_shot_with_delta(
            track_id,
            quality,
            DEFAULT_QUALITY_UPGRADE_DELTA,
            min_extract_quality,
            frame_id,
        )
    }

    /// 判定是否应为当前帧触发 NPU 特征提取（带显式提升增量）。
    pub fn should_update_best_shot_with_delta(
        &self,
        track_id: u64,
        quality: &FaceQuality,
        upgrade_delta: f32,
        min_extract_quality: f32,
        frame_id: usize,
    ) -> bool {
        let Some(record) = self.records.get(&track_id) else {
            return quality.score >= min_extract_quality;
        };

        if record.template_mature || frame_id < record.retry_after_frame_id {
            return false;
        }

        if record.pool_is_empty() {
            return quality.score >= min_extract_quality;
        }

        let quality_improved = quality.score >= record.best_pool_quality() + upgrade_delta;
        let interval_elapsed =
            frame_id.saturating_sub(record.last_extract_frame_id) >= MIN_FUSION_FRAME_INTERVAL;

        if record.pool_len() < MIN_MATURE_POOL_SIZE {
            // 当池内样本数尚未达到成熟所需的最低数量（3 帧）时，
            // 只要达到准入门限且不低于当前峰值 0.10（防止严重劣化帧混入），即允许补充采样，
            // 避免因首帧碰巧处于极高分（如 0.83）而导致后续 0.80~0.82 的优质帧被全盘拒识、永远停留于单帧。
            interval_elapsed
                && quality.score >= min_extract_quality
                && quality.score >= record.best_pool_quality() - 0.10
        } else if record.pool_len() < MAX_FUSED_FRAMES {
            interval_elapsed && (quality_improved || quality.score >= record.best_pool_quality())
        } else {
            interval_elapsed && quality_improved
        }
    }

    /// 记录一次未获得特征向量的提取尝试。
    pub fn record_attempt_without_embedding(
        &mut self,
        track_id: u64,
        bbox: [f32; 4],
        landmarks: [[f32; 2]; 5],
        score: f32,
        quality: FaceQuality,
        frame_id: usize,
    ) {
        let record = self.records.entry(track_id).or_insert_with(|| {
            BestShotRecord::uninitialized(bbox, landmarks, score, quality, frame_id)
        });
        record.mark_extraction_failure(frame_id);
    }

    /// 接收新的特征向量，推进帧池并更新模板。
    #[allow(clippy::too_many_arguments)]
    pub fn update_with_fusion_result(
        &mut self,
        track_id: u64,
        bbox: [f32; 4],
        landmarks: [[f32; 2]; 5],
        score: f32,
        quality: FaceQuality,
        embedding: &[f32; 512],
        frame_id: usize,
    ) -> FusionUpdate {
        let sample = FrameSample {
            embedding: *embedding,
            quality,
            score,
            bbox,
            landmarks,
            frame_id,
        };

        let record = self.records.entry(track_id).or_insert_with(|| {
            BestShotRecord::uninitialized(bbox, landmarks, score, quality, frame_id)
        });

        if !record.pool_is_empty() {
            let similarity = algo_sdk::math::cosine_similarity(record.template(), embedding);

            let is_single_seed = record.pool_len() == 1;
            // 若池内仅有单个初始种子，且后方出现质量显著优越（+0.08 且绝对分 >= 0.70）的高清正脸：
            // 若初始种子本身质量处于极低准入门限附近 (< 0.59)，即便相似度因早期弱照/偏转跌破下限也允许重播种洗掉弱种子；
            // 若初始种子已达 0.60+，则维持 RESEED_SIMILARITY_FLOOR 底线防跨人漂移。
            let single_seed_overwrite = is_single_seed
                && quality.score >= 0.70
                && quality.score >= record.best_pool_quality() + 0.08
                && (similarity >= RESEED_SIMILARITY_FLOOR || record.best_pool_quality() < 0.59);

            let can_reseed = single_seed_overwrite
                || (quality.score >= record.best_pool_quality() + RESEED_QUALITY_DELTA
                    && similarity >= RESEED_SIMILARITY_FLOOR);

            if similarity < DRIFT_REJECTION_SIMILARITY && !can_reseed {
                record.mark_drift_rejection(frame_id);
                return FusionUpdate {
                    template: *record.template(),
                    template_quality: record.template_quality,
                    fused_count: record.fused_count,
                    template_mature: false,
                    template_changed: false,
                };
            }

            if can_reseed && similarity < DRIFT_REJECTION_SIMILARITY {
                let previously_mature = record.template_mature;
                *record = BestShotRecord::uninitialized(bbox, landmarks, score, quality, frame_id);
                record.template_mature = previously_mature;
            }
        }

        let template_changed = record.absorb_sample(sample, frame_id);
        let mature_flipped = record.refresh_maturity(frame_id).flipped();

        FusionUpdate {
            template: *record.template(),
            template_quality: record.template_quality,
            fused_count: record.fused_count,
            template_mature: mature_flipped,
            template_changed,
        }
    }

    /// 轮询航迹是否在当前帧达到平台期成熟。
    pub fn maturity_signal(&mut self, track_id: u64, frame_id: usize) -> Option<FusionUpdate> {
        let record = self.records.get_mut(&track_id)?;
        if !record.refresh_maturity(frame_id).flipped() {
            return None;
        }

        Some(FusionUpdate {
            template: *record.template(),
            template_quality: record.template_quality,
            fused_count: record.fused_count,
            template_mature: true,
            template_changed: false,
        })
    }

    pub fn get(&self, track_id: u64) -> Option<&BestShotRecord> {
        self.records.get(&track_id)
    }

    pub fn remove_tracks(&mut self, track_ids: &[u64]) {
        for track_id in track_ids {
            self.records.remove(track_id);
        }
    }

    pub fn clear(&mut self) {
        self.records.clear();
    }

    pub fn retain_active_tracks(&mut self, active_track_ids: &[u64]) {
        self.records
            .retain(|track_id, _| active_track_ids.contains(track_id));
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

    /// 默认融合门限下的采样判定（测试助手）。
    fn should_sample(
        manager: &BestShotManager,
        track_id: u64,
        quality: &FaceQuality,
        frame_id: usize,
    ) -> bool {
        manager.should_update_best_shot(
            track_id,
            quality,
            DEFAULT_FUSION_MIN_QUALITY_SCORE,
            frame_id,
        )
    }

    #[test]
    fn seed_gate_rejects_weak_frame_before_npu_extraction() {
        let manager = BestShotManager::new();
        assert!(!should_sample(&manager, 7, &quality(0.49, 80), 1));
        assert!(should_sample(&manager, 7, &quality(0.50, 80), 1));
    }

    #[test]
    fn extraction_gate_follows_configured_min_quality() {
        let manager = BestShotManager::new();
        // 放宽到 0.25 后，0.30 的人脸必须能触发提取。
        // 历史缺陷：门限被硬编码常量 0.50 静默覆盖，`.env` 放宽建议完全失效。
        assert!(manager.should_update_best_shot(9, &quality(0.30, 80), 0.25, 1));
        assert!(!manager.should_update_best_shot(9, &quality(0.20, 80), 0.25, 1));
        // 收紧到 0.80 后，0.60 的人脸不再消耗 EdgeFace 前向。
        assert!(!manager.should_update_best_shot(9, &quality(0.60, 80), 0.80, 1));
    }

    #[test]
    fn failed_extraction_enters_exponential_backoff() {
        let mut manager = BestShotManager::new();
        let q = quality(0.60, 80);
        manager.record_attempt_without_embedding(7, [0.1; 4], [[0.0; 2]; 5], 0.9, q, 1);
        assert!(!should_sample(&manager, 7, &q, 1));
        assert!(!should_sample(
            &manager,
            7,
            &q,
            1 + MIN_FUSION_FRAME_INTERVAL - 1
        ));
        assert!(should_sample(
            &manager,
            7,
            &q,
            1 + MIN_FUSION_FRAME_INTERVAL
        ));

        let retry2 = 1 + MIN_FUSION_FRAME_INTERVAL;
        manager.record_attempt_without_embedding(7, [0.1; 4], [[0.0; 2]; 5], 0.9, q, retry2);
        let exp2_delay = MIN_FUSION_FRAME_INTERVAL * 2;
        assert!(!should_sample(&manager, 7, &q, retry2 + exp2_delay - 1));
        assert!(should_sample(&manager, 7, &q, retry2 + exp2_delay));

        let retry3 = retry2 + exp2_delay;
        manager.record_attempt_without_embedding(7, [0.1; 4], [[0.0; 2]; 5], 0.9, q, retry3);
        let exp3_delay = MIN_FUSION_FRAME_INTERVAL * 4;
        assert!(!should_sample(&manager, 7, &q, retry3 + exp3_delay - 1));
        assert!(should_sample(&manager, 7, &q, retry3 + exp3_delay));

        let retry4 = retry3 + exp3_delay;
        manager.record_attempt_without_embedding(7, [0.1; 4], [[0.0; 2]; 5], 0.9, q, retry4);
        let exp4_delay = MIN_FUSION_FRAME_INTERVAL * 8;
        assert!(!should_sample(&manager, 7, &q, retry4 + exp4_delay - 1));
        assert!(should_sample(&manager, 7, &q, retry4 + exp4_delay));
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
        assert!(!should_sample(&manager, 1, &quality(0.99, 120), 20));

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

        let similarity_to_first = algo_sdk::math::cosine_similarity(&update.template, &first);
        let similarity_to_second = algo_sdk::math::cosine_similarity(&update.template, &second);
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
        assert!(should_sample(&manager, 6, &quality(0.90, 160), 13));
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
        let next_f = 1 + MIN_FUSION_FRAME_INTERVAL;
        assert!(should_sample(&manager, 7, &quality(0.95, 120), next_f));
        let update = add(&mut manager, 7, 0.95, 120, &drift, next_f);
        assert!(!update.template_changed);
        let record = manager.get(7).expect("record should exist");
        assert_eq!(record.pool_len(), 1);
        assert_eq!(record.quality.score, 0.60);
        assert_eq!(
            record.retry_after_frame_id,
            next_f + MIN_FUSION_FRAME_INTERVAL
        );
    }

    #[test]
    fn consecutive_drift_rejections_do_not_escalate_exponential_backoff() {
        let mut manager = BestShotManager::new();
        let stable = embedding(0);
        let drift = embedding(100);
        add(&mut manager, 8, 0.60, 80, &stable, 1);

        let step = MIN_FUSION_FRAME_INTERVAL;
        // 连续 3 次漂移拒绝
        add(&mut manager, 8, 0.60, 80, &drift, 1 + step);
        let r1 = manager.get(8).expect("record exists");
        assert_eq!(r1.retry_after_frame_id, 1 + step * 2, "第 1 次漂移");

        add(&mut manager, 8, 0.60, 80, &drift, 1 + step * 2);
        let r2 = manager.get(8).expect("record exists");
        assert_eq!(
            r2.retry_after_frame_id,
            1 + step * 3,
            "第 2 次漂移必须维持单次重试间隔，严禁升级为指数退避"
        );

        add(&mut manager, 8, 0.60, 80, &drift, 1 + step * 3);
        let r3 = manager.get(8).expect("record exists");
        assert_eq!(
            r3.retry_after_frame_id,
            1 + step * 4,
            "第 3 次漂移必须维持单次重试间隔，严禁升级为指数退避"
        );
    }

    #[test]
    fn single_seed_overwritten_by_clear_face_with_low_similarity() {
        let mut manager = BestShotManager::new();
        let polluted_seed = embedding(0);

        // 模拟一个相似度仅 0.30（跌破 0.55 漂移门、但高于 0.25 下限）的高质量帧 (q=0.85)
        let mut clear_face = [0.0f32; 512];
        clear_face[0] = 0.30;
        clear_face[1] = (1.0 - 0.30f32 * 0.30).sqrt();

        add(&mut manager, 88, 0.65, 70, &polluted_seed, 1);
        let before = manager.get(88).expect("record exists");
        assert_eq!(before.best_pool_quality(), 0.65);

        // 高质量帧到达：触发弱单种子覆写重播种
        let update = add(&mut manager, 88, 0.85, 150, &clear_face, 7);
        assert!(
            update.template_changed,
            "受污染的弱种子必须被新高质量帧覆盖"
        );

        let after = manager.get(88).expect("record exists");
        assert_eq!(after.best_pool_quality(), 0.85);
        assert_eq!(after.pool_len(), 1, "弱种子被清空，重置为高质量单样本");

        let sim = algo_sdk::math::cosine_similarity(&update.template, &clear_face);
        assert!((sim - 1.0).abs() < 1e-5);
    }

    #[test]
    fn single_weak_seed_below_068_overwritten_even_with_very_low_similarity() {
        let mut manager = BestShotManager::new();
        let polluted_seed = embedding(0);

        // 模拟一个相似度仅 0.05（极低相似度，远低于 0.25 下限）的高质量帧 (q=0.85)
        let mut clear_face = [0.0f32; 512];
        clear_face[0] = 0.05;
        clear_face[1] = (1.0 - 0.05f32 * 0.05).sqrt();

        // 初始弱种子 (q=0.58 < 0.68)
        add(&mut manager, 89, 0.58, 65, &polluted_seed, 1);
        let before = manager.get(89).expect("record exists");
        assert_eq!(before.best_pool_quality(), 0.58);

        // 高质量帧到达：触发弱单种子覆写重播种
        let update = add(&mut manager, 89, 0.85, 150, &clear_face, 7);
        assert!(
            update.template_changed,
            "极低质量的初生弱种子必须被新高质量正脸彻底覆盖，即便余弦相似度极低"
        );

        let after = manager.get(89).expect("record exists");
        assert_eq!(after.best_pool_quality(), 0.85);
        assert_eq!(after.pool_len(), 1, "弱种子被清空，重置为高质量单样本");

        let sim = algo_sdk::math::cosine_similarity(&update.template, &clear_face);
        assert!((sim - 1.0).abs() < 1e-5);
    }

    #[test]
    fn superior_quality_frame_reseeds_weak_pool() {
        let mut manager = BestShotManager::new();
        let weak_seed = embedding(0);

        let mut superior = [0.0f32; 512];
        superior[0] = 0.48;
        superior[1] = (1.0 - 0.48f32 * 0.48).sqrt();

        add(&mut manager, 99, 0.52, 60, &weak_seed, 1);
        let record_before = manager.get(99).expect("record should exist");
        assert_eq!(record_before.best_pool_quality(), 0.52);

        let update = add(&mut manager, 99, 0.88, 140, &superior, 7);
        assert!(update.template_changed);

        let record_after = manager.get(99).expect("record should exist");
        assert_eq!(record_after.best_pool_quality(), 0.88);
        assert_eq!(record_after.pool_len(), 1, "历史弱池应被清空重播种");

        let sim_to_superior = algo_sdk::math::cosine_similarity(&update.template, &superior);
        assert!((sim_to_superior - 1.0).abs() < 1e-5);
    }

    #[test]
    fn reseed_preserves_mature_monotonicity() {
        let mut manager = BestShotManager::new();
        let first = embedding(0);
        let second = similar_embedding(1);
        let third = similar_embedding(2);
        add(&mut manager, 100, 0.60, 80, &first, 1);
        add(&mut manager, 100, 0.65, 100, &second, 7);
        let update_mature = add(&mut manager, 100, 0.66, SIZE_TARGET_PIXELS, &third, 13);
        assert!(
            update_mature.template_mature,
            "达到 140px 应当首次翻转为成熟"
        );
        assert!(manager.get(100).expect("record exists").template_mature);

        let mut superior = [0.0f32; 512];
        superior[0] = 0.48;
        superior[1] = (1.0 - 0.48f32 * 0.48).sqrt();
        let update_reseed = add(&mut manager, 100, 0.90, 160, &superior, 19);
        assert!(update_reseed.template_changed);
        assert!(
            !update_reseed.template_mature,
            "已成熟轨道重播种不应重复发射成熟握手信号"
        );
        assert!(
            manager.get(100).expect("record exists").template_mature,
            "成熟标记必须保持单调为 true"
        );
    }
}
