//! macOS arm64 人脸检测与 EdgeFace 特征提取算法包。

pub mod align;
pub mod config;
#[cfg(target_os = "macos")]
pub mod coreml;
pub mod detect;
pub mod plugin;
pub mod postprocess;
pub mod quality;

use std::ffi::c_int;

use algo_sdk::c_abi::{AvAlgoLibrary, AvFaceExtractInput, AvFaceExtractOutput};
use algo_sdk::export_algo;
use algo_sdk::plugin::AlgoPlugin;
use plugin::FaceRecognizer;

#[cfg(not(target_os = "macos"))]
use algo_sdk::c_abi::AV_ERR_NOT_IMPLEMENTED;

#[cfg(target_os = "macos")]
use {
    crate::align::align_face,
    crate::coreml::CoreMlFaceModels,
    crate::detect::{decode_face_detections, nms, unmap_letterbox},
    crate::quality::compute_quality,
    algo_sdk::c_abi::{
        AV_ALGO_API_VERSION, AV_ERR_INFERENCE_FAILED, AV_ERR_INTERNAL, AV_ERR_INVALID_ARG,
        AV_ERR_MODEL_LOAD_FAILED, AV_OK,
    },
    algo_sdk::cv::types::{LetterboxLayout, PreprocessMode},
    algo_sdk::error::AlgoError,
    algo_sdk::macros::{validate_abi_header, LibraryContext},
    image::codecs::jpeg::JpegEncoder,
    image::{ExtendedColorType, RgbImage},
    std::ffi::c_char,
    std::panic::{catch_unwind, AssertUnwindSafe},
    std::path::Path,
    std::slice,
    std::sync::{Arc, Mutex, OnceLock},
};

export_algo!(
    FaceRecognizer,
    algo_id: "face_recognition",
    version: "1.0.0",
    algo_type: "face_recognition",
    alarm_type_id: "face_recognize",
    library_open_hook: crate::open_shared_models,
    library_close_hook: crate::close_shared_models
);

#[cfg(target_os = "macos")]
static SHARED_MODELS: OnceLock<Mutex<Option<Arc<CoreMlFaceModels>>>> = OnceLock::new();

#[cfg(target_os = "macos")]
fn shared_model_slot() -> &'static Mutex<Option<Arc<CoreMlFaceModels>>> {
    SHARED_MODELS.get_or_init(|| Mutex::new(None))
}

#[cfg(target_os = "macos")]
pub(crate) fn shared_models(package_root: &Path) -> Result<Arc<CoreMlFaceModels>, AlgoError> {
    {
        let guard = shared_model_slot()
            .lock()
            .map_err(|_| AlgoError::Internal {
                reason: "CoreML 模型共享锁已中毒".to_string(),
            })?;
        if let Some(models) = guard.as_ref() {
            return Ok(Arc::clone(models));
        }
    }

    let models = Arc::new(CoreMlFaceModels::load(package_root)?);
    let mut guard = shared_model_slot()
        .lock()
        .map_err(|_| AlgoError::Internal {
            reason: "CoreML 模型共享锁已中毒".to_string(),
        })?;
    if let Some(existing) = guard.as_ref() {
        Ok(Arc::clone(existing))
    } else {
        *guard = Some(Arc::clone(&models));
        Ok(models)
    }
}

#[cfg(target_os = "macos")]
pub(crate) fn open_shared_models(package_root: &Path) -> Result<(), AlgoError> {
    shared_models(package_root).map(|_| ())
}

#[cfg(target_os = "macos")]
pub(crate) fn close_shared_models(_package_root: &Path) {
    if let Some(slot) = SHARED_MODELS.get() {
        if let Ok(mut guard) = slot.lock() {
            *guard = None;
        }
    }
}

#[cfg(not(target_os = "macos"))]
pub(crate) fn open_shared_models(
    _package_root: &std::path::Path,
) -> Result<(), algo_sdk::error::AlgoError> {
    Ok(())
}

#[cfg(not(target_os = "macos"))]
pub(crate) fn close_shared_models(_package_root: &std::path::Path) {}

/// 计算两个 512D 特征向量之间的余弦相似度。
///
/// 若两向量已完成 L2 归一化，余弦相似度即为其点积；返回值限制在 `[-1.0, 1.0]` 区间内。
pub fn cosine_similarity(a: &[f32; 512], b: &[f32; 512]) -> f32 {
    let dot: f32 = a.iter().zip(b.iter()).map(|(x, y)| x * y).sum();
    dot.clamp(-1.0, 1.0)
}

