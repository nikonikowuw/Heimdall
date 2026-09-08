#[cfg(target_os = "macos")]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    macos_run::run()
}

#[cfg(not(target_os = "macos"))]
fn main() {
    eprintln!("run_local (CoreML) 仅支持在 macOS Apple Silicon 上运行");
}

#[cfg(target_os = "macos")]
mod macos_run {
    use std::path::Path;
    use std::time::{Duration, Instant};

    use algo_sdk::c_abi::AV_OPAQUE_CVPIXELBUFFER;
    use algo_sdk::cv::platforms::apple::AppleCvEngine;
    use algo_sdk::cv::CvEngine;
    use algo_sdk::error::AlgoError;
    use algo_sdk::frame::SafeFrame;
    use algo_sdk::testing::MockFrameBuilder;
    use face_recognition_coreml::config::InstanceConfig;
    use face_recognition_coreml::coreml::CoreMlFaceModels;
    use face_recognition_coreml::detect::{decode_yolov5_face, nms, unmap_letterbox};
    use face_recognition_coreml::normalize_embedding;
    use face_recognition_coreml::postprocess::FaceDetection;
    use face_recognition_coreml::quality::compute_quality;
    use image::RgbImage;

    fn env_string(name: &str, default: &str) -> String {
        std::env::var(name).unwrap_or_else(|_| default.to_string())
    }

    fn env_usize(name: &str, default: usize) -> usize {
        env_string(name, &default.to_string())
            .parse::<usize>()
            .unwrap_or(default)
    }

    fn load_env_file(path: &str) -> std::collections::HashMap<String, String> {
        let mut map = std::collections::HashMap::new();
        if let Ok(content) = std::fs::read_to_string(path) {
            for line in content.lines() {
                let line = line.trim();
                if line.is_empty() || line.starts_with('#') {
                    continue;
                }
                if let Some((k, v)) = line.split_once('=') {
                    map.insert(k.trim().to_string(), v.trim().to_string());
                }
            }
        }
        map
    }

    #[derive(Debug, Clone)]
    pub struct FaceResult {
        pub face: FaceDetection,
        pub embedding: [f32; 512],
    }

    #[derive(Debug, Clone, Default)]
    pub struct StageTimings {
        pub preprocess_ms: f64,
        pub detect_infer_ms: f64,
        pub postprocess_ms: f64,
        pub quality_ms: f64,
        pub align_ms: f64,
        pub embed_infer_ms: f64,
        pub norm_ms: f64,
        pub total_ms: f64,
    }

