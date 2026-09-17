//! 人脸检测插件的标准 `AlgoPlugin` 适配层。
//!
//! 集成 SCRFD 人脸检测、YOLOv8n 人体检测、ByteTrack 航迹追踪与 EdgeFace 512 维特征提取。
//! 结合时域超球面加权特征融合与防漂移校验，提供高鲁棒的人脸识别体验。

use std::sync::Arc;

use algo_sdk::cv::{self, CropRect, PreprocessMode};
use algo_sdk::emitter::ResultEmitter;
use algo_sdk::error::AlgoError;
use algo_sdk::frame::SafeFrame;
use algo_sdk::plugin::{AlgoPlugin, InitContext};

use crate::align::align_face_pixels;
use crate::config::InstanceConfig;
use crate::quality::{compute_quality, FaceQualityExt as _};
use crate::SharedModels;

pub struct FaceRecognizer {
    pub models: Arc<SharedModels>,
    pub config: InstanceConfig,
    /// 仅用于当前算法实例内部的 best-shot 去重与特征融合，不向 C ABI/宿主输出内部 trackId。
    pub tracker: crate::bytetrack::ByteTracker,
    pub best_shots: crate::best_shot::BestShotManager,
}

#[derive(Debug, Clone)]
struct BestShotSidecar {
    embedding: Option<String>,
    fused_count: Option<u32>,
    template_quality: Option<f32>,
    template_mature: Option<bool>,
}

impl std::fmt::Debug for FaceRecognizer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FaceRecognizer")
            .field("config", &self.config)
            .finish()
    }
}

impl AlgoPlugin for FaceRecognizer {
    type Config = InstanceConfig;

    fn init(ctx: &InitContext<'_>, mut config: Self::Config) -> Result<Self, AlgoError> {
        let env = ctx.load_env();
        config.apply_env(&env);
        config
            .validate()
            .map_err(|reason| AlgoError::ConfigParse { reason })?;
        let models = crate::shared_models(ctx.package_root)?;
        Ok(Self {
            models,
            config: config.clone(),
            tracker: crate::bytetrack::ByteTracker::new(face_tracker_config(&config)),
            best_shots: crate::best_shot::BestShotManager::new(),
        })
    }

