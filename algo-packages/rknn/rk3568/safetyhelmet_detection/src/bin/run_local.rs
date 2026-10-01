//! 本地单机评测与可视化工具 (run_local)
//!
//! 专为开发者在本地环境下快速验证 RK3568 安全帽检测效果设计。
//! 支持 `--benchmark` 性能剖析与 `--stress [DURATION]` 满载压力测试。

fn main() -> Result<(), Box<dyn std::error::Error>> {
    runner::main()
}

mod runner {
    use std::collections::HashMap;
    use std::fmt::Write;
    use std::fs;
    use std::path::Path;

    use algo_sdk::cv::transforms::Transform;
    use algo_sdk::math::NormBox;
    use algo_sdk::plugin::{AlgoPlugin, InitContext};
    use algo_sdk::runtime::NpuSession;
    use algo_sdk::testing::{BenchmarkStats, MockEmitter, MockFrameBuilder};
    use image::{Rgb, RgbImage};
    use safetyhelmet_detection::config::InstanceConfig;
    use safetyhelmet_detection::plugin::SafetyHelmetDetector;
    use safetyhelmet_detection::postprocess::parse_and_unmap_output;

    fn load_env_file(path: &str) -> HashMap<String, String> {
        let mut map = HashMap::new();
        if let Ok(content) = fs::read_to_string(path) {
            for line in content.lines() {
                let trimmed = line.trim();
                if trimmed.is_empty() || trimmed.starts_with('#') {
                    continue;
                }
                if let Some((k, v)) = trimmed.split_once('=') {
                    let key = k.trim().to_string();
                    let mut val = v.trim().to_string();
                    if (val.starts_with('"') && val.ends_with('"'))
                        || (val.starts_with('\'') && val.ends_with('\''))
                    {
                        val = val[1..val.len() - 1].to_string();
                    }
                    map.insert(key, val);
                }
            }
        }
        map
    }

    fn get_env_str(key: &str, default_val: &str, env_map: &HashMap<String, String>) -> String {
        env_map
            .get(key)
            .cloned()
            .or_else(|| std::env::var(key).ok())
            .unwrap_or_else(|| default_val.to_string())
    }

    fn get_env_f32(key: &str, default_val: f32, env_map: &HashMap<String, String>) -> f32 {
        get_env_str(key, &default_val.to_string(), env_map)
            .parse::<f32>()
            .unwrap_or(default_val)
    }

    fn get_env_i32(key: &str, default_val: i32, env_map: &HashMap<String, String>) -> i32 {
        get_env_str(key, &default_val.to_string(), env_map)
            .parse::<i32>()
            .unwrap_or(default_val)
    }

    fn format_detection_json(event_id: &str, objects: &[NormBox]) -> String {
        let mut out = String::with_capacity(128 + objects.len() * 128);
        let _ = writeln!(out, "{{\n  \"event_id\": \"{event_id}\",");
        if objects.is_empty() {
            out.push_str("  \"objects\": []\n");
        } else {
            out.push_str("  \"objects\": [\n");
            for (i, obj) in objects.iter().enumerate() {
                let comma = if i + 1 < objects.len() { "," } else { "" };
                let label = obj.label.unwrap_or("unknown");
                let _ = writeln!(out, "    {{\n      \"class_id\": {},", obj.class_id);
                let _ = writeln!(out, "      \"label\": \"{label}\",");
                let _ = writeln!(out, "      \"confidence\": {:.4},", obj.confidence);
                let bbox = algo_sdk::math::box_xywh_to_xyxy([obj.x, obj.y, obj.w, obj.h]);
                let _ = writeln!(
                    out,
                    "      \"bbox\": [{:.4}, {:.4}, {:.4}, {:.4}]\n    }}{comma}",
                    bbox[0], bbox[1], bbox[2], bbox[3]
                );
            }
            out.push_str("  ]\n");
        }
        out.push('}');
        out
    }