    fn infer_all(
        models: &CoreMlFaceModels,
        safe_frame: &SafeFrame<'_>,
        image: &RgbImage,
        config: &InstanceConfig,
    ) -> Result<(Vec<FaceResult>, StageTimings), AlgoError> {
        let t_start = Instant::now();

        // 阶段 1: 硬件预处理 (Apple Accelerate vImage Letterbox 缩放与 NV12->BGRA 色彩转换)
        let t0 = Instant::now();
        let (buffer, mode) = AppleCvEngine.letterbox(safe_frame, 640, 384, [114, 114, 114])?;
        let pixelbuffer = buffer.as_raw_ptr().ok_or_else(|| AlgoError::Preprocess {
            reason: "CoreML 检测输入 CVPixelBuffer 指针为空".to_string(),
        })?;
        let preprocess_ms = t0.elapsed().as_secs_f64() * 1000.0;

        // 阶段 2: YOLOv5n-face 人脸检测 (CoreML / ANE 前向推理)
        let t1 = Instant::now();
        // SAFETY: pixelbuffer 由当前 CvBuffer 持有，在同步预测返回前保持有效生命周期。
        let raw_output = unsafe { models.predict_detector(pixelbuffer)? };
        let detect_infer_ms = t1.elapsed().as_secs_f64() * 1000.0;

        // 阶段 3: 后处理 (张量展开、类别无关 NMS 与 Letterbox 坐标反算还原)
        let t2 = Instant::now();
        let mut raw_faces = decode_yolov5_face(&raw_output, config.detection_confidence_threshold);
        nms(&mut raw_faces, 0.45);
        unmap_letterbox(&mut raw_faces, &mode, image.width(), image.height());
        let postprocess_ms = t2.elapsed().as_secs_f64() * 1000.0;

        // 阶段 4: 人脸质量门控评估 (姿态角、模糊代理、人脸最小有效尺寸筛选)
        let t3 = Instant::now();
        let mut accepted_faces = Vec::with_capacity(raw_faces.len());
        for face in raw_faces {
            let quality = compute_quality(
                &face.landmarks,
                &face.landmark_scores,
                face.bbox[2] * image.width() as f32,
                &config.quality_thresholds,
            );
            if quality.accepted(&config.quality_thresholds, config.min_face_size) {
                accepted_faces.push((face, quality));
            }
        }
        let quality_ms = t3.elapsed().as_secs_f64() * 1000.0;

        if accepted_faces.is_empty() {
            return Err(AlgoError::Inference {
                reason: "测试图像中没有检测到满足质量门控阈值的人脸".to_string(),
            });
        }

        // 阶段 5、6、7: 针对所有人脸执行 ArcFace 5点仿射对齐、EdgeFace-s ANE 推理与 L2 归一化
        let mut align_ms = 0.0;
        let mut embed_infer_ms = 0.0;
        let mut norm_ms = 0.0;

        let mut results = Vec::with_capacity(accepted_faces.len());
        for (face, quality) in accepted_faces {
            let t_al = Instant::now();
            let aligned = face_recognition_coreml::align::align_face(
                image.as_raw(),
                image.width(),
                image.height(),
                &face.landmarks,
            )
            .map_err(|reason| AlgoError::Preprocess {
                reason: reason.to_string(),
            })?;
            align_ms += t_al.elapsed().as_secs_f64() * 1000.0;

            let t_em = Instant::now();
            let values = models.predict_embedding(&aligned)?;
            embed_infer_ms += t_em.elapsed().as_secs_f64() * 1000.0;

            let t_no = Instant::now();
            let embedding = normalize_embedding(&values)?;
            norm_ms += t_no.elapsed().as_secs_f64() * 1000.0;

            results.push(FaceResult {
                face: FaceDetection {
                    bbox: face.bbox,
                    landmarks: face.landmarks,
                    detection_score: face.score,
                    quality,
                },
                embedding,
            });
        }

        let total_ms = t_start.elapsed().as_secs_f64() * 1000.0;

        Ok((
            results,
            StageTimings {
                preprocess_ms,
                detect_infer_ms,
                postprocess_ms,
                quality_ms,
                align_ms,
                embed_infer_ms,
                norm_ms,
                total_ms,
            },
        ))
    }

    fn calc_stats(values: &mut [f64]) -> (f64, f64, f64) {
        if values.is_empty() {
            return (0.0, 0.0, 0.0);
        }
        values.sort_by(f64::total_cmp);
        let avg = values.iter().sum::<f64>() / values.len() as f64;
        let p50 = values[values.len() * 50 / 100];
        let p99 = values[(values.len() * 99 / 100).min(values.len() - 1)];
        (avg, p50, p99)
    }