    fn process(
        &mut self,
        frame: SafeFrame<'_>,
        emitter: &mut ResultEmitter<'_>,
    ) -> Result<(), AlgoError> {
        // 1. 预处理：人脸和人体模型均使用 640x384，共用同一份 RGA RGB 输出。
        let (buf, mode) = cv::letterbox(
            &frame,
            self.models.detector_width,
            self.models.detector_height,
            [114, 114, 114],
        )?;
        let PreprocessMode::Letterbox(layout) = mode else {
            return Err(AlgoError::Preprocess {
                reason: "检测预处理需要 Letterbox 模式".to_string(),
            });
        };

        // 2. 执行人脸与人体 NPU 推理与解码
        let min_face_score = self.config.detection_confidence_threshold;
        let min_person_score = self.config.person_confidence_threshold;
        let (persons, raw_faces) = if buf.as_dma_buf_layout().is_some() {
            self.models
                .worker
                .detect_dma_buf(buf, layout, min_face_score, min_person_score)?
        } else if let Some(host_bytes) = buf.as_host_bytes() {
            self.models.worker.detect_host(
                host_bytes.to_vec(),
                layout,
                min_face_score,
                min_person_score,
            )?
        } else {
            return Err(AlgoError::Preprocess {
                reason: "预处理输出既无有效 DMA-BUF 布局，也无 Host 内存视图".to_string(),
            });
        };

        // 3. 空间几何关联挂载：
        // 真实人体与人脸二分图匹配（未匹配人脸自适应推导虚拟躯干 Pseudo-body 保底）
        let associated = crate::association::associate_persons_and_faces(&persons, &raw_faces);

        // 4. ByteTrack 直接追踪人脸框。人脸 track 是 best-shot 的唯一身份键，
        //    人体框只作为输出上下文，不能反向决定 embedding 所属目标。
        let face_track_dets: Vec<crate::bytetrack::TrackDetection> = raw_faces
            .iter()
            .map(|face| crate::bytetrack::TrackDetection {
                bbox: face.bbox,
                score: face.score,
                class_id: 0,
            })
            .collect();
        let active_face_tracks = self.tracker.update(&face_track_dets);
        let face_track_ids = crate::association::match_face_tracks_to_detections(
            &active_face_tracks,
            &raw_faces,
            0.25,
        );
        self.best_shots
            .remove_tracks(self.tracker.recently_removed_track_ids());

        // 5. 遍历关联目标，执行质量门控、低频 best-shot 时域融合与结果组装
        let mut objects = Vec::with_capacity(associated.len());

        for candidate in associated.iter() {
            // 人脸 track 未确认时只输出检测结果，不创建临时身份键。
            let internal_track_id = candidate
                .face_index
                .and_then(|face_index| face_track_ids.get(face_index).copied().flatten());

            let face_detail = if let Some(face) = candidate.attached_face {
                let face_width_pixels = face.width() * frame.width() as f32;
                let face_height_pixels = face.height() * frame.height() as f32;
                let quality = compute_quality(
                    &face.landmarks,
                    &face.landmark_scores,
                    face_width_pixels.min(face_height_pixels),
                    &self.config.quality_thresholds,
                );

                let embedding_sidecar = match internal_track_id {
                    Some(track_id) => self.best_shot_sidecar(&frame, &face, &quality, track_id)?,
                    None => None,
                };

                // 人脸检测框只要检出，就必须作为精细元数据输出给宿主管线与前端实时绘制
                Some(crate::postprocess::FaceDetailObject {
                    bbox: crate::postprocess::normalized_xywh_to_xyxy(face.bbox),
                    confidence: face.score.clamp(0.0, 1.0),
                    quality_score: Some(quality.score.clamp(0.0, 1.0)),
                    embedding: embedding_sidecar
                        .as_ref()
                        .and_then(|sidecar| sidecar.embedding.clone()),
                    fused_count: embedding_sidecar
                        .as_ref()
                        .and_then(|sidecar| sidecar.fused_count),
                    template_quality: embedding_sidecar
                        .as_ref()
                        .and_then(|sidecar| sidecar.template_quality),
                    template_mature: embedding_sidecar
                        .as_ref()
                        .and_then(|sidecar| sidecar.template_mature),
                })
            } else {
                None
            };

            objects.push(crate::postprocess::DetectionObject {
                class_id: 0,
                label: "person".to_string(),
                confidence: candidate.person_score.clamp(0.0, 1.0),
                bbox: crate::postprocess::normalized_xywh_to_xyxy(candidate.person_bbox),
                face: face_detail,
            });
        }

        crate::postprocess::emit_detection_objects(emitter, &objects)
    }

    fn update_config(&mut self, config: Self::Config) -> Result<(), AlgoError> {
        config
            .validate()
            .map_err(|reason| AlgoError::ConfigParse { reason })?;
        // 建轨门槛由检测阈值派生，配置热更新必须同步刷新跟踪器门限。
        self.tracker.apply_config(face_tracker_config(&config));
        self.config = config;
        Ok(())
    }

    fn flush(&mut self, _emitter: &mut ResultEmitter<'_>) -> Result<(), AlgoError> {
        self.tracker.reset();
        self.best_shots.clear();
        Ok(())
    }
}

