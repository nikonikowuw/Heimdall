//! 算法包 manifest 与模型路径解析契约。
//!
//! 遵循女娲规范：模型属于平台专属权重资产，不强行绑定在 manifest.json 内。
//! 解析顺序：package_root/.env 指定路径 -> 约定的固定模型文件路径（model/*.rknn）。

use std::path::{Path, PathBuf};

use serde::Deserialize;

use algo_sdk::error::AlgoError;

const MAX_MANIFEST_BYTES: u64 = 1024 * 1024;
const MAX_MODEL_BYTES: u64 = 256 * 1024 * 1024;

pub const DETECTOR_OUTPUT_SHAPES: [[u32; 4]; 12] = [
    [1, 64, 48, 80],
    [1, 1, 48, 80],
    [1, 1, 48, 80],
    [1, 15, 48, 80],
    [1, 64, 24, 40],
    [1, 1, 24, 40],
    [1, 1, 24, 40],
    [1, 15, 24, 40],
    [1, 64, 12, 20],
    [1, 1, 12, 20],
    [1, 1, 12, 20],
    [1, 15, 12, 20],
];

pub const DETECTOR_640X640_OUTPUT_SHAPES: [[u32; 4]; 12] = [
    [1, 64, 80, 80],
    [1, 1, 80, 80],
    [1, 1, 80, 80],
    [1, 15, 80, 80],
    [1, 64, 40, 40],
    [1, 1, 40, 40],
    [1, 1, 40, 40],
    [1, 15, 40, 40],
    [1, 64, 20, 20],
    [1, 1, 20, 20],
    [1, 1, 20, 20],
    [1, 15, 20, 20],
];

pub const PERSON_DETECTOR_OUTPUT_SHAPES: [[u32; 4]; 9] = [
    [1, 64, 48, 80],
    [1, 80, 48, 80],
    [1, 1, 48, 80],
    [1, 64, 24, 40],
    [1, 80, 24, 40],
    [1, 1, 24, 40],
    [1, 64, 12, 20],
    [1, 80, 12, 20],
    [1, 1, 12, 20],
];

