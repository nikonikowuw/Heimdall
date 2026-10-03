//! Rockchip RK3568 人脸检测与 EdgeFace 特征提取算法包。
//!
//! 集成 SCRFD 人脸检测器、YOLOv8n 人体检测器、ByteTrack 航迹追踪与 EdgeFace 512 维特征提取与时域超球面融合。
//! RKNN context 常驻独立 OS worker，避免在宿主调用线程之间迁移硬件会话。

pub mod align;
pub mod association;
pub mod best_shot;
pub mod bytetrack;
pub mod config;
pub mod detect;
pub mod extract;
pub mod manifest;
pub mod plugin;
pub mod postprocess;
pub mod quality;
pub mod rknn;
pub mod template_pool;
pub mod worker;

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock, Weak};

use algo_sdk::cv::{compute_letterbox_layout, LetterboxLayout};
use algo_sdk::error::AlgoError;
use algo_sdk::plugin::AlgoPlugin;
use algo_sdk::{export_algo, export_face_gallery};
use image::RgbImage;

pub use extract::{av_algo_extract_face, fuse_spherical_average_embeddings, fuse_tta_embeddings};
use manifest::LoadedPackage;
use plugin::FaceRecognizer;
pub use rknn::{RknnModelContract, RknnRootWeight, RknnRuntime};
pub use worker::{FaceSessions, InferenceWorker, WorkerQueueStats, WorkerSessions};

const MAX_PREPROCESS_BYTES: usize = 128 * 1024 * 1024;

static SHARED_MODELS: OnceLock<Mutex<HashMap<PathBuf, Weak<SharedModels>>>> = OnceLock::new();
static SHARED_ROOTS: OnceLock<Mutex<HashMap<PathBuf, Weak<SharedModelRoots>>>> = OnceLock::new();

fn shared_model_registry() -> &'static Mutex<HashMap<PathBuf, Weak<SharedModels>>> {
    SHARED_MODELS.get_or_init(|| Mutex::new(HashMap::new()))
}

fn shared_roots_registry() -> &'static Mutex<HashMap<PathBuf, Weak<SharedModelRoots>>> {
    SHARED_ROOTS.get_or_init(|| Mutex::new(HashMap::new()))
}

/// 已验证 manifest 身份与模型输入输出契约的共享根权重集合（路线 A）。
#[derive(Debug)]
pub struct SharedModelRoots {
    pub detector: Arc<RknnRootWeight>,
    pub embedder: Arc<RknnRootWeight>,
    pub person_detector: Option<Arc<RknnRootWeight>>,
    pub registration_detector: Option<Arc<RknnRootWeight>>,
    pub detector_width: u32,
    pub detector_height: u32,
    pub registration_detector_width: u32,
    pub registration_detector_height: u32,
    pub has_registration_detector: bool,
    pub embedder_width: u32,
    pub embedder_height: u32,
}

impl SharedModelRoots {
    /// 基于目标核心掩码为独立实例派生子会话。
    pub fn create_face_sessions(&self, core_mask: Option<u32>) -> Result<FaceSessions, AlgoError> {
        let detector = self.detector.create_child_session(core_mask)?;
        let embedder = self.embedder.create_child_session(core_mask)?;
        let person_detector = self
            .person_detector
            .as_ref()
            .map(|r| r.create_child_session(core_mask))
            .transpose()?;
        let registration_detector = self
            .registration_detector
            .as_ref()
            .map(|r| r.create_child_session(core_mask))
            .transpose()?;

        Ok(FaceSessions::new(
            detector,
            embedder,
            person_detector,
            registration_detector,
        ))
    }
}

