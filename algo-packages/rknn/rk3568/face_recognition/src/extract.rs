//! 单图人脸特征提取 C ABI 实现 (`av_algo_extract_face`)。
//!
//! 专用于人脸底库注册、证件照建档与全景底库图片离线提取 512 维特征向量，
//! 与常驻视频流实时推理与抓拍对账彻底解耦（视频流实时识别严禁逆向调用本接口）。

use std::ffi::c_int;
use std::io::Cursor;

use algo_sdk::c_abi::{AvAlgoLibrary, AvFaceExtractInput, AvFaceExtractOutput};
use algo_sdk::c_abi::{
    AV_ALGO_API_VERSION, AV_ERR_INFERENCE_FAILED, AV_ERR_INTERNAL, AV_ERR_INVALID_ARG, AV_OK,
};
use algo_sdk::error::AlgoError;
use algo_sdk::macros::{validate_abi_header, LibraryContext};
use image::{ExtendedColorType, ImageReader, Limits, RgbImage};

use crate::detect::RawFace;
use crate::quality::{FaceQuality, FaceQualityExt as _};
use crate::{align, config, prepare_detector_input_for, quality, shared_models, SharedModels};

const MAX_DECODED_IMAGE_BYTES: u64 = 128 * 1024 * 1024;
/// 注册/离线提取的检测置信度下限。
const REGISTRATION_MIN_SCORE: f32 = 0.30;
/// 注册/离线提取的最小人脸边长（输入图像像素）。
const REGISTRATION_MIN_FACE_SIZE: u32 = 30;
/// 判定输入本身已是「对齐人脸切片」时，人脸高度至少需占图像高度的比例。
const ALIGNED_CHIP_MIN_FACE_RATIO: f32 = 0.6;

struct ExtractCache {
    embedding: [f32; 512],
    aligned_jpeg: Vec<u8>,
}

thread_local! {
    static EXTRACT_CACHE: std::cell::RefCell<ExtractCache> = const {
        std::cell::RefCell::new(ExtractCache {
            embedding: [0.0; 512],
            aligned_jpeg: Vec::new(),
        })
    };
}

fn write_output_error(output: &mut AvFaceExtractOutput, status: c_int) {
    output.status_code = status.unsigned_abs();
    output.embedding = std::ptr::null();
    output.embedding_dim = 0;
    output.aligned_jpeg = std::ptr::null();
    output.aligned_jpeg_len = 0;
}

fn write_output_success(
    output: &mut AvFaceExtractOutput,
    embedding: [f32; 512],
    quality_score: f32,
    detection_score: f32,
    jpeg: Vec<u8>,
) {
    output.status_code = 0;
    output.embedding_dim = 512;
    output.quality_score = quality_score;
    output.detection_score = detection_score;

    EXTRACT_CACHE.with(|cache| {
        let mut cache = cache.borrow_mut();
        cache.embedding = embedding;
        cache.aligned_jpeg = jpeg;
        output.embedding = cache.embedding.as_ptr();
        output.aligned_jpeg = cache.aligned_jpeg.as_ptr();
        output.aligned_jpeg_len = cache.aligned_jpeg.len() as u32;
    });
}

fn encode_aligned_jpeg(rgb: &[u8]) -> Result<Vec<u8>, AlgoError> {
    if rgb.len() != 112 * 112 * 3 {
        return Err(AlgoError::Preprocess {
            reason: "对齐人脸尺寸不是 112×112 RGB".to_string(),
        });
    }
    let mut jpeg = Vec::with_capacity(16 * 1024);
    let mut encoder = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut jpeg, 90);
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

