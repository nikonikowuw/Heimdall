use infer::AlgoManifest;
use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ListAlgorithmsQuery {
    pub page: Option<u64>,
    pub page_size: Option<u64>,
    pub keyword: Option<String>,
    pub algorithm_type: Option<String>,
    pub is_builtin: Option<bool>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AlgorithmVersionItemDto {
    pub id: i64,
    pub algorithm_id: String,
    pub version: String,
    pub platform_id: String,
    pub min_adapter_version: String,
    pub package_root: String,
    pub fps_tiers: serde_json::Value,
    pub config_schema: serde_json::Value,
    pub manifest_raw: serde_json::Value,
    pub package_size_bytes: i64,
    pub is_active: bool,
    pub is_builtin: bool,
    pub created_at: i64,
    pub updated_at: i64,
}

impl From<db::entity::algorithm_version::Model> for AlgorithmVersionItemDto {
    fn from(m: db::entity::algorithm_version::Model) -> Self {
        let fps_tiers =
            serde_json::from_str(&m.fps_tiers).unwrap_or_else(|_| serde_json::json!([]));
        let config_schema =
            serde_json::from_str(&m.config_schema).unwrap_or_else(|_| serde_json::json!({}));
        let manifest_raw =
            serde_json::from_str(&m.manifest_raw).unwrap_or_else(|_| serde_json::json!({}));

        Self {
            id: m.id,
            algorithm_id: m.algorithm_id,
            version: m.version,
            platform_id: m.platform_id,
            min_adapter_version: m.min_adapter_version,
            package_root: m.package_root,
            fps_tiers,
            config_schema,
            manifest_raw,
            package_size_bytes: m.package_size_bytes,
            is_active: m.is_active,
            is_builtin: m.is_builtin,
            created_at: m.created_at.timestamp_millis(),
            updated_at: m.updated_at.timestamp_millis(),
        }
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AlgorithmItemDto {
    pub id: i64,
    pub algorithm_id: String,
    pub name: String,
    pub algorithm_type: String,
    pub alarm_type_id: String,
    pub active_version: String,
    pub description: String,
    pub is_builtin: bool,
    pub created_at: i64,
    pub updated_at: i64,
    pub versions: Vec<AlgorithmVersionItemDto>,
}

impl AlgorithmItemDto {
    pub fn from_model(
        m: db::entity::algorithm::Model,
        versions: Vec<AlgorithmVersionItemDto>,
    ) -> Self {
        Self {
            id: m.id,
            algorithm_id: m.algorithm_id,
            name: m.name,
            algorithm_type: m.algorithm_type,
            alarm_type_id: m.alarm_type_id,
            active_version: m.active_version,
            description: m.description,
            is_builtin: m.is_builtin,
            created_at: m.created_at.timestamp_millis(),
            updated_at: m.updated_at.timestamp_millis(),
            versions,
        }
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PaginatedAlgorithmsDto {
    pub items: Vec<AlgorithmItemDto>,
    pub total: u64,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UploadVersionInfo {
    pub algorithm_id: String,
    pub version: String,
    pub platform_id: String,
    pub package_root: String,
    pub is_active: bool,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SandboxCheckResultDto {
    pub passed: bool,
    pub steps_total: usize,
    pub steps_passed: usize,
    pub steps: Vec<String>,
    pub error_message: Option<String>,
    pub version: Option<UploadVersionInfo>,
    pub manifest: Option<AlgoManifest>,
}

pub fn get_standard_steps() -> Vec<String> {
    vec![
        "1. 路径防穿透与目录结构检查".to_string(),
        "2. 解析 Manifest 与平台拓扑匹配".to_string(),
        "3. Config Schema 参数格式校验".to_string(),
        "4. 派生隔离子进程与超时守护".to_string(),
        "5. 算法库 C ABI 导出符号核对".to_string(),
        "6. 真实前向推理自测与内存复核".to_string(),
    ]
}
