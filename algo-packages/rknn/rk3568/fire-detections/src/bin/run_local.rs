//! 本地单机评测工具 (run_local)
//!
//! 在 RK3568 板端快速验证烟火检测效果。
//! 用法: ./fire_smoke_detection_run_local [OPTIONS]
//!   --benchmark       性能剖析 (100 轮迭代)
//!   --input <PATH>    输入图片路径 (默认 testimage.jpg)
//!   --output <PATH>   输出图片路径 (默认 result.jpg)
//!   --model <PATH>    模型文件路径 (默认 model/best_pure.rknn)
//!   --threshold <F>   置信度阈值 (默认 0.25)

#[cfg(target_os = "linux")]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    use std::ffi::c_void;
    use std::path::Path;

    use algo_sdk::c_abi::AvAlgoResult;
    use algo_sdk::emitter::ResultEmitter;
    use algo_sdk::plugin::{AlgoPlugin, InitContext};
    use algo_sdk::testing::{MockEmitter, MockFrameBuilder};
    use fire_smoke_detection::config::InstanceConfig;
    use fire_smoke_detection::plugin::FireSmokeDetector;

    unsafe extern "C" fn on_result_callback(result: *const AvAlgoResult, user_data: *mut c_void) {
        if !result.is_null() && !user_data.is_null() {
            // SAFETY: user_data 指向有效 MockEmitter 实例
            let emitter = unsafe { &mut *(user_data as *mut MockEmitter) };
            // SAFETY: result 为合法指针
            emitter.record_c_result(unsafe { &*result });
        }
    }

    let args: Vec<String> = std::env::args().collect();
    let benchmark = args.iter().any(|a| a == "--benchmark");

    let input_path = get_arg(&args, "--input", "testimage.jpg");
    let output_path = get_arg(&args, "--output", "result.jpg");
    let model_path = get_arg(&args, "--model", "model/best_pure.rknn");
    let threshold: f32 = get_arg(&args, "--threshold", "0.25").parse()?;
    let loops: i32 = if benchmark { 100 } else { 1 };

    println!("[Config] threshold={threshold} input={input_path} output={output_path} model={model_path} loops={loops}");

    if !Path::new(&input_path).exists() {
        eprintln!("输入图片不存在: {input_path}");
        std::process::exit(1);
    }

    // 读取原图
    let dynamic_img = image::open(&input_path)?;
    let width = dynamic_img.width();
    let height = dynamic_img.height();
    let mut rgb_image = dynamic_img.to_rgb8();

    // 模拟 MPP 硬件解码输出帧
    let mock_frame = MockFrameBuilder::from_image_hardware(&input_path)
        .unwrap_or_else(|_| {
            MockFrameBuilder::new()
                .dimensions(width, height)
                .host_data(rgb_image.clone().into_raw())
                .to_nv12(16)
        })
        .build();

    // 初始化算法实例
    let config = InstanceConfig {
        confidence_threshold: threshold,
        ..Default::default()
    };
    let init_ctx = InitContext {
        package_root: Path::new("."),
        platform_id: "linux-rknn",
        instance_id: "standalone_local",
        is_self_test: false,
    };

    let mut detector = FireSmokeDetector::init(&init_ctx, config)?;
    let mode = if detector.session.is_fallback() {
        "debug_cpu_fallback_path"
    } else {
        "infer_fast_path"
    };
    println!("[Pipeline] {mode} | RK3568 NPU");

    // 主循环
    let mut timings = Vec::with_capacity(loops as usize);

    for i in 0..loops {
        let mut mock_emitter = MockEmitter::new();
        // SAFETY: on_result_callback 与 mock_emitter 在本作用域有效存活
        let mut emitter = unsafe {
            ResultEmitter::from_raw(
                (i + 1) as u64,
                Some(on_result_callback),
                &mut mock_emitter as *mut _ as *mut c_void,
            )
        };

        let t0 = std::time::Instant::now();
        let safe_frame = mock_frame.as_safe_frame();
        detector.process(safe_frame, &mut emitter)?;
        timings.push(t0.elapsed().as_secs_f64() * 1000.0);

        if i == 0 {
            let objects = mock_emitter.detections();
            println!("[Detection] 检测到 {} 个目标", objects.len());
            for obj in objects {
                println!(
                    "  {} @ ({:.2}, {:.2}, {:.2}, {:.2}) conf={:.3}",
                    obj.label.unwrap_or("?"),
                    obj.x,
                    obj.y,
                    obj.w,
                    obj.h,
                    obj.confidence
                );
            }

            // 保存可视化结果
            let red = image::Rgb([255u8, 60u8, 60u8]);
            let orange = image::Rgb([255u8, 165u8, 0u8]);
            let w = rgb_image.width() as f32;
            let h = rgb_image.height() as f32;
            for obj in objects {
                let color = if obj.class_id == 0 { red } else { orange };
                let x1 = (obj.x * w).round() as i32;
                let y1 = (obj.y * h).round() as i32;
                let x2 = ((obj.x + obj.w) * w).round() as i32;
                let y2 = ((obj.y + obj.h) * h).round() as i32;
                for t in 0..3 {
                    draw_rect(&mut rgb_image, x1 - t, y1 - t, x2 + t, y2 + t, color);
                }
            }
            rgb_image.save(&output_path)?;
            println!("[Visualizer] 结果已保存到 {output_path}");
        }
    }

    if benchmark && timings.len() > 1 {
        let stats = compute_stats(&mut timings);
        println!("\n--- Benchmark ({loops} iterations, mode={mode}) ---");
        println!(
            "  Avg {:.2} ms | P50 {:.2} ms | P99 {:.2} ms | FPS {:.1}",
            stats.avg, stats.p50, stats.p99, stats.fps
        );
    }

    Ok(())
}

#[cfg(not(target_os = "linux"))]
fn main() {
    eprintln!("fire_smoke_detection_run_local 仅支持在 Linux/Rockchip 平台上运行");
}

fn get_arg(args: &[String], key: &str, default: &str) -> String {
    args.iter()
        .position(|a| a == key)
        .and_then(|i| args.get(i + 1))
        .cloned()
        .unwrap_or_else(|| default.to_string())
}

fn draw_rect(img: &mut image::RgbImage, x1: i32, y1: i32, x2: i32, y2: i32, color: image::Rgb<u8>) {
    let max_x = img.width() as i32 - 1;
    let max_y = img.height() as i32 - 1;
    for px in x1.clamp(0, max_x)..=x2.clamp(0, max_x) {
        let y1c = y1.clamp(0, max_y) as u32;
        let y2c = y2.clamp(0, max_y) as u32;
        img.put_pixel(px as u32, y1c, color);
        img.put_pixel(px as u32, y2c, color);
    }
    for py in y1.clamp(0, max_y)..=y2.clamp(0, max_y) {
        let x1c = x1.clamp(0, max_x) as u32;
        let x2c = x2.clamp(0, max_x) as u32;
        img.put_pixel(x1c, py as u32, color);
        img.put_pixel(x2c, py as u32, color);
    }
}

struct Stats {
    avg: f64,
    p50: f64,
    p99: f64,
    fps: f64,
}

fn compute_stats(samples: &mut [f64]) -> Stats {
    samples.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let sum: f64 = samples.iter().sum();
    let avg = sum / samples.len() as f64;
    let p50 = samples[samples.len() / 2];
    let p99 = samples[(samples.len() * 99 / 100).min(samples.len() - 1)];
    let fps = if avg > 0.0 { 1000.0 / avg } else { 0.0 };
    Stats { avg, p50, p99, fps }
}
