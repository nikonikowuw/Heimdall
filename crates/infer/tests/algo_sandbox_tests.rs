//! 算法包六步沙箱校验与真实前向推理自测测试套件

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

/// 解析算法包目录，并要求其插件库已构建。
///
/// `lib/` 下的 `.so` / `.dylib` 是本地构建产物、不入版本库（见
/// [algo-sdk-guidelines](../../../docs/nuwa/backend/algo-sdk-guidelines.md)）。干净检出上包目录
/// 齐全但制品缺失，依赖真实制品的前向自测应带着原因跳过，而不是把「尚未构建」报成沙箱缺陷。
fn resolve_built_package(rel: &str, algorithm_id: &str) -> Option<std::path::PathBuf> {
    let pkg_path = resolve_path(rel)?;
    if infer::sandbox::find_entry_library(&pkg_path, algorithm_id).is_ok() {
        return Some(pkg_path);
    }
    eprintln!(
        "跳过 {rel}: 缺少插件库 lib/{algorithm_id}.*（不入库，请先在该包 workspace 执行 make）"
    );
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
    let Some(pkg_path) = resolve_built_package(
        "algo-packages/macos-arm64/general_detection",
        "general_detection",
    ) else {
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
    let Some(pkg_path) = resolve_built_package(
        "algo-packages/macos/arm64/general_detection",
        "general_detection",
    )
    .or_else(|| resolve_built_package("algo-packages/macos/arm64/yolo26n", "general_detection")) else {
        return;
    };

    // 运行重构后的纯 Rust yolo26n 算法包六步沙箱自检
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
    let Some(pkg_path) = resolve_built_package(
        "algo-packages/macos-arm64/general_detection",
        "general_detection",
    ) else {
        return;
    };
    let candidates = [
        Path::new("../../target/debug/heimdall"),
        Path::new("target/debug/heimdall"),
    ];
    if let Some(bin) = candidates.into_iter().find(|p| p.exists()) {
        if let Ok(canon) = bin.canonicalize() {
            std::env::set_var("HEIMDALL_BIN", canon);
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
    // 干净检出上没有任何已构建的 macOS 制品时 `scan_and_register` 必然扫出 0 个包，
    // 断言会退化成「没构建」的报错：跳过，构建后再跑才是有效覆盖。
    let any_built = [
        (
            "algo-packages/macos-arm64/general_detection",
            "general_detection",
        ),
        (
            "algo-packages/macos/arm64/general_detection",
            "general_detection",
        ),
    ]
    .iter()
    .any(|(rel, algorithm_id)| resolve_built_package(rel, algorithm_id).is_some());
    if !any_built {
        return;
    }

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
    let Some(pkg_path) = resolve_built_package(
        "algo-packages/macos-arm64/general_detection",
        "general_detection",
    ) else {
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

    // 检查基础文件存在性（`lib/` 是本地构建产物、不入版本库，故不在此断言）
    assert!(pkg_path.join("manifest.json").is_file());
    assert!(pkg_path.join("config.schema.json").is_file());
    assert!(pkg_path.join("testimage.jpg").is_file());
    assert!(pkg_path.join("model/yolov8n-640x384-rk3576.rknn").is_file());

    // 解析 manifest
    let manifest_str =
        std::fs::read_to_string(pkg_path.join("manifest.json")).expect("读取 manifest 失败");
    let manifest: infer::sandbox::AlgoManifest =
        serde_json::from_str(&manifest_str).expect("解析 manifest 失败");
    assert_eq!(manifest.algorithm_id, "general_detection");
    assert_eq!(manifest.version, "1.0.0");
    assert_eq!(manifest.platform_id, "linux-rknn");
    assert_eq!(normalize_platform_id(&manifest.platform_id), "linux-rknn");

    // 查找动态库入口；未构建该包时跳过制品相关检查，而不是把「没构建」当成结构缺陷。
    let Ok(entry) = infer::sandbox::find_entry_library(&pkg_path, &manifest.algorithm_id) else {
        eprintln!("跳过 rk3576 制品检查: lib/libgeneral_detection.so 尚未构建");
        return;
    };
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

#[tokio::test]
async fn test_algo_package_open_without_sandbox_self_test() {
    if infer::sandbox::current_platform_id() != "macos-arm64" {
        return;
    }
    let Some(pkg_path) = resolve_built_package(
        "algo-packages/macos-arm64/general_detection",
        "general_detection",
    ) else {
        return;
    };

    // 测试通过 open 直接打开已受信任/已落库的算法包，不重复触发沙箱六步推理自测
    let pkg = infer::package::AlgoPackage::open(&pkg_path).expect("AlgoPackage::open 应该成功打开");
    assert_eq!(pkg.manifest().algorithm_id, "general_detection");

    // 测试 AlgoRegistry::open_and_register
    let registry = infer::AlgoRegistry::new();
    let reg_pkg = registry
        .open_and_register(&pkg_path)
        .await
        .expect("open_and_register 应该成功");
    assert_eq!(reg_pkg.manifest().algorithm_id, "general_detection");
    assert!(registry.contains("general_detection").await);

    // 针对不存在路径测试快速失败
    let nonexistent = pkg_path.join("nonexistent_sub_path");
    assert!(infer::package::AlgoPackage::open(&nonexistent).is_err());
}

#[test]
fn test_sandbox_rejects_empty_and_null_byte_package_paths() {
    let empty_path = Path::new("");
    let res = AlgoSandbox::validate_package(empty_path, false);
    assert!(res.is_err());
    let err = res.expect_err("空路径必须报错");
    assert_eq!(err.error_code(), 30016);
    assert!(err.to_string().contains("1.路径防穿透与结构检查"));

    let null_path = Path::new("algo-packages/\0invalid");
    let res_null = AlgoSandbox::validate_package(null_path, false);
    assert!(res_null.is_err());
}

#[test]
fn test_sandbox_rejects_manifest_path_traversal_algorithm_id() {
    let temp_dir = std::env::temp_dir().join(format!(
        "heimdall_test_traversal_algo_id_{}",
        uuid::Uuid::now_v7().simple()
    ));
    std::fs::create_dir_all(temp_dir.join("lib")).expect("创建 lib 目录失败");
    std::fs::write(temp_dir.join("testimage.jpg"), b"fake_jpg_content")
        .expect("写入 testimage.jpg 失败");

    let malicious_ids = [
        "../../etc/passwd",
        "../something",
        "/etc/passwd",
        "foo/bar",
        "foo\\bar",
        "..",
        ".",
        "",
        "foo bar",
        "id_with_null\0_byte",
        "a_very_long_algorithm_id_exceeding_sixty_three_bytes_limit_c_abi_maximum_length_overflow",
    ];

    for bad_id in malicious_ids {
        let manifest_content = serde_json::json!({
            "manifest_version": 1,
            "algorithm_id": bad_id,
            "version": "1.0.0",
            "name": "Malicious Algo",
            "algorithm_type": "object_detection",
            "alarm_type_id": "region_invasion",
            "platform_id": infer::sandbox::current_platform_id(),
        });
        std::fs::write(
            temp_dir.join("manifest.json"),
            serde_json::to_string(&manifest_content).expect("序列化失败"),
        )
        .expect("写入 manifest.json 失败");

        let res = AlgoSandbox::validate_package(&temp_dir, false);
        assert!(
            res.is_err(),
            "必须拦截包含路径穿越的 algorithm_id: {bad_id}"
        );
        let err = res.expect_err("应当报错");
        assert!(
            err.to_string().contains("2.解析 Manifest 与平台匹配"),
            "错误必须发生在第 2 步: {err}"
        );

        // 验证 find_entry_library 直接拦截
        let entry_res = infer::sandbox::find_entry_library(&temp_dir, bad_id);
        assert!(
            entry_res.is_err(),
            "find_entry_library 必须拦截非法 algorithm_id: {bad_id}"
        );

        // 验证 AlgoPackage::open 直接拦截
        let open_res = infer::package::AlgoPackage::open(&temp_dir);
        assert!(
            open_res.is_err(),
            "AlgoPackage::open 必须拦截非法 algorithm_id: {bad_id}"
        );
    }

    let _ = std::fs::remove_dir_all(&temp_dir);
}

#[test]
fn test_sandbox_rejects_manifest_path_traversal_version() {
    let temp_dir = std::env::temp_dir().join(format!(
        "heimdall_test_traversal_version_{}",
        uuid::Uuid::now_v7().simple()
    ));
    std::fs::create_dir_all(temp_dir.join("lib")).expect("创建 lib 目录失败");
    std::fs::write(temp_dir.join("testimage.jpg"), b"fake_jpg_content")
        .expect("写入 testimage.jpg 失败");

    let malicious_versions = [
        "../../etc",
        "../something",
        "/etc",
        "1.0/2.0",
        "1.0\\2.0",
        "..",
        ".",
        "",
        "1.0.0 extra_space",
        "version_longer_than_thirty_one_bytes_c_abi_limit_overflow",
    ];

    for bad_ver in malicious_versions {
        let manifest_content = serde_json::json!({
            "manifest_version": 1,
            "algorithm_id": "valid_algo_id",
            "version": bad_ver,
            "name": "Malicious Version Algo",
            "algorithm_type": "object_detection",
            "alarm_type_id": "region_invasion",
            "platform_id": infer::sandbox::current_platform_id(),
        });
        std::fs::write(
            temp_dir.join("manifest.json"),
            serde_json::to_string(&manifest_content).expect("序列化失败"),
        )
        .expect("写入 manifest.json 失败");

        let res = AlgoSandbox::validate_package(&temp_dir, false);
        assert!(res.is_err(), "必须拦截包含路径穿越的 version: {bad_ver}");
        let err = res.expect_err("应当报错");
        assert!(
            err.to_string().contains("2.解析 Manifest 与平台匹配"),
            "错误必须发生在第 2 步: {err}"
        );

        let open_res = infer::package::AlgoPackage::open(&temp_dir);
        assert!(
            open_res.is_err(),
            "AlgoPackage::open 必须拦截非法 version: {bad_ver}"
        );
    }

    let _ = std::fs::remove_dir_all(&temp_dir);
}

#[cfg(unix)]
#[test]
fn test_sandbox_rejects_symlink_escape() {
    let temp_dir = std::env::temp_dir().join(format!(
        "heimdall_test_symlink_escape_{}",
        uuid::Uuid::now_v7().simple()
    ));
    let outside_dir = std::env::temp_dir().join(format!(
        "heimdall_test_outside_target_{}",
        uuid::Uuid::now_v7().simple()
    ));
    std::fs::create_dir_all(&temp_dir).expect("创建 temp_dir 失败");
    std::fs::create_dir_all(&outside_dir).expect("创建 outside_dir 失败");

    let outside_manifest = outside_dir.join("escaped_manifest.json");
    std::fs::write(&outside_manifest, b"{}").expect("写入 outside_manifest 失败");

    // 创建指向目录外部的 manifest.json 符号链接
    let symlink_manifest = temp_dir.join("manifest.json");
    std::os::unix::fs::symlink(&outside_manifest, &symlink_manifest)
        .expect("创建 symlink_manifest 失败");
    std::fs::create_dir_all(temp_dir.join("lib")).expect("创建 lib 目录失败");
    std::fs::write(temp_dir.join("testimage.jpg"), b"img").expect("写入 testimage.jpg 失败");

    let res = AlgoSandbox::validate_package(&temp_dir, false);
    assert!(res.is_err(), "必须拦截符号链接逃逸的 manifest.json");
    let err = res.expect_err("应当报错");
    assert!(
        err.to_string().contains("符号链接路径逃逸"),
        "错误信息必须明确提示符号链接路径逃逸: {err}"
    );

    let _ = std::fs::remove_dir_all(&temp_dir);
    let _ = std::fs::remove_dir_all(&outside_dir);
}
