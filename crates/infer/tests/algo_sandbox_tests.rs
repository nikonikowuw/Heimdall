//! 算法包七步沙箱校验与真实前向推理自测测试套件

use infer::package::AlgoRegistry;
use infer::sandbox::{normalize_platform_id, AlgoSandbox};
use std::path::Path;

fn resolve_path(rel: &str) -> Option<std::path::PathBuf> {
    let p1 = Path::new(rel);
    if p1.exists() {
        return Some(p1.to_path_buf());
    }
    let p2 = Path::new("../../").join(rel);
    if p2.exists() {
        return Some(p2);
    }
    None
}

#[test]
fn test_platform_id_normalization() {
    assert_eq!(normalize_platform_id("macos-arm64"), "macos-arm64");
    assert_eq!(normalize_platform_id("macos-arm64-coreml"), "macos-arm64");
    assert_eq!(normalize_platform_id("darwin-arm64"), "macos-arm64");
    assert_eq!(normalize_platform_id("linux-rknn"), "linux-rknn");
    assert_eq!(normalize_platform_id("linux-arm64-rknn"), "linux-rknn");
    assert_eq!(normalize_platform_id("rknn"), "linux-rknn");
    assert_eq!(normalize_platform_id("linux-ascend"), "linux-ascend");
    assert_eq!(normalize_platform_id("linux-arm64-ascend"), "linux-ascend");
    assert_eq!(normalize_platform_id("ascend"), "linux-ascend");
    assert_eq!(normalize_platform_id("linux-x64"), "linux-x64");
}

#[test]
fn test_sandbox_rejects_nonexistent_path() {
    let bad_path = Path::new("algo-packages/nonexistent_algo_package_foo_bar");
    let res = AlgoSandbox::validate_package(bad_path, false);
    assert!(res.is_err());
}

#[test]
fn test_sandbox_real_package_in_process_self_test() {
    if infer::sandbox::current_platform_id() != "macos-arm64" {
        return;
    }
    let Some(pkg_path) = resolve_path("algo-packages/macos-arm64/general_detection") else {
        return;
    };

    // 运行真实前向推理自测（进程内模式）
    let res = AlgoSandbox::validate_package(&pkg_path, false);
    assert!(res.is_ok(), "算法包沙箱校验失败: {:?}", res.err());

    let manifest = res.expect("算法包应当成功解出 manifest");
    assert_eq!(manifest.algorithm_id, "general_detection");
    assert_eq!(manifest.version, "1.0.0");
    assert_eq!(manifest.algorithm_type, "object_detection");
}

#[test]
fn test_sandbox_rust_yolo26n_package_in_process_self_test() {
    if infer::sandbox::current_platform_id() != "macos-arm64" {
        return;
    }
    let Some(pkg_path) = resolve_path("algo-packages/macos/arm64/general_detection")
        .or_else(|| resolve_path("algo-packages/macos/arm64/yolo26n"))
    else {
        return;
    };

    // 运行重构后的纯 Rust yolo26n 算法包七步沙箱自检
    let res = AlgoSandbox::validate_package(&pkg_path, false);
    assert!(
        res.is_ok(),
        "Rust yolo26n 算法包沙箱校验失败: {:?}",
        res.err()
    );

    let manifest = res.expect("算法包应当成功解出 manifest");
    assert_eq!(manifest.algorithm_id, "general_detection");
    assert_eq!(manifest.version, "1.0.0");
    assert_eq!(manifest.algorithm_type, "object_detection");
}

#[test]
fn test_sandbox_subprocess_self_test() {
    if infer::sandbox::current_platform_id() != "macos-arm64" {
        return;
    }
    let Some(pkg_path) = resolve_path("algo-packages/macos-arm64/general_detection") else {
        return;
    };
    let candidates = [
        Path::new("../../target/debug/heimdall"),
        Path::new("target/debug/heimdall"),
    ];
    if let Some(bin) = candidates.into_iter().find(|p| p.exists()) {
        if let Ok(canon) = bin.canonicalize() {
            std::env::set_var("ARGUS_BIN", canon);
            let res = AlgoSandbox::validate_package(&pkg_path, true);
            assert!(res.is_ok(), "子进程物理隔离自检失败: {:?}", res.err());
        }
    }
}

#[tokio::test]
async fn test_algo_package_load_and_registry_lifecycle() {
    if infer::sandbox::current_platform_id() != "macos-arm64" {
        return;
    }
    let Some(base_dir) = resolve_path("algo-packages") else {
        return;
    };

    let registry = AlgoRegistry::new();
    let count = registry
        .scan_and_register(&base_dir, false)
        .await
        .expect("扫描并注册算法包失败");
    assert!(count >= 1, "应至少扫描并注册一个算法包");

    let pkg = registry.get("general_detection").await;
    assert!(pkg.is_some(), "应能获取到 general_detection 算法包");

    let pkg = pkg.expect("general_detection 算法包必须存在");
    assert_eq!(pkg.manifest().algorithm_id, "general_detection");

    let valid_cfg = serde_json::json!({
        "confidence_threshold": 0.45,
        "iou_threshold": 0.45,
        "target_classes": ["person", "car"]
    })
    .to_string();

    // 创建推理实例
    let inst_res = pkg.create_instance("test_cam_1", Some(&valid_cfg));
    assert!(inst_res.is_ok(), "创建算法实例失败: {:?}", inst_res.err());

    let inst = inst_res.expect("创建算法实例失败");
    assert_eq!(infer::InferenceBackend::name(&inst), "C-ABI-AlgoInstance");
}

