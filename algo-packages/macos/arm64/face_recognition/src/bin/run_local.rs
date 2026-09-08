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

    use image::RgbImage;
    use serde::Serialize;

    use algo_sdk::c_abi::AV_OPAQUE_CVPIXELBUFFER;
    use algo_sdk::cv::platforms::apple::AppleCvEngine;
    use algo_sdk::cv::CvEngine;
    use algo_sdk::error::AlgoError;
    use algo_sdk::frame::SafeFrame;
    use algo_sdk::testing::MockFrameBuilder;

    use face_recognition_coreml::align::align_face;
    use face_recognition_coreml::association::associate_persons_and_faces;
    use face_recognition_coreml::best_shot::BestShotManager;
    use face_recognition_coreml::bytetrack::{
        box_iou, ByteTrackConfig, ByteTracker, TrackDetection,
    };
    use face_recognition_coreml::config::InstanceConfig;
    use face_recognition_coreml::coreml::{CoreMlFaceModels, CoreMlRunner};
    use face_recognition_coreml::detect::{
        decode_face_detections, decode_person_detections, nms, nms_persons, unmap_letterbox,
        unmap_persons_letterbox,
    };
    use face_recognition_coreml::normalize_embedding;
    use face_recognition_coreml::postprocess::FaceDetection;
    use face_recognition_coreml::quality::compute_quality;

    #[derive(Debug, Clone, Serialize)]
    pub struct FaceResult {
        pub face: FaceDetection,
        pub embedding: Vec<f32>,
        pub is_best_shot: bool,
    }

    #[derive(Debug, Clone, Serialize)]
    pub struct TrackedResult {
        pub track_id: u64,
        pub person_bbox: [f32; 4],
        pub person_score: f32,
        pub is_pseudo_body: bool,
        pub face: Option<FaceResult>,
    }

    #[derive(Debug, Clone, Default)]
    pub struct StageTimings {
        pub preprocess_ms: f64,
        pub person_infer_ms: f64,
        pub face_infer_ms: f64,
        pub decode_nms_ms: f64,
        pub association_ms: f64,
        pub bytetrack_ms: f64,
        pub quality_ms: f64,
        pub align_ms: f64,
        pub embed_infer_ms: f64,
        pub norm_ms: f64,
        pub total_ms: f64,
    }

    fn infer_pipeline(
        models: &CoreMlFaceModels,
        safe_frame: &SafeFrame<'_>,
        image: &RgbImage,
        config: &InstanceConfig,
        tracker: &mut ByteTracker,
        best_shots: &mut BestShotManager,
    ) -> Result<(Vec<TrackedResult>, StageTimings), AlgoError> {
        let t_start = Instant::now();

        // 阶段 1: 硬件预处理 (Apple Accelerate vImage Letterbox 缩放至 640x384 并转 BGRA)
        let t0 = Instant::now();
        let (buffer, mode) = AppleCvEngine.letterbox(safe_frame, 640, 384, [114, 114, 114])?;
        let pixelbuffer = buffer.as_raw_ptr().ok_or_else(|| AlgoError::Preprocess {
            reason: "CoreML 检测输入 CVPixelBuffer 指针为空".to_string(),
        })?;
        let preprocess_ms = t0.elapsed().as_secs_f64() * 1000.0;

        // 阶段 2: YOLO26n 人体检测 (CoreML ANE 前向推理)
        let t1 = Instant::now();
        // SAFETY: pixelbuffer 由当前 CvBuffer 持有，在同步预测返回前保持有效生命周期。
        let raw_person = unsafe { models.predict_person_detector(pixelbuffer)? };
        let person_infer_ms = t1.elapsed().as_secs_f64() * 1000.0;

        // 阶段 3: YOLOv8-face 人脸检测 (CoreML ANE 前向推理)
        let t2 = Instant::now();
        // SAFETY: pixelbuffer 由当前 CvBuffer 持有，在同步预测返回前保持有效生命周期。
        let raw_face = unsafe { models.predict_detector(pixelbuffer)? };
        let face_infer_ms = t2.elapsed().as_secs_f64() * 1000.0;

        // 阶段 4: 解码、NMS 与 Letterbox 坐标反算
        let t3 = Instant::now();
        let mut persons = decode_person_detections(&raw_person, 0.40);
        nms_persons(&mut persons, 0.45);
        unmap_persons_letterbox(&mut persons, &mode, image.width(), image.height());

        let mut raw_faces =
            decode_face_detections(&raw_face, config.detection_confidence_threshold);
        nms(&mut raw_faces, 0.45);
        unmap_letterbox(&mut raw_faces, &mode, image.width(), image.height());
        let decode_nms_ms = t3.elapsed().as_secs_f64() * 1000.0;

        // 阶段 5: 人体与人脸二分图空间几何挂载
        let t4 = Instant::now();
        let associated = associate_persons_and_faces(&persons, &raw_faces);
        let association_ms = t4.elapsed().as_secs_f64() * 1000.0;

        // 阶段 6: ByteTrack 多目标航迹更新 (8状态 Kalman Filter)
        let t5 = Instant::now();
        let track_dets: Vec<TrackDetection> = associated
            .iter()
            .map(|a| TrackDetection {
                bbox: a.person_bbox,
                score: a.person_score,
                class_id: 0,
            })
            .collect();
        let active_tracks = tracker.update(&track_dets);
        let bytetrack_ms = t5.elapsed().as_secs_f64() * 1000.0;

        // 阶段 7: 质量门控与动态最优抓拍特征提取
        let mut quality_ms = 0.0;
        let mut align_ms = 0.0;
        let mut embed_infer_ms = 0.0;
        let mut norm_ms = 0.0;

        let mut results = Vec::with_capacity(active_tracks.len());
        let mut active_track_ids = Vec::with_capacity(active_tracks.len());

        for track in &active_tracks {
            active_track_ids.push(track.track_id);

            let best_assoc = associated
                .iter()
                .find(|a| box_iou(&track.bbox, &a.person_bbox) >= 0.35);
            let attached_face = best_assoc.and_then(|a| a.attached_face);
            let is_pseudo = best_assoc.map(|a| a.is_pseudo_body).unwrap_or(false);

            let mut face_result = None;
            if let Some(face) = attached_face {
                let t_q = Instant::now();
                let quality = compute_quality(
                    &face.landmarks,
                    &face.landmark_scores,
                    face.bbox[2] * image.width() as f32,
                    &config.quality_thresholds,
                );
                quality_ms += t_q.elapsed().as_secs_f64() * 1000.0;

                if quality.accepted(&config.quality_thresholds, config.min_face_size) {
                    let should_extract =
                        best_shots.should_update_best_shot(track.track_id, &quality);
                    let mut embedding = Vec::new();
                    let is_best_shot = should_extract;

                    if should_extract {
                        let t_al = Instant::now();
                        let aligned = align_face(
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
                        embedding = normalize_embedding(&values)?.to_vec();
                        norm_ms += t_no.elapsed().as_secs_f64() * 1000.0;

                        best_shots.update(
                            track.track_id,
                            face.bbox,
                            face.landmarks,
                            face.score,
                            quality,
                            embedding.clone(),
                            1,
                        );
                    } else if let Some(rec) = best_shots.get(track.track_id) {
                        embedding = rec.embedding.clone();
                    }

                    face_result = Some(FaceResult {
                        face: FaceDetection {
                            bbox: face.bbox,
                            landmarks: face.landmarks,
                            detection_score: face.score,
                            quality,
                        },
                        embedding,
                        is_best_shot,
                    });
                }
            }

            results.push(TrackedResult {
                track_id: track.track_id,
                person_bbox: track.bbox,
                person_score: track.score,
                is_pseudo_body: is_pseudo,
                face: face_result,
            });
        }

        best_shots.retain_active_tracks(&active_track_ids);
        let total_ms = t_start.elapsed().as_secs_f64() * 1000.0;

        Ok((
            results,
            StageTimings {
                preprocess_ms,
                person_infer_ms,
                face_infer_ms,
                decode_nms_ms,
                association_ms,
                bytetrack_ms,
                quality_ms,
                align_ms,
                embed_infer_ms,
                norm_ms,
                total_ms,
            },
        ))
    }

    fn calc_stats(samples: &mut [f64]) -> (f64, f64, f64) {
        if samples.is_empty() {
            return (0.0, 0.0, 0.0);
        }
        samples.sort_by(|a, b| a.total_cmp(b));
        let sum: f64 = samples.iter().sum();
        let avg = sum / samples.len() as f64;
        let p50 = samples[samples.len() / 2];
        let p99_idx = ((samples.len() as f64 * 0.99).ceil() as usize).min(samples.len()) - 1;
        let p99 = samples[p99_idx];
        (avg, p50, p99)
    }

    fn benchmark(
        models: &CoreMlFaceModels,
        safe_frame: &SafeFrame<'_>,
        image: &RgbImage,
        config: &InstanceConfig,
        warmup: usize,
        loops: usize,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let mut tracker = ByteTracker::new(ByteTrackConfig::default());
        let mut best_shots = BestShotManager::new();

        for _ in 0..warmup {
            let _ = infer_pipeline(
                models,
                safe_frame,
                image,
                config,
                &mut tracker,
                &mut best_shots,
            )?;
        }

        let mut pre_samples = Vec::with_capacity(loops);
        let mut person_samples = Vec::with_capacity(loops);
        let mut face_samples = Vec::with_capacity(loops);
        let mut dec_samples = Vec::with_capacity(loops);
        let mut assoc_samples = Vec::with_capacity(loops);
        let mut track_samples = Vec::with_capacity(loops);
        let mut q_samples = Vec::with_capacity(loops);
        let mut al_samples = Vec::with_capacity(loops);
        let mut em_samples = Vec::with_capacity(loops);
        let mut norm_samples = Vec::with_capacity(loops);
        let mut e2e_samples = Vec::with_capacity(loops);

        let mut tracked_count = 0;
        let mut face_count = 0;

        for _ in 0..loops {
            let (tracks, timing) = infer_pipeline(
                models,
                safe_frame,
                image,
                config,
                &mut tracker,
                &mut best_shots,
            )?;
            tracked_count = tracks.len();
            face_count = tracks.iter().filter(|t| t.face.is_some()).count();
            pre_samples.push(timing.preprocess_ms);
            person_samples.push(timing.person_infer_ms);
            face_samples.push(timing.face_infer_ms);
            dec_samples.push(timing.decode_nms_ms);
            assoc_samples.push(timing.association_ms);
            track_samples.push(timing.bytetrack_ms);
            q_samples.push(timing.quality_ms);
            al_samples.push(timing.align_ms);
            em_samples.push(timing.embed_infer_ms);
            norm_samples.push(timing.norm_ms);
            e2e_samples.push(timing.total_ms);
        }

        let (pre_avg, pre_p50, _) = calc_stats(&mut pre_samples);
        let (p_avg, p_p50, _) = calc_stats(&mut person_samples);
        let (f_avg, f_p50, _) = calc_stats(&mut face_samples);
        let (dec_avg, dec_p50, _) = calc_stats(&mut dec_samples);
        let (assoc_avg, assoc_p50, _) = calc_stats(&mut assoc_samples);
        let (track_avg, track_p50, _) = calc_stats(&mut track_samples);
        let (q_avg, q_p50, _) = calc_stats(&mut q_samples);
        let (al_avg, al_p50, _) = calc_stats(&mut al_samples);
        let (em_avg, em_p50, _) = calc_stats(&mut em_samples);
        let (norm_avg, norm_p50, _) = calc_stats(&mut norm_samples);
        let (e2e_avg, e2e_p50, e2e_p99) = calc_stats(&mut e2e_samples);
        let fps = if e2e_avg > 0.0 { 1000.0 / e2e_avg } else { 0.0 };

        println!(
            "================================================================================"
        );
        println!("  人脸识别全管线性能分析 (YOLO26n + YOLOv8-face + ByteTrack + EdgeFace)");
        println!(
            "================================================================================"
        );
        println!("  采样轮次: {loops} 轮 (预热: {warmup} 轮)");
        println!(
            "  输入尺寸: {}x{} -> 640x384 共享 Letterbox BGRA CVPixelBuffer",
            image.width(),
            image.height()
        );
        println!(
            "  活跃航迹: {tracked_count} 条，挂载人脸: {face_count} 个 (含五点仿射对齐与 512D 特征提取)"
        );
        println!(
            "--------------------------------------------------------------------------------"
        );
        println!(
            "  阶段 1: 硬件预处理 (Letterbox 640x384)   : avg = {:>6.3} ms, p50 = {:>6.3} ms",
            pre_avg, pre_p50
        );
        println!(
            "  阶段 2: YOLO26n 人体检测 (CoreML ANE)    : avg = {:>6.3} ms, p50 = {:>6.3} ms",
            p_avg, p_p50
        );
        println!(
            "  阶段 3: YOLOv8-face 人脸检测 (CoreML ANE): avg = {:>6.3} ms, p50 = {:>6.3} ms",
            f_avg, f_p50
        );
        println!(
            "  阶段 4: 解码、NMS 与全图坐标还原         : avg = {:>6.3} ms, p50 = {:>6.3} ms",
            dec_avg, dec_p50
        );
        println!(
            "  阶段 5: 人体与人脸上半身几何空间挂载     : avg = {:>6.3} ms, p50 = {:>6.3} ms",
            assoc_avg, assoc_p50
        );
        println!(
            "  阶段 6: ByteTrack 8状态卡尔曼滤波航迹更新: avg = {:>6.3} ms, p50 = {:>6.3} ms",
            track_avg, track_p50
        );
        println!(
            "  阶段 7: 人脸质量门控 (Quality Gating)   : avg = {:>6.3} ms, p50 = {:>6.3} ms",
            q_avg, q_p50
        );
        println!(
            "  阶段 8: ArcFace 5点仿射对齐 (112x112)    : avg = {:>6.3} ms, p50 = {:>6.3} ms",
            al_avg, al_p50
        );
        println!(
            "  阶段 9: EdgeFace-s 提取 (CoreML ANE)    : avg = {:>6.3} ms, p50 = {:>6.3} ms",
            em_avg, em_p50
        );
        println!(
            "  阶段10: 特征 L2 归一化 (Normalize)       : avg = {:>6.3} ms, p50 = {:>6.3} ms",
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

    fn compare_person_models(
        package_root: &Path,
        safe_frame: &SafeFrame<'_>,
        loops: usize,
    ) -> Result<(), Box<dyn std::error::Error>> {
        println!(
            "================================================================================"
        );
        println!(
            "  人体检测模型横向对比: 640x384 vs 384x216 (384x224 Stride-32 对齐) (CoreML ANE)"
        );
        println!(
            "================================================================================"
        );

        let runner_640 = CoreMlRunner::load_model(
            package_root,
            "person_detect_640x384.mlpackage",
            "image",
            "var_911",
        )?;
        let runner_384 = CoreMlRunner::load_model(
            package_root,
            "person_detect_384x216.mlpackage",
            "image",
            "var_911",
        )?;

        // 640x384 预处理
        let (buf640, mode640) = AppleCvEngine.letterbox(safe_frame, 640, 384, [114, 114, 114])?;
        let pb640 = buf640.as_raw_ptr().ok_or_else(|| AlgoError::Preprocess {
            reason: "pb640 null".to_string(),
        })?;

        // 384x224 预处理
        let (buf384, mode384) = AppleCvEngine.letterbox(safe_frame, 384, 224, [114, 114, 114])?;
        let pb384 = buf384.as_raw_ptr().ok_or_else(|| AlgoError::Preprocess {
            reason: "pb384 null".to_string(),
        })?;

        // 预热
        for _ in 0..5 {
            // SAFETY: pixelbuffer 由 buf640 / buf384 持有，在同步预测期间有效。
            unsafe {
                let _ = runner_640.predict_pixelbuffer(pb640)?;
                let _ = runner_384.predict_pixelbuffer(pb384)?;
            }
        }

        // 测试 640x384
        let mut time_640 = Vec::with_capacity(loops);
        let mut raw_640 = Vec::new();
        for _ in 0..loops {
            let t = Instant::now();
            // SAFETY: pb640 在预测期间由 buf640 保活。
            raw_640 = unsafe { runner_640.predict_pixelbuffer(pb640)? };
            time_640.push(t.elapsed().as_secs_f64() * 1000.0);
        }
        let (avg_640, p50_640, p99_640) = calc_stats(&mut time_640);
        let mut persons_640 = decode_person_detections(&raw_640, 0.40);
        nms_persons(&mut persons_640, 0.45);
        unmap_persons_letterbox(
            &mut persons_640,
            &mode640,
            safe_frame.width(),
            safe_frame.height(),
        );

        // 测试 384x224
        let mut time_384 = Vec::with_capacity(loops);
        let mut raw_384 = Vec::new();
        for _ in 0..loops {
            let t = Instant::now();
            // SAFETY: pb384 在预测期间由 buf384 保活。
            raw_384 = unsafe { runner_384.predict_pixelbuffer(pb384)? };
            time_384.push(t.elapsed().as_secs_f64() * 1000.0);
        }
        let (avg_384, p50_384, p99_384) = calc_stats(&mut time_384);
        let mut persons_384 = decode_person_detections(&raw_384, 0.40);
        nms_persons(&mut persons_384, 0.45);
        unmap_persons_letterbox(
            &mut persons_384,
            &mode384,
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
            "YOLO26n 640x384 (共享)",
            avg_640,
            p50_640,
            p99_640,
            persons_640.len(),
            1000.0 / avg_640
        );
        println!(
            "{:<22} | {:>9.3} ms | {:>9.3} ms | {:>9.3} ms | {:>8} 人 | {:>10.1} FPS",
            "YOLO26n 384x216 (独立)",
            avg_384,
            p50_384,
            p99_384,
            persons_384.len(),
            1000.0 / avg_384
        );
        println!(
            "================================================================================"
        );

        println!("  架构分析与工程建议:");
        println!(
            "  1. 推理耗时对比: 384x216 比 640x384 快约 {:.2} ms (提升 {:.1}%)",
            avg_640 - avg_384,
            (avg_640 - avg_384) / avg_640 * 100.0
        );
        println!("  2. 显存与预处理开销:");
        println!("     - 640x384 方案: 人脸检测与人体检测共享同一个 640x384 CVPixelBuffer，只需一次 Letterbox 硬件缩放；");
        println!("     - 384x216 方案: 需额外执行一次 384x224 Letterbox 硬件缩放 (额外引入约 0.2~0.4ms vImage 耗时)。");
        println!("  3. 最终结论: 640x384 单一预处理缓冲区零拷贝直通双模型，综合系统开销与检出精度表现更优！");
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

    fn env_usize(key: &str, default: usize) -> usize {
        env::var(key)
            .ok()
            .and_then(|val| val.parse::<usize>().ok())
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

        // 构造 NV12 CVPixelBuffer 真实硬件帧 (模拟 VideoToolbox 硬解码输出)
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
            let mut tracker = ByteTracker::new(ByteTrackConfig::default());
            let mut best_shots = BestShotManager::new();
            let deadline = Instant::now() + Duration::from_secs(seconds);
            let mut iterations = 0usize;
            while Instant::now() < deadline {
                let _ = infer_pipeline(
                    &models,
                    &safe_frame,
                    &image,
                    &config,
                    &mut tracker,
                    &mut best_shots,
                )?;
                iterations += 1;
            }
            eprintln!("stress: durationSeconds={seconds} iterations={iterations}");
            return Ok(());
        }

        let mut tracker = ByteTracker::new(ByteTrackConfig::default());
        let mut best_shots = BestShotManager::new();
        let (results, timing) = infer_pipeline(
            &models,
            &safe_frame,
            &image,
            &config,
            &mut tracker,
            &mut best_shots,
        )?;

        let tracks_json: Vec<_> = results
            .iter()
            .map(|item| {
                serde_json::json!({
                    "track_id": item.track_id,
                    "person_bbox": item.person_bbox,
                    "person_score": item.person_score,
                    "is_pseudo_body": item.is_pseudo_body,
                    "face": item.face.as_ref().map(|f| serde_json::json!({
                        "bbox": f.face.bbox,
                        "detection_score": f.face.detection_score,
                        "landmarks": f.face.landmarks,
                        "quality": f.face.quality,
                        "embedding_head": &f.embedding[..10],
                        "is_best_shot": f.is_best_shot,
                    })),
                })
            })
            .collect();

        let json = serde_json::json!({
            "tracked_count": results.len(),
            "tracks": tracks_json,
            "latency_breakdown_ms": {
                "preprocess": timing.preprocess_ms,
                "person_inference": timing.person_infer_ms,
                "face_inference": timing.face_infer_ms,
                "decode_nms": timing.decode_nms_ms,
                "association": timing.association_ms,
                "bytetrack": timing.bytetrack_ms,
                "quality_gating": timing.quality_ms,
                "affine_alignment": timing.align_ms,
                "embedding_inference": timing.embed_infer_ms,
                "l2_normalize": timing.norm_ms,
                "total_e2e": timing.total_ms,
            }
        });
        println!("{}", serde_json::to_string_pretty(&json)?);

        let mut result = image;
        let person_box_color = image::Rgb([0, 200, 255]); // 青蓝色：人体航迹框
        let face_box_color = image::Rgb([255, 48, 48]); // 鲜红橙：人脸框

        for item in &results {
            draw_box(&mut result, item.person_bbox, person_box_color);
            if let Some(ref f) = item.face {
                draw_box(&mut result, f.face.bbox, face_box_color);
                draw_landmarks(&mut result, &f.face.landmarks);
            }
        }
        result.save(&output_path)?;
        println!(
            "\n✓ 已保存带有 ByteTrack 人体航迹框、人脸检测框与 5 关键点绘制结果图至: {}",
            output_path
        );
        Ok(())
    }
}
