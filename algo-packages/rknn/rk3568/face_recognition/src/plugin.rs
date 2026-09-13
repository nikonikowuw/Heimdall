//! 人脸检测插件的标准 `AlgoPlugin` 适配层。
//!
//! 集成 YOLOv8n-face 检测、ByteTrack 航迹追踪与 EdgeFace-xs 512 维特征提取。
//! 结合时域超球面加权特征融合与防漂移校验，提供高鲁棒的人脸识别体验。

use std::sync::Arc;

use algo_sdk::cv::{self, PreprocessMode};
use algo_sdk::emitter::ResultEmitter;
use algo_sdk::error::AlgoError;
use algo_sdk::frame::{FrameHandleView, SafeFrame};
use algo_sdk::plugin::{AlgoPlugin, InitContext};

use crate::align::align_face;
use crate::config::InstanceConfig;
use crate::quality::compute_quality;
use crate::SharedModels;

pub struct FaceRecognizer {
    pub models: Arc<SharedModels>,
    pub config: InstanceConfig,
    /// 仅用于当前算法实例内部的 best-shot 去重与特征融合，不向 C ABI/宿主输出内部 trackId。
    pub tracker: crate::bytetrack::ByteTracker,
    pub best_shots: crate::best_shot::BestShotManager,
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
            config,
            tracker: crate::bytetrack::ByteTracker::new(crate::bytetrack::ByteTrackConfig {
                confirm_new_tracks: false,
                ..Default::default()
            }),
            best_shots: crate::best_shot::BestShotManager::new(),
        })
    }

    fn process(
        &mut self,
        frame: SafeFrame<'_>,
        emitter: &mut ResultEmitter<'_>,
    ) -> Result<(), AlgoError> {
        // 1. 预处理：Letterbox 到检测模型输入尺寸 (640x384)
        let (buf, mode) = cv::letterbox(
            &frame,
            self.models.detector_width,
            self.models.detector_height,
            [114, 114, 114],
        )?;

        let PreprocessMode::Letterbox(layout) = mode else {
            return Err(AlgoError::Preprocess {
                reason: "人脸检测需要 Letterbox 预处理模式".to_string(),
            });
        };

        // 2. 执行人脸 NPU 推理与解码
        let min_score = self.config.detection_confidence_threshold;
        let raw_faces = if buf.as_dma_buf_layout().is_some() {
            self.models.worker.detect_dma_buf(buf, layout, min_score)?
        } else if let Some(host_bytes) = buf.as_host_bytes() {
            self.models
                .worker
                .detect_host(host_bytes.to_vec(), layout, min_score)?
        } else {
            return Err(AlgoError::Preprocess {
                reason: "预处理输出既无有效 DMA-BUF 布局，也无 Host 内存视图".to_string(),
            });
        };

        // 3. 空间几何关联挂载：
        // 在纯人脸检测模式下，由未匹配人脸自适应推导虚拟躯干 (Pseudo-body) 保底
        let associated = crate::association::associate_persons_and_faces(&[], &raw_faces);

        // 4. ByteTracker 追踪活跃人体框，维护稳定的内部 internal_track_id
        let track_dets: Vec<crate::bytetrack::TrackDetection> = associated
            .iter()
            .map(|candidate| crate::bytetrack::TrackDetection {
                bbox: candidate.person_bbox,
                score: candidate.person_score,
                class_id: 0,
            })
            .collect();
        let active_tracks = self.tracker.update(&track_dets);
        let active_track_ids: Vec<u64> = active_tracks.iter().map(|track| track.track_id).collect();

        // 5. 遍历关联目标，执行质量门控、低频 best-shot 时域超球面融合与结果组装
        let mut objects = Vec::with_capacity(associated.len());

        for (a_idx, candidate) in associated.iter().enumerate() {
            // 匹配内部 tracker 的 track_id
            let internal_track_id = active_tracks
                .iter()
                .filter_map(|t| {
                    let iou = crate::bytetrack::box_iou(&t.bbox, &candidate.person_bbox);
                    (iou >= 0.25).then_some((iou, t.track_id))
                })
                .max_by(|a, b| a.0.total_cmp(&b.0))
                .map(|(_, id)| id)
                .unwrap_or((a_idx + 1) as u64);

            let face_detail = if let Some(face) = candidate.attached_face {
                let face_width_pixels = face.width() * frame.width() as f32;
                let face_height_pixels = face.height() * frame.height() as f32;
                let quality = compute_quality(
                    &face.landmarks,
                    &face.landmark_scores,
                    face_width_pixels.min(face_height_pixels),
                    &self.config.quality_thresholds,
                );

                // 仅在人脸通过姿态质量门控时触发低频 EdgeFace 特征提取 (best-shot)
                let embedding_str = if quality
                    .accepted(&self.config.quality_thresholds, self.config.min_face_size)
                {
                    let embedding = if self.best_shots.should_update_best_shot(
                        internal_track_id,
                        &quality,
                        frame.frame_id() as usize,
                    ) {
                        let extract = || -> Result<[f32; 512], AlgoError> {
                            let rgb_data = extract_frame_rgb(&frame)?;
                            let aligned = align_face(
                                &rgb_data,
                                frame.width(),
                                frame.height(),
                                &face.landmarks,
                            )?;
                            self.models.worker.embed_host(aligned)
                        };

                        match extract() {
                            Ok(normalized) => {
                                let fused = self.best_shots.update_with_fusion(
                                    internal_track_id,
                                    face.bbox,
                                    face.landmarks,
                                    face.score,
                                    quality,
                                    &normalized,
                                    frame.frame_id() as usize,
                                );
                                Some(fused)
                            }
                            Err(error) => {
                                self.best_shots.record_attempt_without_embedding(
                                    internal_track_id,
                                    face.bbox,
                                    face.landmarks,
                                    face.score,
                                    quality,
                                    frame.frame_id() as usize,
                                );
                                tracing::warn!(
                                    %error,
                                    "best-shot RKNN 设备侧 EdgeFace 提取失败，保留检测结果并允许后续重试"
                                );
                                None
                            }
                        }
                    } else {
                        None
                    };

                    embedding
                        .as_ref()
                        .map(|value| crate::postprocess::encode_embedding(value.as_slice()))
                        .transpose()?
                } else {
                    None
                };

                // 人脸检测框只要检出，就必须作为精细元数据输出给宿主管线与前端实时绘制
                Some(crate::postprocess::FaceDetailObject {
                    bbox: crate::postprocess::normalized_xywh_to_xyxy(face.bbox),
                    confidence: face.score.clamp(0.0, 1.0),
                    quality_score: Some(quality.score.clamp(0.0, 1.0)),
                    embedding: embedding_str,
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

        self.best_shots.retain_active_tracks(&active_track_ids);
        crate::postprocess::emit_detection_objects(emitter, &objects)
    }

    fn update_config(&mut self, config: Self::Config) -> Result<(), AlgoError> {
        config
            .validate()
            .map_err(|reason| AlgoError::ConfigParse { reason })?;
        self.config = config;
        Ok(())
    }

    fn flush(&mut self, _emitter: &mut ResultEmitter<'_>) -> Result<(), AlgoError> {
        self.tracker.reset();
        self.best_shots.clear();
        Ok(())
    }
}

/// 从 SafeFrame 中按需提取连续 RGB24 图像（低频抓拍与 best-shot 路径使用）
fn extract_frame_rgb(frame: &SafeFrame<'_>) -> Result<Vec<u8>, AlgoError> {
    let w = frame.width() as usize;
    let h = frame.height() as usize;
    if w == 0 || h == 0 {
        return Err(AlgoError::Preprocess {
            reason: "帧宽高不能为 0".to_string(),
        });
    }

    match frame.handle_view() {
        FrameHandleView::Host { data } => decode_host_frame_to_rgb(frame, data, w, h),
        FrameHandleView::DmaBuf { fd } => {
            #[cfg(target_os = "linux")]
            {
                decode_dma_buf_to_rgb(frame, fd, w, h)
            }
            #[cfg(not(target_os = "linux"))]
            {
                let _ = fd;
                Err(AlgoError::IncompatibleFrame {
                    reason: "非 Linux 环境无法直接读取 DMA-BUF".to_string(),
                })
            }
        }
        _ => Err(AlgoError::IncompatibleFrame {
            reason: "暂不支持的帧内存句柄类型".to_string(),
        }),
    }
}

/// 将 Host 内存中的帧数据按像素格式转换为 RGB24
fn decode_host_frame_to_rgb(
    frame: &SafeFrame<'_>,
    data: &[u8],
    w: usize,
    h: usize,
) -> Result<Vec<u8>, AlgoError> {
    let output_len = w
        .checked_mul(h)
        .and_then(|pixels| pixels.checked_mul(3))
        .ok_or(AlgoError::OutOfMemory)?;

    match frame.pixel_format() {
        algo_sdk::c_abi::AV_PIX_RGB24 => {
            let min_row = w.checked_mul(3).ok_or(AlgoError::OutOfMemory)?;
            let stride0 = if frame.stride(0) > 0 {
                frame.stride(0) as usize
            } else {
                min_row
            };
            if stride0 == min_row {
                let source = data.get(..output_len).ok_or(AlgoError::OutOfMemory)?;
                Ok(source.to_vec())
            } else {
                let mut rgb = Vec::with_capacity(output_len);
                for y in 0..h {
                    let row_start = y * stride0;
                    let row = data
                        .get(row_start..row_start + min_row)
                        .ok_or(AlgoError::OutOfMemory)?;
                    rgb.extend_from_slice(row);
                }
                Ok(rgb)
            }
        }
        algo_sdk::c_abi::AV_PIX_NV12 => {
            let y_stride = if frame.stride(0) > 0 {
                frame.stride(0) as usize
            } else {
                w
            };
            let uv_stride = if frame.stride(1) > 0 {
                frame.stride(1) as usize
            } else {
                w.div_ceil(2) * 2
            };
            let y_offset = usize::try_from(frame.plane_offset(0)).unwrap_or(0);
            let alloc_h = if frame.alloc_height() > 0 {
                frame.alloc_height() as usize
            } else {
                h
            };
            let default_uv_offset = y_offset
                .checked_add(y_stride.checked_mul(alloc_h).unwrap_or(0))
                .unwrap_or(0);
            let uv_offset = if frame.plane_offset(1) > 0 {
                usize::try_from(frame.plane_offset(1)).unwrap_or(default_uv_offset)
            } else {
                default_uv_offset
            };

            decode_nv12_to_rgb(data, w, h, y_stride, uv_stride, y_offset, uv_offset)
        }
        algo_sdk::c_abi::AV_PIX_BGRA => {
            let min_row = w.checked_mul(4).ok_or(AlgoError::OutOfMemory)?;
            let stride0 = if frame.stride(0) > 0 {
                frame.stride(0) as usize
            } else {
                min_row
            };
            let required_len = (h.saturating_sub(1))
                .checked_mul(stride0)
                .and_then(|v| v.checked_add(min_row))
                .ok_or(AlgoError::OutOfMemory)?;
            if data.len() < required_len {
                return Err(AlgoError::Preprocess {
                    reason: "BGRA 数据长度不足".to_string(),
                });
            }
            let mut rgb = vec![0u8; output_len];
            for y in 0..h {
                let src_row = &data[y * stride0..y * stride0 + min_row];
                let dst_row = &mut rgb[y * w * 3..(y + 1) * w * 3];
                for x in 0..w {
                    let s = x * 4;
                    let d = x * 3;
                    dst_row[d] = src_row[s + 2];
                    dst_row[d + 1] = src_row[s + 1];
                    dst_row[d + 2] = src_row[s];
                }
            }
            Ok(rgb)
        }
        _ => Err(AlgoError::IncompatibleFrame {
            reason: format!("暂不支持由格式 {} 转换为 RGB24", frame.pixel_format()),
        }),
    }
}

/// 高性能 NV12 转 RGB24（水平色度解耦复用与零单像素内层分支）
fn decode_nv12_to_rgb(
    data: &[u8],
    w: usize,
    h: usize,
    y_stride: usize,
    uv_stride: usize,
    y_offset: usize,
    uv_offset: usize,
) -> Result<Vec<u8>, AlgoError> {
    let output_len = w
        .checked_mul(h)
        .and_then(|p| p.checked_mul(3))
        .ok_or(AlgoError::OutOfMemory)?;

    let y_plane = data.get(y_offset..).ok_or_else(|| AlgoError::Preprocess {
        reason: "NV12 Y 平面越界".to_string(),
    })?;
    let uv_plane = data.get(uv_offset..).ok_or_else(|| AlgoError::Preprocess {
        reason: "NV12 UV 平面越界".to_string(),
    })?;

    let y_required = (h.saturating_sub(1))
        .checked_mul(y_stride)
        .and_then(|v| v.checked_add(w))
        .ok_or(AlgoError::OutOfMemory)?;
    let uv_required = (h.div_ceil(2).saturating_sub(1))
        .checked_mul(uv_stride)
        .and_then(|v| v.checked_add(w.div_ceil(2) * 2))
        .ok_or(AlgoError::OutOfMemory)?;

    if y_plane.len() < y_required || uv_plane.len() < uv_required {
        return Err(AlgoError::Preprocess {
            reason: "NV12 平面数据长度不足".to_string(),
        });
    }

    let mut rgb = vec![0u8; output_len];
    let clamp_u8 = |v: f32| -> u8 { v.clamp(0.0, 255.0).round() as u8 };

    for y in 0..h {
        let uv_row_start = (y / 2) * uv_stride;
        let y_row_start = y * y_stride;
        let dst_row_start = y * w * 3;

        let even_w = (w / 2) * 2;
        for pair_idx in 0..(even_w / 2) {
            let x0 = pair_idx * 2;
            let x1 = x0 + 1;
            let uv_idx = uv_row_start + x0;
            let u_val = uv_plane[uv_idx] as f32;
            let v_val = uv_plane[uv_idx + 1] as f32;

            let d = u_val - 128.0;
            let e = v_val - 128.0;
            let r_chroma = 1.596 * e;
            let g_chroma = -0.392 * d - 0.813 * e;
            let b_chroma = 2.017 * d;

            let c0 = (y_plane[y_row_start + x0] as f32 - 16.0) * 1.164;
            let dst_idx0 = dst_row_start + x0 * 3;
            rgb[dst_idx0] = clamp_u8(c0 + r_chroma);
            rgb[dst_idx0 + 1] = clamp_u8(c0 + g_chroma);
            rgb[dst_idx0 + 2] = clamp_u8(c0 + b_chroma);

            let c1 = (y_plane[y_row_start + x1] as f32 - 16.0) * 1.164;
            let dst_idx1 = dst_row_start + x1 * 3;
            rgb[dst_idx1] = clamp_u8(c1 + r_chroma);
            rgb[dst_idx1 + 1] = clamp_u8(c1 + g_chroma);
            rgb[dst_idx1 + 2] = clamp_u8(c1 + b_chroma);
        }

        if even_w < w {
            let x = even_w;
            let uv_idx = uv_row_start + x;
            let u_val = uv_plane[uv_idx] as f32;
            let v_val = uv_plane[uv_idx + 1] as f32;
            let d = u_val - 128.0;
            let e = v_val - 128.0;
            let c = (y_plane[y_row_start + x] as f32 - 16.0) * 1.164;
            let dst_idx = dst_row_start + x * 3;
            rgb[dst_idx] = clamp_u8(c + 1.596 * e);
            rgb[dst_idx + 1] = clamp_u8(c - 0.392 * d - 0.813 * e);
            rgb[dst_idx + 2] = clamp_u8(c + 2.017 * d);
        }
    }
    Ok(rgb)
}

#[cfg(target_os = "linux")]
fn decode_dma_buf_to_rgb(
    frame: &SafeFrame<'_>,
    fd: i32,
    w: usize,
    h: usize,
) -> Result<Vec<u8>, AlgoError> {
    let size = match frame.pixel_format() {
        algo_sdk::c_abi::AV_PIX_NV12 => {
            let y_stride = if frame.stride(0) > 0 {
                frame.stride(0) as usize
            } else {
                w
            };
            let alloc_h = if frame.alloc_height() > 0 {
                frame.alloc_height() as usize
            } else {
                h
            };
            y_stride
                .checked_mul(alloc_h)
                .and_then(|y| y.checked_add(y / 2))
                .ok_or(AlgoError::OutOfMemory)?
        }
        algo_sdk::c_abi::AV_PIX_RGB24 => {
            let stride = if frame.stride(0) > 0 {
                frame.stride(0) as usize
            } else {
                w * 3
            };
            stride.checked_mul(h).ok_or(AlgoError::OutOfMemory)?
        }
        _ => {
            return Err(AlgoError::IncompatibleFrame {
                reason: format!("暂不支持 DMA-BUF 格式: {}", frame.pixel_format()),
            });
        }
    };

    // SAFETY: mmap 使用只读标志映射有效 DMA-BUF fd，失败时返回 MAP_FAILED 由下方判断处理。
    let ptr = unsafe {
        libc::mmap(
            std::ptr::null_mut(),
            size,
            libc::PROT_READ,
            libc::MAP_SHARED,
            fd,
            0,
        )
    };
    if ptr == libc::MAP_FAILED {
        return Err(AlgoError::IncompatibleFrame {
            reason: format!(
                "mmap DMA-BUF fd={fd} 失败: {}",
                std::io::Error::last_os_error()
            ),
        });
    }

    struct MmapGuard {
        ptr: *mut libc::c_void,
        size: usize,
    }
    impl Drop for MmapGuard {
        fn drop(&mut self) {
            // SAFETY: ptr 由 libc::mmap 成功分配，size 与映射大小严格一致。
            unsafe {
                libc::munmap(self.ptr, self.size);
            }
        }
    }
    let _guard = MmapGuard { ptr, size };

    // SAFETY: ptr 经过 MAP_FAILED 检查，size 覆盖该缓冲区，并在 _guard 存活期间只读有效。
    let slice = unsafe { std::slice::from_raw_parts(ptr as *const u8, size) };
    decode_host_frame_to_rgb(frame, slice, w, h)
}
