//! Rockchip RK3576 人脸检测与 EdgeFace 特征提取算法包
//!
//! 基于 YOLOv8n-face 检测 + EdgeFace-xs 嵌入，运行在 RKNN NPU 上。

pub mod align;
pub mod config;
pub mod detect;
pub mod plugin;
pub mod postprocess;
pub mod quality;
pub mod rknn;

use std::ffi::c_int;
use std::path::Path;
use std::sync::{Arc, Mutex, OnceLock};

use algo_sdk::c_abi::{AvAlgoLibrary, AvFaceExtractInput, AvFaceExtractOutput};
use algo_sdk::c_abi::{
    AV_ALGO_API_VERSION, AV_ERR_INFERENCE_FAILED, AV_ERR_INTERNAL, AV_ERR_INVALID_ARG,
    AV_ERR_MODEL_LOAD_FAILED, AV_OK,
};
use algo_sdk::cv::{compute_letterbox_layout, LetterboxLayout};
use algo_sdk::error::AlgoError;
use algo_sdk::export_algo;
use algo_sdk::macros::{validate_abi_header, LibraryContext};
use algo_sdk::plugin::AlgoPlugin;
use image::codecs::jpeg::JpegEncoder;
use image::{ExtendedColorType, RgbImage};
use std::ffi::c_char;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::slice;

use plugin::FaceRecognizer;
use rknn::{RknnRuntime, RknnSession};

/// 共享模型持有者：一个 runtime + 两个带互斥锁的 session
#[doc(hidden)]
pub struct SharedModels {
    _runtime: Arc<RknnRuntime>,
    pub detector: Mutex<RknnSession>,
    pub embedder: Mutex<RknnSession>,
}

impl std::fmt::Debug for SharedModels {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SharedModels")
            .field("detector", &"Mutex<RknnSession>")
            .field("embedder", &"Mutex<RknnSession>")
            .finish()
    }
}

static SHARED_MODELS: OnceLock<Mutex<Option<Arc<SharedModels>>>> = OnceLock::new();

fn shared_model_slot() -> &'static Mutex<Option<Arc<SharedModels>>> {
    SHARED_MODELS.get_or_init(|| Mutex::new(None))
}

pub fn shared_models(package_root: &Path) -> Result<Arc<SharedModels>, AlgoError> {
    {
        let guard = shared_model_slot()
            .lock()
            .map_err(|_| AlgoError::Internal {
                reason: "RKNN 模型共享锁已中毒".to_string(),
            })?;
        if let Some(models) = guard.as_ref() {
            return Ok(Arc::clone(models));
        }
    }

    let runtime = RknnRuntime::load(package_root)?;

    let detector_path = package_root.join("model/yolov8n-face-640x384_mixed_face.rknn");
    let embedder_path = package_root.join("model/edgeface_xs_gamma_06_rk3576_fp16.rknn");

    let detector = RknnSession::new(Arc::clone(&runtime), &detector_path)?;
    let embedder = RknnSession::new(Arc::clone(&runtime), &embedder_path)?;

    tracing::info!("RKNN 双模型加载完成");

    let models = Arc::new(SharedModels {
        _runtime: runtime,
        detector: Mutex::new(detector),
        embedder: Mutex::new(embedder),
    });

    let mut guard = shared_model_slot()
        .lock()
        .map_err(|_| AlgoError::Internal {
            reason: "RKNN 模型共享锁已中毒".to_string(),
        })?;
    if let Some(existing) = guard.as_ref() {
        Ok(Arc::clone(existing))
    } else {
        *guard = Some(Arc::clone(&models));
        Ok(models)
    }
}

pub(crate) fn open_shared_models(package_root: &Path) -> Result<(), AlgoError> {
    shared_models(package_root).map(|_| ())
}

pub(crate) fn close_shared_models(_package_root: &Path) {
    if let Some(slot) = SHARED_MODELS.get() {
        if let Ok(mut guard) = slot.lock() {
            *guard = None;
        }
    }
}

export_algo!(
    FaceRecognizer,
    algo_id: "face_recognition",
    version: "1.0.0",
    algo_type: "face_recognition",
    alarm_type_id: "face_recognize",
    library_open_hook: crate::open_shared_models,
    library_close_hook: crate::close_shared_models
);

/// 计算两个 512D 特征向量之间的余弦相似度。
///
/// 若两向量已完成 L2 归一化，余弦相似度即为其点积。
pub fn cosine_similarity(a: &[f32; 512], b: &[f32; 512]) -> f32 {
    let dot: f32 = a.iter().zip(b.iter()).map(|(x, y)| x * y).sum();
    dot.clamp(-1.0, 1.0)
}