#[tokio::test]
async fn test_algo_instance_detect_with_real_frame() {
    if infer::sandbox::current_platform_id() != "macos-arm64" {
        return;
    }
    let Some(pkg_path) = resolve_path("algo-packages/macos-arm64/general_detection") else {
        return;
    };

    let pkg =
        infer::package::AlgoPackage::load_and_verify(&pkg_path, false).expect("加载算法包失败");
    let pkg = std::sync::Arc::new(pkg);
    let valid_cfg = serde_json::json!({
        "confidence_threshold": 0.45,
        "iou_threshold": 0.45,
        "target_classes": ["person", "car"]
    })
    .to_string();

    let inst = pkg
        .create_instance("test_cam_2", Some(&valid_cfg))
        .expect("创建实例失败");

    // 加载自测图并构建 FrameRef
    let testimage_path = pkg_path.join("testimage.jpg");
    let img = image::open(&testimage_path).expect("读取 testimage.jpg 失败");
    let rgb = img.to_rgb8();
    let (width, height) = rgb.dimensions();
    let rgb_raw = rgb.into_raw();

    #[cfg(target_os = "macos")]
    let pixel_buf =
        infer::c_abi::cvpixelbuffer::NativePixelBuffer::from_rgb_to_nv12(&rgb_raw, width, height)
            .expect("转换 CVPixelBuffer 失败");

    #[cfg(target_os = "macos")]
    let frame = {
        let (ys, _uvs) = pixel_buf.strides();
        let raw_ptr = pixel_buf.into_raw();
        let ptr = std::ptr::NonNull::new(raw_ptr).expect("非空指针");
        types::FrameRef::new(
            "test_cam_2".to_string(),
            1234567890123,
            width & !1,
            height & !1,
            types::StrideInfo::new(ys as u32, height & !1),
            types::PixelFormat::Nv12,
            types::FrameHandle::ApplePixelBuffer { ptr },
        )
    };

    #[cfg(not(target_os = "macos"))]
    let frame = types::FrameRef::new(
        "test_cam_2".to_string(),
        1234567890123,
        width & !1,
        height & !1,
        types::StrideInfo::new(width, height),
        types::PixelFormat::Nv12,
        types::FrameHandle::Host(std::sync::Arc::from(rgb_raw.into_boxed_slice())),
    );

    use infer::InferenceBackend;
    let detections = inst.detect(&frame).await.expect("推理调用失败");
    assert!(
        !detections.is_empty(),
        "推理结果不应为空，回调必须成功收集并解析目标检测框"
    );
}

#[test]
fn test_rknn_rk3576_package_structure_and_sandbox_guards() {
    let Some(pkg_path) = resolve_path("algo-packages/rknn/rk3576/general_detection") else {
        return;
    };

    // 检查基础文件存在性
    assert!(pkg_path.join("manifest.json").is_file());
    assert!(pkg_path.join("config.schema.json").is_file());
    assert!(pkg_path.join("testimage.jpg").is_file());
    assert!(pkg_path.join("model/yolov8n-640x384-rk3576.rknn").is_file());
    assert!(pkg_path.join("lib/libgeneral_detection.so").is_file());

    // 解析 manifest
    let manifest_str =
        std::fs::read_to_string(pkg_path.join("manifest.json")).expect("读取 manifest 失败");
    let manifest: infer::sandbox::AlgoManifest =
        serde_json::from_str(&manifest_str).expect("解析 manifest 失败");
    assert_eq!(manifest.algorithm_id, "general_detection");
    assert_eq!(manifest.version, "1.0.0");
    assert_eq!(manifest.platform_id, "linux-rknn");
    assert_eq!(normalize_platform_id(&manifest.platform_id), "linux-rknn");

    // 查找动态库入口
    let entry = infer::sandbox::find_entry_library(&pkg_path, &manifest.algorithm_id)
        .expect("查找动态库失败");
    assert!(entry.ends_with("libgeneral_detection.so"));

    // 沙箱平台防护测试：在非 rknn 宿主环境上执行校验时，应精准拦截平台不匹配错误
    let res = AlgoSandbox::validate_package(&pkg_path, false);
    if infer::sandbox::current_platform_id() != "linux-rknn" {
        assert!(res.is_err());
        let err_msg = format!("{:?}", res.err());
        assert!(
            err_msg.contains("平台架构不匹配"),
            "错误信息应包含平台不匹配提示: {}",
            err_msg
        );
    } else {
        assert!(
            res.is_ok(),
            "Sandbox validation on linux-rknn failed: {:?}",
            res.err()
        );
    }
}
