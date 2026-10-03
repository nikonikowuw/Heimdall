//! SDK placement 注入、RKNN 共享权重运行时与扩展虚表集成测试

use std::ffi::{c_void, CString};
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};

use algo_sdk::c_abi::*;
use algo_sdk::plugin::{InitContext, WirePlacementMetadata, WireWeightBinding};
use algo_sdk::prelude::*;

struct MockSpec;

impl YoloSpec for MockSpec {
    const INPUT_DIM: (u32, u32) = (640, 640);
    const NUM_CLASSES: usize = 1;
    const LABELS: &'static [&'static str] = &["target"];
    const MODEL_PATH: &'static str = "model/model.rknn";
}

type MockYoloPlugin = GenericYoloDetector<MockSpec>;

export_algo!(
    MockYoloPlugin,
    algo_id: "mock_shared_weights_test",
    version: "1.0.0",
    algo_type: "object_detection",
    alarm_type_id: "placement_test"
);

static TEST_RESULT_COUNT: AtomicUsize = AtomicUsize::new(0);

unsafe extern "C" fn test_result_cb(result: *const AvAlgoResult, _user: *mut c_void) {
    if !result.is_null() {
        TEST_RESULT_COUNT.fetch_add(1, Ordering::SeqCst);
    }
}

fn sample_placement(
    reservation_id: &str,
    core_mask: u32,
    generation: u64,
) -> WirePlacementMetadata {
    WirePlacementMetadata {
        version: 1,
        reservation_id: reservation_id.to_string(),
        generation,
        group_id: "group-0".to_string(),
        device_id: "npu-0".to_string(),
        runtime_device_index: 0,
        strategy: "isolated_core".to_string(),
        core_mask,
        required: true,
        weight_sharing: "shared_weights".to_string(),
        weight_bindings: vec![WireWeightBinding {
            model_key: "model/model.rknn".to_string(),
            weight_id: "mock_yolo".to_string(),
            generation,
        }],
    }
}

#[test]
fn test_init_context_with_placement_builder() {
    let root = PathBuf::from("/tmp/test_pkg");
    let placement = sample_placement("res-abc-001", 0x02, 101);

    let ctx = InitContext::new(&root, "linux-rknn", "inst-1", false)
        .with_placement(Some(placement.clone()));

    assert_eq!(ctx.target_core_mask(), Some(0x02));
    assert_eq!(ctx.instance_id, "inst-1");
    assert!(!ctx.is_self_test);
    assert_eq!(
        ctx.wire_placement
            .as_ref()
            .expect("wire_placement 应存在")
            .reservation_id,
        "res-abc-001"
    );

    let ctx_no_placement = InitContext::new(&root, "linux-rknn", "inst-2", false);
    assert_eq!(ctx_no_placement.target_core_mask(), None);
}

#[test]
fn test_rknn_set_core_mask_per_instance_without_env() {
    // 确保没有进程级全局环境变量污染
    std::env::remove_var("RKNN_CORE_MASK");

    let root = PathBuf::from("/tmp/test_pkg");
    let placement = sample_placement("res-mask-test", 0x04, 1);

    let ctx =
        InitContext::new(&root, "linux-rknn", "inst-mask", false).with_placement(Some(placement));

    assert_eq!(ctx.target_core_mask(), Some(0x04));
    assert!(
        std::env::var("RKNN_CORE_MASK").is_err(),
        "逐实例设核严禁设置全局环境变量 RKNN_CORE_MASK"
    );
}

#[test]
fn test_self_test_requires_hardware_no_fallback() {
    let temp_dir = std::env::temp_dir().join(format!("algo_self_test_{}", uuid::Uuid::new_v4()));
    let model_dir = temp_dir.join("model");
    std::fs::create_dir_all(&model_dir).expect("create model dir");
    std::fs::write(model_dir.join("model.rknn"), b"mock model bytes").expect("write model");

    let placement = sample_placement("res-self-test", 0x01, 1);

    // 在自检模式下（is_self_test = true），若平台为硬件平台但缺少物理 NPU 硬件环境，必须严格报错
    let ctx = InitContext::new(&temp_dir, "linux-rknn", "inst-self-test", true)
        .with_placement(Some(placement));

    let config = <MockYoloPlugin as AlgoPlugin>::Config::default();
    let init_result = MockYoloPlugin::init(&ctx, config);

    assert!(
        init_result.is_err(),
        "自检模式在缺少物理硬件环境时必须严格失败，严禁回退到 CPU 仿真"
    );

    let _ = std::fs::remove_dir_all(temp_dir);
}

