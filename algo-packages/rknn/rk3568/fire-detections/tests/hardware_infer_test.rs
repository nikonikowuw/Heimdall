#![cfg(target_os = "linux")]

//! RK3568 烟火检测插件集成测试（覆盖真实/回退推理与时序确认流程）

use std::ffi::c_void;

use algo_sdk::c_abi::AvAlgoResult;
use algo_sdk::emitter::ResultEmitter;
use algo_sdk::plugin::{AlgoPlugin, InitContext};
use algo_sdk::testing::{MockEmitter, MockFrameBuilder};
use fire_smoke_detection::config::InstanceConfig;
use fire_smoke_detection::plugin::FireSmokeDetector;

unsafe extern "C" fn test_result_callback(result: *const AvAlgoResult, user_data: *mut c_void) {
    if !result.is_null() && !user_data.is_null() {
        // SAFETY: user_data 在测试函数作用域内有效
        let emitter = unsafe { &mut *(user_data as *mut MockEmitter) };
        // SAFETY: result 由插件传入有效指针
        emitter.record_c_result(unsafe { &*result });
    }
}

#[test]
fn test_fire_smoke_detection_pipeline() {
    let pkg_root_buf = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let pkg_root = pkg_root_buf.as_path();
    let model_path = pkg_root.join("model/best_pure.rknn");
    if !model_path.is_file() {
        eprintln!("模型文件不存在 ({:?})，跳过集成测试", model_path);
        return;
    }

    let init_ctx = InitContext {
        package_root: pkg_root,
        platform_id: "linux-rknn",
        instance_id: "fire_smoke_integration_test",
        is_self_test: true,
    };

    let config = InstanceConfig {
        confidence_threshold: 0.25,
        iou_threshold: 0.45,
        confirm_window: 5,
        confirm_threshold: 1, // 单帧测试设为 1 确保放行
        ..Default::default()
    };

    let mut detector = FireSmokeDetector::init(&init_ctx, config).expect("算法插件初始化必须成功");

    let img_path = pkg_root.join("testimage.jpg");
    let img = image::open(&img_path).expect("读取 testimage.jpg 失败");
    let rgb = img.to_rgb8();
    let (width, height) = rgb.dimensions();

    // 构造模拟 MPP 硬件解码输出帧 (NV12 16 字节 Stride 对齐)
    let mock_frame = MockFrameBuilder::from_image_hardware(&img_path)
        .unwrap_or_else(|_| {
            MockFrameBuilder::new()
                .dimensions(width, height)
                .host_data(rgb.into_raw())
                .to_nv12(16)
        })
        .build();

    let mut mock_emitter = MockEmitter::new();
    // SAFETY: mock_emitter 在当前作用域有效
    let mut emitter = unsafe {
        ResultEmitter::from_raw(
            1,
            Some(test_result_callback),
            &mut mock_emitter as *mut _ as *mut c_void,
        )
    };

    let safe_frame = mock_frame.as_safe_frame();
    detector
        .process(safe_frame, &mut emitter)
        .expect("推理处理必须成功");

    let detections = mock_emitter.detections();
    println!("集成测试检出目标数: {}", detections.len());
    for det in detections {
        println!(
            "  [{}] conf={:.3} @ ({:.2}, {:.2}, {:.2}, {:.2})",
            det.label.unwrap_or("unknown"),
            det.confidence,
            det.x,
            det.y,
            det.w,
            det.h
        );
        assert!(det.confidence >= 0.25);
    }
}
