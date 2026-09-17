#[cfg(not(target_os = "macos"))]
fn main() {
    eprintln!("此本地测试驱动仅支持在 macOS (Apple Silicon) 平台运行。");
}

#[cfg(target_os = "macos")]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    macos::run()
}

#[cfg(target_os = "macos")]
mod macos {
    use std::env;
    use std::path::{Path, PathBuf};
    use std::time::{Duration, Instant};

    use algo_sdk::c_abi::AV_OPAQUE_CVPIXELBUFFER;
    use algo_sdk::cv::platforms::apple::AppleCvEngine;
    use algo_sdk::cv::CvEngine;
    use algo_sdk::error::AlgoError;
    use algo_sdk::frame::{FrameHandleView, SafeFrame};
    use algo_sdk::testing::MockFrameBuilder;
    use image::RgbImage;

    use face_recognition_coreml::align::face_alignment_matrix;
    use face_recognition_coreml::association::associate_persons_and_faces;
    use face_recognition_coreml::config::InstanceConfig;
    use face_recognition_coreml::coreml::{CoreMlFaceModels, CoreMlRunner};
    use face_recognition_coreml::detect::{
        decode_face_detections, decode_person_detections, nms, nms_persons, unmap_letterbox,
        unmap_persons_letterbox,
    };
    use face_recognition_coreml::normalize_embedding;
    use face_recognition_coreml::postprocess::{encode_embedding, normalized_xywh_to_xyxy};
    use face_recognition_coreml::quality::{compute_quality, FaceQualityExt};

    #[derive(Debug, Clone)]
    struct FaceAnalyzed {
        bbox: [f32; 4],
        confidence: f32,
        quality_score: f32,
        embedding: Option<String>,
        landmarks: [[f32; 2]; 5],
    }

    #[derive(Debug, Clone)]
    struct AnalyzedObject {
        /// 对外输出的人体主体框。
        bbox: [f32; 4],
        confidence: f32,
        /// 挂载的人脸详情 (若有)
        face: Option<FaceAnalyzed>,
    }

    #[derive(Debug, Clone, Default)]
    struct StageTimings {
        preprocess_ms: f64,
        person_infer_ms: f64,
        face_infer_ms: f64,
        decode_nms_ms: f64,
        association_ms: f64,
        quality_ms: f64,
        embedding_ms: f64,
        total_ms: f64,
    }