#[cfg(target_os = "macos")]
pub fn prepare_detector_input(image: &RgbImage) -> Result<(Vec<u8>, PreprocessMode), AlgoError> {
    let (width, height) = image.dimensions();
    if width == 0 || height == 0 {
        return Err(AlgoError::Preprocess {
            reason: "JPEG 图像尺寸不能为 0".to_string(),
        });
    }
    let scale = (640.0 / width as f32).min(384.0 / height as f32);
    let scaled_w = ((width as f32 * scale).round() as u32).clamp(1, 640);
    let scaled_h = ((height as f32 * scale).round() as u32).clamp(1, 384);
    let pad_left = (640 - scaled_w) / 2;
    let pad_top = (384 - scaled_h) / 2;
    let resized = image::imageops::resize(
        image,
        scaled_w,
        scaled_h,
        image::imageops::FilterType::Triangle,
    );
    let mut canvas = vec![114u8; 640 * 384 * 3];
    for y in 0..scaled_h as usize {
        let src = &resized.as_raw()[y * scaled_w as usize * 3..(y + 1) * scaled_w as usize * 3];
        let dst_offset = ((y + pad_top as usize) * 640 + pad_left as usize) * 3;
        canvas[dst_offset..dst_offset + src.len()].copy_from_slice(src);
    }
    Ok((
        canvas,
        PreprocessMode::Letterbox(LetterboxLayout {
            scale,
            pad_left,
            pad_top,
            dst_w: 640,
            dst_h: 384,
            scaled_w,
            scaled_h,
        }),
    ))
}

#[cfg(target_os = "macos")]
pub fn normalize_embedding(values: &[f32]) -> Result<[f32; 512], AlgoError> {
    if values.len() < 512 {
        return Err(AlgoError::Inference {
            reason: format!("EdgeFace 输出维度不足: {} < 512", values.len()),
        });
    }
    let norm = values[..512]
        .iter()
        .map(|value| (*value as f64) * (*value as f64))
        .sum::<f64>()
        .sqrt();
    if !norm.is_finite() || norm <= f64::EPSILON {
        return Err(AlgoError::Inference {
            reason: "EdgeFace embedding L2 范数无效".to_string(),
        });
    }
    let mut embedding = [0.0f32; 512];
    for (target, source) in embedding.iter_mut().zip(values.iter().take(512)) {
        *target = (*source as f64 / norm) as f32;
    }
    Ok(embedding)
}