fn decode_input_image(bytes: &[u8]) -> Result<RgbImage, AlgoError> {
    let mut reader = ImageReader::new(Cursor::new(bytes))
        .with_guessed_format()
        .map_err(|error| AlgoError::Preprocess {
            reason: format!("无法识别输入图像格式: {error}"),
        })?;
    let mut limits = Limits::default();
    limits.max_alloc = Some(MAX_DECODED_IMAGE_BYTES);
    reader.limits(limits);
    let decoded = reader.decode().map_err(|error| AlgoError::Preprocess {
        reason: format!("图像解码失败或超过资源上限: {error}"),
    })?;
    let decoded_bytes = u64::from(decoded.width())
        .checked_mul(u64::from(decoded.height()))
        .and_then(|pixels| pixels.checked_mul(3))
        .ok_or(AlgoError::OutOfMemory)?;
    if decoded_bytes > MAX_DECODED_IMAGE_BYTES {
        return Err(AlgoError::OutOfMemory);
    }
    Ok(decoded.to_rgb8())
}

#[inline]
fn reflect_coord(coord: u32, pad: u32, dim: u32) -> usize {
    if coord < pad {
        pad - coord
    } else if coord >= dim + pad {
        let diff = coord - (dim + pad) + 1;
        dim.saturating_sub(diff)
    } else {
        coord - pad
    }
    .min(dim.saturating_sub(1)) as usize
}

fn reflect_pad_image(raw: &[u8], orig_w: u32, orig_h: u32, pad_x: u32, pad_y: u32) -> Vec<u8> {
    let padded_w = orig_w + pad_x * 2;
    let padded_h = orig_h + pad_y * 2;
    let mut padded = vec![0u8; (padded_w * padded_h * 3) as usize];
    let orig_w_usize = orig_w as usize;
    let padded_w_usize = padded_w as usize;

    for y in 0..padded_h {
        let src_y = reflect_coord(y, pad_y, orig_h);
        let src_row = src_y * orig_w_usize;
        let dst_row = y as usize * padded_w_usize;
        for x in 0..padded_w {
            let src_x = reflect_coord(x, pad_x, orig_w);
            let src_idx = (src_row + src_x) * 3;
            let dst_idx = (dst_row + x as usize) * 3;
            padded[dst_idx..dst_idx + 3].copy_from_slice(&raw[src_idx..src_idx + 3]);
        }
    }
    padded
}

/// 融合多组扰动特征向量 (超球面均值融合 Spherical Average Embedding)。
///
/// 将所有特征向量相加并在超球面上重新执行 L2 归一化。
/// 若加和模长退化 (< 1e-6)，回退为首个特征向量。
pub fn fuse_spherical_average_embeddings(
    embeddings: &[[f32; 512]],
) -> Result<[f32; 512], AlgoError> {
    if embeddings.is_empty() {
        return Err(AlgoError::Preprocess {
            reason: "待融合向量列表为空".to_string(),
        });
    }
    let mut sum = [0.0f32; 512];
    for emb in embeddings {
        for i in 0..512 {
            sum[i] += emb[i];
        }
    }
    match crate::normalize_embedding(&sum) {
        Ok(fused) => Ok(fused),
        Err(err) => {
            tracing::warn!(%err, "超球面均值融合归一化失败，回退为原始首项特征");
            Ok(embeddings[0])
        }
    }
}

/// 融合原始人脸特征向量与翻转人脸特征向量 (TTA 双向量融合)。
#[inline]
pub fn fuse_tta_embeddings(
    original: &[f32; 512],
    flipped: &[f32; 512],
) -> Result<[f32; 512], AlgoError> {
    fuse_spherical_average_embeddings(&[*original, *flipped])
}