    fn infer_pipeline(
        models: &CoreMlFaceModels,
        safe_frame: &SafeFrame<'_>,
        image_width: u32,
        image_height: u32,
        config: &InstanceConfig,
    ) -> Result<(Vec<AnalyzedObject>, StageTimings), AlgoError> {
        let t_start = Instant::now();

        let t0 = Instant::now();
        let (buffer, mode) = AppleCvEngine.letterbox(safe_frame, 640, 384, [114, 114, 114])?;
        let pixelbuffer = buffer.as_raw_ptr().ok_or_else(|| AlgoError::Preprocess {
            reason: "CoreML 检测输入 CVPixelBuffer 指针为空".to_string(),
        })?;
        let preprocess_ms = t0.elapsed().as_secs_f64() * 1000.0;

        let t1 = Instant::now();
        // SAFETY: pixelbuffer 由当前 CvBuffer 持有，在同步预测返回前保持有效。
        let raw_person = unsafe { models.predict_person_detector(pixelbuffer)? };
        let person_infer_ms = t1.elapsed().as_secs_f64() * 1000.0;

        let t2 = Instant::now();
        // SAFETY: pixelbuffer 由当前 CvBuffer 持有，在同步预测返回前保持有效。
        let raw_face = unsafe { models.predict_detector(pixelbuffer)? };
        let face_infer_ms = t2.elapsed().as_secs_f64() * 1000.0;

        let t3 = Instant::now();
        let mut persons = decode_person_detections(&raw_person, config.person_confidence_threshold);
        nms_persons(&mut persons, 0.45);
        unmap_persons_letterbox(&mut persons, &mode, image_width, image_height);

        let mut faces = decode_face_detections(&raw_face, config.detection_confidence_threshold);
        nms(&mut faces, 0.45);
        unmap_letterbox(&mut faces, &mode, image_width, image_height);
        let decode_nms_ms = t3.elapsed().as_secs_f64() * 1000.0;

        let t4 = Instant::now();
        let associated = associate_persons_and_faces(&persons, &faces);
        let association_ms = t4.elapsed().as_secs_f64() * 1000.0;

        let mut objects = Vec::with_capacity(associated.len());
        let mut quality_ms = 0.0;
        let mut embedding_ms = 0.0;
        for candidate in associated {
            let face = if let Some(face_cand) = candidate.attached_face {
                let quality_start = Instant::now();
                let quality = compute_quality(
                    &face_cand.landmarks,
                    &face_cand.landmark_scores,
                    face_cand.bbox[2] * image_width as f32,
                    &config.quality_thresholds,
                );
                quality_ms += quality_start.elapsed().as_secs_f64() * 1000.0;
                let embedding =
                    if quality.accepted(&config.quality_thresholds, config.min_face_size) {
                        let embedding_start = Instant::now();
                        let emb = match safe_frame.handle_view() {
                            FrameHandleView::ApplePixelBuffer { ptr } => {
                                let matrix = face_alignment_matrix(
                                    image_width,
                                    image_height,
                                    &face_cand.landmarks,
                                )
                                .map_err(|error| {
                                    AlgoError::Preprocess {
                                        reason: error.to_string(),
                                    }
                                })?;
                                // SAFETY: ptr 来自当前 SafeFrame，且 predict 在本次调用内同步完成；
                                // CoreML 不会保存该裸指针或把它交给异步任务。
                                let values = unsafe {
                                    models.predict_embedding_from_pixelbuffer(
                                        ptr,
                                        image_width,
                                        image_height,
                                        matrix,
                                    )?
                                };
                                let normalized = normalize_embedding(&values)?;
                                Some(encode_embedding(&normalized)?)
                            }
                            _ => {
                                return Err(AlgoError::IncompatibleFrame {
                                    reason: "单帧 best-shot 特征提取需要原生 Apple CVPixelBuffer"
                                        .to_string(),
                                });
                            }
                        };
                        embedding_ms += embedding_start.elapsed().as_secs_f64() * 1000.0;
                        emb
                    } else {
                        None
                    };

                Some(FaceAnalyzed {
                    bbox: normalized_xywh_to_xyxy(face_cand.bbox),
                    confidence: face_cand.score.clamp(0.0, 1.0),
                    quality_score: quality.score.clamp(0.0, 1.0),
                    embedding,
                    landmarks: face_cand.landmarks,
                })
            } else {
                None
            };

            objects.push(AnalyzedObject {
                bbox: normalized_xywh_to_xyxy(candidate.person_bbox),
                confidence: candidate.person_score.clamp(0.0, 1.0),
                face,
            });
        }
        let total_ms = t_start.elapsed().as_secs_f64() * 1000.0;

        Ok((
            objects,
            StageTimings {
                preprocess_ms,
                person_infer_ms,
                face_infer_ms,
                decode_nms_ms,
                association_ms,
                quality_ms,
                embedding_ms,
                total_ms,
            },
        ))
    }

    fn detection_json(results: &[AnalyzedObject]) -> serde_json::Value {
        let objects: Vec<_> = results
            .iter()
            .map(|object| {
                let mut value = serde_json::json!({
                    "class_id": 0,
                    "label": "person",
                    "confidence": object.confidence,
                    "bbox": object.bbox,
                });
                if let Some(face) = &object.face {
                    let mut face_val = serde_json::json!({
                        "bbox": face.bbox,
                        "confidence": face.confidence,
                        "quality_score": face.quality_score,
                    });
                    if let Some(embedding) = face.embedding.as_ref() {
                        face_val["embedding"] = serde_json::Value::String(embedding.clone());
                    }
                    value["face"] = face_val;
                }
                value
            })
            .collect();
        serde_json::json!({
            "schema_version": 1,
            "objects": objects,
        })
    }

    fn calc_stats(samples: &mut [f64]) -> (f64, f64, f64) {
        if samples.is_empty() {
            return (0.0, 0.0, 0.0);
        }
        samples.sort_by(|a, b| a.total_cmp(b));
        let sum: f64 = samples.iter().sum();
        let avg = sum / samples.len() as f64;
        let p50 = samples[samples.len() / 2];
        let p99_idx = ((samples.len() as f64 * 0.99).ceil() as usize)
            .saturating_sub(1)
            .min(samples.len() - 1);
        (avg, p50, samples[p99_idx])
    }