    fn benchmark(
        models: &CoreMlFaceModels,
        safe_frame: &SafeFrame<'_>,
        image: &RgbImage,
        config: &InstanceConfig,
        warmup: usize,
        loops: usize,
    ) -> Result<(), AlgoError> {
        for _ in 0..warmup {
            let _ = infer_all(models, safe_frame, image, config)?;
        }

        let mut pre_samples = Vec::with_capacity(loops);
        let mut det_samples = Vec::with_capacity(loops);
        let mut post_samples = Vec::with_capacity(loops);
        let mut q_samples = Vec::with_capacity(loops);
        let mut al_samples = Vec::with_capacity(loops);
        let mut em_samples = Vec::with_capacity(loops);
        let mut norm_samples = Vec::with_capacity(loops);
        let mut e2e_samples = Vec::with_capacity(loops);

        let mut detected_face_count = 0;

        for _ in 0..loops {
            let (faces, timing) = infer_all(models, safe_frame, image, config)?;
            detected_face_count = faces.len();
            pre_samples.push(timing.preprocess_ms);
            det_samples.push(timing.detect_infer_ms);
            post_samples.push(timing.postprocess_ms);
            q_samples.push(timing.quality_ms);
            al_samples.push(timing.align_ms);
            em_samples.push(timing.embed_infer_ms);
            norm_samples.push(timing.norm_ms);
            e2e_samples.push(timing.total_ms);
        }

        let (pre_avg, pre_p50, _) = calc_stats(&mut pre_samples);
        let (det_avg, det_p50, _) = calc_stats(&mut det_samples);
        let (post_avg, post_p50, _) = calc_stats(&mut post_samples);
        let (q_avg, q_p50, _) = calc_stats(&mut q_samples);
        let (al_avg, al_p50, _) = calc_stats(&mut al_samples);
        let (em_avg, em_p50, _) = calc_stats(&mut em_samples);
        let (norm_avg, norm_p50, _) = calc_stats(&mut norm_samples);
        let (e2e_avg, e2e_p50, e2e_p99) = calc_stats(&mut e2e_samples);
        let fps = if e2e_avg > 0.0 { 1000.0 / e2e_avg } else { 0.0 };

        println!(
            "================================================================================"
        );
        println!("  Face Recognition (EdgeFace + YOLOv5n-face) 性能分析采样统计 (CoreML ANE)");
        println!(
            "================================================================================"
        );
        println!("  采样轮次: {loops} 轮 (预热: {warmup} 轮)");
        println!(
            "  输入尺寸: {}x{} -> 640x384 BGRA CVPixelBuffer",
            image.width(),
            image.height()
        );
        println!("  检出人脸: {detected_face_count} 个 (均已完成 5 点仿射对齐与 512D 特征提取)");
        println!(
            "--------------------------------------------------------------------------------"
        );
        println!(
            "  阶段 1: 硬件预处理 (Letterbox 640x384)   : avg = {:>6.3} ms, p50 = {:>6.3} ms",
            pre_avg, pre_p50
        );
        println!(
            "  阶段 2: YOLOv5n-face 检测 (CoreML ANE)  : avg = {:>6.3} ms, p50 = {:>6.3} ms",
            det_avg, det_p50
        );
        println!(
            "  阶段 3: 后处理 (Decode + NMS + 坐标还原) : avg = {:>6.3} ms, p50 = {:>6.3} ms",
            post_avg, post_p50
        );
        println!(
            "  阶段 4: 人脸质量门控 (Quality Gating)   : avg = {:>6.3} ms, p50 = {:>6.3} ms",
            q_avg, q_p50
        );
        println!(
            "  阶段 5: ArcFace 5点仿射对齐 (112x112)    : avg = {:>6.3} ms, p50 = {:>6.3} ms",
            al_avg, al_p50
        );
        println!(
            "  阶段 6: EdgeFace-s 提取 (CoreML ANE)    : avg = {:>6.3} ms, p50 = {:>6.3} ms",
            em_avg, em_p50
        );
        println!(
            "  阶段 7: 特征 L2 归一化 (Normalize)       : avg = {:>6.3} ms, p50 = {:>6.3} ms",
            norm_avg, norm_p50
        );
        println!(
            "--------------------------------------------------------------------------------"
        );
        println!(
            "  全链路端到端耗时 (End-to-End Latency)   : avg = {:>6.3} ms, p50 = {:>6.3} ms, p99 = {:>6.3} ms",
            e2e_avg, e2e_p50, e2e_p99
        );
        println!(
            "  全链路实时吞吐量 (Throughput)           : {:>6.2} FPS",
            fps
        );
        println!(
            "================================================================================"
        );

        eprintln!(
            "benchmark: samples={} avgMs={e2e_avg:.3} p50Ms={e2e_p50:.3} p99Ms={e2e_p99:.3} fps={fps:.2}",
            loops
        );
        Ok(())
    }

    fn draw_box(image: &mut RgbImage, bbox: [f32; 4], color: image::Rgb<u8>) {
        let width = image.width() as i32;
        let height = image.height() as i32;
        if width <= 0 || height <= 0 {
            return;
        }
        let x1 = (bbox[0] * width as f32).round() as i32;
        let y1 = (bbox[1] * height as f32).round() as i32;
        let x2 = ((bbox[0] + bbox[2]) * width as f32).round() as i32;
        let y2 = ((bbox[1] + bbox[3]) * height as f32).round() as i32;
        for x in x1.clamp(0, width - 1)..=x2.clamp(0, width - 1) {
            for thickness in 0..3 {
                if (y1 + thickness).clamp(0, height - 1) < height {
                    image.put_pixel(
                        x as u32,
                        (y1 + thickness).clamp(0, height - 1) as u32,
                        color,
                    );
                }
                if (y2 - thickness).clamp(0, height - 1) >= 0 {
                    image.put_pixel(
                        x as u32,
                        (y2 - thickness).clamp(0, height - 1) as u32,
                        color,
                    );
                }
            }
        }
        for y in y1.clamp(0, height - 1)..=y2.clamp(0, height - 1) {
            for thickness in 0..3 {
                if (x1 + thickness).clamp(0, width - 1) < width {
                    image.put_pixel((x1 + thickness).clamp(0, width - 1) as u32, y as u32, color);
                }
                if (x2 - thickness).clamp(0, width - 1) >= 0 {
                    image.put_pixel((x2 - thickness).clamp(0, width - 1) as u32, y as u32, color);
                }
            }
        }
    }