/// 注册人脸特征提取：执行 5 组几何与色彩扰动测试时增强 (TTA)，并在超球面上做球面均值融合。
///
/// 1. 原图对齐人脸 `e_orig` (1.0 基础尺度)
/// 2. 水平微翻转 `e_flip` (左右镜像不变性)
/// 3. 高光亮度微调 `e_bright` (+12% 增益，模拟日光/过曝场景)
/// 4. 暗光亮度微调 `e_dark` (-12% 衰减，模拟阴影/弱光场景)
/// 5. 0.95 多尺度中心微裁切重采样 `e_scale` (近景/瞳距尺度容差)
///
/// 对上述有效特征在 512 维单位超球面上做 Spherical Average Embedding 融合存库。
fn extract_embedding_with_registration_tta(
    worker: &crate::InferenceWorker,
    aligned: &[u8],
    debug_tag: &str,
    quality_score: f32,
) -> Result<[f32; 512], AlgoError> {
    let orig_embedding = worker.embed_host(aligned.to_vec())?;

    if std::env::var("HEIMDALL_DISABLE_REGISTRATION_TTA")
        .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
        .unwrap_or(false)
    {
        return Ok(orig_embedding);
    }

    let mut embeddings = Vec::with_capacity(5);
    embeddings.push(orig_embedding);

    // 1. 水平翻转 (Horizontal Flip)
    if let Ok(flipped) = align::flip_horizontal_112(aligned) {
        align::dump_debug_aligned_face(&format!("{debug_tag}_tta_flip"), &flipped, quality_score);
        if let Ok(emb) = worker.embed_host(flipped) {
            embeddings.push(emb);
        }
    }

    // 2. 亮度高光微调 (+12%)
    let bright = align::adjust_brightness_112(aligned, 1.12);
    if let Ok(emb) = worker.embed_host(bright) {
        embeddings.push(emb);
    }

    // 3. 亮度暗光微调 (-12%)
    let dark = align::adjust_brightness_112(aligned, 0.88);
    if let Ok(emb) = worker.embed_host(dark) {
        embeddings.push(emb);
    }

    // 4. 0.95 多尺度中心微裁切重采样
    let scaled = align::crop_and_resize_chip_112(aligned, 0.95);
    if let Ok(emb) = worker.embed_host(scaled) {
        embeddings.push(emb);
    }

    fuse_spherical_average_embeddings(&embeddings)
}

