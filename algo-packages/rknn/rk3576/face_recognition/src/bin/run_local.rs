//! 本地单机评测工具 (run_local)
//!
//! 模拟真实 MPP 硬件解码输出帧（16 字节对齐 NV12 / DMA-BUF），
//! 通过 RGA 硬件 2D 引擎完成色彩空间转换与 Letterbox，
//! 并直通 RKNN NPU 进行全硬件管线前向推理。
//!
//! 用法: `cargo run -p face-recognition-rknn --bin face_recognition_rknn_run_local -- [image_path] [--loops N]`

use std::env;
use std::ffi::c_void;
use std::path::{Path, PathBuf};
use std::time::Instant;

use algo_sdk::c_abi::AvAlgoResult;
use algo_sdk::emitter::ResultEmitter;
use algo_sdk::plugin::{AlgoPlugin, InitContext};
use algo_sdk::testing::{MockEmitter, MockFrameBuilder};
use face_recognition::config::InstanceConfig;
use face_recognition::plugin::FaceRecognizer;

unsafe extern "C" fn on_result_callback(result: *const AvAlgoResult, user_data: *mut c_void) {
    if !result.is_null() && !user_data.is_null() {
        // SAFETY: user_data 指向有效 MockEmitter 实例
        let emitter = unsafe { &mut *(user_data as *mut MockEmitter) };
        // SAFETY: result 为合法指针
        emitter.record_c_result(unsafe { &*result });
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let env_filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info"));
    tracing_subscriber::fmt().with_env_filter(env_filter).init();

    let args: Vec<String> = env::args().collect();
    let package_root_buf = if Path::new(env!("CARGO_MANIFEST_DIR")).exists() {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
    } else {
        PathBuf::from(".")
    };
    let package_root = package_root_buf.as_path();
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
    println!("  RK3576 人脸识别算法包全硬件流转本地测试");
    println!("  模型: YOLOv8n-face (检测) + EdgeFace-xs (嵌入)");
    println!("  图片: {}", image_path.display());
    println!("================================================================");

    // 1. 初始化算法实例 (插件接口)
    println!("\n[1/4] 初始化 FaceRecognizer 插件实例...");
    let t0 = Instant::now();
    let init_ctx = InitContext {
        package_root,
        platform_id: "linux-rknn",
        instance_id: "local_hardware_test",
        is_self_test: false,
    };
    let mut recognizer = FaceRecognizer::init(&init_ctx, InstanceConfig::default())?;
    println!(
        "  插件实例与 RKNN 会话初始化耗时: {:.2} ms",
        t0.elapsed().as_secs_f64() * 1000.0
    );

    // 2. 模拟真实 MPP 硬件解码输出帧：
    // MPP 解码器输出为 16 字节行跨距对齐的 NV12 (YUV420SP) 格式，通过 DMA-BUF 零拷贝直通传递。
    println!("\n[2/4] 加载图片并构建 MPP 硬件解码帧格式 (NV12 + DMA-BUF)...");
    let mock_frame = MockFrameBuilder::from_image_hardware(&image_path).unwrap_or_else(|error| {
        tracing::warn!("无法从 dma_heap 直接分配硬件 DMA-BUF ({error})，回退到步长对齐 NV12");
        let img = image::open(&image_path).expect("读取图片失败");
        MockFrameBuilder::new()
            .dimensions(img.width(), img.height())
            .host_data(img.to_rgb8().into_raw())
            .to_nv12(16)
    });
    let mock_frame = mock_frame.build();
    let safe_frame = mock_frame.as_safe_frame();

    let is_dma_buf = matches!(
        safe_frame.handle_view(),
        algo_sdk::frame::FrameHandleView::DmaBuf { .. }
    );
    let dma_status = if is_dma_buf {
        "已分配物理 DMA-BUF fd (零拷贝直通模式)"
    } else {
        "Host NV12 内存布局"
    };
    println!(
        "  帧格式: {:?}, 尺寸: {}×{}, 步长: [{}, {}], 存储: {}",
        safe_frame.pixel_format(),
        safe_frame.width(),
        safe_frame.height(),
        safe_frame.stride(0),
        safe_frame.stride(1),
        dma_status
    );

    // 3. 执行单帧全硬件前向检测推理 (MPP NV12 -> RGA Letterbox -> RKNN NPU)
    println!("\n[3/4] 执行全硬件前向流程 (MPP NV12 -> RGA -> RKNN NPU)...");
    let mut mock_emitter = MockEmitter::new();
    let t1 = Instant::now();
    {
        // SAFETY: on_result_callback 与 mock_emitter 在本作用域有效存活，未转移所有权。
        let mut emitter = unsafe {
            ResultEmitter::from_raw(
                1,
                Some(on_result_callback),
                &mut mock_emitter as *mut _ as *mut c_void,
            )
        };
        recognizer.process(safe_frame, &mut emitter)?;
    }
    let process_ms = t1.elapsed().as_secs_f64() * 1000.0;
    println!("  全流程前向耗时: {:.2} ms", process_ms);
    println!(
        "  发射事件数量: {}，原始 JSON:\n{}",
        mock_emitter.raw_json_events().len(),
        mock_emitter
            .raw_json_events()
            .first()
            .map(|s| s.as_str())
            .unwrap_or("(无结果)")
    );

    // 4. 特征嵌入测试 (EdgeFace-xs)
    println!("\n[4/4] 特征嵌入提取测试 (EdgeFace-xs)...");
    let dynamic_img = image::open(&image_path)?.to_rgb8();
    let (orig_w, orig_h) = (dynamic_img.width(), dynamic_img.height());
    let models = face_recognition::shared_models(package_root)?;

    let faces = if is_dma_buf {
        let (buf, mode) = algo_sdk::cv::letterbox(
            &safe_frame,
            models.detector_width,
            models.detector_height,
            [114, 114, 114],
        )?;
        let algo_sdk::cv::PreprocessMode::Letterbox(layout) = mode else {
            return Err("预处理模式非 Letterbox".into());
        };
        models.worker.detect_dma_buf(buf, layout, 0.25)?
    } else {
        let (detector_rgb, layout) = face_recognition::prepare_detector_input_for(
            &dynamic_img,
            models.detector_width,
            models.detector_height,
        )?;
        models.worker.detect_host(detector_rgb, layout, 0.25)?
    };

    if let Some(best) = faces
        .iter()
        .max_by(|left, right| left.score.total_cmp(&right.score))
    {
        let aligned = face_recognition::align::align_face(
            dynamic_img.as_raw(),
            orig_w,
            orig_h,
            &best.landmarks,
        )?;
        let t2 = Instant::now();
        let embed_result = models.worker.embed_host(aligned)?;
        let embed_ms = t2.elapsed().as_secs_f64() * 1000.0;
        println!("  嵌入推理耗时: {:.2} ms", embed_ms);
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
    } else {
        println!("  未检出有效人脸，跳过特征提取");
    }

    // 5. 性能压测模式
    if loops > 1 {
        println!("\n================================================================");
        println!("  硬件流转性能测试: {} 轮连续推理", loops);
        println!("================================================================");

        // Warmup
        for _ in 0..3 {
            // SAFETY: on_result_callback 与 mock_emitter 在本作用域有效存活。
            let mut emitter = unsafe {
                ResultEmitter::from_raw(
                    0,
                    Some(on_result_callback),
                    &mut mock_emitter as *mut _ as *mut c_void,
                )
            };
            recognizer.process(safe_frame, &mut emitter)?;
        }

        let mut times = Vec::with_capacity(loops);
        for i in 0..loops {
            let t_start = Instant::now();
            // SAFETY: on_result_callback 与 mock_emitter 在本作用域有效存活。
            let mut emitter = unsafe {
                ResultEmitter::from_raw(
                    (i + 1) as u64,
                    Some(on_result_callback),
                    &mut mock_emitter as *mut _ as *mut c_void,
                )
            };
            recognizer.process(safe_frame, &mut emitter)?;
            times.push(t_start.elapsed().as_secs_f64() * 1000.0);
        }

        times.sort_by(|left, right| left.total_cmp(right));
        let avg_ms = times.iter().sum::<f64>() / times.len() as f64;
        let p50_ms = times[times.len() / 2];
        let p99_ms = times[(times.len() * 99 / 100).min(times.len() - 1)];
        let min_ms = times.first().copied().unwrap_or(0.0);
        let max_ms = times.last().copied().unwrap_or(0.0);
        let fps = if avg_ms > 0.0 { 1000.0 / avg_ms } else { 0.0 };

        println!(
            "  全流程 (RGA Letterbox + NPU 推理 + 后处理):\n    avg={:.2}ms, p50={:.2}ms, p99={:.2}ms, min={:.2}ms, max={:.2}ms\n    吞吐量: {:.1} FPS",
            avg_ms, p50_ms, p99_ms, min_ms, max_ms, fps
        );
    }

    println!("\n================================================================");
    println!("  测试完成");
    println!("================================================================");

    Ok(())
}
