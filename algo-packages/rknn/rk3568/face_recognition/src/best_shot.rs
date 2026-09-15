//! 基于 ByteTrack 航迹生命周期的动态最佳人脸抓拍与时域特征融合状态机
//!
//! 1. 维护每个活跃 `track_id` 的历史最优人脸质量分与超球面加权融合特征向量；
//! 2. 初次入镜捕获合格人脸即触发特征提取，并在后续时序中以适度采样间隔融合高质量帧；
//! 3. 引入余弦相似度防漂移校验（Outlier Defense），防止跟踪漂移或遮挡误检污染特征池；
//! 4. 随 ByteTrack 航迹注销级联清理，保证内存严格有界。

use std::collections::HashMap;

use crate::quality::FaceQuality;

/// 默认最优抓拍质量分提升门限（当前质量分至少比历史高 0.08 才允许强制升级）
pub const DEFAULT_QUALITY_UPGRADE_DELTA: f32 = 0.08;

/// 单条航迹最多融合的高质量人脸特征帧数 (兼顾时序信噪比与边缘算力开销)
pub const MAX_FUSED_FRAMES: usize = 4;

/// 多帧特征融合间的最小采样帧间隔 (避免连续帧提取高度相关的冗余特征)
pub const MIN_FUSION_FRAME_INTERVAL: usize = 6;

/// 特征融合防漂移余弦相似度门限 (低于 0.55 则拒绝融合，防御跟踪漂移或人脸混淆)
pub const DRIFT_REJECTION_SIMILARITY: f32 = 0.55;

/// 允许参与特征融合的最低质量分门限
pub const MIN_FUSION_QUALITY_SCORE: f32 = 0.50;

/// 失败退避的最大帧间隔上限 (避免长时遮挡下无限期停止重试)
const MAX_RETRY_DELAY_FRAMES: usize = 48;

/// 最佳抓拍人脸记录与多帧特征融合状态
#[derive(Debug, Clone)]
pub struct BestShotRecord {
    pub bbox: [f32; 4],
    pub landmarks: [[f32; 2]; 5],
    pub score: f32,
    pub quality: FaceQuality,
    pub embedding: Vec<f32>,
    pub frame_id: usize,
    /// 已参与特征融合的帧数
    pub fused_count: usize,
    /// 累积的质量权重和
    pub total_weight: f32,
    /// 最近一次提取特征的帧序号
    pub last_extract_frame_id: usize,
    /// 下一次允许重试的帧序号；用于隔离设备/队列瞬时失败
    pub retry_after_frame_id: usize,
    failed_attempts: u8,
}

impl BestShotRecord {
    /// 新建一条尚未获得特征的记录；`retry_after_frame_id` 默认立即可重试。
    fn pending(
        bbox: [f32; 4],
        landmarks: [[f32; 2]; 5],
        score: f32,
        quality: FaceQuality,
        embedding: Vec<f32>,
        frame_id: usize,
    ) -> Self {
        Self {
            bbox,
            landmarks,
            score,
            quality,
            embedding,
            frame_id,
            fused_count: 0,
            total_weight: 0.0,
            last_extract_frame_id: frame_id,
            retry_after_frame_id: frame_id,
            failed_attempts: 0,
        }
    }

    /// 若特征向量已提取且非空，则返回只读切片；流式仅标记阶段返回 None。
    #[inline]
    pub fn embedding_opt(&self) -> Option<&[f32]> {
        if self.embedding.is_empty() {
            None
        } else {
            Some(&self.embedding)
        }
    }
}