unsafe fn extract_face_impl(
    lib: AvAlgoLibrary,
    input: *const AvFaceExtractInput,
    output: *mut AvFaceExtractOutput,
) -> c_int {
    if lib.is_null() || input.is_null() || output.is_null() {
        return AV_ERR_INVALID_ARG;
    }
    // SAFETY: validate_abi_header 只读取 input 指向的 ABI 头；非空和对齐由该函数校验。
    if let Err(error) = unsafe { validate_abi_header(input, "AvFaceExtractInput") } {
        return error.to_c_status();
    }
    // SAFETY: validate_abi_header 只读取 output 指向的 ABI 头；非空和对齐由该函数校验。
    if let Err(error) = unsafe { validate_abi_header(output, "AvFaceExtractOutput") } {
        return error.to_c_status();
    }
    // SAFETY: validate_abi_header 校验通过保证 input 指针非空、内存对齐且布局合法。
    let input_ref = unsafe { &*input };
    // SAFETY: output 指针非空且指向满足 AvFaceExtractOutput 尺寸的合法可写内存。
    unsafe {
        std::ptr::write_bytes(
            output.cast::<u8>(),
            0,
            std::mem::size_of::<AvFaceExtractOutput>(),
        );
    }
    // SAFETY: output_ref 在写入零字节后初始化，且 validate_abi_header 已验证对齐与尺寸。
    let output_ref = unsafe { &mut *output };
    output_ref.size = std::mem::size_of::<AvFaceExtractOutput>() as u32;
    output_ref.api_version = AV_ALGO_API_VERSION;

    if input_ref.image_bytes.is_null() || input_ref.image_bytes_len == 0 {
        algo_sdk::macros::set_last_error("image_bytes 为空");
        write_output_error(output_ref, AV_ERR_INVALID_ARG);
        return AV_ERR_INVALID_ARG;
    }
    let image_len = input_ref.image_bytes_len as usize;
    if image_len > 32 * 1024 * 1024 {
        algo_sdk::macros::set_last_error("压缩图像输入超过 32 MiB 限制");
        write_output_error(output_ref, AV_ERR_INVALID_ARG);
        return AV_ERR_INVALID_ARG;
    }
    // SAFETY: input 结构体中的 image_bytes 指向至少 image_len 字节的有效图像字节流。
    let image_bytes = unsafe { std::slice::from_raw_parts(input_ref.image_bytes, image_len) };
    let image = match decode_input_image(image_bytes) {
        Ok(image) => image,
        Err(error) => {
            algo_sdk::macros::set_last_error(error.to_string());
            write_output_error(output_ref, error.to_c_status());
            return error.to_c_status();
        }
    };

    // SAFETY: lib 必须为库导出接口所生成的非空 LibraryContext 不透明句柄。
    let library = unsafe { &*(lib as *const LibraryContext) };
    let models = match shared_models(&library.package_root) {
        Ok(models) => models,
        Err(error) => {
            let status = error.to_c_status();
            algo_sdk::macros::set_last_error(error.to_string());
            write_output_error(output_ref, status);
            return status;
        }
    };

    let (orig_w, orig_h) = (image.width(), image.height());

    // 检测与质量门禁先于任何分支：112×112 直通同样必须经过真实检测。
    // 历史缺陷：直通分支不做检测却上报 quality/detection = 1.0，使宿主侧唯一的质量门禁
    // （capture/注册链路 `quality_score >= 0.50`）被无条件穿透，任意 112×112 图片即可注册为模板。
    let (best_face, face_quality) = match detect_best_face(&image, &models) {
        Ok(Some(found)) => found,
        Ok(None) => {
            algo_sdk::macros::set_last_error(
                "NO_FACE_DETECTED: 输入图像未检出满足置信度与质量阈值的人脸",
            );
            write_output_error(output_ref, AV_ERR_INFERENCE_FAILED);
            return AV_ERR_INFERENCE_FAILED;
        }
        Err(error) => {
            let status = error.to_c_status();
            algo_sdk::macros::set_last_error(error.to_string());
            write_output_error(output_ref, status);
            return status;
        }
    };

    let aligned = if is_aligned_face_chip(orig_w, orig_h, &best_face) {
        // 输入本身就是一张紧凑对齐人脸：直通以保留原始像素，不做二次仿射重采样。
        // 对齐图与正常路径的预处理必须一致（对齐函数内部同样执行这两步）。
        let mut chip = image.as_raw().to_vec();
        align::normalize_illumination_inplace(&mut chip);
        align::enhance_face_details_inplace(&mut chip);
        align::dump_debug_aligned_face("extract_chip112", &chip, face_quality.score);
        chip
    } else {
        let is_compact_crop = (orig_w < 320 || orig_h < 320)
            && (best_face.width() * orig_w as f32 > 0.6 * orig_w as f32);

        let align_result = if is_compact_crop {
            let pad_x = (orig_w as f32 * 0.25).round() as u32;
            let pad_y = (orig_h as f32 * 0.25).round() as u32;
            let padded_w = orig_w + pad_x * 2;
            let padded_h = orig_h + pad_y * 2;
            let padded_image = reflect_pad_image(image.as_raw(), orig_w, orig_h, pad_x, pad_y);

            let mut padded_landmarks = [[0.0f32; 2]; 5];
            for (dst, src) in padded_landmarks.iter_mut().zip(&best_face.landmarks) {
                dst[0] = src[0] * orig_w as f32 + pad_x as f32;
                dst[1] = src[1] * orig_h as f32 + pad_y as f32;
            }
            align::align_face(&padded_image, padded_w, padded_h, &padded_landmarks)
        } else {
            align::align_face(image.as_raw(), orig_w, orig_h, &best_face.landmarks)
        };

        match align_result {
            Ok(aligned) => {
                align::dump_debug_aligned_face("extract_face", &aligned, face_quality.score);
                aligned
            }
            Err(error) => {
                algo_sdk::macros::set_last_error(error.to_string());
                write_output_error(output_ref, error.to_c_status());
                return error.to_c_status();
            }
        }
    };
    let embedding = match extract_embedding_with_registration_tta(
        &models.worker,
        &aligned,
        "extract_face",
        face_quality.score,
    ) {
        Ok(embedding) => embedding,
        Err(error) => {
            let status = error.to_c_status();
            algo_sdk::macros::set_last_error(format!("EMBED_INFERENCE_FAILED: {error}"));
            write_output_error(output_ref, status);
            return status;
        }
    };
    let jpeg = match encode_aligned_jpeg(&aligned) {
        Ok(jpeg) => jpeg,
        Err(error) => {
            algo_sdk::macros::set_last_error(error.to_string());
            write_output_error(output_ref, error.to_c_status());
            return error.to_c_status();
        }
    };
    write_output_success(
        output_ref,
        embedding,
        face_quality.score,
        best_face.score,
        jpeg,
    );
    AV_OK
}

