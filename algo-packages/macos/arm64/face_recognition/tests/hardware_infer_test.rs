#[cfg(target_os = "macos")]
#[test]
#[ignore = "需要 macOS 14+、Apple Silicon 和两个 CoreML 模型包"]
fn hardware_inference_requires_real_models() {
    let package_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    assert!(package_root.join("model/yolov5n_face.mlpackage").exists());
    assert!(package_root.join("model/edgeface_s.mlpackage").exists());
}

#[cfg(target_os = "macos")]
#[test]
#[ignore = "需要 macOS 14+、Apple Silicon 和两个 CoreML 模型包"]
fn library_hooks_load_and_release_coreml_models() {
    use std::ffi::CString;

    use algo_sdk::c_abi::{AvAlgoLibraryArgs, AV_ALGO_API_VERSION};
    use face_recognition_coreml::av_algo_get_abi;

    let package_root = CString::new(env!("CARGO_MANIFEST_DIR")).expect("包路径应无 NUL");
    let platform_id = CString::new("macos-arm64-coreml").expect("平台标识应无 NUL");
    let args = AvAlgoLibraryArgs {
        size: std::mem::size_of::<AvAlgoLibraryArgs>() as u32,
        api_version: AV_ALGO_API_VERSION,
        package_root: package_root.as_ptr(),
        platform_id: platform_id.as_ptr(),
        platform_tag: 0,
        log: None,
        log_user: std::ptr::null_mut(),
    };
    let mut library = std::ptr::null_mut();
    // SAFETY: ABI 指针指向当前测试作用域内保持有效的参数和句柄槽位。
    let abi = unsafe { &*av_algo_get_abi(AV_ALGO_API_VERSION) };
    let open = abi.library_open.expect("library_open 应导出");
    let close = abi.library_close.expect("library_close 应导出");
    // SAFETY: args 和 library 满足 library_open 的 ABI 约束。
    assert_eq!(unsafe { open(&args, &mut library) }, 0);
    assert!(!library.is_null());
    // SAFETY: library 由上面的 library_open 创建，且尚未释放。
    assert_eq!(unsafe { close(library) }, 0);
}

#[cfg(target_os = "macos")]
#[test]
#[ignore = "需要 macOS 14+、Apple Silicon 和两个 CoreML 模型包"]
fn test_face_extraction_and_cosine_similarity() {
    use std::ffi::CString;

    use algo_sdk::c_abi::{
        AvAlgoLibraryArgs, AvFaceExtractInput, AvFaceExtractOutput, AV_ALGO_API_VERSION,
    };
    use face_recognition_coreml::{av_algo_extract_face, av_algo_get_abi, cosine_similarity};

    let package_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let image_bytes =
        std::fs::read(package_root.join("testimage.jpg")).expect("读取测试图片应成功");

    let package_root_c = CString::new(env!("CARGO_MANIFEST_DIR")).expect("包路径应无 NUL");
    let platform_id_c = CString::new("macos-arm64-coreml").expect("平台标识应无 NUL");
    let args = AvAlgoLibraryArgs {
        size: std::mem::size_of::<AvAlgoLibraryArgs>() as u32,
        api_version: AV_ALGO_API_VERSION,
        package_root: package_root_c.as_ptr(),
        platform_id: platform_id_c.as_ptr(),
        platform_tag: 0,
        log: None,
        log_user: std::ptr::null_mut(),
    };
    let mut library = std::ptr::null_mut();
    // SAFETY: ABI 指针指向当前测试作用域内保持有效的参数和句柄槽位。
    let abi = unsafe { &*av_algo_get_abi(AV_ALGO_API_VERSION) };
    let open = abi.library_open.expect("library_open 应导出");
    let close = abi.library_close.expect("library_close 应导出");
    // SAFETY: args 和 library 指针在同步调用期间保持有效。
    assert_eq!(unsafe { open(&args, &mut library) }, 0);
    assert!(!library.is_null());

    let input = AvFaceExtractInput {
        size: std::mem::size_of::<AvFaceExtractInput>() as u32,
        api_version: AV_ALGO_API_VERSION,
        image_bytes: image_bytes.as_ptr(),
        image_bytes_len: image_bytes.len() as u32,
        min_detection_score: 0.5,
        min_face_size: 30.0,
        min_quality_score: 0.3,
        reserved: 0,
    };

    // SAFETY: AvFaceExtractOutput 是纯 POD 内存布局，零初始化符合 C ABI 约定。
    let mut output = unsafe { std::mem::zeroed::<AvFaceExtractOutput>() };
    output.size = std::mem::size_of::<AvFaceExtractOutput>() as u32;
    output.api_version = AV_ALGO_API_VERSION;

    // SAFETY: input 与 output 在同步调用期间保持有效内存。
    let status = unsafe { av_algo_extract_face(library, &input, &mut output) };
    assert_eq!(status, 0, "extract_face 应成功返回 0");
    assert_eq!(output.status_code, 0);
    assert_eq!(output.embedding_dim, 512);
    assert!(output.aligned_jpeg_len > 0);
    assert!(output.quality_score > 0.3);

    // 验证 L2 范数约为 1.0
    let l2_norm: f32 = output.embedding.iter().map(|v| v * v).sum::<f32>().sqrt();
    assert!(
        (l2_norm - 1.0).abs() < 1e-4,
        "特征向量 L2 范数应接近 1.0: {l2_norm}"
    );

    // 验证自身与自身余弦相似度为 1.0
    let self_sim = cosine_similarity(&output.embedding, &output.embedding);
    assert!(
        (self_sim - 1.0).abs() < 1e-5,
        "同一特征余弦相似度应为 1.0: {self_sim}"
    );

    // SAFETY: library 由上面的 library_open 创建，且尚未释放。
    assert_eq!(unsafe { close(library) }, 0);
}

#[cfg(not(target_os = "macos"))]
#[test]
fn hardware_inference_is_not_run_on_non_macos() {}