    fn save_visualization(
        output_path: &str,
        img: &mut RgbImage,
        objects: &[NormBox],
    ) -> Result<(), Box<dyn std::error::Error>> {
        let red = Rgb([255u8, 60u8, 60u8]);
        let green = Rgb([60u8, 220u8, 60u8]);
        let width = img.width() as f32;
        let height = img.height() as f32;
        let max_x = img.width() as i32 - 1;
        let max_y = img.height() as i32 - 1;

        for obj in objects {
            // Hardhat=绿色, NO-Hardhat=红色
            let color = if obj.class_id == 0 { green } else { red };
            let x = (obj.x * width).round() as i32;
            let y = (obj.y * height).round() as i32;
            let w = (obj.w * width).round() as i32;
            let h = (obj.h * height).round() as i32;

            for thickness in 0..3 {
                let x1 = (x - thickness).clamp(0, max_x);
                let y1 = (y - thickness).clamp(0, max_y);
                let x2 = (x + w + thickness).clamp(0, max_x);
                let y2 = (y + h + thickness).clamp(0, max_y);

                for px in x1..=x2 {
                    img.put_pixel(px as u32, y1 as u32, color);
                    img.put_pixel(px as u32, y2 as u32, color);
                }
                for py in y1..=y2 {
                    img.put_pixel(x1 as u32, py as u32, color);
                    img.put_pixel(x2 as u32, py as u32, color);
                }
            }
        }

        img.save(output_path)?;
        Ok(())
    }

    /// 构造本地评测用的模拟帧。
    ///
    /// 启用 `testing-hardware` 特性时优先构造真实 DMA-BUF / CVPixelBuffer 硬件帧；
    /// 该特性未开启时（如 `cargo build -p` / `make host` / `make test` 的包作用域构建）
    /// 退化为步长对齐的 Host NV12 模拟帧，保证本工具始终可编译运行。
    fn build_mock_frame(
        #[cfg_attr(not(feature = "testing-hardware"), allow(unused_variables))] input_path: &str,
        rgb_image: &RgbImage,
        width: u32,
        height: u32,
    ) -> algo_sdk::testing::MockFrame {
        #[cfg(feature = "testing-hardware")]
        {
            if let Ok(builder) = MockFrameBuilder::from_image_hardware(input_path) {
                return builder.build();
            }
        }

        MockFrameBuilder::new()
            .dimensions(width, height)
            .host_data(rgb_image.clone().into_raw())
            .to_nv12(16)
            .build()
    }