/// L2 归一化 embedding 向量。
pub fn normalize_embedding(values: &[f32]) -> Result<[f32; 512], AlgoError> {
    if values.len() < 512 {
        return Err(AlgoError::Inference {
            reason: format!("EdgeFace 输出维度不足: {} < 512", values.len()),
        });
    }
    let norm = values[..512].iter().map(|v| v * v).sum::<f32>().sqrt();
    if !norm.is_finite() || norm <= f32::EPSILON {
        return Err(AlgoError::Inference {
            reason: "EdgeFace embedding L2 范数无效".to_string(),
        });
    }
    let mut embedding = [0.0f32; 512];
    for (target, source) in embedding.iter_mut().zip(values.iter().take(512)) {
        *target = *source / norm;
    }
    Ok(embedding)
}

/// Letterbox 预处理：将 RGB 图像缩放并填充到目标尺寸，返回填充后的画布与 LetterboxLayout
pub fn prepare_detector_input(image: &RgbImage) -> (Vec<u8>, LetterboxLayout) {
    let (width, height) = image.dimensions();
    let layout = compute_letterbox_layout(width, height, 640, 384);

    let resized = image::imageops::resize(
        image,
        layout.scaled_w,
        layout.scaled_h,
        image::imageops::FilterType::Triangle,
    );

    let mut canvas = vec![114u8; (layout.dst_w * layout.dst_h * 3) as usize];
    for y in 0..layout.scaled_h as usize {
        let src_row = &resized.as_raw()
            [y * layout.scaled_w as usize * 3..(y + 1) * layout.scaled_w as usize * 3];
        let dst_offset =
            ((y + layout.pad_top as usize) * layout.dst_w as usize + layout.pad_left as usize) * 3;
        canvas[dst_offset..dst_offset + src_row.len()].copy_from_slice(src_row);
    }

    (canvas, layout)
}

/// 编码 112×112 RGB 为 JPEG
fn encode_aligned_jpeg(rgb: &[u8]) -> Result<Vec<u8>, AlgoError> {
    if rgb.len() != 112 * 112 * 3 {
        return Err(AlgoError::Preprocess {
            reason: "对齐人脸尺寸不是 112×112 RGB".to_string(),
        });
    }
    let mut jpeg = Vec::with_capacity(16 * 1024);
    let mut encoder = JpegEncoder::new_with_quality(&mut jpeg, 90);
    encoder
        .encode(rgb, 112, 112, ExtendedColorType::Rgb8)
        .map_err(|error| AlgoError::Preprocess {
            reason: format!("对齐人脸 JPEG 编码失败: {error}"),
        })?;
    if jpeg.len() > 65_536 {
        return Err(AlgoError::OutOfMemory);
    }
    Ok(jpeg)
}

fn write_output_error(output: &mut AvFaceExtractOutput, message: &str, status: c_int) {
    output.status_code = status.unsigned_abs();
    let bytes = message.as_bytes();
    let copy_len = bytes
        .len()
        .min(output.error_message.len().saturating_sub(1));
    for (target, source) in output
        .error_message
        .iter_mut()
        .zip(bytes.iter())
        .take(copy_len)
    {
        *target = *source as c_char;
    }
    if copy_len < output.error_message.len() {
        output.error_message[copy_len] = 0;
    }
}

fn write_output_success(
    output: &mut AvFaceExtractOutput,
    embedding: [f32; 512],
    bbox: [f32; 4],
    quality_score: f32,
    detection_score: f32,
    jpeg: &[u8],
) {
    output.status_code = 0;
    output.embedding = embedding;
    output.embedding_dim = 512;
    output.bbox = bbox;
    output.quality_score = quality_score;
    output.detection_score = detection_score;
    output.aligned_jpeg_data[..jpeg.len()].copy_from_slice(jpeg);
    output.aligned_jpeg_len = jpeg.len() as u32;
}

