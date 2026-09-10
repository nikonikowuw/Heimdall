//! 算法包 manifest 的唯一模型路径与 I/O 契约来源。

use std::path::{Component, Path, PathBuf};

use serde::Deserialize;
use sha2::{Digest, Sha256};

use algo_sdk::error::AlgoError;

const MAX_MANIFEST_BYTES: u64 = 1024 * 1024;
const MAX_MODEL_BYTES: u64 = 256 * 1024 * 1024;

#[derive(Debug, Clone, Deserialize)]
pub struct PackageManifest {
    pub manifest_version: u32,
    pub algorithm_id: String,
    pub platform_id: String,
    pub runtime_constraints: RuntimeConstraints,
    pub models: ModelsManifest,
    pub self_test: SelfTestManifest,
}

#[derive(Debug, Clone, Deserialize)]
pub struct RuntimeConstraints {
    #[serde(alias = "requiresRknnRuntime")]
    pub requires_rknn_runtime: bool,
    #[serde(alias = "minRknnrtVersion")]
    pub min_rknnrt_version: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ModelsManifest {
    #[serde(deserialize_with = "deserialize_detector")]
    pub detector: ModelManifest,
    #[serde(deserialize_with = "deserialize_embedder")]
    pub embedder: ModelManifest,
    #[serde(rename = "embedding_dimension")]
    pub embedding_dimension: u32,
}

fn deserialize_detector<'de, D>(deserializer: D) -> Result<ModelManifest, D::Error>
where
    D: serde::Deserializer<'de>,
{
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum Helper {
        Detailed(ModelManifest),
        Simple(String),
    }

    match Helper::deserialize(deserializer)? {
        Helper::Detailed(m) => Ok(m),
        Helper::Simple(path) => Ok(ModelManifest {
            path,
            sha256: String::new(),
            input: ModelInputManifest {
                width: 640,
                height: 384,
                channels: 3,
                pixel_format: "rgb24".to_string(),
                layout: "nchw".to_string(),
                data_type: "uint8".to_string(),
                pass_through: false,
                mean_values: [0.0; 3],
                std_values: [255.0; 3],
            },
            outputs: DETECTOR_OUTPUT_SHAPES
                .iter()
                .map(|shape| ModelOutputManifest {
                    shape: *shape,
                    layout: "nchw".to_string(),
                    data_type: "float32".to_string(),
                })
                .collect(),
        }),
    }
}

fn deserialize_embedder<'de, D>(deserializer: D) -> Result<ModelManifest, D::Error>
where
    D: serde::Deserializer<'de>,
{
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum Helper {
        Detailed(ModelManifest),
        Simple(String),
    }

    match Helper::deserialize(deserializer)? {
        Helper::Detailed(m) => Ok(m),
        Helper::Simple(path) => Ok(ModelManifest {
            path,
            sha256: String::new(),
            input: ModelInputManifest {
                width: 112,
                height: 112,
                channels: 3,
                pixel_format: "rgb24".to_string(),
                layout: "nchw".to_string(),
                data_type: "uint8".to_string(),
                pass_through: false,
                mean_values: [127.5; 3],
                std_values: [127.5; 3],
            },
            outputs: vec![ModelOutputManifest {
                shape: [1, 512, 1, 1],
                layout: "nc".to_string(),
                data_type: "float32".to_string(),
            }],
        }),
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct ModelManifest {
    pub path: String,
    pub sha256: String,
    pub input: ModelInputManifest,
    pub outputs: Vec<ModelOutputManifest>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelInputManifest {
    pub width: u32,
    pub height: u32,
    pub channels: u32,
    pub pixel_format: String,
    pub layout: String,
    pub data_type: String,
    pub pass_through: bool,
    pub mean_values: [f32; 3],
    pub std_values: [f32; 3],
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelOutputManifest {
    pub shape: [u32; 4],
    pub layout: String,
    pub data_type: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SelfTestManifest {
    #[serde(alias = "timeout_ms", alias = "timeoutMs")]
    pub timeout_ms: u64,
    #[serde(alias = "input_mode", alias = "inputMode")]
    pub input_mode: String,
}

#[derive(Debug, Clone)]
pub struct LoadedPackage {
    pub root: PathBuf,
    pub manifest: PackageManifest,
    pub detector_path: PathBuf,
    pub embedder_path: PathBuf,
}

impl LoadedPackage {
    pub fn load(package_root: &Path) -> Result<Self, AlgoError> {
        let root = package_root
            .canonicalize()
            .map_err(|error| AlgoError::ModelLoad {
                reason: format!("算法包根目录无法规范化 ({package_root:?}): {error}"),
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
        if manifest.models.embedding_dimension != 512 {
            return Err(AlgoError::ModelLoad {
                reason: format!(
                    "EdgeFace embedding_dimension 必须为 512，实际 {}",
                    manifest.models.embedding_dimension
                ),
            });
        }
        validate_input_manifest(
            "detector",
            &manifest.models.detector.input,
            640,
            384,
            [0.0, 0.0, 0.0],
            [255.0, 255.0, 255.0],
        )?;
        validate_input_manifest(
            "embedder",
            &manifest.models.embedder.input,
            112,
            112,
            [127.5, 127.5, 127.5],
            [127.5, 127.5, 127.5],
        )?;
        validate_model_outputs(
            "detector",
            &manifest.models.detector.outputs,
            &DETECTOR_OUTPUT_SHAPES,
            &["nchw"],
        )?;
        validate_model_outputs(
            "embedder",
            &manifest.models.embedder.outputs,
            &[[1, 512, 1, 1]],
            &["nc", "nchw"],
        )?;

        let detector_path = verify_model(&root, &manifest.models.detector)?;
        let embedder_path = verify_model(&root, &manifest.models.embedder)?;
        Ok(Self {
            root,
            manifest,
            detector_path,
            embedder_path,
        })
    }
}

fn validate_input_manifest(
    name: &str,
    input: &ModelInputManifest,
    expected_width: u32,
    expected_height: u32,
    expected_mean: [f32; 3],
    expected_std: [f32; 3],
) -> Result<(), AlgoError> {
    if input.width != expected_width
        || input.height != expected_height
        || input.channels != 3
        || input.pixel_format != "rgb24"
        || input.layout != "nchw"
        || input.data_type != "uint8"
        || input.pass_through
        || input.mean_values != expected_mean
        || input.std_values != expected_std
        || input.mean_values.iter().any(|v| !v.is_finite())
        || input.std_values.iter().any(|v| !v.is_finite() || *v <= 0.0)
    {
        return Err(AlgoError::ModelLoad {
            reason: format!("{name} manifest 输入契约不支持当前硬件路径"),
        });
    }
    Ok(())
}

const DETECTOR_OUTPUT_SHAPES: [[u32; 4]; 12] = [
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

fn validate_model_outputs(
    name: &str,
    outputs: &[ModelOutputManifest],
    expected_shapes: &[[u32; 4]],
    allowed_layouts: &[&str],
) -> Result<(), AlgoError> {
    if outputs.len() != expected_shapes.len() {
        return Err(AlgoError::ModelLoad {
            reason: format!(
                "{name} 输出分支数量不匹配: expected={}, actual={}",
                expected_shapes.len(),
                outputs.len()
            ),
        });
    }
    for (index, (output, expected_shape)) in outputs.iter().zip(expected_shapes).enumerate() {
        if output.shape != *expected_shape
            || !allowed_layouts.contains(&output.layout.as_str())
            || output.data_type != "float32"
        {
            return Err(AlgoError::ModelLoad {
                reason: format!(
                    "{name} 输出 {index} 契约不匹配: expected_shape={expected_shape:?}, actual_shape={:?}, layout={}, data_type={}",
                    output.shape, output.layout, output.data_type
                ),
            });
        }
    }
    Ok(())
}
fn verify_model(root: &Path, model: &ModelManifest) -> Result<PathBuf, AlgoError> {
    let relative = Path::new(&model.path);
    if relative.as_os_str().is_empty()
        || relative.is_absolute()
        || relative.components().any(|component| {
            matches!(
                component,
                Component::ParentDir | Component::RootDir | Component::Prefix(_)
            )
        })
    {
        return Err(AlgoError::ModelLoad {
            reason: format!("模型路径不安全: {}", model.path),
        });
    }
    let path = root
        .join(relative)
        .canonicalize()
        .map_err(|error| AlgoError::ModelLoad {
            reason: format!("模型文件无法规范化 ({}): {error}", model.path),
        })?;
    if !path.starts_with(root) {
        return Err(AlgoError::ModelLoad {
            reason: format!("模型路径越出算法包根目录: {}", model.path),
        });
    }
    let metadata = std::fs::metadata(&path).map_err(|error| AlgoError::ModelLoad {
        reason: format!("读取模型元数据失败 ({path:?}): {error}"),
    })?;
    if !metadata.is_file() || metadata.len() == 0 || metadata.len() > MAX_MODEL_BYTES {
        return Err(AlgoError::ModelLoad {
            reason: format!("模型文件大小或类型非法 ({path:?})"),
        });
    }
    if !model.sha256.is_empty() {
        let bytes = std::fs::read(&path).map_err(|error| AlgoError::ModelLoad {
            reason: format!("读取模型文件失败 ({path:?}): {error}"),
        })?;
        let digest = Sha256::digest(&bytes);
        let actual = digest
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>();
        if !actual.eq_ignore_ascii_case(&model.sha256) {
            return Err(AlgoError::ModelLoad {
                reason: format!(
                    "模型 SHA-256 不匹配 ({path:?}): expected={}, actual={actual}",
                    model.sha256
                ),
            });
        }
    }
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn package_manifest_matches_checked_in_models() {
        let package = LoadedPackage::load(Path::new(env!("CARGO_MANIFEST_DIR")))
            .expect("checked-in RKNN manifest and model hashes must be valid");
        assert_eq!(package.manifest.models.detector.outputs.len(), 12);
        assert_eq!(package.manifest.models.embedding_dimension, 512);
    }
}