    pub fn main() -> Result<(), Box<dyn std::error::Error>> {
        let env_map = load_env_file(".env");
        let confidence = get_env_f32("CONF_THRESH", 0.45, &env_map);
        let iou = get_env_f32("IOU_THRESH", 0.45, &env_map);
        let input_path = get_env_str("INPUT_IMAGE", "testimage.jpg", &env_map);
        let output_path = get_env_str("OUTPUT_IMAGE", "result.jpg", &env_map);
        let model_path = get_env_str("MODEL_PATH", "model/yolov8_hard_hat.rknn", &env_map);

        let args: Vec<String> = std::env::args().collect();
        let is_stress = args.iter().any(|arg| arg == "--stress");
        let stress_duration_secs = if is_stress {
            let mut dur = get_env_i32("DURATION", 0, &env_map);
            if dur <= 0 {
                dur = get_env_i32("STRESS_SECS", 30, &env_map);
            }
            for (idx, arg) in args.iter().enumerate() {
                if (arg == "--stress" || arg == "--duration") && idx + 1 < args.len() {
                    if let Ok(val) = args[idx + 1].parse::<i32>() {
                        dur = val;
                        break;
                    }
                }
            }
            if dur <= 0 {
                30
            } else {
                dur
            }
        } else {
            0
        };

        let benchmark = args.iter().any(|arg| arg == "--benchmark");
        let default_loops = if benchmark { 100 } else { 1 };
        let default_warmup = if benchmark { 5 } else { 0 };
        let loops = get_env_i32("LOOPS", default_loops, &env_map);
        let warmup = get_env_i32("WARMUP", default_warmup, &env_map);

        if loops <= 0 || warmup < 0 {
            eprintln!("LOOPS must be positive and WARMUP must be non-negative");
            std::process::exit(1);
        }

        println!(
            "[Config] confidence={confidence} iou={iou} input={input_path} output={output_path} model={model_path} loops={loops} warmup={warmup}"
        );

        if !Path::new(&input_path).exists() {
            eprintln!("Failed to load input image: {input_path}");
            std::process::exit(1);
        }

        // 1. 读取原图
        let dynamic_img = image::open(&input_path)?;
        let width = dynamic_img.width();
        let height = dynamic_img.height();
        let mut rgb_image = dynamic_img.to_rgb8();

        // 2. 模拟 MPP 硬件解码输出帧（16 字节行跨距对齐 NV12）
        let mock_frame = build_mock_frame(&input_path, &rgb_image, width, height);

        // 3. 初始化算法实例
        let config = InstanceConfig {
            confidence_threshold: confidence,
            iou_threshold: iou,
            custom_alarm_label: None,
            explicit_fields: ["confidence_threshold", "iou_threshold"]
                .into_iter()
                .map(String::from)
                .collect(),
        };
        let init_ctx = InitContext::new(Path::new("."), "linux-rknn", "standalone_local", false);

        let mut detector = SafetyHelmetDetector::init(&init_ctx, config)?;
        let is_fallback = detector.session.is_fallback();
        #[cfg(all(target_os = "linux", feature = "rga"))]
        let rga_hw = algo_sdk::cv::platforms::rockchip::RgaCvEngine::new().hardware_available();
        #[cfg(not(all(target_os = "linux", feature = "rga")))]
        let rga_hw = false;

        if is_fallback {
            println!("[Pipeline Mode] debug_cpu_fallback_path | 未检测到 librknnrt.so");
        } else {
            println!("[Pipeline Mode] infer_fast_path | RKNN NPU 硬件加速已就绪 (RK3568 单核)");
        }
        if !rga_hw {
            println!("[RGA Hardware]  RGA 2D 加速未就绪，预处理采用 CPU Letterbox 回退");
        } else {
            println!("[RGA Hardware]  RGA 2D 硬件加速已就绪");
        }

        if is_stress {
            println!("============================================================");
            println!("  RK3568 安全帽检测算法包高负荷持续压力测试 (Stress Test)");
            println!("============================================================");
            println!("[Stress] 计划运行时长: {stress_duration_secs} 秒");

            let start_time = std::time::Instant::now();
            let target_duration = std::time::Duration::from_secs(stress_duration_secs as u64);
            let mut total_frames: u64 = 0;
            let mut last_report = std::time::Instant::now();
            let mut interval_frames: u64 = 0;

            while start_time.elapsed() < target_duration {
                let mut mock_emitter = MockEmitter::new();
                // SAFETY: mock_emitter 存活至 process 调用结束
                let mut emitter = unsafe { mock_emitter.as_emitter(total_frames + 1) };
                let safe_frame = mock_frame.as_safe_frame();
                detector.process(safe_frame, &mut emitter)?;
                total_frames += 1;
                interval_frames += 1;

                if last_report.elapsed() >= std::time::Duration::from_secs(5) {
                    let interval_secs = last_report.elapsed().as_secs_f64();
                    let current_fps = interval_frames as f64 / interval_secs;
                    let elapsed_secs = start_time.elapsed().as_secs_f64();
                    println!(
                        "  [Progress] {elapsed_secs:.1}s / {stress_duration_secs}s | {total_frames} frames | {current_fps:.1} FPS"
                    );
                    last_report = std::time::Instant::now();
                    interval_frames = 0;
                }
            }

            let total_elapsed = start_time.elapsed().as_secs_f64();
            let avg_fps = total_frames as f64 / total_elapsed;
            let avg_latency_ms = total_elapsed * 1000.0 / total_frames as f64;

            println!("============================================================");
            println!("[Stress Test Completed] 总 {total_elapsed:.2}s | {total_frames} 帧 | {avg_fps:.1} FPS | {avg_latency_ms:.2} ms/帧");
            println!("============================================================");

            return Ok(());
        }

        // 4. Warmup
        for _ in 0..warmup {
            let safe_frame = mock_frame.as_safe_frame();
            let mut mock_emitter = MockEmitter::new();
            // SAFETY: mock_emitter 存活至 process 调用结束
            let mut emitter = unsafe { mock_emitter.as_emitter(1) };
            detector.process(safe_frame, &mut emitter)?;
        }

        // 5. 主循环
        let mut preprocess_samples = Vec::with_capacity(loops as usize);
        let mut inference_samples = Vec::with_capacity(loops as usize);
        let mut postprocess_samples = Vec::with_capacity(loops as usize);
        let mut end_to_end_samples = Vec::with_capacity(loops as usize);
        let mut abi_process_samples = Vec::with_capacity(loops as usize);

        for i in 0..loops {
            let mut mock_emitter = MockEmitter::new();
            // SAFETY: mock_emitter 存活至 process 调用结束
            let mut emitter = unsafe { mock_emitter.as_emitter((i + 1) as u64) };

            let t_abi_start = std::time::Instant::now();
            let safe_frame = mock_frame.as_safe_frame();
            detector.process(safe_frame, &mut emitter)?;
            let abi_elapsed = t_abi_start.elapsed().as_secs_f64() * 1000.0;
            abi_process_samples.push(abi_elapsed);

            if benchmark {
                let safe_frame = mock_frame.as_safe_frame();
                let t0 = std::time::Instant::now();
                let (buf, mode) = detector.transform.apply(&safe_frame)?;
                let t1 = std::time::Instant::now();
                let orig_w = safe_frame.width();
                let orig_h = safe_frame.height();
                // 分阶段计时需要绕过 `process` 单独驱动推理；解码仍复用
                // `parse_and_unmap_output`（配置来自 `SafetyHelmetSpec`，与生产同源），
                // 因此这里测的是真实后处理而非一份手写的近似实现。
                let custom_label: Option<&'static str> = None;
                let mut t2 = std::time::Instant::now();
                let mut t3 = std::time::Instant::now();

                detector.session.infer_with(&buf, |net_out| {
                    t2 = std::time::Instant::now();
                    let _boxes = parse_and_unmap_output(
                        net_out,
                        &detector.config,
                        custom_label,
                        &mode,
                        orig_w,
                        orig_h,
                    );
                    t3 = std::time::Instant::now();
                    Ok(())
                })?;

                preprocess_samples.push((t1 - t0).as_secs_f64() * 1000.0);
                inference_samples.push((t2 - t1).as_secs_f64() * 1000.0);
                postprocess_samples.push((t3 - t2).as_secs_f64() * 1000.0);
                end_to_end_samples.push((t3 - t0).as_secs_f64() * 1000.0);
            }

            // 首帧输出检测结果
            if i == 0 {
                let event_id = mock_emitter
                    .raw_json_events()
                    .first()
                    .and_then(|raw| serde_json::from_str::<serde_json::Value>(raw).ok())
                    .and_then(|v| {
                        v.get("event_id")
                            .and_then(|id| id.as_str())
                            .map(|s| s.to_string())
                    })
                    .unwrap_or_else(|| "standalone-event".to_string());

                let pretty_json = format_detection_json(&event_id, mock_emitter.detections());
                println!("[Detection Result]\n{pretty_json}");

                save_visualization(&output_path, &mut rgb_image, mock_emitter.detections())?;
                println!("[Visualizer] Saved result image to {output_path}");
            }
        }

        if benchmark {
            let pp_stats = BenchmarkStats::compute(preprocess_samples);
            let inf_stats = BenchmarkStats::compute(inference_samples);
            let post_stats = BenchmarkStats::compute(postprocess_samples);
            let e2e_stats = BenchmarkStats::compute(end_to_end_samples);
            let abi_stats = BenchmarkStats::compute(abi_process_samples);

            let mode_desc = if is_fallback {
                "debug_cpu_fallback_path"
            } else {
                "infer_fast_path"
            };

            println!(
                "\n--- Benchmark Report ({loops} iterations, {warmup} warmup) [{mode_desc}] ---"
            );
            if is_fallback {
                println!("  [说明] 当前环境非 RK3568 实体板端，Inference 耗时仅反映回退传递开销");
            }
            println!(
                "  Preprocess:  Avg {:.2} ms | P50 {:.2} ms | P99 {:.2} ms",
                pp_stats.avg_ms, pp_stats.p50_ms, pp_stats.p99_ms
            );
            println!(
                "  Inference:   Avg {:.2} ms | P50 {:.2} ms | P99 {:.2} ms",
                inf_stats.avg_ms, inf_stats.p50_ms, inf_stats.p99_ms
            );
            println!(
                "  Postprocess: Avg {:.2} ms | P50 {:.2} ms | P99 {:.2} ms",
                post_stats.avg_ms, post_stats.p50_ms, post_stats.p99_ms
            );
            println!(
                "  End-to-end:  Avg {:.2} ms | P50 {:.2} ms | P99 {:.2} ms | FPS: {:.1}",
                e2e_stats.avg_ms, e2e_stats.p50_ms, e2e_stats.p99_ms, e2e_stats.fps
            );
            println!(
                "  ABI process: Avg {:.2} ms | P50 {:.2} ms | P99 {:.2} ms | FPS: {:.1}",
                abi_stats.avg_ms, abi_stats.p50_ms, abi_stats.p99_ms, abi_stats.fps
            );
        }

        Ok(())
    }
}
