//! 本地单机评测与可视化工具 (run_local)
//!
//! 支持从 .env / 环境变量加载调试配置，
//! 支持运行单帧检测并输出结果图片 (result.jpg)，
//! 支持 `--benchmark` 性能剖析模式计算 Preprocess / Inference / Postprocess / End-to-end / ABI process 的 P50/P99/Avg 与 FPS。

use std::collections::HashMap;
use std::ffi::c_void;
use std::fs;
use std::path::Path;

use algo_sdk::c_abi::{AvAlgoResult, AV_OPAQUE_CVPIXELBUFFER};
use algo_sdk::cv::engine::CvEngine;
use algo_sdk::cv::platforms::apple::AppleCvEngine;
use algo_sdk::emitter::ResultEmitter;
use algo_sdk::math::NormBox;
use algo_sdk::plugin::{AlgoPlugin, InitContext};
use algo_sdk::testing::{MockEmitter, MockFrameBuilder};
use general_detection::config::InstanceConfig;
use general_detection::plugin::GeneralDetector;
use general_detection::postprocess::parse_and_unmap_detections;
use image::{Rgb, RgbImage};

/// 解析 .env 文件为键值对
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

fn get_env_str(key: &str, default_val: &str, local_env: &HashMap<String, String>) -> String {
    local_env
        .get(key)
        .cloned()
        .or_else(|| std::env::var(key).ok())
        .unwrap_or_else(|| default_val.to_string())
}

fn get_env_f32(key: &str, default_val: f32, local_env: &HashMap<String, String>) -> f32 {
    get_env_str(key, &default_val.to_string(), local_env)
        .parse::<f32>()
        .unwrap_or(default_val)
}

fn get_env_i32(key: &str, default_val: i32, local_env: &HashMap<String, String>) -> i32 {
    get_env_str(key, &default_val.to_string(), local_env)
        .parse::<i32>()
        .unwrap_or(default_val)
}

/// 性能基准统计
#[derive(Debug, Default)]
struct BenchmarkStats {
    avg_ms: f64,
    p50_ms: f64,
    p99_ms: f64,
    fps: f64,
}

impl BenchmarkStats {
    fn compute(mut samples: Vec<f64>) -> Self {
        if samples.is_empty() {
            return Self::default();
        }
        samples.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        let sum: f64 = samples.iter().sum();
        let avg_ms = sum / samples.len() as f64;
        let p50_idx = samples.len() * 50 / 100;
        let p99_idx = (samples.len() * 99 / 100).min(samples.len() - 1);
        let p50_ms = samples[p50_idx];
        let p99_ms = samples[p99_idx];
        let fps = if avg_ms > 0.0 { 1000.0 / avg_ms } else { 0.0 };
        Self {
            avg_ms,
            p50_ms,
            p99_ms,
            fps,
        }
    }
}

unsafe extern "C" fn on_result_callback(result: *const AvAlgoResult, user_data: *mut c_void) {
    if !result.is_null() && !user_data.is_null() {
        // SAFETY: user_data 指向有效 MockEmitter 实例
        let emitter = unsafe { &mut *(user_data as *mut MockEmitter) };
        // SAFETY: result 为合法指针
        emitter.record_c_result(unsafe { &*result });
    }
}

