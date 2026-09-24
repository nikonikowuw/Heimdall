//! 时域融合标定探针（fusion probe）
//!
//! 与 `run_local` 的唯一本质差别：**按序列喂帧，并为每帧分配单调递增的 `frame_id`**。
//!
//! 为什么必须这样做：包内身份链路的时间轴是 `frame.frame_id()`
//! （`plugin.rs`: `let frame_id = frame.frame_id() as usize`），提取间隔
//! （`MIN_FUSION_FRAME_INTERVAL`）、成熟平台期（`PLATEAU_FRAMES`）、失败退避
//! （`retry_after_frame_id`）全部按帧序号计数。而 `MockFrameBuilder` 的
//! `frame_id` 默认为 1，所以 `run_local --loops N` 反复喂同一帧时
//! `frame_id - last_extract_frame_id >= 6` 永不为真：每轨只提取一次特征，
//! 模板池永远填不满也永不成熟，**融合链路在本地工具里完全测不到**。
//!
//! 输出 `*.jsonl`：每帧一行，`result` 为该帧发射的原始结果 JSON，包含 sidecar 的
//! `embedding`（base64 of 512×f32 LE）、`fused_count`、`template_quality`、
//! `template_mature`。与底库向量的余弦、帧间相似度矩阵等离线分析由外部脚本完成。
//!
//! 用法:
//! ```text
//! # 25 fps 分析（每帧喂入）
//! face_fusion_probe --frames frames25 --fps 25 --stride 1 --out trace_25fps.jsonl
//! # 等效 5 fps 分析（每 5 帧喂 1 帧，帧序号仍单调递增）
//! face_fusion_probe --frames frames25 --fps 25 --stride 5 --out trace_5fps.jsonl
//! ```
//!
//! `--stride` 的目的：身份链路的门控是**帧序号**语义，同等时间窗内的可用帧数随分析
//! 帧率线性变化，5 fps 档位下 6 帧间隔 = 1.2 s。用它复现低帧率下融合成熟是否可达。

use std::env;
use std::ffi::c_void;
use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use algo_sdk::c_abi::AvAlgoResult;
use algo_sdk::emitter::ResultEmitter;
use algo_sdk::plugin::{AlgoPlugin, InitContext};
use algo_sdk::testing::{MockEmitter, MockFrame, MockFrameBuilder};
use face_recognition_rk3588::config::InstanceConfig;
use face_recognition_rk3588::plugin::FaceRecognizer;

/// 单张人脸的标定相关字段（由发射结果 JSON 解析）。
#[derive(Debug, Default, Clone, Copy)]
struct FaceStat {
    confidence: f32,
    quality_score: Option<f32>,
    has_embedding: bool,
    fused_count: Option<u32>,
    template_quality: Option<f32>,
    template_mature: bool,
}

unsafe extern "C" fn on_result_callback(result: *const AvAlgoResult, user_data: *mut c_void) {
    if !result.is_null() && !user_data.is_null() {
        // SAFETY: user_data 指向本轮 process 作用域内有效存活的 MockEmitter。
        let emitter = unsafe { &mut *(user_data as *mut MockEmitter) };
        // SAFETY: result 为 C ABI 回调传入的合法指针，调用期间有效。
        emitter.record_c_result(unsafe { &*result });
    }
}

fn parse_faces(result: &serde_json::Value) -> Vec<FaceStat> {
    let Some(objects) = result["objects"].as_array() else {
        return Vec::new();
    };
    objects
        .iter()
        .filter_map(|object| object.get("face"))
        .map(|face| FaceStat {
            confidence: face["confidence"].as_f64().unwrap_or(0.0) as f32,
            quality_score: face["quality_score"].as_f64().map(|v| v as f32),
            has_embedding: face["embedding"].as_str().is_some(),
            fused_count: face["fused_count"].as_u64().map(|v| v as u32),
            template_quality: face["template_quality"].as_f64().map(|v| v as f32),
            template_mature: face["template_mature"].as_bool().unwrap_or(false),
        })
        .collect()
}

fn arg_value(args: &[String], name: &str) -> Option<String> {
    args.iter()
        .position(|arg| arg == name)
        .and_then(|index| args.get(index + 1))
        .cloned()
}

