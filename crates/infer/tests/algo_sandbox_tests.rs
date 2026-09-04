//! 算法包七步沙箱校验与真实前向推理自测测试套件

use infer::package::AlgoRegistry;
use infer::sandbox::{normalize_platform_id, AlgoSandbox};
use std::path::Path;

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
    let pkg_path = Path::new("algo-packages/macos-arm64/general_detection");
    if !pkg_path.exists() {
        return;
    }

    // 运行真实前向推理自测（进程内模式）
    let res = AlgoSandbox::validate_package(pkg_path, false);
    assert!(res.is_ok(), "算法包沙箱校验失败: {:?}", res.err());

    let manifest = res.expect("算法包应当成功解出 manifest");
    assert_eq!(manifest.algorithm_id, "general_detection");
    assert_eq!(manifest.version, "1.0.0");
    assert_eq!(manifest.algorithm_type, "object_detection");
}

#[test]
fn test_sandbox_subprocess_self_test() {
    let pkg_path = Path::new("algo-packages/macos-arm64/general_detection");
    if !pkg_path.exists() {
        return;
    }
    let candidates = [
        Path::new("../../target/debug/argus"),
        Path::new("target/debug/argus"),
    ];
    if let Some(bin) = candidates.into_iter().find(|p| p.exists()) {
        if let Ok(canon) = bin.canonicalize() {
            std::env::set_var("ARGUS_BIN", canon);
            let res = AlgoSandbox::validate_package(pkg_path, true);
            assert!(res.is_ok(), "子进程物理隔离自检失败: {:?}", res.err());
        }
    }
}

#[tokio::test]
async fn test_algo_package_load_and_registry_lifecycle() {
    let base_dir = Path::new("algo-packages");
    if !base_dir.exists() {
        return;
    }

    let registry = AlgoRegistry::new();
    let count = registry
        .scan_and_register(base_dir, false)
        .await
        .expect("扫描并注册算法包失败");
    assert!(count >= 1, "应至少扫描并注册一个算法包");

    let pkg = registry.get("general_detection").await;
    assert!(pkg.is_some(), "应能获取到 general_detection 算法包");

    let pkg = pkg.expect("general_detection 算法包必须存在");
    assert_eq!(pkg.manifest().algorithm_id, "general_detection");

    // 创建推理实例
    let inst_res = pkg.create_instance("test_cam_1", None);
    assert!(inst_res.is_ok(), "创建算法实例失败: {:?}", inst_res.err());

    let inst = inst_res.expect("创建算法实例失败");
    assert_eq!(infer::InferenceBackend::name(&inst), "C-ABI-AlgoInstance");
}

#[tokio::test]
async fn test_algo_instance_detect_with_real_frame() {
    let pkg_path = Path::new("algo-packages/macos-arm64/general_detection");
    if !pkg_path.exists() {
        return;
    }

    let pkg =
        infer::package::AlgoPackage::load_and_verify(pkg_path, false).expect("加载算法包失败");
    let pkg = std::sync::Arc::new(pkg);
    let inst = pkg
        .create_instance("test_cam_2", None)
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
        let (ys, vs) = pixel_buf.strides();
        let ptr = std::ptr::NonNull::new(pixel_buf.as_raw()).expect("非空指针");
        types::FrameRef::new(
            "test_cam_2".to_string(),
            1234567890123,
            width & !1,
            height & !1,
            types::StrideInfo::new(ys as u32, vs as u32),
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
fn test_sha256_computation() {
    let manifest_path = Path::new("algo-packages/macos-arm64/general_detection/manifest.json");
    if manifest_path.exists() {
        let hash = infer::sandbox::compute_file_sha256(manifest_path).expect("计算 SHA256 失败");
        assert_eq!(hash.len(), 64);
    }
}
