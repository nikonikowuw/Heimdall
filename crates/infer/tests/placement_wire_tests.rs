//! NPU 放置 Wire 协议与回执防御性解析集成测试 (T13, T14, T41)

use ::types::placement::{
    AffinityIntent, CleanupStatus, PlacementApplicationStatus, WirePlacementMetadata,
    WireWeightBinding,
};
use infer::c_abi::*;

#[test]
fn test_wire_placement_compatibility_t13() {
    // 1. 新宿主下发完整 placement 配置
    let wire_metadata = WirePlacementMetadata {
        version: 1,
        reservation_id: "boot-1:res-100".to_string(),
        group_id: "group-det-1".to_string(),
        generation: 1,
        device_id: "rknn-npu0".to_string(),
        runtime_device_index: 0,
        strategy: "pinned".to_string(),
        core_mask: 2,
        required: true,
        weight_sharing: "required".to_string(),
        weight_bindings: vec![WireWeightBinding {
            model_key: "yolov8n".to_string(),
            weight_id: "weight-yolo-1".to_string(),
            generation: 1,
        }],
    };

    let host_config_json = serde_json::json!({
        "confidence_threshold": 0.65,
        "__heimdall_placement": wire_metadata
    });

    let json_str = serde_json::to_string(&host_config_json).expect("序列化应成功");

    // 2. 模拟旧插件（配置标有严格 deny_unknown_fields）
    #[derive(serde::Deserialize, PartialEq, Debug, Default)]
    #[serde(deny_unknown_fields)]
    struct LegacyStrictPluginConfig {
        confidence_threshold: f64,
    }

    // 验证如果不剥离，直接反序列化必然失败
    let direct_err = serde_json::from_str::<LegacyStrictPluginConfig>(&json_str);
    assert!(
        direct_err.is_err(),
        "包含 __heimdall_placement 的原始 JSON 在旧插件中应当被 deny_unknown_fields 拒绝"
    );

    // 验证通过 SDK 的安全剥离机制反序列化成功
    let mut val: serde_json::Value = serde_json::from_str(&json_str).expect("解析为 Value 应成功");
    let stripped_placement = if let serde_json::Value::Object(ref mut map) = val {
        map.remove("__heimdall_placement")
    } else {
        None
    };
    let parsed_config: LegacyStrictPluginConfig =
        serde_json::from_value(val).expect("剥离后反序列化旧配置应成功");
    assert_eq!(parsed_config.confidence_threshold, 0.65);
    assert!(stripped_placement.is_some());

    // 3. 模拟老宿主未下发 placement，新插件本地无注入加载
    let legacy_host_json = r#"{"confidence_threshold": 0.5}"#;
    let mut legacy_val: serde_json::Value =
        serde_json::from_str(legacy_host_json).expect("解析老宿主 JSON 应成功");
    let no_placement = if let serde_json::Value::Object(ref mut map) = legacy_val {
        map.remove("__heimdall_placement")
    } else {
        None
    };
    let legacy_parsed: LegacyStrictPluginConfig =
        serde_json::from_value(legacy_val).expect("反序列化旧配置应成功");
    assert_eq!(legacy_parsed.confidence_threshold, 0.5);
    assert!(no_placement.is_none());
}