#[test]
fn test_placement_extension_query_and_lifecycle() {
    let temp_dir = std::env::temp_dir().join(format!("algo_ext_test_{}", uuid::Uuid::new_v4()));
    let model_dir = temp_dir.join("model");
    std::fs::create_dir_all(&model_dir).expect("create model dir");
    std::fs::write(model_dir.join("model.rknn"), b"mock model bytes").expect("write model");

    // 1. 获取 C ABI 虚表与 Placement Extension
    // SAFETY: 测试中直接调用同一二进制中宏展开的全局符号
    let ext_ptr = unsafe { av_algo_get_placement_extension(1) };
    assert!(
        !ext_ptr.is_null(),
        "av_algo_get_placement_extension(1) 应返回有效虚表"
    );
    // SAFETY: 指针非空
    let ext = unsafe { &*ext_ptr };
    assert_eq!(ext.api_version, 1);

    // 2. 查询能力 Caps
    let mut caps = AvAlgoPlacementCapsPod {
        size: 0,
        api_version: 0,
        caps: 0,
        supported_core_mask: 0,
        max_child_contexts_per_root: 0,
        reserved0: 0,
        reserved1: 0,
        reserved2: 0,
    };
    let query_caps_fn = ext.query_capabilities.expect("query_capabilities 应存在");
    // SAFETY: caps 局部变量指针有效且可写
    let status = unsafe { query_caps_fn(&mut caps) };
    assert_eq!(status, AV_OK);
    assert_eq!(
        caps.size,
        std::mem::size_of::<AvAlgoPlacementCapsPod>() as u32
    );
    assert_ne!(caps.caps & AV_PLACEMENT_CAP_WEIGHT_SHARING, 0);
    assert_ne!(caps.caps & AV_PLACEMENT_CAP_EXECUTION_ISOLATION, 0);
    assert_eq!(caps.supported_core_mask, 0x07);

    // 3. 打开 Library
    // SAFETY: 调用宏展开的 av_algo_get_abi
    let abi_ptr = unsafe { av_algo_get_abi(AV_ALGO_API_VERSION) };
    assert!(!abi_ptr.is_null());
    // SAFETY: 指针非空
    let abi = unsafe { &*abi_ptr };

    let pkg_root = CString::new(temp_dir.to_str().expect("path utf8")).expect("cstring");
    let platform = CString::new("rk3568").expect("cstring");
    let open_args = AvAlgoLibraryArgs {
        size: std::mem::size_of::<AvAlgoLibraryArgs>() as u32,
        api_version: AV_ALGO_API_VERSION,
        package_root: pkg_root.as_ptr(),
        platform_id: platform.as_ptr(),
        platform_tag: 0,
        log: None,
        log_user: std::ptr::null_mut(),
    };

    let mut lib: AvAlgoLibrary = std::ptr::null_mut();
    // SAFETY: 结构体生命周期有效
    let status = unsafe { (abi.library_open.expect("open"))(&open_args, &mut lib) };
    assert_eq!(status, AV_OK);

    // 4. 准备携带 __heimdall_placement 的 JSON 配置创建实例
    let reservation_id_str = "res-e2e-receipt-001";
    let placement = sample_placement(reservation_id_str, 4, 77);
    let host_config = serde_json::json!({
        "confidence_threshold": 0.5,
        "__heimdall_placement": placement
    });
    let config_json = serde_json::to_string(&host_config).expect("json serialize");
    let config_c = CString::new(config_json).expect("cstring");
    let instance_id_c = CString::new("inst-e2e").expect("cstring");
    let run_id_c = CString::new("run-e2e").expect("cstring");

    let create_args = AvAlgoInstanceArgs {
        size: std::mem::size_of::<AvAlgoInstanceArgs>() as u32,
        api_version: AV_ALGO_API_VERSION,
        mode: AV_INSTANCE_NORMAL,
        reserved0: 0,
        instance_id: instance_id_c.as_ptr(),
        instance_run_id: run_id_c.as_ptr(),
        config_json: config_c.as_ptr(),
        config_json_len: config_c.as_bytes().len() as u32,
        reserved1: 0,
        frame_ops: std::ptr::null(),
        image_ops: std::ptr::null(),
        on_result: Some(test_result_cb),
        result_user: std::ptr::null_mut(),
        rules: std::ptr::null(),
        rule_count: 0,
    };

    let mut inst: AvAlgoInstance = std::ptr::null_mut();
    // SAFETY: create_args 指针有效且可读
    let status = unsafe { (abi.instance_create.expect("create"))(lib, &create_args, &mut inst) };
    assert_eq!(status, AV_OK);
    assert!(!inst.is_null());

    // 5. 查询 Instance Receipt
    let query_inst_receipt_fn = ext
        .query_instance_receipt
        .expect("query_instance_receipt 应存在");
    let mut inst_receipt = AvAlgoInstanceReceiptPod {
        size: 0,
        api_version: 0,
        status: 0,
        assigned_core_mask: 0,
        actual_core_mask: 0,
        weight_sharing_confirmed: 0,
        generation: 0,
        sdk_error_code: 0,
        reserved0: 0,
        reservation_id: [0u8; 32],
    };
    // SAFETY: inst_receipt 局部变量指针有效且可写
    let status = unsafe { query_inst_receipt_fn(inst, &mut inst_receipt) };
    assert_eq!(status, AV_OK);
    assert_eq!(inst_receipt.status, AV_PLACEMENT_STATUS_ACKNOWLEDGED);
    assert_eq!(inst_receipt.assigned_core_mask, 4);
    assert_eq!(inst_receipt.actual_core_mask, 4);
    assert_eq!(inst_receipt.generation, 77);
    let returned_res_id = std::str::from_utf8(&inst_receipt.reservation_id)
        .expect("utf8")
        .trim_matches('\0');
    assert_eq!(returned_res_id, reservation_id_str);

    // 6. 销毁实例，触发清理回执记录
    // SAFETY: inst 非空且来自 instance_create
    let status = unsafe { (abi.instance_destroy.expect("destroy"))(inst) };
    assert_eq!(status, AV_OK);

    // 7. 查询 Cleanup Receipt
    let query_cleanup_receipt_fn = ext
        .query_cleanup_receipt
        .expect("query_cleanup_receipt 应存在");
    let mut cleanup_receipt = AvAlgoCleanupReceiptPod {
        size: 0,
        api_version: 0,
        cleanup_status: 0,
        sdk_error_code: 0,
        generation: 0,
        reservation_id: [0u8; 32],
        reserved0: 0,
        reserved1: 0,
    };
    let res_c = CString::new(reservation_id_str).expect("cstring");
    // SAFETY: res_c 和 cleanup_receipt 指针有效
    let status = unsafe { query_cleanup_receipt_fn(res_c.as_ptr(), &mut cleanup_receipt) };
    assert_eq!(status, AV_OK);
    assert_eq!(cleanup_receipt.cleanup_status, AV_CLEANUP_STATUS_CLEANED);
    let cleanup_res_id = std::str::from_utf8(&cleanup_receipt.reservation_id)
        .expect("utf8")
        .trim_matches('\0');
    assert_eq!(cleanup_res_id, reservation_id_str);

    // 8. 关闭 Library
    // SAFETY: lib 非空
    let status = unsafe { (abi.library_close.expect("close"))(lib) };
    assert_eq!(status, AV_OK);

    let _ = std::fs::remove_dir_all(temp_dir);
}