    fn benchmark(
        models: &CoreMlFaceModels,
        safe_frame: &SafeFrame<'_>,
        image_width: u32,
        image_height: u32,
        config: &InstanceConfig,
        warmup: usize,
        loops: usize,
    ) -> Result<(), Box<dyn std::error::Error>> {
        for _ in 0..warmup {
            let _ = infer_pipeline(models, safe_frame, image_width, image_height, config)?;
        }

        let mut pre_samples = Vec::with_capacity(loops);
        let mut person_samples = Vec::with_capacity(loops);
        let mut face_samples = Vec::with_capacity(loops);
        let mut dec_samples = Vec::with_capacity(loops);
        let mut assoc_samples = Vec::with_capacity(loops);
        let mut quality_samples = Vec::with_capacity(loops);
        let mut embedding_samples = Vec::with_capacity(loops);
        let mut e2e_samples = Vec::with_capacity(loops);
        let mut object_count = 0;

        for _ in 0..loops {
            let (objects, timing) =
                infer_pipeline(models, safe_frame, image_width, image_height, config)?;
            object_count = objects.len();
            pre_samples.push(timing.preprocess_ms);
            person_samples.push(timing.person_infer_ms);
            face_samples.push(timing.face_infer_ms);
            dec_samples.push(timing.decode_nms_ms);
            assoc_samples.push(timing.association_ms);
            quality_samples.push(timing.quality_ms);
            embedding_samples.push(timing.embedding_ms);
            e2e_samples.push(timing.total_ms);
        }

        let (pre_avg, pre_p50, _) = calc_stats(&mut pre_samples);
        let (p_avg, p_p50, _) = calc_stats(&mut person_samples);
        let (f_avg, f_p50, _) = calc_stats(&mut face_samples);
        let (dec_avg, dec_p50, _) = calc_stats(&mut dec_samples);
        let (assoc_avg, assoc_p50, _) = calc_stats(&mut assoc_samples);
        let (q_avg, q_p50, _) = calc_stats(&mut quality_samples);
        let (embedding_avg, embedding_p50, _) = calc_stats(&mut embedding_samples);
        let (e2e_avg, e2e_p50, e2e_p99) = calc_stats(&mut e2e_samples);
        let fps = if e2e_avg > 0.0 { 1000.0 / e2e_avg } else { 0.0 };

        println!(
            "================================================================================"
        );
        println!("  人脸识别检测管线性能分析 (YOLO26n + YOLOv8-face + Quality Gating)");
        println!(
            "================================================================================"
        );
        println!("  采样轮次: {loops} 轮 (预热: {warmup} 轮)");
        println!(
            "  输入尺寸: {image_width}x{image_height} -> 640x384 共享 Letterbox CVPixelBuffer"
        );
        println!("  最近一轮可识别目标: {object_count} 个");
        println!(
            "--------------------------------------------------------------------------------"
        );
        println!("  阶段 1: 硬件预处理                     : avg = {pre_avg:>6.3} ms, p50 = {pre_p50:>6.3} ms");
        println!("  阶段 2: YOLO26n 人体检测 (CoreML ANE)  : avg = {p_avg:>6.3} ms, p50 = {p_p50:>6.3} ms");
        println!("  阶段 3: YOLOv8-face 人脸检测 (CoreML)  : avg = {f_avg:>6.3} ms, p50 = {f_p50:>6.3} ms");
        println!("  阶段 4: 解码、NMS 与坐标还原            : avg = {dec_avg:>6.3} ms, p50 = {dec_p50:>6.3} ms");
        println!("  阶段 5: 人体/人脸当前帧空间关联         : avg = {assoc_avg:>6.3} ms, p50 = {assoc_p50:>6.3} ms");
        println!("  阶段 6: 人脸质量门控                    : avg = {q_avg:>6.3} ms, p50 = {q_p50:>6.3} ms");
        println!("  阶段 7: EdgeFace 设备侧特征提取         : avg = {embedding_avg:>6.3} ms, p50 = {embedding_p50:>6.3} ms");
        println!(
            "--------------------------------------------------------------------------------"
        );
        println!("  检测端到端耗时                         : avg = {e2e_avg:>6.3} ms, p50 = {e2e_p50:>6.3} ms, p99 = {e2e_p99:>6.3} ms");
        println!("  检测吞吐量                             : {fps:>6.2} FPS");
        println!(
            "================================================================================"
        );

        eprintln!(
            "benchmark: samples={} avgMs={e2e_avg:.3} p50Ms={e2e_p50:.3} p99Ms={e2e_p99:.3} fps={fps:.2}",
            loops
        );
        Ok(())
    }