impl FaceRecognizer {
    /// 低频 best-shot 链路：质量门控 → 帧池采样 → 时域融合 → sidecar 编码。
    ///
    /// 质量不足或尚未到重试窗口时返回 `None`，不触碰特征提取；模板成熟但本帧没有
    /// 新 embedding 时，也可以只发射一次 `template_mature` 握手。
    fn best_shot_sidecar(
        &mut self,
        frame: &SafeFrame<'_>,
        face: &crate::detect::RawFace,
        quality: &crate::quality::FaceQuality,
        track_id: u64,
    ) -> Result<Option<BestShotSidecar>, AlgoError> {
        if !quality.accepted(&self.config.quality_thresholds, self.config.min_face_size) {
            return Ok(None);
        }
        let frame_id = frame.frame_id() as usize;
        if self
            .best_shots
            .should_update_best_shot(track_id, quality, frame_id)
        {
            // ROI 裁切失败与设备侧推理失败共用同一退避重试路径。
            let extract = || -> Result<[f32; 512], AlgoError> {
                let aligned = extract_aligned_face(frame, face)?;
                crate::align::dump_debug_aligned_face(
                    &format!("live_track{track_id}"),
                    &aligned,
                    quality.score,
                );
                self.models.worker.embed_host(aligned)
            };
            match extract() {
                Ok(normalized) => {
                    let update = self.best_shots.update_with_fusion_result(
                        track_id,
                        face.bbox,
                        face.landmarks,
                        face.score,
                        *quality,
                        &normalized,
                        frame_id,
                    );
                    return encode_fusion_sidecar(update);
                }
                Err(error) => {
                    self.best_shots.record_attempt_without_embedding(
                        track_id,
                        face.bbox,
                        face.landmarks,
                        face.score,
                        *quality,
                        frame_id,
                    );
                    tracing::warn!(
                        %error,
                        track_id,
                        "best-shot RKNN 设备侧 EdgeFace 提取失败，按退避策略允许后续重试"
                    );
                    return Ok(None);
                }
            }
        }

        if let Some(update) = self.best_shots.maturity_signal(track_id, frame_id) {
            encode_fusion_sidecar(update)
        } else {
            Ok(None)
        }
    }
}

fn encode_fusion_sidecar(
    update: crate::best_shot::FusionUpdate,
) -> Result<Option<BestShotSidecar>, AlgoError> {
    if !update.template_changed && !update.template_mature {
        return Ok(None);
    }
    let fused_count = u32::try_from(update.fused_count).map_err(|_| AlgoError::Inference {
        reason: format!("融合帧计数超出 sidecar 范围: {}", update.fused_count),
    })?;
    let embedding = if update.template_changed {
        Some(crate::postprocess::encode_embedding(
            update.template.as_slice(),
        )?)
    } else {
        None
    };
    Ok(Some(BestShotSidecar {
        embedding,
        fused_count: Some(fused_count),
        template_quality: Some(update.template_quality.clamp(0.0, 1.0)),
        template_mature: update.template_mature.then_some(true),
    }))
}

/// 只读取对齐采样域覆盖的 ROI，避免 best-shot 路径把整帧 DMA-BUF 映射回 CPU。
fn extract_aligned_face(
    frame: &SafeFrame<'_>,
    face: &crate::detect::RawFace,
) -> Result<Vec<u8>, AlgoError> {
    // 关键点先换算到全帧像素坐标：ROI 推导与裁剪后平移都需要像素空间。
    let mut landmarks = face.landmarks;
    for point in &mut landmarks {
        point[0] *= frame.width() as f32;
        point[1] *= frame.height() as f32;
    }
    // ROI 只读取对齐采样域覆盖的区域：避免 best-shot 路径把整帧 DMA-BUF 映射回 CPU。
    // 画布档位化并不改变采样落点，只影响读取范围（见 `SNAPSHOT_ROI_TIERS`）。
    let rect = face_snapshot_rect(frame, &landmarks)?;
    let cropped = cv::crop_rgb(frame, rect)?;
    let rgb = cropped.readback_rgb24()?;
    for point in &mut landmarks {
        point[0] -= rect.x as f32;
        point[1] -= rect.y as f32;
    }
    align_face_pixels(&rgb, rect.width, rect.height, &landmarks)
}

/// 由检测阈值派生人脸航迹跟踪门限。
///
/// 设计依据：凡已通过人脸检测门控的目标即视为高分检测，
/// 跟踪阈值直接由检测阈值派生，不设双重硬编码门限。
///
/// `confirm_new_tracks = false`：二次确认会把建轨推迟到第二帧，而 best-shot 需要在
/// 第一个过门的帧上就开始积累质量分，因此不做置信度二次确认，改由模板池的
/// `MIN_MATURE_POOL_SIZE` 与质量门控在特征层把关。
fn face_tracker_config(config: &InstanceConfig) -> crate::bytetrack::ByteTrackConfig {
    let gate = config.detection_confidence_threshold;
    crate::bytetrack::ByteTrackConfig {
        high_thresh: gate,
        track_thresh: gate,
        confirm_new_tracks: false,
        ..Default::default()
    }
}

