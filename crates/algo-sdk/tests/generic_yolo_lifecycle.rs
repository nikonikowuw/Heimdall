//! GenericYoloDetector 完整 C ABI 生命周期集成测试

use std::ffi::{c_void, CStr, CString};
use std::sync::atomic::{AtomicUsize, Ordering};

use algo_sdk::prelude::*;

// 1. 声明模型 Spec
struct MyHelmetSpec;

impl YoloSpec for MyHelmetSpec {
    const INPUT_DIM: (u32, u32) = (640, 640);
    const NUM_CLASSES: usize = 2;
    const LABELS: &'static [&'static str] = &["Hardhat", "NO-Hardhat"];
    const MODEL_PATH: &'static str = "model/model.rknn";
}

type MyHelmetPlugin = GenericYoloDetector<MyHelmetSpec>;

// 2. 导出 C ABI 虚表宏
export_algo!(
    MyHelmetPlugin,
    algo_id: "generic_helmet_test",
    version: "1.0.0",
    algo_type: "object_detection",
    alarm_type_id: "safety_violation"
);

static RESULT_COUNT: AtomicUsize = AtomicUsize::new(0);

unsafe extern "C" fn test_on_result(result: *const AvAlgoResult, _user: *mut c_void) {
    if !result.is_null() {
        RESULT_COUNT.fetch_add(1, Ordering::SeqCst);
    }
}

#[test]
fn test_generic_yolo_c_abi_export_lifecycle() {
    let temp_dir = std::env::temp_dir().join(format!("algo_abi_test_{}", uuid::Uuid::new_v4()));
    let model_dir = temp_dir.join("model");
    std::fs::create_dir_all(&model_dir).expect("create model dir");
    std::fs::write(model_dir.join("model.rknn"), b"mock rknn bytes").expect("write model");

    // 1. 获取 ABI 虚表
    // SAFETY: 测试中调用本测试文件展开的 export_algo! 虚表符号
    let abi_ptr = unsafe { av_algo_get_abi(AV_ALGO_API_VERSION) };
    assert!(!abi_ptr.is_null(), "av_algo_get_abi 应返回有效指针");
    // SAFETY: 指针非空
    let abi = unsafe { &*abi_ptr };

    // 2. 打开 Library
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
    assert!(!lib.is_null());

    // 3. 查询 Library 元信息
    let mut info = AvAlgoLibraryInfo {
        size: std::mem::size_of::<AvAlgoLibraryInfo>() as u32,
        api_version: AV_ALGO_API_VERSION,
        algorithm_id: [0; 64],
        version: [0; 32],
        algorithm_type: [0; 32],
        alarm_type_id: [0; 64],
    };
    // SAFETY: info 指针合法
    let query_status = unsafe { (abi.library_query.expect("query"))(lib, &mut info) };
    assert_eq!(query_status, AV_OK);
    // SAFETY: 字符串必定以 \0 结尾
    let id_str = unsafe { CStr::from_ptr(info.algorithm_id.as_ptr()) }
        .to_str()
        .expect("utf8");
    assert_eq!(id_str, "generic_helmet_test");

    // 4. 创建实例
    let inst_id = CString::new("inst_001").expect("cstring");
    let run_id = CString::new("run_001").expect("cstring");
    let config_json =
        CString::new(r#"{"confidence_threshold": 0.55, "iou_threshold": 0.40}"#).expect("cstring");

    let inst_args = AvAlgoInstanceArgs {
        size: std::mem::size_of::<AvAlgoInstanceArgs>() as u32,
        api_version: AV_ALGO_API_VERSION,
        mode: AV_INSTANCE_NORMAL,
        reserved0: 0,
        instance_id: inst_id.as_ptr(),
        instance_run_id: run_id.as_ptr(),
        config_json: config_json.as_ptr(),
        config_json_len: config_json.as_bytes().len() as u32,
        reserved1: 0,
        frame_ops: std::ptr::null(),
        image_ops: std::ptr::null(),
        on_result: Some(test_on_result),
        result_user: std::ptr::null_mut(),
        rules: std::ptr::null(),
        rule_count: 0,
    };

    let mut inst: AvAlgoInstance = std::ptr::null_mut();
    // SAFETY: 参数生命周期在调用栈中有效
    let create_status =
        unsafe { (abi.instance_create.expect("create"))(lib, &inst_args, &mut inst) };
    assert_eq!(create_status, AV_OK);
    assert!(!inst.is_null());

    // 5. 帧推理
    let mock_frame = MockFrameBuilder::new()
        .dimensions(1920, 1080)
        .to_nv12(16)
        .build();

    // SAFETY: 实例有效，帧描述符符合 ABI 约束
    let process_status =
        unsafe { (abi.instance_process.expect("process"))(inst, mock_frame.raw_desc()) };
    assert_eq!(process_status, AV_OK);
    assert!(
        RESULT_COUNT.load(Ordering::SeqCst) > 0,
        "CPU fallback 应成功解析并触发 on_result 告警回调"
    );

    // 6. 动态更新配置
    let new_cfg = CString::new(r#"{"confidence_threshold": 0.70}"#).expect("cstring");
    // SAFETY: 实例有效，JSON 字符串为有效的 CString
    let update_status = unsafe {
        (abi.instance_update_config.expect("update"))(
            inst,
            new_cfg.as_ptr(),
            new_cfg.as_bytes().len() as u32,
        )
    };
    assert_eq!(update_status, AV_OK);

    // 7. 销毁实例与关闭库
    // SAFETY: 实例有效
    let destroy_status = unsafe { (abi.instance_destroy.expect("destroy"))(inst) };
    assert_eq!(destroy_status, AV_OK);

    // SAFETY: 库句柄有效
    let close_status = unsafe { (abi.library_close.expect("close"))(lib) };
    assert_eq!(close_status, AV_OK);

    let _ = std::fs::remove_dir_all(&temp_dir);
}