pub fn shared_model_roots(package_root: &Path) -> Result<Arc<SharedModelRoots>, AlgoError> {
    let key = package_root
        .canonicalize()
        .map_err(|error| AlgoError::ModelLoad {
            reason: format!("算法包根目录无法规范化 ({package_root:?}): {error}"),
        })?;
    {
        let registry = shared_roots_registry()
            .lock()
            .map_err(|_| AlgoError::Internal {
                reason: "RKNN 模型根权重 registry 锁已中毒".to_string(),
            })?;
        if let Some(roots) = registry.get(&key).and_then(Weak::upgrade) {
            return Ok(roots);
        }
    }

    let package = LoadedPackage::load(&key)?;
    let runtime = RknnRuntime::load(&package.root)?;

    let detector_contract = RknnModelContract {
        input_width: package.detector_width,
        input_height: package.detector_height,
        input_channels: 3,
        output_shapes: manifest::DETECTOR_OUTPUT_SHAPES.to_vec(),
    };
    let embedder_contract = RknnModelContract {
        input_width: package.embedder_width,
        input_height: package.embedder_height,
        input_channels: 3,
        output_shapes: vec![[1, 512, 1, 1]],
    };
    let is_yolov6 = package
        .person_detector_path
        .file_name()
        .and_then(|name| name.to_str())
        .map(|name| name.contains("yolov6"))
        .unwrap_or(false);

    let person_detector_contract = RknnModelContract {
        input_width: package.detector_width,
        input_height: package.detector_height,
        input_channels: 3,
        output_shapes: if is_yolov6 {
            manifest::YOLOV6_PERSON_DETECTOR_OUTPUT_SHAPES.to_vec()
        } else {
            manifest::PERSON_DETECTOR_OUTPUT_SHAPES.to_vec()
        },
    };
    let registration_detector_contract = RknnModelContract {
        input_width: package.registration_detector_width,
        input_height: package.registration_detector_height,
        input_channels: 3,
        output_shapes: manifest::DETECTOR_640X640_OUTPUT_SHAPES.to_vec(),
    };

    let detector = RknnRootWeight::load(
        Arc::clone(&runtime),
        &package.detector_path,
        detector_contract,
    )?;
    let embedder = RknnRootWeight::load(
        Arc::clone(&runtime),
        &package.embedder_path,
        embedder_contract,
    )?;

    let registration_detector = if package.registration_detector_path.is_file() {
        match RknnRootWeight::load(
            Arc::clone(&runtime),
            &package.registration_detector_path,
            registration_detector_contract,
        ) {
            Ok(root) => Some(root),
            Err(e) => {
                tracing::warn!(%e, "可选 640x640 人脸注册根模型初始化失败，回退使用 640x384");
                None
            }
        }
    } else {
        None
    };

    let person_detector = if package.person_detector_path.is_file() {
        match RknnRootWeight::load(
            Arc::clone(&runtime),
            &package.person_detector_path,
            person_detector_contract,
        ) {
            Ok(root) => Some(root),
            Err(e) => {
                tracing::warn!(%e, "可选人体检测根模型初始化失败，使用纯人脸推导");
                None
            }
        }
    } else {
        None
    };

    let roots = Arc::new(SharedModelRoots {
        detector,
        embedder,
        has_registration_detector: registration_detector.is_some(),
        registration_detector,
        person_detector,
        detector_width: package.detector_width,
        detector_height: package.detector_height,
        registration_detector_width: package.registration_detector_width,
        registration_detector_height: package.registration_detector_height,
        embedder_width: package.embedder_width,
        embedder_height: package.embedder_height,
    });

    let mut registry = shared_roots_registry()
        .lock()
        .map_err(|_| AlgoError::Internal {
            reason: "RKNN 模型根权重 registry 锁已中毒".to_string(),
        })?;
    if let Some(existing) = registry.get(&key).and_then(Weak::upgrade) {
        Ok(existing)
    } else {
        registry.insert(key, Arc::downgrade(&roots));
        Ok(roots)
    }
}

/// 已验证 manifest 身份与模型输入输出契约的共享 worker（兼容层）。
#[derive(Debug)]
pub struct SharedModels {
    pub roots: Arc<SharedModelRoots>,
    pub worker: Arc<InferenceWorker>,
    pub detector_width: u32,
    pub detector_height: u32,
    pub registration_detector_width: u32,
    pub registration_detector_height: u32,
    pub has_registration_detector: bool,
    pub embedder_width: u32,
    pub embedder_height: u32,
}

pub fn shared_models(package_root: &Path) -> Result<Arc<SharedModels>, AlgoError> {
    let key = package_root
        .canonicalize()
        .map_err(|error| AlgoError::ModelLoad {
            reason: format!("算法包根目录无法规范化 ({package_root:?}): {error}"),
        })?;
    {
        let registry = shared_model_registry()
            .lock()
            .map_err(|_| AlgoError::Internal {
                reason: "RKNN 模型 registry 锁已中毒".to_string(),
            })?;
        if let Some(models) = registry.get(&key).and_then(Weak::upgrade) {
            return Ok(models);
        }
    }

    let roots = shared_model_roots(package_root)?;
    let sessions = roots.create_face_sessions(None)?;
    let worker = Arc::new(InferenceWorker::from_sessions(sessions));

    let models = Arc::new(SharedModels {
        roots: Arc::clone(&roots),
        worker,
        detector_width: roots.detector_width,
        detector_height: roots.detector_height,
        registration_detector_width: roots.registration_detector_width,
        registration_detector_height: roots.registration_detector_height,
        has_registration_detector: roots.has_registration_detector,
        embedder_width: roots.embedder_width,
        embedder_height: roots.embedder_height,
    });

    let mut registry = shared_model_registry()
        .lock()
        .map_err(|_| AlgoError::Internal {
            reason: "RKNN 模型 registry 锁已中毒".to_string(),
        })?;
    if let Some(existing) = registry.get(&key).and_then(Weak::upgrade) {
        Ok(existing)
    } else {
        registry.insert(key, Arc::downgrade(&models));
        Ok(models)
    }
}