/// ROI 画布档位（边长，像素）。
///
/// 为什么必须存在：SDK 的 `RgaCvEngine` 为每个 `(width, height)` 组合维护独立输出池，
/// 全进程最多缓存 `MAX_CACHED_POOLS`(16) 种几何且**不淘汰**，超限后 `cv::crop_rgb` 永久失败
/// （见 `crates/algo-sdk/src/cv/platforms/rockchip/engine.rs`）。best-shot 的精确采样域逐帧
/// 亚像素抖动，直接作为裁剪尺寸时十几条航迹就会耗尽该预算，使整包特征提取链路静默失效。
/// 档位只增不减：画布变大不改变仿射采样落点，只增加少量低频回读字节；
/// 所有流、所有帧共用同一组档位，几何种数因此有界。
const SNAPSHOT_ROI_TIERS: [u32; 4] = [128, 192, 256, 384];

/// 档位相对采样域的额外余量，用于吸收 YUV 4:2:0 偶数对齐与画布居中取整带来的 ±1 像素外扩。
const SNAPSHOT_ROI_PARITY_MARGIN: u32 = 2;

/// 选择覆盖 `required` 的最小档位；档位高于帧尺寸或全部不够时退化为整帧该轴尺寸。
///
/// 退化到整帧同样覆盖采样域（采样域已被收窄到帧内），且每路流只贡献 1 种几何。
fn canvas_extent(required: u32, frame_extent: u32) -> u32 {
    let needed = required.saturating_add(SNAPSHOT_ROI_PARITY_MARGIN);
    SNAPSHOT_ROI_TIERS
        .iter()
        .copied()
        .filter(|tier| *tier <= frame_extent)
        .find(|tier| *tier >= needed)
        .unwrap_or(frame_extent)
}

/// 把画布中心对齐到采样域中心，并收在帧内。
fn centered_start(exact_start: u32, exact_extent: u32, canvas: u32, frame_extent: u32) -> u32 {
    let center = exact_start.saturating_add(exact_extent / 2);
    center
        .saturating_sub(canvas / 2)
        .min(frame_extent.saturating_sub(canvas))
}

/// 由仿射采样域反推快照 ROI。
///
/// ROI 必须覆盖 112×112 对齐输出的全部采样像素：“检测框 + 固定 margin”与采样域不同量纲，
/// 紧框或关键点跨度较大时会把对齐图边缘填黑，使 embedding 偏离整帧路径。
/// 采样域本身被收窄到帧内（超界区域在整帧路径同样被填黑），画布再按 `SNAPSHOT_ROI_TIERS`
/// 放大到有界档位——画布只影响读取范围，不改变采样落点。
fn face_snapshot_rect(
    frame: &SafeFrame<'_>,
    landmarks: &[[f32; 2]; 5],
) -> Result<CropRect, AlgoError> {
    if landmarks.iter().flatten().any(|value| !value.is_finite()) {
        return Err(AlgoError::Preprocess {
            reason: "人脸关键点包含非有限浮点数".to_string(),
        });
    }
    let frame_width = frame.width();
    let frame_height = frame.height();
    let [left, top, right, bottom] =
        crate::align::aligned_source_bounds(landmarks, crate::align::ALIGNED_SIZE).ok_or_else(
            || AlgoError::Preprocess {
                reason: "人脸关键点无法构成有效相似变换".to_string(),
            },
        )?;

    // 按帧尺寸收窄：超界区域在整帧路径同样被填黑，收窄不改变采样结果。
    let x = left.clamp(0.0, frame_width as f32) as u32;
    let y = top.clamp(0.0, frame_height as f32) as u32;
    let exact = CropRect {
        x,
        y,
        width: (right.clamp(0.0, frame_width as f32) as u32).saturating_sub(x),
        height: (bottom.clamp(0.0, frame_height as f32) as u32).saturating_sub(y),
    };

    let is_yuv = matches!(
        frame.pixel_format(),
        algo_sdk::c_abi::AV_PIX_NV12 | algo_sdk::c_abi::AV_PIX_I420
    );
    // YUV 4:2:0 的采样域起止必须落在偶数像素，先得到采样域的真实上界再选档位。
    let exact = if is_yuv {
        align_yuv_crop(exact, frame_width, frame_height)?.validate(frame_width, frame_height)?
    } else {
        exact.validate(frame_width, frame_height)?
    };

    // 两个轴共用同一个 required：否则会出现 (384, 256) 这类混合档位对，几何种数不再可枚举。
    let required = exact.width.max(exact.height);
    let canvas_width = canvas_extent(required, frame_width);
    let canvas_height = canvas_extent(required, frame_height);
    let rect = CropRect {
        x: centered_start(exact.x, exact.width, canvas_width, frame_width),
        y: centered_start(exact.y, exact.height, canvas_height, frame_height),
        width: canvas_width,
        height: canvas_height,
    };

    // YUV 起点必须偶数；档位已预留 ±1 余量，向左取偶不会破坏采样域覆盖。
    if is_yuv {
        return align_yuv_crop(
            CropRect {
                x: rect.x & !1,
                y: rect.y & !1,
                ..rect
            },
            frame_width,
            frame_height,
        )?
        .validate(frame_width, frame_height);
    }
    rect.validate(frame_width, frame_height)
}