/// 在单图上执行人脸检测并返回通过置信度与质量门控的最佳人脸。
///
/// 注册检测优先使用清单解析出的 640×640 专用会话，缺失时回退视频流检测器。
/// 返回 `Ok(None)` 表示「检测成功但无合格人脸」，与检测链路错误严格区分。
fn detect_best_face(
    image: &RgbImage,
    models: &SharedModels,
) -> Result<Option<(RawFace, FaceQuality)>, AlgoError> {
    let (det_w, det_h) = if models.has_registration_detector {
        (
            models.registration_detector_width,
            models.registration_detector_height,
        )
    } else {
        (models.detector_width, models.detector_height)
    };
    let (detector_rgb, layout) = prepare_detector_input_for(image, det_w, det_h)?;
    let raw_faces =
        models
            .worker
            .detect_registration_host(detector_rgb, layout, REGISTRATION_MIN_SCORE)?;

    let (orig_w, orig_h) = (image.width(), image.height());
    let thresholds = config::QualityThresholds {
        min_score: REGISTRATION_MIN_SCORE,
        ..config::QualityThresholds::default()
    };
    Ok(raw_faces
        .into_iter()
        .filter_map(|face| {
            let face_width = face.width() * orig_w as f32;
            let face_height = face.height() * orig_h as f32;
            let quality = quality::compute_quality(
                &face.landmarks,
                &face.landmark_scores,
                face_width.min(face_height),
                &thresholds,
            );
            quality
                .accepted(&thresholds, REGISTRATION_MIN_FACE_SIZE)
                .then_some((face, quality))
        })
        .max_by(|left, right| left.0.score.total_cmp(&right.0.score)))
}

/// 输入本身是否已是一张标准对齐人脸切片（可直接直通 EdgeFace）。
///
/// 判定必须依赖**真实检测结果**：仅凭「输入尺寸 = 112×112」就直通等价于跳过检测，
/// 任意同尺寸图片都会拿到一个看似合理却无意义的模板。这里额外要求检出人脸
/// 高度占图像高度至少 `ALIGNED_CHIP_MIN_FACE_RATIO`，即画面确实被人脸充满。
fn is_aligned_face_chip(width: u32, height: u32, face: &RawFace) -> bool {
    width == align::ALIGNED_SIZE
        && height == align::ALIGNED_SIZE
        && face.height() >= ALIGNED_CHIP_MIN_FACE_RATIO
}