fn format_detection_json(event_id: &str, objects: &[NormBox]) -> String {
    let mut out = String::new();
    out.push_str("{\n");
    out.push_str(&format!("  \"event_id\": \"{event_id}\",\n"));
    if objects.is_empty() {
        out.push_str("  \"objects\": []\n");
    } else {
        out.push_str("  \"objects\": [\n");
        for (i, obj) in objects.iter().enumerate() {
            let comma = if i + 1 < objects.len() { "," } else { "" };
            let label = obj.label.unwrap_or("object");
            out.push_str("    {\n");
            out.push_str(&format!("      \"class_id\": {},\n", obj.class_id));
            out.push_str(&format!("      \"label\": \"{label}\",\n"));
            out.push_str(&format!("      \"confidence\": {:.4},\n", obj.confidence));
            out.push_str(&format!(
                "      \"bbox\": [{:.4}, {:.4}, {:.4}, {:.4}]\n",
                obj.x, obj.y, obj.w, obj.h
            ));
            out.push_str(&format!("    }}{comma}\n"));
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
    let green = Rgb([0u8, 255u8, 0u8]);
    let width = img.width() as f32;
    let height = img.height() as f32;
    let max_x = img.width() as i32 - 1;
    let max_y = img.height() as i32 - 1;

    for obj in objects {
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
                img.put_pixel(px as u32, y1 as u32, green);
                img.put_pixel(px as u32, y2 as u32, green);
            }
            for py in y1..=y2 {
                img.put_pixel(x1 as u32, py as u32, green);
                img.put_pixel(x2 as u32, py as u32, green);
            }
        }
    }

    img.save(output_path)?;
    Ok(())
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let env_map = load_env_file(".env");
    let confidence = get_env_f32("CONF_THRESH", 0.5, &env_map);
    let iou = get_env_f32("IOU_THRESH", 0.45, &env_map);
    let input_path = get_env_str("INPUT_IMAGE", "testimage.jpg", &env_map);
    let output_path = get_env_str("OUTPUT_IMAGE", "result.jpg", &env_map);
    let model_path = get_env_str("MODEL_PATH", "model/yolo26n.mlpackage", &env_map);
    let target_classes_raw = get_env_str("TARGET_CLASSES", "", &env_map);
    let target_classes: Vec<String> = if target_classes_raw.trim().is_empty() {
        Vec::new()
    } else {
        target_classes_raw
            .split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect()
    };

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

    // 2. 构造 NV12 CVPixelBuffer 真实硬件帧
    let mock_frame = MockFrameBuilder::new()
        .dimensions(width, height)
        .host_data(rgb_image.clone().into_raw())
        .to_nv12(width)
        .opaque_kind(AV_OPAQUE_CVPIXELBUFFER)
        .build();

    // 3. 初始化算法实例
    let config = InstanceConfig {
        confidence_threshold: confidence,
        iou_threshold: iou,
        target_classes,
        custom_alarm_label: None,
    };
    let init_ctx = InitContext {
        package_root: Path::new("."),
        platform_id: "macos-arm64-coreml",
        instance_id: "standalone",
        is_self_test: false,
    };

    let mut detector = GeneralDetector::init(&init_ctx, config)?;

    if is_stress {
        println!("============================================================");
        println!("  CoreML 算法包高负荷持续压力测试 (Stress Test)");
        println!("============================================================");
        println!("[Stress] 计划运行时长: {stress_duration_secs} 秒");
        println!("[Stress] 目标模型: {model_path}");
        println!("[Stress] 开始满载连续推理...");

        let start_time = std::time::Instant::now();
        let target_duration = std::time::Duration::from_secs(stress_duration_secs as u64);
        let mut total_frames: u64 = 0;
        let mut last_report = std::time::Instant::now();
        let mut interval_frames: u64 = 0;

        while start_time.elapsed() < target_duration {
            let mut mock_emitter = MockEmitter::new();
            // SAFETY: on_result_callback 与 mock_emitter 在本作用域有效存活
            let mut emitter = unsafe {
                ResultEmitter::from_raw(
                    total_frames + 1,
                    Some(on_result_callback),
                    &mut mock_emitter as *mut _ as *mut c_void,
                )
            };
            let safe_frame = mock_frame.as_safe_frame();
            detector.process(safe_frame, &mut emitter)?;
            total_frames += 1;
            interval_frames += 1;

            if last_report.elapsed() >= std::time::Duration::from_secs(5) {
                let interval_secs = last_report.elapsed().as_secs_f64();
                let current_fps = interval_frames as f64 / interval_secs;
                let elapsed_secs = start_time.elapsed().as_secs_f64();
                println!(
                    "  [Progress] 已运行: {:.1}s / {}s | 累计处理: {} 帧 | 当前实时 FPS: {:.1}",
                    elapsed_secs, stress_duration_secs, total_frames, current_fps
                );
                last_report = std::time::Instant::now();
                interval_frames = 0;
            }
        }

        let total_elapsed = start_time.elapsed().as_secs_f64();
        let avg_fps = total_frames as f64 / total_elapsed;
        let avg_latency_ms = total_elapsed * 1000.0 / total_frames as f64;

        println!("============================================================");
        println!("✓ [Stress Test Completed] 压力测试顺利完成，无异常或崩溃！");
        println!("  总运行时长: {total_elapsed:.2} 秒");
        println!("  总处理帧数: {total_frames} 帧");
        println!("  全链路平均吞吐: {avg_fps:.1} FPS");
        println!("  单帧平均耗时: {avg_latency_ms:.2} ms");
        println!("============================================================");

        return Ok(());
    }

    // 4. Warmup 预热轮次
    for _ in 0..warmup {
        let safe_frame = mock_frame.as_safe_frame();
        let mut mock_emitter = MockEmitter::new();
        // SAFETY: on_result_callback 与 mock_emitter 在本作用域有效存活
        let mut emitter = unsafe {
            ResultEmitter::from_raw(
                1,
                Some(on_result_callback),
                &mut mock_emitter as *mut _ as *mut c_void,
            )
        };
        detector.process(safe_frame, &mut emitter)?;
    }

    // 5. 主循环测试与性能分析采样
    let mut preprocess_samples = Vec::with_capacity(loops as usize);
    let mut inference_samples = Vec::with_capacity(loops as usize);
    let mut postprocess_samples = Vec::with_capacity(loops as usize);
    let mut end_to_end_samples = Vec::with_capacity(loops as usize);
    let mut abi_process_samples = Vec::with_capacity(loops as usize);

    for i in 0..loops {
        let mut mock_emitter = MockEmitter::new();
        // SAFETY: on_result_callback 与 mock_emitter 在本作用域有效存活
        let mut emitter = unsafe {
            ResultEmitter::from_raw(
                1,
                Some(on_result_callback),
                &mut mock_emitter as *mut _ as *mut c_void,
            )
        };

        let t_abi_start = std::time::Instant::now();
        let safe_frame = mock_frame.as_safe_frame();
        detector.process(safe_frame, &mut emitter)?;
        let abi_elapsed = t_abi_start.elapsed().as_secs_f64() * 1000.0;
        abi_process_samples.push(abi_elapsed);

        if benchmark {
            let safe_frame = mock_frame.as_safe_frame();
            let t0 = std::time::Instant::now();
            let (buf, mode) = AppleCvEngine.letterbox(&safe_frame, 640, 384, [114, 114, 114])?;
            let t1 = std::time::Instant::now();
            let pixelbuffer = buf
                .as_raw_ptr()
                .ok_or_else(|| std::io::Error::other("CVPixelBuffer 为空"))?;
            // SAFETY: pixelbuffer 在当前作用域生命周期内有效
            let raw_output = unsafe { detector.runner.predict_pixelbuffer(pixelbuffer)? };
            let t2 = std::time::Instant::now();
            let _boxes = parse_and_unmap_detections(
                &raw_output,
                &detector.config,
                &detector.mask,
                &mode,
                safe_frame.width(),
                safe_frame.height(),
            );
            let t3 = std::time::Instant::now();

            preprocess_samples.push((t1 - t0).as_secs_f64() * 1000.0);
            inference_samples.push((t2 - t1).as_secs_f64() * 1000.0);
            postprocess_samples.push((t3 - t2).as_secs_f64() * 1000.0);
            end_to_end_samples.push((t3 - t0).as_secs_f64() * 1000.0);
        }

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
        let preprocess_stats = BenchmarkStats::compute(preprocess_samples);
        let inference_stats = BenchmarkStats::compute(inference_samples);
        let postprocess_stats = BenchmarkStats::compute(postprocess_samples);
        let end_to_end_stats = BenchmarkStats::compute(end_to_end_samples);
        let abi_stats = BenchmarkStats::compute(abi_process_samples);

        println!("\n--- Benchmark Report ({loops} iterations, {warmup} warmup) ---");
        println!(
            "  Preprocess:  Avg {:.2} ms | P50 {:.2} ms | P99 {:.2} ms",
            preprocess_stats.avg_ms, preprocess_stats.p50_ms, preprocess_stats.p99_ms
        );
        println!(
            "  Inference:   Avg {:.2} ms | P50 {:.2} ms | P99 {:.2} ms",
            inference_stats.avg_ms, inference_stats.p50_ms, inference_stats.p99_ms
        );
        println!(
            "  Postprocess: Avg {:.2} ms | P50 {:.2} ms | P99 {:.2} ms",
            postprocess_stats.avg_ms, postprocess_stats.p50_ms, postprocess_stats.p99_ms
        );
        println!(
            "  End-to-end:  Avg {:.2} ms | P50 {:.2} ms | P99 {:.2} ms | FPS: {:.1}",
            end_to_end_stats.avg_ms,
            end_to_end_stats.p50_ms,
            end_to_end_stats.p99_ms,
            end_to_end_stats.fps
        );
        println!(
            "  ABI process: Avg {:.2} ms | P50 {:.2} ms | P99 {:.2} ms | FPS: {:.1}",
            abi_stats.avg_ms, abi_stats.p50_ms, abi_stats.p99_ms, abi_stats.fps
        );
    }

    Ok(())
}