fn align_yuv_crop(
    mut rect: CropRect,
    frame_width: u32,
    frame_height: u32,
) -> Result<CropRect, AlgoError> {
    // YUV 4:2:0 的 ROI 起止必须落在偶数像素，否则色度平面会错位。
    let align_up_even = |value: u32, limit: u32| {
        if value >= limit {
            limit
        } else {
            (value + 1) & !1
        }
    };
    let left = rect.x & !1;
    let top = rect.y & !1;
    // 终点必须按原始边界推导：若先左移起点再用其加宽度，终点会跟着内缩一列/行，
    // 采样域（`aligned_source_bounds`）的右/下边缘会落到 ROI 外被填黑。
    let right = align_up_even(
        rect.x
            .checked_add(rect.width)
            .ok_or(AlgoError::OutOfMemory)?,
        frame_width,
    );
    let bottom = align_up_even(
        rect.y
            .checked_add(rect.height)
            .ok_or(AlgoError::OutOfMemory)?,
        frame_height,
    );
    rect.x = left;
    rect.y = top;
    rect.width = right.saturating_sub(left);
    rect.height = bottom.saturating_sub(top);
    if rect.width < 2 || rect.height < 2 {
        return Err(AlgoError::Preprocess {
            reason: "YUV 快照 ROI 对齐后尺寸过小".to_string(),
        });
    }
    Ok(rect)
}

#[cfg(test)]
mod tests {
    use super::*;
    use algo_sdk::testing::MockFrameBuilder;

    #[test]
    fn face_tracker_gate_follows_detection_threshold() {
        let mut config = InstanceConfig::default();
        config.detection_confidence_threshold = 0.25;

        let tracker_config = face_tracker_config(&config);
        assert_eq!(tracker_config.high_thresh, 0.25);
        assert_eq!(tracker_config.track_thresh, 0.25);
        assert!(!tracker_config.confirm_new_tracks);
        // 关联阈值仍保留 ByteTrack 默认的 IoU 门限，不被分数耦合影响。
        assert_eq!(
            tracker_config.match_thresh,
            crate::bytetrack::ByteTrackConfig::default().match_thresh
        );
    }

    /// 生成逐像素可区分的 RGB24 图案（任何采样偏移都会直接体现为像素差异）。
    ///
    /// 各通道下界抬到 33/41/53，使「填黑」（0）与合法像素的差异远大于 1 LSB。
    fn synthetic_rgb(width: u32, height: u32) -> Vec<u8> {
        let mut pixels = vec![0u8; (width * height * 3) as usize];
        for (index, pixel) in pixels.as_chunks_mut::<3>().0.iter_mut().enumerate() {
            let x = index as u32 % width;
            let y = index as u32 / width;
            pixel[0] = 33 + (x * 5 % 191) as u8;
            pixel[1] = 41 + (y * 7 % 173) as u8;
            pixel[2] = 53 + ((x + y) * 3 % 149) as u8;
        }
        pixels
    }

    /// 用同一引擎读取整帧紧凑 RGB24，作为"改动前整帧对齐"的像素基准。
    fn full_frame_rgb(frame: &SafeFrame<'_>) -> Vec<u8> {
        let rect = CropRect {
            x: 0,
            y: 0,
            width: frame.width(),
            height: frame.height(),
        };
        cv::crop_rgb(frame, rect)
            .expect("整帧读取失败")
            .readback_rgb24()
            .expect("整帧 readback 失败")
    }