#[derive(Debug, Clone, Deserialize)]
pub struct PackageManifest {
    pub manifest_version: u32,
    pub algorithm_id: String,
    pub platform_id: String,
    #[serde(default)]
    pub runtime_constraints: Option<RuntimeConstraints>,
    #[serde(default)]
    pub self_test: Option<SelfTestManifest>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct RuntimeConstraints {
    #[serde(alias = "requiresRknnRuntime")]
    pub requires_rknn_runtime: bool,
    #[serde(alias = "minRknnrtVersion")]
    pub min_rknnrt_version: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SelfTestManifest {
    #[serde(alias = "timeoutMs")]
    pub timeout_ms: u64,
    #[serde(alias = "inputMode")]
    pub input_mode: String,
}

#[derive(Debug, Clone)]
pub struct LoadedPackage {
    pub root: PathBuf,
    pub person_detector_path: PathBuf,
    pub detector_path: PathBuf,
    pub registration_detector_path: PathBuf,
    pub embedder_path: PathBuf,
    pub detector_width: u32,
    pub detector_height: u32,
    pub registration_detector_width: u32,
    pub registration_detector_height: u32,
    pub embedder_width: u32,
    pub embedder_height: u32,
    pub embedding_dimension: u32,
}

impl LoadedPackage {
    pub fn load(package_root: &Path) -> Result<Self, AlgoError> {
        let root = package_root
            .canonicalize()
            .map_err(|error| AlgoError::ModelLoad {
                reason: format!("规范化算法包根目录失败 ({package_root:?}): {error}"),
            })?;
        let manifest_path = root.join("manifest.json");
        let manifest_meta =
            std::fs::metadata(&manifest_path).map_err(|error| AlgoError::ModelLoad {
                reason: format!("读取算法包 manifest 元数据失败 ({manifest_path:?}): {error}"),
            })?;
        if !manifest_meta.is_file() || manifest_meta.len() > MAX_MANIFEST_BYTES {
            return Err(AlgoError::ModelLoad {
                reason: format!("manifest.json 不是有效的小型普通文件: {manifest_path:?}"),
            });
        }
        let manifest_bytes =
            std::fs::read(&manifest_path).map_err(|error| AlgoError::ModelLoad {
                reason: format!("读取算法包 manifest 失败 ({manifest_path:?}): {error}"),
            })?;
        let manifest: PackageManifest =
            serde_json::from_slice(&manifest_bytes).map_err(|error| AlgoError::ModelLoad {
                reason: format!("解析算法包 manifest 失败: {error}"),
            })?;

        if manifest.manifest_version != 1
            || manifest.algorithm_id != "face_recognition"
            || !matches!(
                manifest.platform_id.as_str(),
                "linux-rknn" | "rknn-rk3568" | "rknn-rk3576"
            )
        {
            return Err(AlgoError::ModelLoad {
                reason: format!(
                    "manifest 身份不匹配: version={}, algorithm_id={}, platform_id={}",
                    manifest.manifest_version, manifest.algorithm_id, manifest.platform_id
                ),
            });
        }

        let env = algo_sdk::env::PackageEnv::load(&root);

        let person_detector_path = env.resolve_model_path(
            &root,
            "PERSON_DETECTOR_MODEL_PATH",
            "model/yolov8n-640x384-rk3568.rknn",
        )?;
        let detector_path = env.resolve_model_path(
            &root,
            "DETECTOR_MODEL_PATH",
            "model/yolov8n-face-640x384_rk3568_mixed_face.rknn",
        )?;
        let registration_detector_path = env.resolve_model_path(
            &root,
            "REGISTRATION_DETECTOR_MODEL_PATH",
            "model/yolov8n-face-640x640_rk3568_mixed_face.rknn",
        )?;
        let embedder_path = env.resolve_model_path(
            &root,
            "EMBEDDER_MODEL_PATH",
            "model/edgeface_xs_gamma_06_rk3568_fp16.rknn",
        )?;

        verify_model_file(&detector_path)?;
        verify_model_file(&embedder_path)?;
        // 人体检测模型与 640x640 人脸注册检测模型若存在则验证
        if person_detector_path.is_file() {
            verify_model_file(&person_detector_path)?;
        }
        if registration_detector_path.is_file() {
            verify_model_file(&registration_detector_path)?;
        }

        Ok(Self {
            root,
            person_detector_path,
            detector_path,
            registration_detector_path,
            embedder_path,
            detector_width: 640,
            detector_height: 384,
            registration_detector_width: 640,
            registration_detector_height: 640,
            embedder_width: 112,
            embedder_height: 112,
            embedding_dimension: 512,
        })
    }
}

fn verify_model_file(path: &Path) -> Result<(), AlgoError> {
    let metadata = std::fs::metadata(path).map_err(|error| AlgoError::ModelLoad {
        reason: format!("读取模型元数据失败 ({path:?}): {error}"),
    })?;
    if !metadata.is_file() || metadata.len() == 0 || metadata.len() > MAX_MODEL_BYTES {
        return Err(AlgoError::ModelLoad {
            reason: format!("模型文件大小或类型非法 ({path:?})"),
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn package_manifest_matches_checked_in_models() {
        let package = LoadedPackage::load(Path::new(env!("CARGO_MANIFEST_DIR")))
            .expect("checked-in RKNN manifest and model paths must be valid");
        assert_eq!(package.detector_width, 640);
        assert_eq!(package.detector_height, 384);
        assert_eq!(package.registration_detector_width, 640);
        assert_eq!(package.registration_detector_height, 640);
        assert_eq!(package.embedder_width, 112);
        assert_eq!(package.embedder_height, 112);
        assert_eq!(package.embedding_dimension, 512);
        assert!(package.detector_path.is_file());
        assert!(package.registration_detector_path.is_file());
        assert!(package.embedder_path.is_file());
        assert!(package.person_detector_path.is_file());
    }
}
