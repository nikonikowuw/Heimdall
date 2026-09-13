#[cfg(target_os = "macos")]
#[test]
#[ignore = "需要 macOS 14+、Apple Silicon 和三个 CoreML 模型包"]
fn hardware_inference_requires_real_models() {
    let package_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    assert!(package_root.join("model/yolov8_face.mlpackage").exists());
    assert!(package_root.join("model/yolo26n.mlpackage").exists());
    assert!(package_root.join("model/edgeface_s.mlpackage").exists());
}

#[cfg(target_os = "macos")]
#[test]
#[ignore = "需要 macOS 14+、Apple Silicon 和三个 CoreML 模型包"]
fn test_device_side_embedding_keeps_pixelbuffer_path() {
    use face_recognition_coreml::align::{align_face, face_alignment_matrix};
    use face_recognition_coreml::coreml::{CoreMlFaceModels, OwnedPixelBuffer};
    use face_recognition_coreml::detect::{decode_face_detections, nms, unmap_letterbox};
    use face_recognition_coreml::{cosine_similarity, normalize_embedding, prepare_detector_input};

    let package_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let image = image::open(package_root.join("testimage.jpg"))
        .expect("读取测试图片应成功")
        .to_rgb8();
    let (width, height) = image.dimensions();
    let models = CoreMlFaceModels::load(package_root).expect("CoreML 模型应加载成功");

    let (detector_rgb, detector_mode) = prepare_detector_input(&image).expect("检测预处理应成功");
    let detector_surface =
        OwnedPixelBuffer::from_rgb(&detector_rgb, 640, 384).expect("创建检测 CVPixelBuffer 应成功");
    // SAFETY: detector_surface 在同步预测完成前保持所有权。
    let raw_output =
        unsafe { models.predict_detector(detector_surface.as_ptr()) }.expect("人脸检测应成功");
    let mut faces = decode_face_detections(&raw_output, 0.5);
    nms(&mut faces, 0.45);
    unmap_letterbox(&mut faces, &detector_mode, width, height);
    let face = faces
        .into_iter()
        .max_by(|left, right| left.score.total_cmp(&right.score))
        .expect("测试图应至少包含一张人脸");

    let matrix =
        face_alignment_matrix(width, height, &face.landmarks).expect("人脸关键点应能生成仿射矩阵");
    let source = OwnedPixelBuffer::from_rgb(image.as_raw(), width, height)
        .expect("创建源 CVPixelBuffer 应成功");
    // SAFETY: source 在同步调用完成前保持所有权；模型不会保存该输入 surface。
    let device_values = unsafe {
        models
            .predict_embedding_from_pixelbuffer(source.as_ptr(), width, height, matrix)
            .expect("设备侧 Core Image -> CoreML 特征提取应成功")
    };
    let device_embedding = normalize_embedding(&device_values).expect("设备 embedding 应有效");

    let aligned =
        align_face(image.as_raw(), width, height, &face.landmarks).expect("CPU 对齐回归路径应成功");
    let cpu_values = models
        .predict_embedding(&aligned)
        .expect("CPU 对齐后的 EdgeFace 应成功");
    let cpu_embedding = normalize_embedding(&cpu_values).expect("CPU embedding 应有效");

    let similarity = cosine_similarity(&device_embedding, &cpu_embedding);
    assert!(
        similarity > 0.90,
        "设备 affine 与 CPU affine 的 embedding 相似度过低: {similarity}"
    );
}

#[cfg(target_os = "macos")]
#[test]
#[ignore = "需要 macOS 14+、Apple Silicon 和三个 CoreML 模型包"]
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
#[ignore = "需要 macOS 14+、Apple Silicon 和三个 CoreML 模型包"]
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
    assert!(!output.embedding.is_null());
    assert!(!output.aligned_jpeg.is_null());
    assert!(output.aligned_jpeg_len > 0);
    assert!(output.quality_score > 0.3);

    // SAFETY: extract_face 成功返回后，embedding 保证指向有效的 embedding_dim 个 f32 浮点数。
    let embedding_slice =
        unsafe { std::slice::from_raw_parts(output.embedding, output.embedding_dim as usize) };

    // 验证 L2 范数约为 1.0
    let l2_norm: f32 = embedding_slice.iter().map(|v| v * v).sum::<f32>().sqrt();
    assert!(
        (l2_norm - 1.0).abs() < 1e-4,
        "特征向量 L2 范数应接近 1.0: {l2_norm}"
    );

    // 验证自身与自身余弦相似度为 1.0
    let embedding_array: &[f32; 512] = embedding_slice.try_into().expect("512 dims");
    let self_sim = cosine_similarity(embedding_array, embedding_array);
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