    /// 按中心与双眼间距合成五点关键点（比例取自真实人脸：眼距约占脸宽 0.7）。
    fn landmarks_with_eye_span(cx: f32, cy: f32, eye_span: f32) -> [[f32; 2]; 5] {
        [
            [cx - eye_span * 0.5, cy - eye_span * 0.18],
            [cx + eye_span * 0.5, cy - eye_span * 0.18],
            [cx, cy + eye_span * 0.02],
            [cx - eye_span * 0.42, cy + eye_span * 0.30],
            [cx + eye_span * 0.42, cy + eye_span * 0.30],
        ]
    }

    /// ROI 快照路径必须与「整帧读回 + 全图仿射」采样到同一批像素。
    ///
    /// 允许 3 LSB 阶差：ROI 路径在裁剪后的局部坐标系内重估仿射矩阵，与整帧坐标系的 f32
    /// 舍入不同，双线性权重恰好落在整像素边界时会有个别像素抖动（实测 37632 像素中 1 个）。
    /// 真实几何偏差（ROI 未覆盖采样域导致的填黑、采样错位）必是成片大偏差：
    /// 合成图案各通道下界为 33/41/53，填黑后与合法像素的差异远大于 3。
    fn assert_roi_matches_full_frame(
        label: &str,
        frame: &algo_sdk::testing::MockFrame,
        landmarks: &[[f32; 2]; 5],
    ) -> CropRect {
        let safe = frame.as_safe_frame();
        let (width, height) = (safe.width(), safe.height());
        let face = crate::detect::RawFace {
            bbox: [
                landmarks[0][0] / width as f32,
                landmarks[0][1] / height as f32,
                0.2,
                0.26,
            ],
            landmarks: landmarks.map(|point| [point[0] / width as f32, point[1] / height as f32]),
            landmark_scores: [0.9; 5],
            score: 0.9,
        };
        let expected = crate::align::align_face(
            &full_frame_rgb(&safe),
            width,
            height,
            &landmarks.map(|point| [point[0] / width as f32, point[1] / height as f32]),
        )
        .expect("整帧对齐失败");
        let actual = extract_aligned_face(&safe, &face).expect("ROI 对齐失败");
        let rect = face_snapshot_rect(&safe, landmarks).expect("ROI 计算失败");
        assert!(
            rect.x + rect.width <= width && rect.y + rect.height <= height,
            "{label} 帧 ROI=({},{},{}x{}) 超出帧范围",
            rect.x,
            rect.y,
            rect.width,
            rect.height
        );
        let max_delta = actual
            .iter()
            .zip(expected.iter())
            .map(|(left, right)| left.abs_diff(*right))
            .max()
            .unwrap_or(u8::MAX);
        assert!(
            max_delta <= 3,
            "{label} 帧 ROI=({},{},{}x{}) 与整帧路径采样偏差 {max_delta}：ROI 未覆盖仿射采样域",
            rect.x,
            rect.y,
            rect.width,
            rect.height
        );
        rect
    }

    /// ROI 裁剪 + 关键点平移必须与"整帧对齐"采样到完全相同的像素。
    ///
    /// best-shot 改走 ROI 硬件裁剪的前提是：ROI 只改变读取范围，不允许改变仿射采样
    /// 落在原图上的位置，否则 embedding 会随裁剪窗口抖动。
    #[test]
    fn roi_crop_geometry_matches_full_frame_alignment() {
        let (width, height) = (320u32, 240u32);
        let rgb = synthetic_rgb(width, height);
        let landmarks = [
            [0.32, 0.30],
            [0.42, 0.30],
            [0.37, 0.39],
            [0.32, 0.47],
            [0.42, 0.47],
        ];
        let pixel_landmarks =
            landmarks.map(|point| [point[0] * width as f32, point[1] * height as f32]);

        let host_rgb = MockFrameBuilder::new()
            .dimensions(width, height)
            .host_data(rgb.clone())
            .build();
        let nv12 = MockFrameBuilder::new()
            .dimensions(width, height)
            .host_data(rgb.clone())
            .to_nv12(16)
            .build();

        for (label, frame) in [("RGB24", &host_rgb), ("NV12", &nv12)] {
            let rect = assert_roi_matches_full_frame(label, frame, &pixel_landmarks);
            assert!(
                rect.width < width && rect.height < height,
                "{label} 帧 ROI=({},{},{}x{}) 未真正裁剪",
                rect.x,
                rect.y,
                rect.width,
                rect.height
            );
        }
    }