/// 独立的人脸特征提取 C ABI 符号。
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
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        // SAFETY: extract_face_impl 在进入业务逻辑前验证 ABI，内部不让 panic 穿越 C 栈。
        unsafe { extract_face_impl(lib, input, output) }
    }));
    result.unwrap_or(AV_ERR_INTERNAL)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn face_with_height(height: f32) -> RawFace {
        RawFace {
            bbox: [0.05, 0.05, 0.90, height],
            landmarks: [[0.0; 2]; 5],
            landmark_scores: [0.9; 5],
            score: 0.9,
        }
    }

    /// 112×112 直通必须同时满足「尺寸相符」与「检出的人脸确实充满画面」。
    ///
    /// 回归缺陷：早期实现只看输入尺寸就直通并上报 1.0 分，使任意 112×112 图片
    /// 都能绕过检测与质量门禁入库。
    #[test]
    fn aligned_face_chip_requires_detected_large_face() {
        assert!(is_aligned_face_chip(112, 112, &face_with_height(0.90)));
        assert!(is_aligned_face_chip(112, 112, &face_with_height(0.60)));
        // 人脸占比不足：不是对齐切片，必须回到「检测框 + 五点仿射对齐」路径。
        assert!(!is_aligned_face_chip(112, 112, &face_with_height(0.59)));
        // 尺寸不符：EdgeFace 只接受 112×112×3，永远不直通。
        assert!(!is_aligned_face_chip(224, 224, &face_with_height(0.90)));
        assert!(!is_aligned_face_chip(112, 108, &face_with_height(0.90)));
    }

    #[test]
    fn extract_face_rejects_null_abi_pointers() {
        // SAFETY: 测试传入空指针验证 C ABI 防御性参数检查与错误返回码。
        let status = unsafe {
            av_algo_extract_face(std::ptr::null_mut(), std::ptr::null(), std::ptr::null_mut())
        };
        assert_eq!(status, AV_ERR_INVALID_ARG);
    }

    #[test]
    fn test_fuse_tta_embeddings_identical() {
        let mut original = [0.0f32; 512];
        original[0] = 1.0;
        let flipped = original;

        let fused = fuse_tta_embeddings(&original, &flipped).expect("融合应成功");
        assert!((fused[0] - 1.0).abs() < 1e-6);
        let l2_sq: f32 = fused.iter().map(|v| v * v).sum();
        assert!((l2_sq.sqrt() - 1.0).abs() < 1e-6);
    }

    #[test]
    fn test_fuse_tta_embeddings_orthogonal() {
        let mut v1 = [0.0f32; 512];
        let mut v2 = [0.0f32; 512];
        v1[0] = 1.0;
        v2[1] = 1.0;

        let fused = fuse_tta_embeddings(&v1, &v2).expect("正交融合应成功");
        let expected = 1.0 / 2.0f32.sqrt();
        assert!((fused[0] - expected).abs() < 1e-6);
        assert!((fused[1] - expected).abs() < 1e-6);
        let l2_sq: f32 = fused.iter().map(|v| v * v).sum();
        assert!((l2_sq.sqrt() - 1.0).abs() < 1e-6);
    }

    #[test]
    fn test_fuse_tta_embeddings_opposite_fallback() {
        let mut v1 = [0.0f32; 512];
        let mut v2 = [0.0f32; 512];
        v1[0] = 1.0;
        v2[0] = -1.0;

        // 相反向量相加为零，应优雅回退至原始特征
        let fused = fuse_tta_embeddings(&v1, &v2).expect("相反向量应回退");
        assert_eq!(fused, v1);
    }

    #[test]
    fn test_fuse_spherical_average_embeddings_multi() {
        let mut v1 = [0.0f32; 512];
        let mut v2 = [0.0f32; 512];
        let mut v3 = [0.0f32; 512];
        v1[0] = 1.0;
        v2[0] = 1.0;
        v3[0] = 1.0;

        let fused = fuse_spherical_average_embeddings(&[v1, v2, v3]).expect("多向量融合应成功");
        assert!((fused[0] - 1.0).abs() < 1e-6);

        // 验证空切片错误保护
        assert!(fuse_spherical_average_embeddings(&[]).is_err());
    }
}
