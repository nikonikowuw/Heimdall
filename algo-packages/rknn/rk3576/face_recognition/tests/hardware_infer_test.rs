//! RK3576 真实 NPU 硬件前向推理与模型初始化集成测试
//!
//! 在具备 RKNN NPU 硬件和 librknnrt.so 的目标板（RK3576）上运行：
//! `cargo test -p face-recognition-rknn -- --ignored`

use std::path::Path;

use face_recognition::align::align_face;
use face_recognition::{cosine_similarity, prepare_detector_input_for, shared_models};

#[test]
#[ignore = "需要物理 RK3576 NPU 硬件和 librknnrt.so 环境"]
fn test_rknn_hardware_face_detection_and_embedding() {
    let pkg_root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let detector_model = pkg_root.join("model/yolov8n-face-640x384_mixed_face.rknn");
    let embedder_model = pkg_root.join("model/edgeface_xs_gamma_06_rk3576_fp16.rknn");
    let test_image_path = pkg_root.join("testimage.jpg");

    assert!(
        detector_model.is_file(),
        "检测模型文件不存在: {:?}",
        detector_model
    );
    assert!(
        embedder_model.is_file(),
        "特征模型文件不存在: {:?}",
        embedder_model
    );
    assert!(
        test_image_path.is_file(),
        "测试图片不存在: {:?}",
        test_image_path
    );

    // 1. 加载双模型
    let models = shared_models(pkg_root).expect("加载 RKNN 模型失败");

    // 2. 读取测试图像
    let image = image::open(&test_image_path)
        .expect("读取 testimage.jpg 失败")
        .to_rgb8();
    let (orig_w, orig_h) = (image.width(), image.height());
    assert!(orig_w > 0 && orig_h > 0);

    // 3. 预处理与检测推理
    let (detector_rgb, layout) =
        prepare_detector_input_for(&image, models.detector_width, models.detector_height)
            .expect("detector 预处理失败");
    assert_eq!(
        detector_rgb.len(),
        (models.detector_width * models.detector_height * 3) as usize
    );

    let faces = models
        .worker
        .detect_host(detector_rgb, layout, 0.25)
        .expect("人脸检测推理失败");

    assert!(!faces.is_empty(), "在 testimage.jpg 中未检出任何有效人脸");

    // 验证检测框与关键点归一化范围
    let best_face = faces
        .iter()
        .max_by(|a, b| a.score.total_cmp(&b.score))
        .expect("应存在置信度最高的人脸");
    assert!(best_face.score >= 0.25);
    assert!(best_face.bbox[0] >= 0.0 && best_face.bbox[0] <= 1.0);
    assert!(best_face.bbox[1] >= 0.0 && best_face.bbox[1] <= 1.0);
    assert!(best_face.bbox[2] > 0.0 && best_face.bbox[2] <= 1.0);
    assert!(best_face.bbox[3] > 0.0 && best_face.bbox[3] <= 1.0);

    for landmark in &best_face.landmarks {
        assert!(landmark[0] >= 0.0 && landmark[0] <= 1.0);
        assert!(landmark[1] >= 0.0 && landmark[1] <= 1.0);
    }

    // 4. 对齐人脸
    let aligned =
        align_face(image.as_raw(), orig_w, orig_h, &best_face.landmarks).expect("人脸仿射对齐失败");
    assert_eq!(aligned.len(), 112 * 112 * 3);

    // 5. 提取特征嵌入
    let embedding = models.worker.embed_host(aligned).expect("特征提取推理失败");

    // 6. 验证特征向量
    assert_eq!(embedding.len(), 512);
    let l2_norm = embedding.iter().map(|x| x * x).sum::<f32>().sqrt();
    assert!(
        (l2_norm - 1.0).abs() < 1e-4,
        "归一化后的特征向量模长应为 1.0, 实际: {l2_norm}"
    );

    // 余弦自相似度
    let self_similarity = cosine_similarity(&embedding, &embedding);
    assert!(
        (self_similarity - 1.0).abs() < 1e-5,
        "自身余弦相似度应为 1.0, 实际: {self_similarity}"
    );
}