#[cfg(target_os = "macos")]
fn encode_aligned_jpeg(rgb: &[u8]) -> Result<Vec<u8>, AlgoError> {
    if rgb.len() != 112 * 112 * 3 {
        return Err(AlgoError::Preprocess {
            reason: "对齐人脸尺寸不是 112x112 RGB".to_string(),
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

#[cfg(target_os = "macos")]
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

#[cfg(target_os = "macos")]
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

#[cfg(target_os = "macos")]
unsafe fn extract_face_impl(
    lib: AvAlgoLibrary,
    input: *const AvFaceExtractInput,
    output: *mut AvFaceExtractOutput,
) -> c_int {
    if lib.is_null() || input.is_null() || output.is_null() {
        return AV_ERR_INVALID_ARG;
    }
    if let Err(error) = unsafe {
        // SAFETY: input 指针非空且调用方 ABI 契约要求至少可读取完整头部。
        validate_abi_header(input, "AvFaceExtractInput")
    } {
        return error.to_c_status();
    }
    if let Err(error) = unsafe {
        // SAFETY: output 指针非空且调用方 ABI 契约要求至少可读取完整头部。
        validate_abi_header(output, "AvFaceExtractOutput")
    } {
        return error.to_c_status();
    }
    // SAFETY: 两个结构体已通过非空、对齐、size 和 api_version 校验。
    let input_ref = unsafe { &*input };
    // SAFETY: 同上，output 指向调用方提供的完整可写 POD 缓冲区。
    unsafe {
        std::ptr::write_bytes(
            output.cast::<u8>(),
            0,
            std::mem::size_of::<AvFaceExtractOutput>(),
        )
    };
    // SAFETY: 上方已确认 output 覆盖完整结构体。
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
    // SAFETY: C ABI 输入契约保证 image_bytes 指向 image_bytes_len 个只读字节。
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
    let (detector_rgb, detector_mode) = match prepare_detector_input(&image) {
        Ok(value) => value,
        Err(error) => {
            write_output_error(output_ref, &error.to_string(), error.to_c_status());
            return error.to_c_status();
        }
    };

    // SAFETY: lib 是 export_algo!::library_open 返回的 LibraryContext 句柄，且在本次同步调用期间有效。
    let library = unsafe { &*(lib as *const LibraryContext) };
    let models = match shared_models(&library.package_root) {
        Ok(models) => models,
        Err(error) => {
            write_output_error(output_ref, &error.to_string(), AV_ERR_MODEL_LOAD_FAILED);
            return AV_ERR_MODEL_LOAD_FAILED;
        }
    };
    let detector_buffer = match coreml::OwnedPixelBuffer::from_rgb(&detector_rgb, 640, 384) {
        Ok(buffer) => buffer,
        Err(error) => {
            write_output_error(output_ref, &error.to_string(), error.to_c_status());
            return error.to_c_status();
        }
    };
    // SAFETY: detector_buffer 在调用返回前保持有效。
    let raw_output = match unsafe { models.predict_detector(detector_buffer.as_ptr()) } {
        Ok(output) => output,
        Err(error) => {
            write_output_error(output_ref, &error.to_string(), AV_ERR_INFERENCE_FAILED);
            return AV_ERR_INFERENCE_FAILED;
        }
    };
    let min_score = if input_ref.min_detection_score.is_finite() {
        input_ref.min_detection_score.clamp(0.0, 1.0)
    } else {
        0.5
    };
    let mut faces = decode_face_detections(&raw_output, min_score);
    nms(&mut faces, 0.45);
    unmap_letterbox(&mut faces, &detector_mode, image.width(), image.height());
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
    let Some((face, quality)) = faces
        .into_iter()
        .filter_map(|face| {
            let quality = compute_quality(
                &face.landmarks,
                &face.landmark_scores,
                face.bbox[2] * image.width() as f32,
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

    let aligned = match align_face(&image, image.width(), image.height(), &face.landmarks) {
        Ok(aligned) => aligned,
        Err(reason) => {
            let error = AlgoError::Preprocess {
                reason: reason.to_string(),
            };
            write_output_error(output_ref, &error.to_string(), error.to_c_status());
            return error.to_c_status();
        }
    };
    let embedding_values = match models.predict_embedding(&aligned) {
        Ok(values) => values,
        Err(error) => {
            write_output_error(
                output_ref,
                &format!("EMBED_INFERENCE_FAILED: {error}"),
                AV_ERR_INFERENCE_FAILED,
            );
            return AV_ERR_INFERENCE_FAILED;
        }
    };
    let embedding = match normalize_embedding(&embedding_values) {
        Ok(embedding) => embedding,
        Err(error) => {
            write_output_error(
                output_ref,
                &format!("EMBED_NORM_FAILED: {error}"),
                AV_ERR_INFERENCE_FAILED,
            );
            return AV_ERR_INFERENCE_FAILED;
        }
    };
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
        face.bbox,
        quality.score,
        face.score,
        &jpeg,
    );
    AV_OK
}

/// 独立的人脸特征提取 C ABI 符号。
///
/// # Safety
/// `lib` 必须来自本动态库导出的 `library_open`，`input`/`output` 必须分别指向
/// 满足 `AvFaceExtractInput`/`AvFaceExtractOutput` ABI 版本和尺寸约束的有效内存；
/// 输入 JPEG 在调用期间保持只读有效，输出缓冲区由调用方独占可写。
#[no_mangle]
pub unsafe extern "C" fn av_algo_extract_face(
    lib: AvAlgoLibrary,
    input: *const AvFaceExtractInput,
    output: *mut AvFaceExtractOutput,
) -> c_int {
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (lib, input, output);
        return AV_ERR_NOT_IMPLEMENTED;
    }

    #[cfg(target_os = "macos")]
    {
        let result = catch_unwind(AssertUnwindSafe(|| {
            // SAFETY: 本函数的 ABI 调用方契约由 extract_face_impl 校验；内部不会让 panic 穿越 C 栈。
            unsafe { extract_face_impl(lib, input, output) }
        }));
        match result {
            Ok(status) => status,
            Err(_) => {
                if !output.is_null()
                    && (output as usize).is_multiple_of(std::mem::align_of::<AvFaceExtractOutput>())
                {
                    // SAFETY: 只在指针对齐且非空时访问；size 校验失败时不写入调用方缓冲区。
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

        v2[0] = -1.0;
        v2[1] = 0.0;
        assert!((cosine_similarity(&v1, &v2) - (-1.0)).abs() < 1e-6);
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