fn parse_f32_option(args: &[String], name: &str) -> Option<f32> {
    arg_value(args, name)?
        .parse::<f32>()
        .ok()
        .filter(|value| value.is_finite())
}

fn parse_usize_option(args: &[String], name: &str) -> Option<usize> {
    arg_value(args, name)?.parse::<usize>().ok()
}

/// 收集目录下所有图片，按文件名排序（帧序列依赖零填充命名，如 `0001.jpg`）。
fn collect_frames(dir: &Path) -> Result<Vec<PathBuf>, Box<dyn std::error::Error>> {
    let mut frames: Vec<PathBuf> = std::fs::read_dir(dir)
        .map_err(|error| format!("无法读取帧目录 {dir:?}: {error}"))?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| {
            path.is_file()
                && path
                    .extension()
                    .and_then(|ext| ext.to_str())
                    .map(|ext| matches!(ext.to_ascii_lowercase().as_str(), "jpg" | "jpeg" | "png"))
                    .unwrap_or(false)
        })
        .collect();
    frames.sort();
    Ok(frames)
}

/// 构建一帧模拟 MPP 解码输出：优先真实 DMA-BUF，失败回退 16 字节对齐 NV12。
fn build_frame(path: &Path, frame_id: u64, pts_ns: i64, allow_dma: bool) -> (MockFrame, bool) {
    if allow_dma {
        if let Ok(builder) = MockFrameBuilder::from_image_hardware(path) {
            return (
                builder.frame_id(frame_id).timestamp_ns(pts_ns).build(),
                true,
            );
        }
    }
    let image = image::open(path).expect("读取图片失败");
    let builder = MockFrameBuilder::new()
        .dimensions(image.width(), image.height())
        .host_data(image.to_rgb8().into_raw())
        .to_nv12(16)
        .frame_id(frame_id)
        .timestamp_ns(pts_ns);
    (builder.build(), false)
}

