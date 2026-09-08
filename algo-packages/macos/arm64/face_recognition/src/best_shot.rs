//! 基于 ByteTrack 航迹生命周期的动态最佳人脸抓拍 (Best-Shot) 状态机
//!
//! 1. 维护每个活跃 `track_id` 的历史最优人脸质量分与特征向量；
//! 2. 初次入镜捕获合格人脸即触发特征提取，随后仅在质量显著提升时刷新；
//! 3. 随 ByteTrack 航迹注销级联清理，保证内存严格有界。

use std::collections::HashMap;

use crate::quality::FaceQuality;

/// 默认最优抓拍质量分提升门限（当前质量分至少比历史高 0.10 才允许触发刷新）
pub const DEFAULT_QUALITY_UPGRADE_DELTA: f32 = 0.10;

/// 最佳抓拍人脸记录
#[derive(Debug, Clone)]
pub struct BestShotRecord {
    pub bbox: [f32; 4],
    pub landmarks: [[f32; 2]; 5],
    pub score: f32,
    pub quality: FaceQuality,
    pub embedding: Vec<f32>,
    pub frame_id: usize,
}

impl BestShotRecord {
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

    /// 判定当前帧人脸是否应该触发特征提取与最优抓拍刷新
    ///
    /// 触发条件：
    /// 1. 该航迹此前从未提取过人脸特征；
    /// 2. 当前人脸综合质量分比历史最优高出至少 `DEFAULT_QUALITY_UPGRADE_DELTA`（显著改善，如侧脸转正脸）。
    pub fn should_update_best_shot(&self, track_id: u64, new_quality: &FaceQuality) -> bool {
        self.should_update_best_shot_with_delta(
            track_id,
            new_quality,
            DEFAULT_QUALITY_UPGRADE_DELTA,
        )
    }

    /// 带有自定义质量增量阈值的最优抓拍升级判定
    pub fn should_update_best_shot_with_delta(
        &self,
        track_id: u64,
        new_quality: &FaceQuality,
        delta: f32,
    ) -> bool {
        match self.records.get(&track_id) {
            None => true,
            Some(prev) => new_quality.score > prev.quality.score + delta,
        }
    }

    /// 更新某条航迹的最优抓拍记录
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
        self.records.insert(
            track_id,
            BestShotRecord {
                bbox,
                landmarks,
                score,
                quality,
                embedding,
                frame_id,
            },
        );
    }

    /// 清理已消亡航迹对应的最优抓拍记录，防止内存泄漏
    pub fn retain_active_tracks(&mut self, active_track_ids: &[u64]) {
        self.records
            .retain(|track_id, _| active_track_ids.contains(track_id));
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
        assert!(mgr.should_update_best_shot(track_id, &q1));
        mgr.update(
            track_id,
            [0.1, 0.1, 0.1, 0.1],
            [[0.0; 2]; 5],
            0.8,
            q1,
            vec![0.1; 512],
            1,
        );

        // 2. 质量仅微弱提升 (0.50 -> 0.55)，不应浪费算力重复提取
        let q2 = FaceQuality {
            score: 0.55,
            blur: 0.22,
            yaw: 22.0,
            pitch: 8.0,
            face_size: 45,
        };
        assert!(!mgr.should_update_best_shot(track_id, &q2));

        // 3. 质量显著提升 (0.50 -> 0.75)，触发刷新
        let q3 = FaceQuality {
            score: 0.75,
            blur: 0.35,
            yaw: 5.0,
            pitch: 2.0,
            face_size: 80,
        };
        assert!(mgr.should_update_best_shot(track_id, &q3));
    }
}
