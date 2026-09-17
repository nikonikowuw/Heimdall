//! 航迹帧池状态与纯数学部分：有限样本池、Top-K 去冗余融合、模板成熟判定。
//!
//! 本模块只做「给定航迹状态与样本 ⇒ 新的航迹状态」这一步，不关心采样时机决策，
//! 也不做任何设备调用；采样时机见 [`crate::best_shot::BestShotManager`]。
//! 拆开的理由：池淘汰、超球面加权融合与成熟门限三者互相依赖但各自有独立的边界条件，
//! 与「何时采样」混在一个文件里时，任何一处改动都要求通读全部规则。

use std::cmp::Ordering;

use crate::best_shot::{
    MAX_FUSED_FRAMES, MIN_FUSION_FRAME_INTERVAL, MIN_MATURE_POOL_SIZE, PLATEAU_FRAMES,
    REDUNDANCY_SIMILARITY, SIZE_TARGET_PIXELS, TOP_K_FUSED_FRAMES,
};
use crate::quality::FaceQuality;

/// 失败退避的最大帧间隔上限。
const MAX_RETRY_DELAY_FRAMES: usize = 48;

/// 模板成熟是否在本次推进中首次翻转。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum MaturityFlip {
    /// 尚未成熟或已发射过，本次不产生握手信号。
    None,
    /// 本次首次翻转：调用方需发射一次成熟信号。
    FirstTime,
}

impl MaturityFlip {
    pub(crate) fn flipped(self) -> bool {
        matches!(self, Self::FirstTime)
    }
}

/// 连续失败时的重试间隔：6 → 12 → 24 → 48 帧。
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

/// 单个进入帧池的特征样本。
#[derive(Debug, Clone)]
pub(crate) struct FrameSample {
    pub(crate) embedding: [f32; 512],
    pub(crate) quality: FaceQuality,
    pub(crate) score: f32,
    pub(crate) bbox: [f32; 4],
    pub(crate) landmarks: [[f32; 2]; 5],
    pub(crate) frame_id: usize,
}

/// 航迹最佳人脸记录与多帧特征融合状态。
///
/// 「当前峰值样本」只有一份权威副本：`best_frame` 持有完整样本，下面几个标量字段是它的
/// 投影，供宿主直接读取；两者只在 [`BestShotRecord::update_best_sample_metadata`] 中同步。
#[derive(Debug, Clone)]
pub struct BestShotRecord {
    /// 当前池内质量最高样本的检测框。
    pub bbox: [f32; 4],
    /// 当前池内质量最高样本的关键点。
    pub landmarks: [[f32; 2]; 5],
    /// 当前池内最高质量分。
    pub score: f32,
    /// 当前池内最高质量样本的质量明细。
    pub quality: FaceQuality,
    /// 当前池内质量最高样本的帧序号。
    pub frame_id: usize,
    /// 实际参与当前模板融合的 Top-K 样本数。
    pub fused_count: usize,
    /// 当前模板的质量加权均值。
    pub template_quality: f32,
    /// 模板是否已经成熟。
    pub template_mature: bool,
    /// 最近一次提取特征的帧序号。
    pub last_extract_frame_id: usize,
    /// 下一次允许重试的帧序号；用于隔离设备/队列瞬时失败。
    pub retry_after_frame_id: usize,
    pool: Vec<FrameSample>,
    template: [f32; 512],
    best_frame: Option<FrameSample>,
    last_improve_frame_id: usize,
    mature_emitted: bool,
    failed_attempts: u8,
}

impl BestShotRecord {
    /// 新建一条尚未获得有效特征的记录；`retry_after_frame_id` 默认立即可重试。
    pub(crate) fn uninitialized(
        bbox: [f32; 4],
        landmarks: [[f32; 2]; 5],
        score: f32,
        quality: FaceQuality,
        frame_id: usize,
    ) -> Self {
        Self {
            bbox,
            landmarks,
            score,
            quality,
            frame_id,
            fused_count: 0,
            template_quality: 0.0,
            template_mature: false,
            last_extract_frame_id: frame_id,
            retry_after_frame_id: frame_id,
            pool: Vec::with_capacity(MAX_FUSED_FRAMES),
            template: [0.0; 512],
            best_frame: None,
            last_improve_frame_id: frame_id,
            mature_emitted: false,
            failed_attempts: 0,
        }
    }

    /// 当前帧池样本数（不等于 `fused_count`，因为还要做 Top-K 去冗余）。
    #[inline]
    pub fn pool_len(&self) -> usize {
        self.pool.len()
    }

    /// 池内是否已有样本（即模板基准是否已建立）。
    #[inline]
    pub(crate) fn pool_is_empty(&self) -> bool {
        self.pool.is_empty()
    }

    /// 当前融合模板。
    #[inline]
    pub(crate) fn template(&self) -> &[f32; 512] {
        &self.template
    }

    /// 返回当前峰值样本的人脸尺寸，供诊断使用。
    pub fn best_face_size(&self) -> Option<u32> {
        self.best_frame
            .as_ref()
            .map(|sample| sample.quality.face_size)
    }

    /// 记录一次成功的特征提取：解除失败退避窗口。
    pub(crate) fn mark_extraction_success(&mut self, frame_id: usize) {
        self.last_extract_frame_id = frame_id;
        self.retry_after_frame_id = frame_id;
        self.failed_attempts = 0;
    }

    /// 记录一次软失败（提取失败或防漂移拒绝）。
    pub(crate) fn mark_extraction_failure(&mut self, frame_id: usize) {
        self.last_extract_frame_id = frame_id;
        self.failed_attempts = self.failed_attempts.saturating_add(1);
        self.retry_after_frame_id =
            frame_id.saturating_add(retry_delay_frames(self.failed_attempts));
    }