/// 航迹最佳人脸抓拍状态机
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

    /// 查询某条航迹当前已有的最优抓拍
    pub fn get(&self, track_id: u64) -> Option<&BestShotRecord> {
        self.records.get(&track_id)
    }

    /// 连续失败时的重试间隔按失败次数翻倍：6 → 12 → 24 帧，封顶 `MAX_RETRY_DELAY_FRAMES`。
    const fn retry_delay_frames(failed_attempts: u8) -> usize {
        let shift = match failed_attempts {
            0 | 1 => 0,
            2 => 1,
            3 => 2,
            _ => 3,
        };
        let delay = MIN_FUSION_FRAME_INTERVAL << shift;
        if delay > MAX_RETRY_DELAY_FRAMES {
            MAX_RETRY_DELAY_FRAMES
        } else {
            delay
        }
    }

    /// 判定当前帧人脸是否应该触发特征提取与特征融合
    ///
    /// 触发条件：
    /// 1. 该航迹此前从未提取过人脸特征，且没有处于失败退避窗口；
    /// 2. 当前人脸综合质量分比历史最优高出至少 `DEFAULT_QUALITY_UPGRADE_DELTA`；
    /// 3. 或已融合帧数未达上限，且距离上次提取已间隔足够帧数，且达到融合门限。
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

    /// 带有自定义质量增量阈值的最优抓拍与特征融合升级判定
    pub fn should_update_best_shot_with_delta(
        &self,
        track_id: u64,
        new_quality: &FaceQuality,
        delta: f32,
        current_frame_id: usize,
    ) -> bool {
        match self.records.get(&track_id) {
            None => true,
            Some(prev) if prev.fused_count >= MAX_FUSED_FRAMES => false,
            Some(prev) if prev.embedding.is_empty() => {
                current_frame_id >= prev.retry_after_frame_id
            }
            Some(prev) => {
                new_quality.score > prev.quality.score + delta
                    || (current_frame_id.saturating_sub(prev.last_extract_frame_id)
                        >= MIN_FUSION_FRAME_INTERVAL
                        && new_quality.score >= MIN_FUSION_QUALITY_SCORE)
            }
        }
    }

    /// 更新某条航迹并执行超球面加权特征融合
    ///
    /// 1. 防漂移校验：计算新特征与已有融合特征的余弦相似度，若低于门限则拒绝融合；
    /// 2. 加权融合：按质量平方对单位向量加权累加，并重新 L2 归一化；
    /// 3. 返回当前最新的融合特征向量。
    #[allow(clippy::too_many_arguments)]
    pub fn update_with_fusion(
        &mut self,
        track_id: u64,
        bbox: [f32; 4],
        landmarks: [[f32; 2]; 5],
        score: f32,
        quality: FaceQuality,
        new_embedding: &[f32; 512],
        frame_id: usize,
    ) -> [f32; 512] {
        let q = quality.score.clamp(0.1, 1.0);
        let weight = q * q;

        let record = self.records.entry(track_id).or_insert_with(|| {
            BestShotRecord::pending(bbox, landmarks, score, quality, Vec::new(), frame_id)
        });

        if record.embedding.len() == 512 {
            let current: &[f32; 512] = match record.embedding.as_slice().try_into() {
                Ok(arr) => arr,
                Err(_) => return *new_embedding,
            };

            // 防漂移校验 (Anti-Drift Outlier Defense)
            let sim = crate::cosine_similarity(new_embedding, current);
            if sim < DRIFT_REJECTION_SIMILARITY {
                tracing::warn!(
                    track_id,
                    similarity = sim,
                    threshold = DRIFT_REJECTION_SIMILARITY,
                    "特征融合防漂移校验拦截：新特征与历史融合特征余弦相似度过低，拒绝污染特征池"
                );
                record.last_extract_frame_id = frame_id;
                return *current;
            }

            // 超球面加权累加与归一化
            let prev_weight = record.total_weight;
            let mut fused = [0.0f32; 512];
            let mut norm_sq = 0.0f32;
            for (out, (&curr, &new)) in fused.iter_mut().zip(current.iter().zip(new_embedding)) {
                let val = curr * prev_weight + new * weight;
                *out = val;
                norm_sq += val * val;
            }

            if norm_sq > 1e-12 {
                let inv_norm = 1.0 / norm_sq.sqrt();
                for v in &mut fused {
                    *v *= inv_norm;
                }
            } else {
                fused = *new_embedding;
            }

            if quality.score > record.quality.score {
                record.bbox = bbox;
                record.landmarks = landmarks;
                record.score = score;
                record.quality = quality;
            }
            record.embedding.copy_from_slice(&fused);
            record.fused_count += 1;
            record.total_weight = prev_weight + weight;
            record.last_extract_frame_id = frame_id;
            record.retry_after_frame_id = frame_id;
            record.failed_attempts = 0;

            fused
        } else {
            record.bbox = bbox;
            record.landmarks = landmarks;
            record.score = score;
            record.quality = quality;
            record.embedding = new_embedding.to_vec();
            record.fused_count = 1;
            record.total_weight = weight;
            record.last_extract_frame_id = frame_id;
            record.retry_after_frame_id = frame_id;
            record.failed_attempts = 0;
            *new_embedding
        }
    }

    /// 更新某条航迹的最优抓拍记录（兼容旧接口）
    #[allow(clippy::too_many_arguments)]
    pub fn update(
        &mut self,
        track_id: u64,
        bbox: [f32; 4],
        landmarks: [[f32; 2]; 5],
        score: f32,
        quality: FaceQuality,
        embedding: Vec<f32>,
        frame_id: usize,
    ) {
        if embedding.len() == 512 {
            let mut arr = [0.0f32; 512];
            arr.copy_from_slice(&embedding);
            self.update_with_fusion(track_id, bbox, landmarks, score, quality, &arr, frame_id);
        } else {
            self.records.insert(
                track_id,
                BestShotRecord::pending(bbox, landmarks, score, quality, embedding, frame_id),
            );
        }
    }

    /// 记录一次 best-shot 尝试但没有得到新 embedding。
    ///
    /// 保留已有特征，避免暂时性的读回或模型错误导致每帧重复执行重型路径。
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
                record.last_extract_frame_id = frame_id;
                record.failed_attempts = record.failed_attempts.saturating_add(1);
                record.retry_after_frame_id =
                    frame_id.saturating_add(Self::retry_delay_frames(record.failed_attempts));
                if quality.score > record.quality.score {
                    record.bbox = bbox;
                    record.landmarks = landmarks;
                    record.score = score;
                    record.quality = quality;
                }
            })
            .or_insert_with(|| {
                let mut record =
                    BestShotRecord::pending(bbox, landmarks, score, quality, Vec::new(), frame_id);
                // 首次失败即进入退避窗口，避免逐帧重复触发重型提取。
                record.failed_attempts = 1;
                record.retry_after_frame_id = frame_id.saturating_add(Self::retry_delay_frames(1));
                record
            });
    }

    /// 删除已经进入 `Removed` 状态的航迹对应记录；`Lost` 航迹必须保留。
    pub fn remove_tracks(&mut self, removed_track_ids: &[u64]) {
        for track_id in removed_track_ids {
            self.records.remove(track_id);
        }
    }

    /// 清空所有状态
    pub fn clear(&mut self) {
        self.records.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_missing_embedding_remains_retryable() {
        let mut mgr = BestShotManager::new();
        let track_id = 7;

        let quality = FaceQuality {
            score: 0.60,
            blur: 0.30,
            yaw: 4.0,
            pitch: 2.0,
            face_size: 80,
        };

        mgr.record_attempt_without_embedding(
            track_id,
            [0.1, 0.1, 0.2, 0.2],
            [[0.0; 2]; 5],
            0.9,
            quality,
            1,
        );
        assert!(!mgr.should_update_best_shot(track_id, &quality, 1));
        assert!(!mgr.should_update_best_shot(track_id, &quality, 6));
        assert!(mgr.should_update_best_shot(track_id, &quality, 7));
    }

    #[test]
    fn test_temporal_spherical_fusion_and_drift_rejection() {
        let mut mgr = BestShotManager::new();
        let track_id = 100;

        let mut v1 = [0.0f32; 512];
        v1[0] = 1.0; // 单位向量 (1, 0, 0, ...)
        let q1 = FaceQuality {
            score: 0.60,
            blur: 0.30,
            yaw: 10.0,
            pitch: 5.0,
            face_size: 60,
        };

        // 帧 1: 首次提取
        assert!(mgr.should_update_best_shot(track_id, &q1, 1));
        let fused1 = mgr.update_with_fusion(
            track_id,
            [0.1, 0.1, 0.2, 0.2],
            [[0.0; 2]; 5],
            0.9,
            q1,
            &v1,
            1,
        );
        assert_eq!(fused1, v1);
        let rec = mgr.get(track_id).expect("航迹记录应存在");
        assert_eq!(rec.fused_count, 1);

        // 帧 3 (仅间隔 2 帧，未达 MIN_FUSION_FRAME_INTERVAL 门限): 相同质量不应提取
        let q_same = FaceQuality {
            score: 0.62,
            blur: 0.30,
            yaw: 10.0,
            pitch: 5.0,
            face_size: 60,
        };
        assert!(!mgr.should_update_best_shot(track_id, &q_same, 3));

        // 帧 8 (间隔 7 帧，达到采样间隔且质量合格): 允许融合第 2 帧
        let mut v2 = [0.0f32; 512];
        v2[0] = 0.8;
        v2[1] = 0.6; // 单位向量，与 v1 余弦相似度 = 0.80 >= 0.55
        let q2 = FaceQuality {
            score: 0.80,
            blur: 0.40,
            yaw: 4.0,
            pitch: 2.0,
            face_size: 90,
        };
        assert!(mgr.should_update_best_shot(track_id, &q2, 8));
        let fused2 = mgr.update_with_fusion(
            track_id,
            [0.12, 0.12, 0.22, 0.22],
            [[0.0; 2]; 5],
            0.95,
            q2,
            &v2,
            8,
        );

        let rec = mgr.get(track_id).expect("航迹记录应存在");
        assert_eq!(rec.fused_count, 2);
        // 融合后的单位向量应介于 v1 与 v2 之间，且更偏向质量更高的 v2
        let sim_to_v1 = crate::cosine_similarity(&fused2, &v1);
        let sim_to_v2 = crate::cosine_similarity(&fused2, &v2);
        assert!(sim_to_v1 > 0.85);
        assert!(sim_to_v2 > 0.95);
        assert!(sim_to_v2 > sim_to_v1, "高分人脸权重应更大");

        // 验证融合向量自身严格保持 L2 单位长度
        let norm: f32 = fused2.iter().map(|x| x * x).sum::<f32>().sqrt();
        assert!((norm - 1.0).abs() < 1e-5);

        // 帧 15: 异常漂移目标 (余弦相似度仅 0.0 < 0.55，如遮挡或错跟他人)
        let mut v_drift = [0.0f32; 512];
        v_drift[10] = 1.0;
        let q_drift = FaceQuality {
            score: 0.85,
            blur: 0.45,
            yaw: 0.0,
            pitch: 0.0,
            face_size: 100,
        };
        let fused_after_drift = mgr.update_with_fusion(
            track_id,
            [0.15, 0.15, 0.25, 0.25],
            [[0.0; 2]; 5],
            0.98,
            q_drift,
            &v_drift,
            15,
        );
        // 验证防漂移拦截：融合特征被安全保护，未受污染
        assert_eq!(fused_after_drift, fused2);
        let rec = mgr.get(track_id).expect("航迹记录应存在");
        assert_eq!(rec.fused_count, 2, "漂移特征不计入融合计数");
    }

    #[test]
    fn test_best_shot_upgrade_policy() {
        let mut mgr = BestShotManager::new();
        let track_id = 42;

        let q1 = FaceQuality {
            score: 0.50,
            blur: 0.20,
            yaw: 25.0,
            pitch: 10.0,
            face_size: 40,
        };

        // 1. 初次必须触发
        assert!(mgr.should_update_best_shot(track_id, &q1, 1));
        let mut v = [0.0f32; 512];
        v[0] = 1.0;
        mgr.update_with_fusion(
            track_id,
            [0.1, 0.1, 0.1, 0.1],
            [[0.0; 2]; 5],
            0.8,
            q1,
            &v,
            1,
        );

        // 2. 质量仅微弱提升 (0.50 -> 0.53) 且帧间隔过短，不应重复提取
        let q2 = FaceQuality {
            score: 0.53,
            blur: 0.22,
            yaw: 22.0,
            pitch: 8.0,
            face_size: 45,
        };
        assert!(!mgr.should_update_best_shot(track_id, &q2, 2));

        // 3. 质量显著超越历史最高 (0.50 -> 0.75)，即使帧间隔短也强制触发
        let q3 = FaceQuality {
            score: 0.75,
            blur: 0.35,
            yaw: 5.0,
            pitch: 2.0,
            face_size: 80,
        };
        assert!(mgr.should_update_best_shot(track_id, &q3, 3));
    }
}