unsafe fn extract_face_impl(
    lib: AvAlgoLibrary,
    input: *const AvFaceExtractInput,
    output: *mut AvFaceExtractOutput,
) -> c_int {
    if lib.is_null() || input.is_null() || output.is_null() {
        return AV_ERR_INVALID_ARG;
    }
    if let Err(error) = unsafe {
        // SAFETY: input 指针非空且调用方 ABI 契约要求至少可读取完整头部
        validate_abi_header(input, "AvFaceExtractInput")
    } {
        return error.to_c_status();
    }
    if let Err(error) = unsafe {
        // SAFETY: output 指针非空且调用方 ABI 契约要求至少可读取完整头部
        validate_abi_header(output, "AvFaceExtractOutput")
    } {
        return error.to_c_status();
    }
    // SAFETY: 两个结构体已通过非空、对齐、size 和 api_version 校验
    let input_ref = unsafe { &*input };
    // SAFETY: output 指向调用方提供的完整可写 POD 缓冲区
    unsafe {
        std::ptr::write_bytes(
            output.cast::<u8>(),
            0,
            std::mem::size_of::<AvFaceExtractOutput>(),
        )
    };
    // SAFETY: output 指向调用方提供的完整可写 POD 缓冲区，已通过 size/api_version 校验
    let output_ref = unsafe { &mut *output };
    output_ref.size = std::mem::size_of::<AvFaceExtractOutput>() as u32;
    output_ref.api_version = AV_ALGO_API_VERSION;

    if input_ref.image_bytes.is_null() || input_ref.image_bytes_len == 0 {
        write_output_error(output_ref, "image_bytes 为空", AV_ERR_INVALID_ARG);
        return AV_ERR_INVALID_ARG;
    }
    let image_len = input_ref.image_bytes_len as usize;
    if image_len > 32 * 1024 * 1024 {
        write_output_error(output_ref, "JPEG 输入超过 32 MiB 限制", AV_ERR_INVALID_ARG);
        return AV_ERR_INVALID_ARG;
    }
    // SAFETY: C ABI 输入契约保证 image_bytes 指向 image_bytes_len 个只读字节
    let image_bytes = unsafe { slice::from_raw_parts(input_ref.image_bytes, image_len) };
    let image = match image::load_from_memory(image_bytes) {
        Ok(image) => image.to_rgb8(),
        Err(error) => {
            let algo_error = AlgoError::Preprocess {
                reason: format!("JPEG 解码失败: {error}"),
            };
            write_output_error(
                output_ref,
                &algo_error.to_string(),
                algo_error.to_c_status(),
            );
            return algo_error.to_c_status();
        }
    };

    // 1. Letterbox 预处理
    let (detector_rgb, layout) = prepare_detector_input(&image);
    let (orig_w, orig_h) = (image.width(), image.height());

    // 2. 获取共享 RKNN 模型
    // SAFETY: lib 是 export_algo!::library_open 返回的 LibraryContext 句柄，且在本次同步调用期间有效
    let library = unsafe { &*(lib as *const LibraryContext) };
    let models = match shared_models(&library.package_root) {
        Ok(models) => models,
        Err(error) => {
            write_output_error(output_ref, &error.to_string(), AV_ERR_MODEL_LOAD_FAILED);
            return AV_ERR_MODEL_LOAD_FAILED;
        }
    };

    // 3. 检测推理
    let min_score = if input_ref.min_detection_score.is_finite() {
        input_ref.min_detection_score.clamp(0.0, 1.0)
    } else {
        0.5
    };

    let detect_result = {
        let detector = match models.detector.lock() {
            Ok(guard) => guard,
            Err(_) => {
                write_output_error(output_ref, "RKNN 检测会话互斥锁中毒", AV_ERR_INTERNAL);
                return AV_ERR_INTERNAL;
            }
        };

        let attrs: Vec<[u32; 4]> = detector
            .output_attrs
            .iter()
            .map(|a| [a.dims[0], a.dims[1], a.dims[2], a.dims[3]])
            .collect();

        detector.infer_with_host_bytes(&detector_rgb, |output| match output {
            rknn::RknnInferenceOutput::Float32(float_views) => {
                let faces =
                    detect::decode_yolov8_face(float_views, &attrs, &layout, min_score, 0.45);
                Ok(faces)
            }
        })
    };

    let raw_faces = match detect_result {
        Ok(faces) => faces,
        Err(error) => {
            write_output_error(output_ref, &error.to_string(), AV_ERR_INFERENCE_FAILED);
            return AV_ERR_INFERENCE_FAILED;
        }
    };

    // 4. 质量过滤
    let min_face_size = if input_ref.min_face_size.is_finite() {
        input_ref.min_face_size.max(1.0).round() as u32
    } else {
        30
    };
    let min_quality_score = if input_ref.min_quality_score.is_finite() {
        input_ref.min_quality_score.clamp(0.0, 1.0)
    } else {
        0.3
    };
    let thresholds = config::QualityThresholds {
        min_score: min_quality_score,
        ..config::QualityThresholds::default()
    };

    // 在原始图像坐标空间计算质量（landmarks 已归一化，face_size 需要原始像素）
    let Some((best_face, quality)) = raw_faces
        .into_iter()
        .filter_map(|face| {
            let face_width_pixels = face.bbox[2] * orig_w as f32;
            let quality = quality::compute_quality(
                &face.landmarks,
                &face.landmark_scores,
                face_width_pixels,
                &thresholds,
            );
            if quality.accepted(&thresholds, min_face_size) {
                Some((face, quality))
            } else {
                None
            }
        })
        .max_by(|left, right| left.0.score.total_cmp(&right.0.score))
    else {
        write_output_error(
            output_ref,
            "NO_FACE_DETECTED: 输入图像未检出满足置信度与质量阈值的人脸",
            AV_ERR_INFERENCE_FAILED,
        );
        return AV_ERR_INFERENCE_FAILED;
    };

    // 5. 对齐
    let aligned = match align::align_face(image.as_raw(), orig_w, orig_h, &best_face.landmarks) {
        Ok(aligned) => aligned,
        Err(error) => {
            write_output_error(output_ref, &error.to_string(), error.to_c_status());
            return error.to_c_status();
        }
    };

    // 6. 嵌入提取
    let embedding_result = {
        let embedder = match models.embedder.lock() {
            Ok(guard) => guard,
            Err(_) => {
                write_output_error(output_ref, "RKNN 嵌入会话互斥锁中毒", AV_ERR_INTERNAL);
                return AV_ERR_INTERNAL;
            }
        };
        embedder.infer_with_host_bytes(&aligned, |output| match output {
            rknn::RknnInferenceOutput::Float32(float_views) => {
                let raw_emb = float_views[0];
                let embedding = normalize_embedding(raw_emb)?;
                Ok(embedding)
            }
        })
    };

    let embedding = match embedding_result {
        Ok(embedding) => embedding,
        Err(error) => {
            write_output_error(
                output_ref,
                &format!("EMBED_INFERENCE_FAILED: {error}"),
                AV_ERR_INFERENCE_FAILED,
            );
            return AV_ERR_INFERENCE_FAILED;
        }
    };

    // 7. 编码对齐后 JPEG
    let jpeg = match encode_aligned_jpeg(&aligned) {
        Ok(jpeg) => jpeg,
        Err(error) => {
            write_output_error(output_ref, &error.to_string(), error.to_c_status());
            return error.to_c_status();
        }
    };

    write_output_success(
        output_ref,
        embedding,
        best_face.bbox,
        quality.score,
        best_face.score,
        &jpeg,
    );
    AV_OK
}

