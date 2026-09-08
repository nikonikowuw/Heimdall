//! 本地测试入口：加载 RKNN 模型 → 读图 → 检测 + 嵌入提取 → 输出结果
//!
//! 用法: `cargo run -p face-recognition-rknn --bin face_recognition_run_local [image_path]`

use std::env;
use std::path::{Path, PathBuf};
use std::time::Instant;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let env_filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info"));
    tracing_subscriber::fmt().with_env_filter(env_filter).init();

    let args: Vec<String> = env::args().collect();
    let package_root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let default_image = package_root.join("testimage.jpg");
    let image_path = args
        .iter()
        .find(|arg| {
            !arg.starts_with("--")
                && (arg.ends_with(".jpg") || arg.ends_with(".jpeg") || arg.ends_with(".png"))
        })
        .map(PathBuf::from)
        .unwrap_or(default_image);

    let loops: usize = args
        .iter()
        .position(|arg| arg == "--loops")
        .and_then(|i| args.get(i + 1)?.parse().ok())
        .unwrap_or(1);

    println!("================================================================");
    println!("  RK3576 人脸识别算法包本地测试");
    println!("  模型: YOLOv8n-face (检测) + EdgeFace-xs (嵌入)");
    println!("  图片: {}", image_path.display());
    println!("================================================================");

    // 加载模型
    println!("\n[1/4] 加载 RKNN 模型...");
    let t0 = Instant::now();
    let models = face_recognition::shared_models(package_root)?;
    println!(
        "  模型加载耗时: {:.2} ms",
        t0.elapsed().as_secs_f64() * 1000.0
    );

    // 读取图片
    println!("\n[2/4] 读取图片...");
    let image = image::open(&image_path)?.to_rgb8();
    let (orig_w, orig_h) = (image.width(), image.height());
    println!("  图片尺寸: {}×{}", orig_w, orig_h);

    // 检测推理
    println!("\n[3/4] 检测推理 (YOLOv8n-face)...");
    let (detector_rgb, layout) = face_recognition::prepare_detector_input(&image);

    let t1 = Instant::now();
    let detect_result = {
        let detector = models
            .detector
            .lock()
            .map_err(|_| "RKNN 检测会话互斥锁中毒")?;
        let attrs: Vec<[u32; 4]> = detector
            .output_attrs
            .iter()
            .map(|a| [a.dims[0], a.dims[1], a.dims[2], a.dims[3]])
            .collect();
        detector.infer_with_host_bytes(&detector_rgb, |output| match output {
            face_recognition::rknn::RknnInferenceOutput::Float32(float_views) => {
                let faces = face_recognition::detect::decode_yolov8_face(
                    float_views,
                    &attrs,
                    &layout,
                    0.25,
                    0.45,
                );
                Ok(faces)
            }
        })?
    };
    let detect_ms = t1.elapsed().as_secs_f64() * 1000.0;
    println!("  检测耗时: {:.2} ms", detect_ms);
    println!("  检出人脸: {} 个", detect_result.len());

    for (i, face) in detect_result.iter().enumerate() {
        println!(
            "  [{}] bbox=[{:.4}, {:.4}, {:.4}, {:.4}] score={:.4}",
            i, face.bbox[0], face.bbox[1], face.bbox[2], face.bbox[3], face.score
        );
        for (j, lm) in face.landmarks.iter().enumerate() {
            println!(
                "       landmark{}: [{:.4}, {:.4}] conf={:.4}",
                j, lm[0], lm[1], face.landmark_scores[j]
            );
        }
    }

    // 嵌入提取
    if let Some(best) = detect_result
        .iter()
        .max_by(|a, b| a.score.total_cmp(&b.score))
    {
        println!("\n[4/4] 嵌入提取 (EdgeFace-xs)...");
        let t2 = Instant::now();
        let aligned =
            face_recognition::align::align_face(image.as_raw(), orig_w, orig_h, &best.landmarks)?;
        let embed_result = {
            let embedder = models
                .embedder
                .lock()
                .map_err(|_| "RKNN 嵌入会话互斥锁中毒")?;
            embedder.infer_with_host_bytes(&aligned, |output| match output {
                face_recognition::rknn::RknnInferenceOutput::Float32(float_views) => {
                    let raw_emb = float_views[0];
                    let embedding = face_recognition::normalize_embedding(raw_emb)?;
                    Ok(embedding)
                }
            })?
        };
        let embed_ms = t2.elapsed().as_secs_f64() * 1000.0;
        println!("  嵌入耗时: {:.2} ms", embed_ms);
        println!("  Embedding 维度: 512");
        println!(
            "  L2 norm: {:.6}",
            embed_result.iter().map(|x| x * x).sum::<f32>().sqrt()
        );
        println!("  前 10 维: {:?}", &embed_result[..10]);
    }

    // 多轮性能测试
    if loops > 1 {
        println!("\n================================================================");
        println!("  性能测试: {} 轮", loops);
        println!("================================================================");

        // 预热
        for _ in 0..3 {
            let detector = models
                .detector
                .lock()
                .map_err(|_| "RKNN 检测会话互斥锁中毒")?;
            let _ = detector
                .infer_with_host_bytes(&detector_rgb, |_| Ok::<(), algo_sdk::error::AlgoError>(()));
        }

        let mut detect_times = Vec::with_capacity(loops);
        let mut embed_times = Vec::with_capacity(loops);

        for _ in 0..loops {
            let t1 = Instant::now();
            {
                let detector = models
                    .detector
                    .lock()
                    .map_err(|_| "RKNN 检测会话互斥锁中毒")?;
                let attrs: Vec<[u32; 4]> = detector
                    .output_attrs
                    .iter()
                    .map(|a| [a.dims[0], a.dims[1], a.dims[2], a.dims[3]])
                    .collect();
                detector.infer_with_host_bytes(&detector_rgb, |output| match output {
                    face_recognition::rknn::RknnInferenceOutput::Float32(float_views) => {
                        let _ = face_recognition::detect::decode_yolov8_face(
                            float_views,
                            &attrs,
                            &layout,
                            0.25,
                            0.45,
                        );
                        Ok(())
                    }
                })?;
            }
            detect_times.push(t1.elapsed().as_secs_f64() * 1000.0);

            if let Some(best) = detect_result
                .iter()
                .max_by(|a, b| a.score.total_cmp(&b.score))
            {
                let aligned = face_recognition::align::align_face(
                    image.as_raw(),
                    orig_w,
                    orig_h,
                    &best.landmarks,
                )?;
                let t2 = Instant::now();
                let embedder = models
                    .embedder
                    .lock()
                    .map_err(|_| "RKNN 嵌入会话互斥锁中毒")?;
                embedder.infer_with_host_bytes(&aligned, |_| {
                    Ok::<(), algo_sdk::error::AlgoError>(())
                })?;
                embed_times.push(t2.elapsed().as_secs_f64() * 1000.0);
            }
        }

        detect_times.sort_by(|a, b| a.total_cmp(b));
        embed_times.sort_by(|a, b| a.total_cmp(b));

        let avg_detect: f64 = detect_times.iter().sum::<f64>() / detect_times.len() as f64;
        let avg_embed: f64 = embed_times.iter().sum::<f64>() / embed_times.len() as f64;
        let p50_detect = detect_times[detect_times.len() / 2];
        let p50_embed = embed_times[embed_times.len() / 2];

        println!(
            "  检测: avg={:.2}ms, p50={:.2}ms, min={:.2}ms, max={:.2}ms",
            avg_detect,
            p50_detect,
            detect_times.first().copied().unwrap_or(0.0),
            detect_times.last().copied().unwrap_or(0.0)
        );
        println!(
            "  嵌入: avg={:.2}ms, p50={:.2}ms, min={:.2}ms, max={:.2}ms",
            avg_embed,
            p50_embed,
            embed_times.first().copied().unwrap_or(0.0),
            embed_times.last().copied().unwrap_or(0.0)
        );
        println!(
            "  总计: avg={:.2}ms, FPS={:.1}",
            avg_detect + avg_embed,
            1000.0 / (avg_detect + avg_embed)
        );
    }

    println!("\n================================================================");
    println!("  测试完成");
    println!("================================================================");

    Ok(())
}