    /// ROI 覆盖性必须在所有档位与帧边界处都成立。
    ///
    /// 档位画布带 ±1 像素居中取整、YUV 路径还要向偶对齐，两边都可能把画布从理想位置推偏；
    /// 一旦推偏超过档位预留的余量，采样域就会落到画布外被填黑，embedding 静默退化。
    #[test]
    fn snapshot_roi_coverage_holds_across_tiers_and_frame_edges() {
        let (width, height) = (640u32, 360u32);
        let rgb = synthetic_rgb(width, height);
        let frames = [
            (
                "RGB24",
                MockFrameBuilder::new()
                    .dimensions(width, height)
                    .host_data(rgb.clone())
                    .build(),
            ),
            (
                "NV12",
                MockFrameBuilder::new()
                    .dimensions(width, height)
                    .host_data(rgb)
                    .to_nv12(16)
                    .build(),
            ),
        ];
        for (format, frame) in &frames {
            for eye_span in [18.0f32, 60.0, 120.0, 300.0] {
                for (cx_ratio, cy_ratio) in
                    [(0.5f32, 0.5f32), (0.06, 0.5), (0.94, 0.5), (0.5, 0.06)]
                {
                    let label = format!("{format} eye_span={eye_span} cx={cx_ratio}");
                    assert_roi_matches_full_frame(
                        &label,
                        frame,
                        &landmarks_with_eye_span(
                            width as f32 * cx_ratio,
                            height as f32 * cy_ratio,
                            eye_span,
                        ),
                    );
                }
            }
        }
    }

    /// best-shot 裁剪几何必须收敛到有界档位集合。
    ///
    /// 回归缺陷：精确采样域逐帧抖动会把 SDK 的 RGA 输出池规格预算（全进程 16 种，永不淘汰）
    /// 耗尽，此后所有 `cv::crop_rgb` 永久失败，best-shot 特征链路整体失效。
    #[test]
    fn snapshot_roi_geometry_stays_within_tier_budget() {
        for (width, height) in [(640u32, 360u32), (1280, 720), (704, 576)] {
            let rgb = synthetic_rgb(width, height);
            let mut frames = vec![(
                "RGB24",
                MockFrameBuilder::new()
                    .dimensions(width, height)
                    .host_data(rgb.clone())
                    .build(),
            )];
            if width.is_multiple_of(2) && height.is_multiple_of(2) {
                frames.push((
                    "NV12",
                    MockFrameBuilder::new()
                        .dimensions(width, height)
                        .host_data(rgb)
                        .to_nv12(16)
                        .build(),
                ));
            }
            for (format, frame) in &frames {
                let safe = frame.as_safe_frame();
                let mut geometries = std::collections::BTreeSet::new();
                // 横扫尺度（含越过最高档位）与画面位置，模拟人走动时的采样域抖动
                for step in 0..40 {
                    let eye_span = height.min(width) as f32 * (0.02 + 0.019 * step as f32);
                    for offset in 0..6 {
                        let landmarks = landmarks_with_eye_span(
                            width as f32 * (0.15 + 0.14 * offset as f32),
                            height as f32 * (0.35 + 0.06 * offset as f32),
                            eye_span,
                        );
                        let rect = face_snapshot_rect(&safe, &landmarks).expect("ROI 计算失败");
                        geometries.insert((rect.width, rect.height));
                    }
                }
                assert!(
                    geometries.len() <= SNAPSHOT_ROI_TIERS.len() + 1,
                    "{format} {width}x{height} 帧上出现 {} 种 ROI 几何（预算 {}）：{geometries:?}",
                    geometries.len(),
                    SNAPSHOT_ROI_TIERS.len() + 1
                );
            }
        }
    }
}