    fn draw_landmarks(image: &mut RgbImage, landmarks: &[[f32; 2]; 5]) {
        let width = image.width() as i32;
        let height = image.height() as i32;
        // 5 个关键点专属配色：左眼(绿), 右眼(绿), 鼻尖(黄), 左嘴角(青), 右嘴角(青)
        let colors = [
            image::Rgb([0, 255, 0]),   // 0: 左眼 (亮绿)
            image::Rgb([0, 255, 0]),   // 1: 右眼 (亮绿)
            image::Rgb([255, 255, 0]), // 2: 鼻尖 (明黄)
            image::Rgb([0, 255, 255]), // 3: 左嘴角 (亮青)
            image::Rgb([0, 255, 255]), // 4: 右嘴角 (亮青)
        ];
        let radius = 3;
        for (i, point) in landmarks.iter().enumerate() {
            let cx = (point[0] * width as f32).round() as i32;
            let cy = (point[1] * height as f32).round() as i32;
            let color = colors[i % colors.len()];
            for dy in -radius..=radius {
                for dx in -radius..=radius {
                    if dx * dx + dy * dy <= radius * radius {
                        let x = cx + dx;
                        let y = cy + dy;
                        if x >= 0 && x < width && y >= 0 && y < height {
                            image.put_pixel(x as u32, y as u32, color);
                        }
                    }
                }
            }
        }
    }

    fn compare_images(
        models: &CoreMlFaceModels,
        config: &InstanceConfig,
        path1: &str,
        path2: &str,
    ) -> Result<(), Box<dyn std::error::Error>> {
        println!(
            "================================================================================"
        );
        println!("  Face Recognition (EdgeFace) 两图余弦相似度比对模式 (CoreML ANE)");
        println!(
            "================================================================================"
        );
        let img1 = image::open(path1)?.to_rgb8();
        let img2 = image::open(path2)?.to_rgb8();

        let frame1 = MockFrameBuilder::new()
            .dimensions(img1.width(), img1.height())
            .host_data(img1.clone().into_raw())
            .to_nv12(img1.width())
            .opaque_kind(AV_OPAQUE_CVPIXELBUFFER)
            .build();
        let frame2 = MockFrameBuilder::new()
            .dimensions(img2.width(), img2.height())
            .host_data(img2.clone().into_raw())
            .to_nv12(img2.width())
            .opaque_kind(AV_OPAQUE_CVPIXELBUFFER)
            .build();

        let (res1, _) = infer_all(models, &frame1.as_safe_frame(), &img1, config)?;
        let (res2, _) = infer_all(models, &frame2.as_safe_frame(), &img2, config)?;

        let best1 = res1
            .iter()
            .max_by(|a, b| a.face.detection_score.total_cmp(&b.face.detection_score))
            .ok_or("图 1 未检测到有效人脸")?;
        let best2 = res2
            .iter()
            .max_by(|a, b| a.face.detection_score.total_cmp(&b.face.detection_score))
            .ok_or("图 2 未检测到有效人脸")?;

        let similarity =
            face_recognition_coreml::cosine_similarity(&best1.embedding, &best2.embedding);
        let decision = if similarity >= 0.5 {
            "同一人 (Same Person Match: >= 0.5)"
        } else if similarity < 0.4 {
            "不同人 (Different Person: < 0.4)"
        } else {
            "不确定区间 / 灰度判定 (Uncertain: 0.4 ~ 0.5)"
        };

        println!(
            "  图 1: {} -> 人脸置信度 = {:.3}, 质量分 = {:.3}",
            path1, best1.face.detection_score, best1.face.quality.score
        );
        println!(
            "  图 2: {} -> 人脸置信度 = {:.3}, 质量分 = {:.3}",
            path2, best2.face.detection_score, best2.face.quality.score
        );
        println!(
            "--------------------------------------------------------------------------------"
        );
        println!("  余弦相似度 (Cosine Similarity) : {:.4}", similarity);
        println!("  判定结果 (Verification Decision): {decision}");
        println!(
            "================================================================================"
        );
        Ok(())
    }

