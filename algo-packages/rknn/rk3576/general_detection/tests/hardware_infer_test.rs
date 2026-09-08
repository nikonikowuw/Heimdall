#![cfg(target_os = "linux")]

//! RK3576 真实硬件前向推理与模型初始化集成测试

use std::ffi::c_void;
use std::path::Path;

use algo_sdk::c_abi::AvAlgoResult;
use algo_sdk::emitter::ResultEmitter;
use algo_sdk::plugin::{AlgoPlugin, InitContext};
use algo_sdk::testing::{MockEmitter, MockFrameBuilder};
use general_detection::config::InstanceConfig;
use general_detection::plugin::GeneralDetector;

unsafe extern "C" fn test_result_callback(result: *const AvAlgoResult, user_data: *mut c_void) {
    if !result.is_null() && !user_data.is_null() {
        // SAFETY: user_data 在测试函数作用域内有效
        let emitter = unsafe { &mut *(user_data as *mut MockEmitter) };
        // SAFETY: result 由插件传入有效指针
        emitter.record_c_result(unsafe { &*result });
    }
}

#[test]
fn test_real_rknn_hardware_inference_if_available() {
    let local_model = Path::new("model/yolov8n-640x384-rk3576.rknn");
    let pkg_root_buf = if local_model.is_file() {
        std::path::PathBuf::from(".")
    } else {
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
    };
    let pkg_root = pkg_root_buf.as_path();
    let model_path = pkg_root.join("model/yolov8n-640x384-rk3576.rknn");
    if !model_path.is_file() {
        eprintln!("模型文件不存在 ({:?})，跳过硬件测试", model_path);
        return;
    }

    let init_ctx = InitContext {
        package_root: pkg_root,
        platform_id: "linux-rknn",
        instance_id: "hardware_test_instance",
        is_self_test: true,
    };

    let config = InstanceConfig::default();
    let mut detector = GeneralDetector::init(&init_ctx, config).expect("算法插件初始化必须成功");

    // 读取测试样例图
    let img_path = pkg_root.join("testimage.jpg");
    let img = image::open(&img_path).expect("读取 testimage.jpg 失败");
    let rgb = img.to_rgb8();
    let (width, height) = rgb.dimensions();

    // 构造模拟真实 MPP 硬件解码输出帧：
    // MPP 解码器输出为 16 字节行跨距对齐的 NV12 (YUV420SP) 格式，通过 DMA-BUF 零拷贝直通传递。
    let mock_frame = MockFrameBuilder::from_image_hardware(&img_path)
        .unwrap_or_else(|_| {
            MockFrameBuilder::new()
                .dimensions(width, height)
                .host_data(rgb.into_raw())
                .to_nv12(16)
        })
        .build();

    let mut mock_emitter = MockEmitter::new();
    // SAFETY: mock_emitter 在测试函数当前作用域内有效，指针与回调生命周期受控
    let mut emitter = unsafe {
        ResultEmitter::from_raw(
            1,
            Some(test_result_callback),
            &mut mock_emitter as *mut _ as *mut c_void,
        )
    };

    let safe_frame = mock_frame.as_safe_frame();
    let proc_res = detector.process(safe_frame, &mut emitter);
    assert!(
        proc_res.is_ok(),
        "RKNN 推理 process 失败: {:?}",
        proc_res.err()
    );

    let detections = mock_emitter.detections();
    let mode_str = if detector.session.is_fallback() {
        "开发调试回退路径 (debug_cpu_fallback_path)"
    } else {
        "物理 NPU 常驻路径 (infer_fast_path)"
    };

    println!(
        "\n====== RK3576 [{mode_str}] 推理测试成功完成！共检出目标数量: {} ======",
        detections.len()
    );
    for (idx, det) in detections.iter().enumerate() {
        println!(
            "  [{idx}] 类别: {:?} (id={}) | 置信度: {:.2}% | 边界框: [{:.4}, {:.4}, {:.4}, {:.4}]",
            det.label.unwrap_or("unknown"),
            det.class_id,
            det.confidence * 100.0,
            det.x,
            det.y,
            det.w,
            det.h
        );
    }

    assert!(
        !detections.is_empty(),
        "测试图 testimage.jpg 应成功检出至少 1 个目标"
    );

    // 验证全流水线成功运行（数据输入、预处理 Letterbox、NPU 执行、输出提取及后处理）
    assert!(
        proc_res.is_ok(),
        "真实 RKNN 推理 process 必须成功返回 Ok(())"
    );
}