    fn compare_person_models(
        package_root: &Path,
        safe_frame: &SafeFrame<'_>,
        loops: usize,
    ) -> Result<(), Box<dyn std::error::Error>> {
        println!(
            "================================================================================"
        );
        println!("  人体检测性能基准: yolo26n 640x384 (CoreML ANE)");
        println!(
            "================================================================================"
        );

        let runner_person =
            CoreMlRunner::load_model(package_root, "yolo26n.mlpackage", "image", "var_911")?;
        let (buf640, mode640) = AppleCvEngine.letterbox(safe_frame, 640, 384, [114, 114, 114])?;
        let pb640 = buf640.as_raw_ptr().ok_or_else(|| AlgoError::Preprocess {
            reason: "pb640 null".to_string(),
        })?;

        for _ in 0..5 {
            // SAFETY: pixelbuffer 由 buf640 持有，在同步预测期间有效。
            unsafe {
                let _ = runner_person.predict_pixelbuffer(pb640)?;
            }
        }

        let mut times = Vec::with_capacity(loops);
        let mut raw_person = Vec::new();
        for _ in 0..loops {
            let t = Instant::now();
            // SAFETY: pb640 在预测期间由 buf640 保活。
            raw_person = unsafe { runner_person.predict_pixelbuffer(pb640)? };
            times.push(t.elapsed().as_secs_f64() * 1000.0);
        }
        let (avg_ms, p50_ms, p99_ms) = calc_stats(&mut times);
        let default_person_conf = InstanceConfig::default().person_confidence_threshold;
        let mut persons = decode_person_detections(&raw_person, default_person_conf);
        nms_persons(&mut persons, 0.45);
        unmap_persons_letterbox(
            &mut persons,
            &mode640,
            safe_frame.width(),
            safe_frame.height(),
        );

        println!(
            "{:<22} | {:<12} | {:<12} | {:<12} | {:<10} | {:<12}",
            "模型规格 (Model)", "平均耗时", "P50 耗时", "P99 耗时", "检出人体", "单模型 FPS"
        );
        println!(
            "--------------------------------------------------------------------------------"
        );
        println!(
            "{:<22} | {:>9.3} ms | {:>9.3} ms | {:>9.3} ms | {:>8} 人 | {:>10.1} FPS",
            "YOLO26n 640x384",
            avg_ms,
            p50_ms,
            p99_ms,
            persons.len(),
            if avg_ms > 0.0 { 1000.0 / avg_ms } else { 0.0 }
        );
        println!(
            "================================================================================"
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
        let x2 = (bbox[2] * width as f32).round() as i32;
        let y2 = (bbox[3] * height as f32).round() as i32;
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
        let colors = [
            image::Rgb([0, 255, 0]),
            image::Rgb([0, 255, 0]),
            image::Rgb([255, 255, 0]),
            image::Rgb([0, 255, 255]),
            image::Rgb([0, 255, 255]),
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

    fn env_usize(key: &str, default: usize) -> usize {
        env::var(key)
            .ok()
            .and_then(|value| value.parse::<usize>().ok())
            .unwrap_or(default)
    }

    fn env_string(key: &str, default: &str) -> String {
        env::var(key).unwrap_or_else(|_| default.to_string())
    }

    pub fn run() -> Result<(), Box<dyn std::error::Error>> {
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
        let output_path = args
            .iter()
            .position(|arg| arg == "--output")
            .and_then(|index| args.get(index + 1))
            .cloned()
            .unwrap_or_else(|| env_string("OUTPUT_IMAGE", "result.jpg"));

        let models = CoreMlFaceModels::load(package_root)?;
        let config = InstanceConfig::default();
        let image = image::open(&image_path)?.to_rgb8();
        let mock_frame = MockFrameBuilder::new()
            .dimensions(image.width(), image.height())
            .host_data(image.clone().into_raw())
            .to_nv12(image.width())
            .opaque_kind(AV_OPAQUE_CVPIXELBUFFER)
            .build();
        let safe_frame = mock_frame.as_safe_frame();

        if args.iter().any(|arg| arg == "--compare-person-res") {
            return compare_person_models(package_root, &safe_frame, env_usize("LOOPS", 50));
        }

        let is_benchmark = args.iter().any(|arg| arg == "--benchmark");
        let stress = args.iter().position(|arg| arg == "--stress").map(|index| {
            args.get(index + 1)
                .and_then(|value| value.parse::<u64>().ok())
                .unwrap_or(30)
        });

        if is_benchmark {
            return benchmark(
                &models,
                &safe_frame,
                image.width(),
                image.height(),
                &config,
                env_usize("WARMUP", 5),
                env_usize("LOOPS", 100).max(1),
            );
        }

        if let Some(seconds) = stress {
            let deadline = Instant::now() + Duration::from_secs(seconds);
            let mut iterations = 0usize;
            while Instant::now() < deadline {
                let _ =
                    infer_pipeline(&models, &safe_frame, image.width(), image.height(), &config)?;
                iterations += 1;
            }
            eprintln!("stress: durationSeconds={seconds} iterations={iterations}");
            return Ok(());
        }

        let (results, _) =
            infer_pipeline(&models, &safe_frame, image.width(), image.height(), &config)?;
        let json = detection_json(&results);
        println!("{}", serde_json::to_string_pretty(&json)?);

        let mut result = image;
        for object in &results {
            draw_box(&mut result, object.bbox, image::Rgb([0, 200, 255]));
            if let Some(face) = &object.face {
                draw_box(&mut result, face.bbox, image::Rgb([255, 48, 48]));
                draw_landmarks(&mut result, &face.landmarks);
            }
        }
        result.save(&output_path)?;
        eprintln!("已保存检测结果图至: {output_path}");
        Ok(())
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn local_detection_json_excludes_latency_breakdown() {
            let object = AnalyzedObject {
                bbox: [0.1, 0.2, 0.4, 0.8],
                confidence: 0.9,
                face: Some(FaceAnalyzed {
                    bbox: [0.15, 0.22, 0.25, 0.35],
                    confidence: 0.95,
                    quality_score: 0.8,
                    embedding: None,
                    landmarks: [[0.25, 0.35]; 5],
                }),
            };

            let json = detection_json(&[object]);
            assert_eq!(json["schema_version"], 1);
            assert_eq!(json["objects"].as_array().map(Vec::len), Some(1));
            assert_eq!(json["objects"][0]["label"], "person");
            let face_x1 = json["objects"][0]["face"]["bbox"][0]
                .as_f64()
                .expect("face bbox x1 应为数字");
            assert!((face_x1 - 0.15).abs() < 1e-4);
            let bbox_x1 = json["objects"][0]["bbox"][0]
                .as_f64()
                .expect("person bbox x1 应为数字");
            assert!((bbox_x1 - 0.1).abs() < 1e-6);
            assert!(json.get("latency_breakdown_ms").is_none());
            assert!(json.get("preprocess").is_none());
        }

        #[test]
        fn local_detection_json_includes_backend_embedding_sidecar() {
            let object = AnalyzedObject {
                bbox: [0.1, 0.2, 0.4, 0.8],
                confidence: 0.9,
                face: Some(FaceAnalyzed {
                    bbox: [0.15, 0.22, 0.25, 0.35],
                    confidence: 0.95,
                    quality_score: 0.8,
                    embedding: Some("encoded-512d-sidecar".to_string()),
                    landmarks: [[0.25, 0.35]; 5],
                }),
            };

            let json = detection_json(&[object]);
            assert_eq!(json["objects"][0]["label"], "person");
            assert_eq!(
                json["objects"][0]["face"]["embedding"],
                "encoded-512d-sidecar"
            );
            let face_x1 = json["objects"][0]["face"]["bbox"][0]
                .as_f64()
                .expect("face bbox x1 应为数字");
            assert!((face_x1 - 0.15).abs() < 1e-4);
        }
    }
}