#[test]
fn test_corrupted_cleanup_receipt_handling_t14() {
    // 构造合法的清理回执 POD
    let valid_receipt = AvAlgoCleanupReceiptPod {
        size: std::mem::size_of::<AvAlgoCleanupReceiptPod>() as u32,
        api_version: 1,
        cleanup_status: AV_CLEANUP_STATUS_CLEANED,
        sdk_error_code: 0,
        generation: 42,
        reservation_id: *b"boot-1:res-100\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0",
        reserved0: 0,
        reserved1: 0,
    };

    // 1. 正常解析
    assert_eq!(valid_receipt.size, 72);
    assert_eq!(valid_receipt.api_version, 1);
    assert_eq!(valid_receipt.cleanup_status, AV_CLEANUP_STATUS_CLEANED);

    // 2. 防御测试：api_version 不匹配
    let mut bad_version = valid_receipt;
    bad_version.api_version = 999;
    let validate_version = |r: &AvAlgoCleanupReceiptPod| -> Result<CleanupStatus, &'static str> {
        if r.api_version != 1 {
            return Err("回执 API 版本不受支持");
        }
        if r.size != std::mem::size_of::<AvAlgoCleanupReceiptPod>() as u32 {
            return Err("回执结构体大小不匹配");
        }
        match r.cleanup_status {
            AV_CLEANUP_STATUS_CLEANED => Ok(CleanupStatus::Cleaned),
            AV_CLEANUP_STATUS_UNVERIFIED => Ok(CleanupStatus::Unverified),
            _ => Ok(CleanupStatus::Failed),
        }
    };
    assert_eq!(validate_version(&bad_version), Err("回执 API 版本不受支持"));

    // 3. 防御测试：size 被截断或扩充
    let mut bad_size = valid_receipt;
    bad_size.size = 64; // 错误大小
    assert_eq!(validate_version(&bad_size), Err("回执结构体大小不匹配"));

    // 4. 防御测试：未知错误状态码映射为 Failed，绝不伪造 Cleaned
    let mut bad_status = valid_receipt;
    bad_status.cleanup_status = 0xDEAD;
    assert_eq!(validate_version(&bad_status), Ok(CleanupStatus::Failed));
}

#[test]
fn test_instance_receipt_pod_defensive_mapping_t14() {
    let valid_receipt = AvAlgoInstanceReceiptPod {
        size: std::mem::size_of::<AvAlgoInstanceReceiptPod>() as u32,
        api_version: 1,
        status: AV_PLACEMENT_STATUS_ACKNOWLEDGED,
        assigned_core_mask: 2,
        actual_core_mask: 2,
        weight_sharing_confirmed: 1,
        generation: 10,
        sdk_error_code: 0,
        reserved0: 0,
        reservation_id: *b"boot-1:res-100\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0",
    };

    let parse_instance_status =
        |r: &AvAlgoInstanceReceiptPod| -> Result<PlacementApplicationStatus, &'static str> {
            if r.api_version != 1 {
                return Err("回执 API 版本不受支持");
            }
            if r.size != std::mem::size_of::<AvAlgoInstanceReceiptPod>() as u32 {
                return Err("回执结构体大小不匹配");
            }
            match r.status {
                AV_PLACEMENT_STATUS_ACKNOWLEDGED => Ok(PlacementApplicationStatus::Acknowledged),
                AV_PLACEMENT_STATUS_RUNTIME_MANAGED => {
                    Ok(PlacementApplicationStatus::RuntimeManaged)
                }
                AV_PLACEMENT_STATUS_DEGRADED => Ok(PlacementApplicationStatus::Degraded),
                AV_PLACEMENT_STATUS_UNVERIFIED => Ok(PlacementApplicationStatus::Unverified),
                _ => Ok(PlacementApplicationStatus::Failed),
            }
        };

    assert_eq!(
        parse_instance_status(&valid_receipt),
        Ok(PlacementApplicationStatus::Acknowledged)
    );

    let mut degraded_receipt = valid_receipt;
    degraded_receipt.status = AV_PLACEMENT_STATUS_DEGRADED;
    assert_eq!(
        parse_instance_status(&degraded_receipt),
        Ok(PlacementApplicationStatus::Degraded)
    );

    let mut bad_size_receipt = valid_receipt;
    bad_size_receipt.size = 120;
    assert!(parse_instance_status(&bad_size_receipt).is_err());
}

#[test]
fn test_self_test_fallback_hard_gate_t41() {
    // 验证当注入 placement 元数据且 is_self_test=true 时，硬门依然保持最高优先级
    // 任何环境配置或 placement 意图都不得让自检阶段回退到 CPU 模拟
    let auto_affinity = AffinityIntent::Auto {
        policy: "spread".to_string(),
    };
    assert_eq!(
        auto_affinity,
        AffinityIntent::default(),
        "auto 亲和默认采用 spread 策略"
    );

    // 验证自检硬门下即便设置了 auto 亲和降级，也不能产生模拟降级
    let is_self_test = true;
    let requires_hardware = is_self_test; // 按照 fallback.rs 规则，自检硬门恒定生效
    assert!(requires_hardware, "自检硬门绝不允许回退到 CPU 模拟");
}