/// 独立的人脸特征提取 C ABI 符号
///
/// # Safety
/// `lib` 必须来自本动态库导出的 `library_open`，`input`/`output` 必须分别指向
/// 满足 ABI 版本和尺寸约束的有效内存。
#[no_mangle]
pub unsafe extern "C" fn av_algo_extract_face(
    lib: AvAlgoLibrary,
    input: *const AvFaceExtractInput,
    output: *mut AvFaceExtractOutput,
) -> c_int {
    let result = catch_unwind(AssertUnwindSafe(|| {
        // SAFETY: ABI 调用方契约由 extract_face_impl 校验；内部不会让 panic 穿越 C 栈
        unsafe { extract_face_impl(lib, input, output) }
    }));
    match result {
        Ok(status) => status,
        Err(_) => {
            if !output.is_null()
                && (output as usize).is_multiple_of(std::mem::align_of::<AvFaceExtractOutput>())
            {
                // SAFETY: 只在指针对齐且非空时访问
                unsafe {
                    if (*output).size as usize >= std::mem::size_of::<AvFaceExtractOutput>() {
                        write_output_error(
                            &mut *output,
                            "av_algo_extract_face 发生 Panic",
                            AV_ERR_INTERNAL,
                        );
                    }
                }
            }
            AV_ERR_INTERNAL
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cosine_similarity_properties() {
        let mut v1 = [0.0f32; 512];
        let mut v2 = [0.0f32; 512];
        v1[0] = 1.0;
        v2[0] = 1.0;
        assert!((cosine_similarity(&v1, &v2) - 1.0).abs() < 1e-6);

        v2[0] = 0.0;
        v2[1] = 1.0;
        assert!(cosine_similarity(&v1, &v2).abs() < 1e-6);
    }

    #[test]
    fn test_normalize_embedding_unit_length() {
        let mut input = vec![0.0f32; 512];
        for (i, val) in input.iter_mut().enumerate() {
            *val = (i + 1) as f32;
        }
        let normalized = normalize_embedding(&input).expect("归一化应成功");
        let l2_sq: f32 = normalized.iter().map(|x| x * x).sum();
        assert!((l2_sq.sqrt() - 1.0).abs() < 1e-5);
    }
}