    pub fn run() -> Result<(), Box<dyn std::error::Error>> {
        let args: Vec<String> = std::env::args().collect();
        if args.iter().any(|arg| arg == "--help" || arg == "-h") {
            println!("Usage: face_recognition_run_local [OPTIONS]");
            println!("Options:");
            println!("  --benchmark            运行性能采样统计 (P50/P99/Avg/FPS)");
            println!("  --stress [DURATION]    满载连续压力测试 (默认 30 秒)");
            println!("  --compare IMG1 IMG2    比对两张图片中的人脸并计算特征余弦相似度");
            println!("  -h, --help             显示帮助信息");
            return Ok(());
        }

        let package_root = Path::new(env!("CARGO_MANIFEST_DIR"));
        let env_map = load_env_file(".env");
        let raw_image_path = env_map
            .get("INPUT_IMAGE")
            .cloned()
            .unwrap_or_else(|| env_string("INPUT_IMAGE", "testimage.jpg"));
        let image_path = if Path::new(&raw_image_path).exists() {
            std::path::PathBuf::from(&raw_image_path)
        } else {
            package_root.join(&raw_image_path)
        };
        let output_path = env_map
            .get("OUTPUT_IMAGE")
            .cloned()
            .unwrap_or_else(|| env_string("OUTPUT_IMAGE", "result.jpg"));

        let models = CoreMlFaceModels::load(package_root)?;
        let config = InstanceConfig::default();

        if let Some(pos) = args.iter().position(|arg| arg == "--compare") {
            let p1 = args
                .get(pos + 1)
                .map(String::as_str)
                .unwrap_or_else(|| image_path.to_str().unwrap_or("testimage.jpg"));
            let p2 = args
                .get(pos + 2)
                .map(String::as_str)
                .unwrap_or_else(|| image_path.to_str().unwrap_or("testimage.jpg"));
            let p1_buf = if Path::new(p1).exists() {
                std::path::PathBuf::from(p1)
            } else {
                package_root.join(p1)
            };
            let p2_buf = if Path::new(p2).exists() {
                std::path::PathBuf::from(p2)
            } else {
                package_root.join(p2)
            };
            return compare_images(
                &models,
                &config,
                p1_buf.to_str().unwrap_or(p1),
                p2_buf.to_str().unwrap_or(p2),
            );
        }

        let image = image::open(&image_path)?.to_rgb8();

        // 构造 NV12 CVPixelBuffer 真实硬件帧 (模拟 VideoToolbox 硬解码输出)
        let mock_frame = MockFrameBuilder::new()
            .dimensions(image.width(), image.height())
            .host_data(image.clone().into_raw())
            .to_nv12(image.width())
            .opaque_kind(AV_OPAQUE_CVPIXELBUFFER)
            .build();
        let safe_frame = mock_frame.as_safe_frame();

        let is_benchmark = args.iter().any(|arg| arg == "--benchmark");
        let stress = args.iter().position(|arg| arg == "--stress").map(|index| {
            args.get(index + 1)
                .and_then(|value| value.parse::<u64>().ok())
                .unwrap_or(30)
        });

        if is_benchmark {
            benchmark(
                &models,
                &safe_frame,
                &image,
                &config,
                env_usize("WARMUP", 5),
                env_usize("LOOPS", 100).max(1),
            )?;
            return Ok(());
        }
        if let Some(seconds) = stress {
            let deadline = Instant::now() + Duration::from_secs(seconds);
            let mut iterations = 0usize;
            while Instant::now() < deadline {
                let _ = infer_all(&models, &safe_frame, &image, &config)?;
                iterations += 1;
            }
            eprintln!("stress: durationSeconds={seconds} iterations={iterations}");
            return Ok(());
        }

        let (results, timing) = infer_all(&models, &safe_frame, &image, &config)?;
        let faces_json: Vec<_> = results
            .iter()
            .enumerate()
            .map(|(index, item)| {
                serde_json::json!({
                    "face_index": index,
                    "bbox": item.face.bbox,
                    "detection_score": item.face.detection_score,
                    "landmarks": item.face.landmarks,
                    "quality": item.face.quality,
                    "embedding_head": &item.embedding[..10],
                })
            })
            .collect();

        let json = serde_json::json!({
            "face_count": results.len(),
            "faces": faces_json,
            "latency_breakdown_ms": {
                "preprocess": timing.preprocess_ms,
                "detect_inference": timing.detect_infer_ms,
                "postprocess_nms": timing.postprocess_ms,
                "quality_gating": timing.quality_ms,
                "affine_alignment": timing.align_ms,
                "embedding_inference": timing.embed_infer_ms,
                "l2_normalize": timing.norm_ms,
                "total_e2e": timing.total_ms,
            }
        });
        println!("{}", serde_json::to_string_pretty(&json)?);

        let mut result = image;
        let box_color = image::Rgb([255, 32, 32]);
        for item in &results {
            draw_box(&mut result, item.face.bbox, box_color);
            draw_landmarks(&mut result, &item.face.landmarks);
        }
        result.save(&output_path)?;
        println!(
            "\n✓ 已保存带有人脸检测框与 5 关键点绘制结果图至: {}",
            output_path
        );
        Ok(())
    }
}
