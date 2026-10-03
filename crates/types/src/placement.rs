use serde::{Deserialize, Serialize};

fn default_spread_policy() -> String {
    "spread".to_string()
}

/// 算法实例放置偏好与约束意图
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "mode", rename_all = "camelCase")]
pub enum AffinityIntent {
    /// 自动放置模式（默认打散到负载最小核心）
    #[serde(rename_all = "camelCase")]
    Auto {
        #[serde(default = "default_spread_policy")]
        policy: String,
    },
    /// 人工指定目标设备与核心索引（严格约束）
    #[serde(rename_all = "camelCase")]
    Manual { device_id: String, core_index: u32 },
}

impl Default for AffinityIntent {
    fn default() -> Self {
        Self::Auto {
            policy: default_spread_policy(),
        }
    }
}

impl AffinityIntent {
    /// 归一化亲和意图（将空 policy 填充为默认 "spread"）
    pub fn normalized(&self) -> Self {
        match self {
            Self::Auto { policy } => {
                let p = if policy.trim().is_empty() {
                    default_spread_policy()
                } else {
                    policy.clone()
                };
                Self::Auto { policy: p }
            }
            Self::Manual {
                device_id,
                core_index,
            } => Self::Manual {
                device_id: device_id.clone(),
                core_index: *core_index,
            },
        }
    }

    /// 判断两个亲和意图是否等价
    pub fn is_equivalent_to(&self, other: &Self) -> bool {
        self.normalized() == other.normalized()
    }
}

/// 比较两个可选亲和意图是否等价（None 视为默认 auto spread）
pub fn is_affinity_equivalent(a: Option<&AffinityIntent>, b: Option<&AffinityIntent>) -> bool {
    let a_norm = a.map(|v| v.normalized()).unwrap_or_default();
    let b_norm = b.map(|v| v.normalized()).unwrap_or_default();
    a_norm == b_norm
}

/// 宿主注入算法配置的 Wire Placement 元数据 (`__heimdall_placement`)
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WirePlacementMetadata {
    pub version: u32,
    pub reservation_id: String,
    pub group_id: String,
    pub generation: u64,
    pub device_id: String,
    pub runtime_device_index: u32,
    pub strategy: String,
    pub core_mask: u32,
    pub required: bool,
    pub weight_sharing: String,
    #[serde(default)]
    pub weight_bindings: Vec<WireWeightBinding>,
}

/// 单个模型物理权重引用绑定
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WireWeightBinding {
    pub model_key: String,
    pub weight_id: String,
    pub generation: u64,
}

/// 实例与权重回执应用状态
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PlacementApplicationStatus {
    Acknowledged,
    RuntimeManaged,
    Degraded,
    Unverified,
    Failed,
}

/// 资源清理确认状态
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum CleanupStatus {
    Cleaned,
    Unverified,
    Failed,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_affinity_intent_serde_round_trip() {
        let auto_default = AffinityIntent::default();
        let json = serde_json::to_string(&auto_default).expect("序列化默认 auto 应成功");
        assert_eq!(json, r#"{"mode":"auto","policy":"spread"}"#);

        let parsed: AffinityIntent = serde_json::from_str(&json).expect("反序列化 auto 应成功");
        assert_eq!(parsed, auto_default);

        let manual = AffinityIntent::Manual {
            device_id: "rknn-npu0".to_string(),
            core_index: 1,
        };
        let json_manual = serde_json::to_string(&manual).expect("序列化 manual 应成功");
        assert_eq!(
            json_manual,
            r#"{"mode":"manual","deviceId":"rknn-npu0","coreIndex":1}"#
        );

        let parsed_manual: AffinityIntent =
            serde_json::from_str(&json_manual).expect("反序列化 manual 应成功");
        assert_eq!(parsed_manual, manual);
    }

    #[test]
    fn test_wire_placement_metadata_serde() {
        let wire = WirePlacementMetadata {
            version: 1,
            reservation_id: "boot-1:0".to_string(),
            group_id: "grp-1".to_string(),
            generation: 3,
            device_id: "rknn-npu0".to_string(),
            runtime_device_index: 0,
            strategy: "pinned".to_string(),
            core_mask: 2,
            required: true,
            weight_sharing: "required".to_string(),
            weight_bindings: vec![WireWeightBinding {
                model_key: "yolo".to_string(),
                weight_id: "w-yolo".to_string(),
                generation: 1,
            }],
        };

        let json = serde_json::to_string(&wire).expect("序列化 wire 应成功");
        let parsed: WirePlacementMetadata =
            serde_json::from_str(&json).expect("反序列化 wire 应成功");
        assert_eq!(parsed, wire);
    }
}