fn print_usage() {
    println!(
        "时域融合标定探针\n\n\
         face_fusion_probe --frames <帧目录> [选项]\n\n\
         选项:\n\
         \x20 --frames DIR     帧序列目录（jpg/jpeg/png，按文件名排序）\n\
         \x20 --fps F          源帧率，用于计算 pts（默认 25）\n\
         \x20 --stride N       每 N 帧喂入 1 帧，等效降低分析帧率（默认 1）\n\
         \x20 --limit N        最多喂入 N 帧\n\
         \x20 --out FILE       输出 JSONL 轨迹文件（默认 fusion_trace.jsonl）\n\
         \x20 --params JSON|FILE  复现宿主下发的实例参数（algorithm_instances.params_json）\n\
         \x20 --face-conf F    覆盖 detection_confidence_threshold\n\
         \x20 --person-conf F  覆盖 person_confidence_threshold\n\
         \x20 --no-dma         跳过 DMA-BUF，直接使用 16 字节对齐 NV12\n\
         \x20 --help           显示本帮助\n"
    );
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let env_filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info"));
    tracing_subscriber::fmt().with_env_filter(env_filter).init();

    let args: Vec<String> = env::args().collect();
    if args.iter().any(|arg| arg == "--help" || arg == "-h") {
        print_usage();
        return Ok(());
    }

    let package_root_buf = if Path::new(env!("CARGO_MANIFEST_DIR")).exists() {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
    } else {
        PathBuf::from(".")
    };
    let package_root = package_root_buf.as_path();

    let frames_dir = arg_value(&args, "--frames")
        .map(PathBuf::from)
        .ok_or("缺少必需参数 --frames <帧目录>（--help 查看用法）")?;
    let fps = parse_f32_option(&args, "--fps").unwrap_or(25.0);
    let stride = parse_usize_option(&args, "--stride").unwrap_or(1).max(1);
    let target_fps = parse_f32_option(&args, "--target-fps");
    let realtime = args.iter().any(|arg| arg == "--realtime");
    let preload = args.iter().any(|arg| arg == "--preload");
    let delay_ms = parse_usize_option(&args, "--delay-ms").unwrap_or(0);
    let channel_id = parse_usize_option(&args, "--channel").unwrap_or(0);
    let limit = parse_usize_option(&args, "--limit");
    let output_path = arg_value(&args, "--out")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("fusion_trace.jsonl"));
    let allow_dma = !args.iter().any(|arg| arg == "--no-dma");
    let is_gallery = args.iter().any(|arg| arg == "--gallery");

    let all_frames = collect_frames(&frames_dir)?;
    if all_frames.is_empty() {
        return Err(format!("帧目录 {frames_dir:?} 中未找到任何图片").into());
    }
    let mut selected: Vec<PathBuf> = if let Some(target_fps) = target_fps {
        let interval_ms = 1000.0 / target_fps as f64;
        let mut sampled = Vec::new();
        let mut last_pts_ms: Option<f64> = None;
        for (i, p) in all_frames.iter().enumerate() {
            let pts_ms = (i as f64) * 1000.0 / (fps as f64);
            let take = match last_pts_ms {
                None => true,
                Some(last) => pts_ms >= last + interval_ms - 1.0,
            };
            if take {
                sampled.push(p.clone());
                last_pts_ms = Some(pts_ms);
            }
        }
        sampled
    } else {
        all_frames.iter().step_by(stride).cloned().collect()
    };
    if let Some(limit) = limit {
        selected.truncate(limit);
    }
    let effective_fps = target_fps.unwrap_or(fps / stride as f32);

    // 阈值覆盖走正常 Deserialize 路径，保证命令行优先级高于包内 .env。
    // `--params` 用于精确复现宿主下发的实例参数（`algorithm_instances.params_json`），
    // 按字段局部覆盖；`--face-conf` / `--person-conf` 为高频调参快捷方式，优先级最高。
    let mut overrides = serde_json::Map::new();
    if let Some(raw) = arg_value(&args, "--params") {
        let text = if raw.trim_start().starts_with('{') {
            raw
        } else {
            std::fs::read_to_string(&raw)
                .map_err(|error| format!("无法读取 --params 文件 {raw}: {error}"))?
        };
        let parsed: serde_json::Value = serde_json::from_str(&text)
            .map_err(|error| format!("--params 不是合法 JSON 对象: {error}"))?;
        let object = parsed
            .as_object()
            .ok_or("--params 必须是 JSON 对象（如 algorithm_instances.params_json）")?;
        overrides.extend(object.clone());
    }
    if let Some(value) = parse_f32_option(&args, "--face-conf") {
        overrides.insert("detection_confidence_threshold".to_string(), value.into());
    }
    if let Some(value) = parse_f32_option(&args, "--person-conf") {
        overrides.insert("person_confidence_threshold".to_string(), value.into());
    }
    let config: InstanceConfig = if overrides.is_empty() {
        InstanceConfig::default()
    } else {
        serde_json::from_value(serde_json::Value::Object(overrides))?
    };

    print!("{}", "=".repeat(64));
    println!("\n  RK3568 人脸识别 · 时域融合标定探针");
    println!("  帧目录: {}", frames_dir.display());
    println!(
        "  源帧率 {:.3} fps → 分析帧率 {:.3} fps (抽帧数 {}) [通道 #{} 延迟 {}ms 实时模式: {}]",
        fps,
        effective_fps,
        selected.len(),
        channel_id,
        delay_ms,
        realtime
    );
    println!(
        "  喂入帧数: {} / 目录内 {}",
        selected.len(),
        all_frames.len()
    );
    println!("  轨迹输出: {}", output_path.display());
    println!("  阈值: detection={} person={} min_face_size={} quality_min={} quality(max_yaw={} max_pitch={} max_blur={}) fusion_min={}",
        config.detection_confidence_threshold,
        config.person_confidence_threshold,
        config.min_face_size,
        config.quality_thresholds.min_score,
        config.quality_thresholds.max_yaw,
        config.quality_thresholds.max_pitch,
        config.quality_thresholds.max_blur,
        config.fusion_min_quality_score,
    );
    println!("{}\n", "=".repeat(64));

    let init_ctx = InitContext {
        package_root,
        platform_id: "linux-rknn",
        instance_id: "fusion_probe",
        is_self_test: false,
    };
    let fusion_min_quality = config.fusion_min_quality_score;
    let quality_min_score = config.quality_thresholds.min_score;
    let init_started = Instant::now();
    let mut recognizer = FaceRecognizer::init(&init_ctx, config)?;
    println!(
        "插件与 RKNN 会话初始化耗时: {:.2} ms\n",
        init_started.elapsed().as_secs_f64() * 1000.0
    );

    let output = File::create(&output_path)?;
    let mut writer = BufWriter::new(output);

    let preloaded_frames: Vec<(MockFrame, bool)> = if preload {
        println!(
            "  正在预加载 {} 帧到内存与 DMA-BUF（模拟硬件硬解零拷贝直通）...",
            selected.len()
        );
        let mut vec = Vec::with_capacity(selected.len());
        for (index, path) in selected.iter().enumerate() {
            let frame_id = if is_gallery { 1 } else { (index + 1) as u64 };
            let pts_ns = if is_gallery {
                0
            } else {
                (((index as f64) * 1_000_000_000.0 / effective_fps as f64).round()) as i64
            };
            vec.push(build_frame(path, frame_id, pts_ns, allow_dma));
        }
        vec
    } else {
        Vec::new()
    };

    let mut frames_with_face = 0usize;
    let mut extraction_frames = 0usize;
    let mut max_quality = f32::NAN;
    let mut max_face_conf = f32::NAN;
    let mut mature_at: Option<(usize, u64, u32)> = None;
    let mut fused_histogram: std::collections::BTreeMap<u32, usize> =
        std::collections::BTreeMap::new();
    let mut dma_buf_used: Option<bool> = None;
    let mut latencies: Vec<f64> = Vec::with_capacity(selected.len());
    let total_started = Instant::now();

    for (index, path) in selected.iter().enumerate() {
        if delay_ms > 0 && index == 0 {
            std::thread::sleep(Duration::from_millis(delay_ms as u64));
        }
        if realtime {
            let target_elapsed = Duration::from_secs_f64((index as f64) / (effective_fps as f64));
            let cur = total_started.elapsed();
            if cur < target_elapsed {
                std::thread::sleep(target_elapsed - cur);
            }
        }
        if is_gallery {
            recognizer.reset_tracking();
        }
        let frame_id = if is_gallery { 1 } else { (index + 1) as u64 };
        let pts_ns = if is_gallery {
            0
        } else {
            (((index as f64) * 1_000_000_000.0 / effective_fps as f64).round()) as i64
        };
        let tmp_frame;
        let (frame_ref, used_dma) = if preload {
            (&preloaded_frames[index].0, preloaded_frames[index].1)
        } else {
            tmp_frame = build_frame(path, frame_id, pts_ns, allow_dma);
            (&tmp_frame.0, tmp_frame.1)
        };
        dma_buf_used = Some(dma_buf_used.unwrap_or(used_dma) && used_dma);
        let safe_frame = frame_ref.as_safe_frame();

        let mut emitter = MockEmitter::new();
        let process_started = Instant::now();
        {
            // SAFETY: on_result_callback 与 emitter 在本作用域内有效存活，所有权未转移。
            let mut result_emitter = unsafe {
                ResultEmitter::from_raw(
                    frame_id,
                    Some(on_result_callback),
                    &mut emitter as *mut _ as *mut c_void,
                )
            };
            recognizer.process(safe_frame, &mut result_emitter)?;
        }
        let process_ms = process_started.elapsed().as_secs_f64() * 1000.0;
        latencies.push(process_ms);

        let result_value = emitter
            .raw_json_events()
            .iter()
            .rev()
            .find_map(|json| serde_json::from_str::<serde_json::Value>(json).ok());
        let faces = result_value.as_ref().map(parse_faces).unwrap_or_default();

        if !faces.is_empty() {
            frames_with_face += 1;
        }

        let mut extraction_note = String::new();
        for face in &faces {
            if let Some(quality) = face.quality_score {
                max_quality = if max_quality.is_nan() {
                    quality
                } else {
                    max_quality.max(quality)
                };
            }
            max_face_conf = if max_face_conf.is_nan() {
                face.confidence
            } else {
                max_face_conf.max(face.confidence)
            };
            if face.has_embedding {
                extraction_frames += 1;
                let fused = face.fused_count.unwrap_or(0);
                *fused_histogram.entry(fused).or_default() += 1;
                if face.template_mature && mature_at.is_none() {
                    mature_at = Some((index, frame_id, fused));
                }
            }
            extraction_note.push_str(&format!(
                " [conf={:.3} q={} emb={} fused={} tq={} mature={}]",
                face.confidence,
                face.quality_score
                    .map(|value| format!("{value:.3}"))
                    .unwrap_or_else(|| "-".to_string()),
                if face.has_embedding { "Y" } else { "n" },
                face.fused_count
                    .map(|value| value.to_string())
                    .unwrap_or_else(|| "-".to_string()),
                face.template_quality
                    .map(|value| format!("{value:.3}"))
                    .unwrap_or_else(|| "-".to_string()),
                if face.template_mature { "Y" } else { "-" },
            ));
        }

        println!(
            "[{:>4}] fid={:<4} pts={:>6}ms faces={} {:>6.1}ms{}",
            index,
            frame_id,
            pts_ns / 1_000_000,
            faces.len(),
            process_ms,
            if extraction_note.is_empty() {
                " (无过门人脸)".to_string()
            } else {
                extraction_note
            }
        );

        let record = serde_json::json!({
            "frame_index": index,
            "frame_id": frame_id,
            "pts_ms": pts_ns / 1_000_000,
            "stride": stride,
            "source_frame": path.file_name().and_then(|name| name.to_str()).unwrap_or(""),
            "result": result_value,
        });
        writeln!(writer, "{record}")?;
    }
    writer.flush()?;

    println!("\n{}", "=".repeat(64));
    println!("  标定摘要");
    println!("{}", "=".repeat(64));
    println!(
        "  帧存储: {}",
        match dma_buf_used {
            Some(true) => "全程 DMA-BUF（零拷贝直通）",
            _ => "NV12 主机内存（DMA-BUF 不可用或部分回退）",
        }
    );
    println!(
        "  喂入 {} 帧，其中 {} 帧过门（检出人脸）；最高人脸分 {:.4}，最高质量分 {}",
        selected.len(),
        frames_with_face,
        max_face_conf,
        if max_quality.is_nan() {
            "-".to_string()
        } else {
            format!("{max_quality:.4}")
        }
    );
    println!(
        "  触发特征提取: {} 帧（fused_count 分布 {:?}）",
        extraction_frames, fused_histogram
    );
    match mature_at {
        Some((index, frame_id, fused)) => println!(
            "  模板成熟握手: 第 {} 帧 (fid={})，fused_count={}",
            index, frame_id, fused
        ),
        None => println!("  模板成熟握手: 未触发"),
    }
    if extraction_frames == 0 {
        println!(
            "\n  诊断: 全程未触发特征提取。请对照上行的最高质量分与 fusion_min={} / quality_min={} —— \
             两者取较大值决定准入（只放宽 quality_min 会得到「有框有分无向量」的静默中间态）。",
            fusion_min_quality, quality_min_score
        );
    } else if fused_histogram.keys().all(|fused| *fused <= 1) {
        println!(
            "\n  诊断: 提取帧的 fused_count 全为 1 —— 模板等价于最佳单帧，本序列上融合未产生多帧平均。"
        );
    }

    let total_secs = total_started.elapsed().as_secs_f64();
    let avg_ms = if latencies.is_empty() {
        0.0
    } else {
        latencies.iter().sum::<f64>() / latencies.len() as f64
    };
    let mut sorted_lat = latencies.clone();
    sorted_lat.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let p95_ms = sorted_lat
        .get((sorted_lat.len() as f64 * 0.95) as usize)
        .copied()
        .unwrap_or(0.0);
    let p99_ms = sorted_lat
        .get((sorted_lat.len() as f64 * 0.99) as usize)
        .copied()
        .unwrap_or(0.0);
    let max_ms = sorted_lat.last().copied().unwrap_or(0.0);
    let budget_ms = 1000.0 / effective_fps as f64;
    let overdue_count = latencies.iter().filter(|&&ms| ms > budget_ms).count();

    println!(
        "\n  性能度量 (预算 {:.1}ms/帧): 平均 {:.1}ms | P95 {:.1}ms | P99 {:.1}ms | 最大 {:.1}ms | 超时帧: {} / {}",
        budget_ms, avg_ms, p95_ms, p99_ms, max_ms, overdue_count, latencies.len()
    );
    println!(
        "  总耗时 {:.2}s (综合吞吐: {:.2} FPS{})，轨迹已写入 {}",
        total_secs,
        selected.len() as f64 / total_secs.max(0.001),
        if realtime {
            " [实时节拍锁定]"
        } else {
            " [极速吞吐模式]"
        },
        output_path.display()
    );
    Ok(())
}