    /// 池内当前最高质量分（无样本时回退到当前峰值分）。
    pub(crate) fn best_pool_quality(&self) -> f32 {
        self.best_frame
            .as_ref()
            .map_or(self.quality.score, |sample| sample.quality.score)
    }

    /// 插入新样本并重算峰值/模板；返回模板是否发生实质变化。
    pub(crate) fn absorb_sample(&mut self, sample: FrameSample, frame_id: usize) -> bool {
        let previous_template = self.template;
        let inserted = self.insert_sample(sample);
        self.mark_extraction_success(frame_id);
        if inserted {
            self.update_best_sample_metadata(frame_id);
            self.recompute_template();
        }
        inserted && self.template != previous_template
    }

    /// 将新样本插入有限帧池；池满时淘汰最低质量样本。
    fn insert_sample(&mut self, sample: FrameSample) -> bool {
        if self.pool.len() < MAX_FUSED_FRAMES {
            self.pool.push(sample);
            return true;
        }

        let Some((lowest_index, lowest_quality)) = self
            .pool
            .iter()
            .enumerate()
            .min_by(|(_, left), (_, right)| {
                left.quality
                    .score
                    .partial_cmp(&right.quality.score)
                    .unwrap_or(Ordering::Equal)
            })
            .map(|(index, sample)| (index, sample.quality.score))
        else {
            return false;
        };

        if sample.quality.score > lowest_quality {
            self.pool[lowest_index] = sample;
            true
        } else {
            false
        }
    }

    /// 用池内最高质量样本更新峰值几何与平台计时基准。
    fn update_best_sample_metadata(&mut self, frame_id: usize) {
        let Some(sample) = self
            .pool
            .iter()
            .max_by(|left, right| {
                left.quality
                    .score
                    .partial_cmp(&right.quality.score)
                    .unwrap_or(Ordering::Equal)
            })
            .cloned()
        else {
            return;
        };

        let improved = match self.best_frame.as_ref() {
            None => true,
            Some(best) => sample.quality.score > best.quality.score,
        };
        if improved {
            self.bbox = sample.bbox;
            self.landmarks = sample.landmarks;
            self.score = sample.score;
            self.quality = sample.quality;
            self.frame_id = sample.frame_id;
            self.last_improve_frame_id = frame_id;
            self.best_frame = Some(sample);
        }
    }

    /// 按质量降序选取 Top-K 非冗余样本，并重算单位模板。
    fn recompute_template(&mut self) {
        let count = self.pool.len();
        if count == 0 {
            return;
        }

        let mut order = [0usize; MAX_FUSED_FRAMES];
        for (index, slot) in order.iter_mut().take(count).enumerate() {
            *slot = index;
        }
        order[..count].sort_unstable_by(|left, right| {
            self.pool[*right]
                .quality
                .score
                .partial_cmp(&self.pool[*left].quality.score)
                .unwrap_or(Ordering::Equal)
        });

        let mut selected = [0usize; TOP_K_FUSED_FRAMES];
        let mut selected_count = 0usize;
        let mut weighted = [0.0f32; 512];
        let mut total_weight = 0.0f32;
        let mut weighted_quality = 0.0f32;

        for &candidate_index in order[..count].iter() {
            if selected_count >= TOP_K_FUSED_FRAMES {
                break;
            }
            let candidate = &self.pool[candidate_index];
            let redundant = selected[..selected_count].iter().any(|selected_index| {
                algo_sdk::math::cosine_similarity(
                    &candidate.embedding,
                    &self.pool[*selected_index].embedding,
                ) >= REDUNDANCY_SIMILARITY
            });
            if redundant {
                continue;
            }

            selected[selected_count] = candidate_index;
            selected_count += 1;
            let quality = candidate.quality.score.clamp(0.1, 1.0);
            let weight = quality * quality;
            total_weight += weight;
            weighted_quality += candidate.quality.score.clamp(0.0, 1.0) * weight;
            for (output, &value) in weighted.iter_mut().zip(candidate.embedding.iter()) {
                *output += value * weight;
            }
        }

        if selected_count == 0 || !total_weight.is_finite() || total_weight <= f32::EPSILON {
            return;
        }

        let norm_sq: f32 = weighted.iter().map(|value| value * value).sum();
        if norm_sq.is_finite() && norm_sq > 1e-12 {
            let inverse_norm = 1.0 / norm_sq.sqrt();
            for value in &mut weighted {
                *value *= inverse_norm;
            }
        } else {
            weighted.copy_from_slice(&self.pool[selected[0]].embedding);
        }

        self.template = weighted;
        self.fused_count = selected_count;
        self.template_quality = weighted_quality / total_weight;
    }

    /// 判断并记录成熟首次翻转。
    pub(crate) fn refresh_maturity(&mut self, frame_id: usize) -> MaturityFlip {
        if self.mature_emitted || self.pool.len() < MIN_MATURE_POOL_SIZE {
            return MaturityFlip::None;
        }

        let large_face = self
            .best_frame
            .as_ref()
            .is_some_and(|sample| sample.quality.face_size >= SIZE_TARGET_PIXELS);
        let mature = self.pool.len() >= MAX_FUSED_FRAMES
            || frame_id.saturating_sub(self.last_improve_frame_id) >= PLATEAU_FRAMES
            || large_face;
        if !mature {
            return MaturityFlip::None;
        }

        self.mature_emitted = true;
        self.template_mature = true;
        MaturityFlip::FirstTime
    }
}