pub(crate) fn close_shared_models(_package_root: &Path) {
    if let Some(registry) = SHARED_MODELS.get() {
        if let Ok(mut registry) = registry.lock() {
            registry.retain(|_, weak| weak.strong_count() != 0);
        }
    }
}

export_algo!(
    FaceRecognizer,
    algo_id: "face_recognition",
    version: "1.0.0",
    algo_type: "face_recognition",
    alarm_type_id: "face_recognize",
    library_open_hook: algo_sdk::macros::noop_library_open,
    library_close_hook: crate::close_shared_models
);

export_face_gallery!();

/// 计算两个 512D 特征向量之间的余弦相似度。
#[inline]
pub fn cosine_similarity(a: &[f32; 512], b: &[f32; 512]) -> f32 {
    algo_sdk::math::cosine_similarity(a, b)
}

/// L2 归一化 embedding。输出长度必须严格匹配 manifest 的 512D 契约。
pub fn normalize_embedding(values: &[f32]) -> Result<[f32; 512], AlgoError> {
    if values.len() != 512 {
        return Err(AlgoError::Inference {
            reason: format!(
                "EdgeFace 输出维度非法: expected=512, actual={}",
                values.len()
            ),
        });
    }
    let norm = algo_sdk::math::l2_normalize(values).map_err(|e| AlgoError::Inference {
        reason: format!("EdgeFace embedding L2 归一化失败: {e}"),
    })?;
    let mut out = [0.0f32; 512];
    out.copy_from_slice(&norm);
    Ok(out)
}

/// 使用 manifest 指定的尺寸执行 Host 侧低频 detector letterbox。
pub fn prepare_detector_input_for(
    image: &RgbImage,
    dst_w: u32,
    dst_h: u32,
) -> Result<(Vec<u8>, LetterboxLayout), AlgoError> {
    if image.width() == 0 || image.height() == 0 || dst_w == 0 || dst_h == 0 {
        return Err(AlgoError::Preprocess {
            reason: "detector 输入图像和目标尺寸不能为 0".to_string(),
        });
    }
    let layout = compute_letterbox_layout(image.width(), image.height(), dst_w, dst_h);
    let canvas_len = usize::try_from(dst_w)
        .ok()
        .and_then(|w| usize::try_from(dst_h).ok().and_then(|h| w.checked_mul(h)))
        .and_then(|pixels| pixels.checked_mul(3))
        .ok_or(AlgoError::OutOfMemory)?;
    if canvas_len > MAX_PREPROCESS_BYTES {
        return Err(AlgoError::OutOfMemory);
    }
    let resized = image::imageops::resize(
        image,
        layout.scaled_w,
        layout.scaled_h,
        image::imageops::FilterType::Triangle,
    );
    let mut canvas = vec![114u8; canvas_len];
    let dst_width = dst_w as usize;
    let scaled_width = layout.scaled_w as usize;
    let row_len = scaled_width.checked_mul(3).ok_or(AlgoError::OutOfMemory)?;
    for y in 0..layout.scaled_h as usize {
        let src_offset = y.checked_mul(row_len).ok_or(AlgoError::OutOfMemory)?;
        let src_end = src_offset
            .checked_add(row_len)
            .ok_or(AlgoError::OutOfMemory)?;
        let dst_offset = (y + layout.pad_top as usize)
            .checked_mul(dst_width)
            .and_then(|offset| offset.checked_add(layout.pad_left as usize))
            .and_then(|offset| offset.checked_mul(3))
            .ok_or(AlgoError::OutOfMemory)?;
        let dst_end = dst_offset
            .checked_add(row_len)
            .ok_or(AlgoError::OutOfMemory)?;
        if src_end > resized.as_raw().len() || dst_end > canvas.len() {
            return Err(AlgoError::Preprocess {
                reason: "detector letterbox 行范围越界".to_string(),
            });
        }
        canvas[dst_offset..dst_end].copy_from_slice(&resized.as_raw()[src_offset..src_end]);
    }
    Ok((canvas, layout))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cosine_similarity_properties() {
        let mut first = [0.0f32; 512];
        let mut second = [0.0f32; 512];
        first[0] = 1.0;
        second[0] = 1.0;
        assert!((cosine_similarity(&first, &second) - 1.0).abs() < 1e-6);
        second[0] = 0.0;
        second[1] = 1.0;
        assert!(cosine_similarity(&first, &second).abs() < 1e-6);
        let mut scaled = [0.0f32; 512];
        scaled[0] = 3.0;
        assert!((cosine_similarity(&scaled, &first) - 1.0).abs() < 1e-6);
    }

    #[test]
    fn test_normalize_embedding_unit_length_and_dimension() {
        let input = vec![1.0f32; 512];
        let normalized = normalize_embedding(&input).expect("512D 归一化应成功");
        let l2_sq: f32 = normalized.iter().map(|value| value * value).sum();
        assert!((l2_sq.sqrt() - 1.0).abs() < 1e-5);
        assert!(normalize_embedding(&[1.0f32; 513]).is_err());
    }
}
