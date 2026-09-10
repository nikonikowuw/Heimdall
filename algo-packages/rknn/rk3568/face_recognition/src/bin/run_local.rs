//! 本地测试入口：加载 RKNN 模型 -> 读图 -> 检测 + 嵌入提取 -> 输出结果。
//!
//! 用法: `cargo run -p face-recognition-rk3568-rknn --bin face_recognition_rk3568_rknn_run_local [image_path]`

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
        .and_then(|index| args.get(index + 1)?.parse().ok())
        .unwrap_or(1);

    println!("================================================================");
    println!("  RK3568 人脸识别算法包本地测试");
    println!("  模型: YOLOv8n-face (检测) + EdgeFace-xs (嵌入)");
    println!("  图片: {}", image_path.display());
    println!("================================================================");

    println!("\n[1/4] 加载 RKNN 模型...");
    let t0 = Instant::now();
    let models = face_recognition_rk3568::shared_models(package_root)?;
    println!(
        "  模型加载耗时: {:.2} ms",
        t0.elapsed().as_secs_f64() * 1000.0
    );

    println!("\n[2/4] 读取图片...");
    let image = image::open(&image_path)?.to_rgb8();
    let (orig_w, orig_h) = (image.width(), image.height());
    println!("  图片尺寸: {}×{}", orig_w, orig_h);

    println!("\n[3/4] 检测推理 (YOLOv8n-face)...");
    let (detector_rgb, layout) = face_recognition_rk3568::prepare_detector_input_for(
        &image,
        models.detector_width,
        models.detector_height,
    )?;
    let t1 = Instant::now();
    let detect_result = models
        .worker
        .detect_host(detector_rgb.clone(), layout, 0.25)?;
    let detect_ms = t1.elapsed().as_secs_f64() * 1000.0;
    println!("  检测耗时: {:.2} ms", detect_ms);
    println!("  检出人脸: {} 个", detect_result.len());

    for (index, face) in detect_result.iter().enumerate() {
        println!(
            "  [{}] bbox=[{:.4}, {:.4}, {:.4}, {:.4}] score={:.4}",
            index, face.bbox[0], face.bbox[1], face.bbox[2], face.bbox[3], face.score
        );
        for (landmark_index, landmark) in face.landmarks.iter().enumerate() {
            println!(
                "       landmark{}: [{:.4}, {:.4}] conf={:.4}",
                landmark_index, landmark[0], landmark[1], face.landmark_scores[landmark_index]
            );
        }
    }

    if let Some(best) = detect_result
        .iter()
        .max_by(|left, right| left.score.total_cmp(&right.score))
    {
        println!("\n[4/4] 嵌入提取 (EdgeFace-xs)...");
        let aligned = face_recognition_rk3568::align::align_face(
            image.as_raw(),
            orig_w,
            orig_h,
            &best.landmarks,
        )?;
        let t2 = Instant::now();
        let embed_result = models.worker.embed_host(aligned)?;
        let embed_ms = t2.elapsed().as_secs_f64() * 1000.0;
        println!("  嵌入耗时: {:.2} ms", embed_ms);
        println!("  Embedding 维度: {}", embed_result.len());
        println!(
            "  L2 norm: {:.6}",
            embed_result
                .iter()
                .map(|value| value * value)
                .sum::<f32>()
                .sqrt()
        );
        println!("  前 10 维: {:?}", &embed_result[..10]);
    }

    if loops > 1 {
        println!("\n================================================================");
        println!("  性能测试: {} 轮", loops);
        println!("================================================================");

        for _ in 0..3 {
            let _ = models
                .worker
                .detect_host(detector_rgb.clone(), layout, 0.25)?;
        }

        let mut detect_times = Vec::with_capacity(loops);
        let mut embed_times = Vec::with_capacity(loops);
        for _ in 0..loops {
            let t1 = Instant::now();
            let faces = models
                .worker
                .detect_host(detector_rgb.clone(), layout, 0.25)?;
            detect_times.push(t1.elapsed().as_secs_f64() * 1000.0);

            if let Some(best) = faces
                .iter()
                .max_by(|left, right| left.score.total_cmp(&right.score))
            {
                let aligned = face_recognition_rk3568::align::align_face(
                    image.as_raw(),
                    orig_w,
                    orig_h,
                    &best.landmarks,
                )?;
                let t2 = Instant::now();
                let _ = models.worker.embed_host(aligned)?;
                embed_times.push(t2.elapsed().as_secs_f64() * 1000.0);
            }
        }

        detect_times.sort_by(|left, right| left.total_cmp(right));
        embed_times.sort_by(|left, right| left.total_cmp(right));
        if !detect_times.is_empty() {
            let avg_detect = detect_times.iter().sum::<f64>() / detect_times.len() as f64;
            let p50_detect = detect_times[detect_times.len() / 2];
            println!(
                "  检测: avg={:.2}ms, p50={:.2}ms, min={:.2}ms, max={:.2}ms",
                avg_detect,
                p50_detect,
                detect_times.first().copied().unwrap_or(0.0),
                detect_times.last().copied().unwrap_or(0.0)
            );
        }
        if !embed_times.is_empty() {
            let avg_embed = embed_times.iter().sum::<f64>() / embed_times.len() as f64;
            let p50_embed = embed_times[embed_times.len() / 2];
            println!(
                "  嵌入: avg={:.2}ms, p50={:.2}ms, min={:.2}ms, max={:.2}ms",
                avg_embed,
                p50_embed,
                embed_times.first().copied().unwrap_or(0.0),
                embed_times.last().copied().unwrap_or(0.0)
            );
        }
    }

    println!("\n================================================================");
    println!("  测试完成");
    println!("================================================================");
    Ok(())
}
