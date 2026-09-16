//! 双码流快照管线集成测试
//! 验证主码流 GOP 快进解码、子码流平滑降级保底与 JPEG 高清全景及扩边特写切片生成。

use bytes::Bytes;
use media::decoders::MockDecoder;
use media::ring_buffer::{MainStreamRingBuffer, RingBufferConfig};
use media::StreamClockAnchor;
use pipeline::{EvidenceTarget, PipelineManager, SnapshotCaptureMode, SnapshotConfig};
use std::fs;
use std::sync::Arc;
use types::{
    BoundingBox, CodecType, EncodedPacket, FrameHandle, FrameRef, PixelFormat, StrideInfo,
};

fn make_packet(pts_ms: i64, is_keyframe: bool) -> Arc<EncodedPacket> {
    Arc::new(EncodedPacket {
        pts_ms,
        is_keyframe,
        codec: CodecType::H264,
        payload: Bytes::from_static(b"\x00\x00\x00\x01\x67fake_nalu_payload"),
        ..Default::default()
    })
}

/// 构造一个已标定的接入时延锚点（低分位数需要足量样本才收敛）。
fn calibrated_anchor(lag_ms: i64) -> Arc<StreamClockAnchor> {
    let anchor = Arc::new(StreamClockAnchor::new());
    for _ in 0..64 {
        anchor.observe(lag_ms);
    }
    assert!(anchor.lag_ms().is_some());
    anchor
}

/// 声明「主码流与分析流位于同一条 PTS 轴」：两路接入时延相等，跨流偏移为 0。
///
/// 真实双流部署中两条轴由各自的 `PLAY` 应答决定，必须由接入层实测时延换算；
/// 本 helper 仅用于复现「恰好同轴」的测试场景，不注入锚点时跨流取证会退回检测帧。
async fn install_aligned_stream_clocks(manager: &PipelineManager, cam_id: &str) {
    manager
        .set_stream_clock_anchors(cam_id, calibrated_anchor(120), calibrated_anchor(120))
        .await;
}

#[tokio::test]
async fn test_dual_stream_main_stream_target_decode_flow() {
    let temp_dir = std::env::temp_dir().join(format!(
        "test_dual_stream_int_{}",
        uuid::Uuid::now_v7().simple()
    ));
    let manager = PipelineManager::with_evidence_dir(&temp_dir);
    let cam_id = "cam_test_hq_001";
    install_aligned_stream_clocks(&manager, cam_id).await;

    let ctx = manager.get_or_create_context(cam_id).await;

    // 1. 设置主流快拍解码器 (模拟 1920x1080 解码)
    {
        let mut decoder_guard = ctx.snapshot_decoder.lock().await;
        *decoder_guard = Some(Box::new(MockDecoder::new(
            cam_id,
            CodecType::H264,
            1920,
            1080,
        )));
    }

    // 2. 注入模拟主码流 GOP (I 帧 + 3 个 P 帧)
    manager
        .push_main_packet(cam_id, make_packet(1741100000000, true))
        .await;
    manager
        .push_main_packet(cam_id, make_packet(1741100000040, false))
        .await;
    manager
        .push_main_packet(cam_id, make_packet(1741100000080, false))
        .await;
    manager
        .push_main_packet(cam_id, make_packet(1741100000120, false))
        .await;

    // 3. 注入子码流备用帧 (640x360)
    let fallback_nv12 = vec![128u8; (640 * 360 * 3 / 2) as usize].into();
    let fallback_frame = FrameRef::new(
        cam_id.to_string(),
        1741100000080,
        640,
        360,
        StrideInfo::new(640, 360),
        PixelFormat::Nv12,
        FrameHandle::Host(fallback_nv12),
    );
    manager
        .update_sub_stream_frame(cam_id, fallback_frame)
        .await;

    // 4. 触发报警抓拍 (目标时标 1741100000080，带人脸/目标框)
    let bbox = BoundingBox::new(0.25, 0.25, 0.45, 0.55);
    let snapshot = manager
        .trigger_snapshot(cam_id, 1741100000080, Some(bbox))
        .await
        .expect("主流快进解码抓拍应成功");

    // 5. 验证主流靶向解码成功
    assert!(!snapshot.is_sub_stream(), "主流就绪时不应降级");
    assert_eq!(snapshot.width, 1920, "主流快照应保持 1080P 高清分辨率");
    assert_eq!(snapshot.height, 1080);
    assert!(snapshot.file_size_bytes > 0, "JPEG 全景切片应正常产出");

    let full_img_path = temp_dir.join(&snapshot.image_rel_path);
    let crop_img_path = temp_dir.join(&snapshot.crop_image_rel_path);
    assert!(
        full_img_path.is_file(),
        "全景 JPEG 文件必须存在: {:?}",
        full_img_path
    );
    assert!(
        crop_img_path.is_file(),
        "特写 JPEG 文件必须存在: {:?}",
        crop_img_path
    );

    // 6. 清理临时测试证据目录
    let _ = fs::remove_dir_all(&temp_dir);
}

#[tokio::test]
async fn test_dual_stream_fallback_to_sub_stream() {
    let temp_dir = std::env::temp_dir().join(format!(
        "test_dual_stream_fallback_{}",
        uuid::Uuid::now_v7().simple()
    ));
    let manager = PipelineManager::with_evidence_dir(&temp_dir);
    let cam_id = "cam_test_fallback_002";
    install_aligned_stream_clocks(&manager, cam_id).await;

    // 不设置主流解码器，环形队列为空，仅有子码流帧
    let fallback_nv12 = vec![150u8; (640 * 360 * 3 / 2) as usize].into();
    let fallback_frame = FrameRef::new(
        cam_id.to_string(),
        1741100010000,
        640,
        360,
        StrideInfo::new(640, 360),
        PixelFormat::Nv12,
        FrameHandle::Host(fallback_nv12),
    );
    manager
        .update_sub_stream_frame(cam_id, fallback_frame)
        .await;

    // 触发抓拍
    let target = BoundingBox::new(0.1, 0.1, 0.3, 0.3);
    let snapshot = manager
        .trigger_snapshot(cam_id, 1741100010000, Some(target))
        .await
        .expect("平滑降级抓拍应成功");

    assert!(snapshot.is_sub_stream(), "应标志为降级使用子码流");
    assert_eq!(snapshot.width, 640);
    assert_eq!(snapshot.height, 360);

    let full_img_path = temp_dir.join(&snapshot.image_rel_path);
    assert!(full_img_path.is_file());

    let _ = fs::remove_dir_all(&temp_dir);
}

#[tokio::test]
async fn test_large_gop_fast_mode_within_threshold() {
    let temp_dir = std::env::temp_dir().join(format!(
        "test_large_gop_fast_{}",
        uuid::Uuid::now_v7().simple()
    ));
    let config = SnapshotConfig {
        phase_diff_threshold_ms: 500,
        max_burst_packets: 30,
        max_burst_timeout_ms: 80,
        capture_mode: SnapshotCaptureMode::AdaptiveDualMode,
        ..Default::default()
    };
    let manager = PipelineManager::with_evidence_dir_and_snapshot_config(&temp_dir, config);
    let cam_id = "cam_large_gop_fast";
    install_aligned_stream_clocks(&manager, cam_id).await;

    let ctx = manager.get_or_create_context(cam_id).await;
    {
        let mut decoder_guard = ctx.snapshot_decoder.lock().await;
        *decoder_guard = Some(Box::new(MockDecoder::new(
            cam_id,
            CodecType::H264,
            1920,
            1080,
        )));
    }

    // 注入主码流：I 帧位于 1000ms，P 帧位于 1200ms
    manager
        .push_main_packet(cam_id, make_packet(1000, true))
        .await;
    manager
        .push_main_packet(cam_id, make_packet(1200, false))
        .await;

    // 注入子码流备用帧 (640x360)
    let fallback_nv12 = vec![128u8; (640 * 360 * 3 / 2) as usize].into();
    let fallback_frame = FrameRef::new(
        cam_id.to_string(),
        1200,
        640,
        360,
        StrideInfo::new(640, 360),
        PixelFormat::Nv12,
        FrameHandle::Host(fallback_nv12),
    );
    manager
        .update_sub_stream_frame(cam_id, fallback_frame)
        .await;

    // 相位差在阈值内仍必须前向解码到 1200ms，不能停在 1000ms 的 I 帧。
    let target = BoundingBox::new(0.1, 0.1, 0.3, 0.3);
    let snapshot = manager
        .trigger_snapshot(cam_id, 1200, Some(target))
        .await
        .expect("极速模式单帧解码抓拍应成功");

    // 保持 1080P 高清，并验证目标帧追解成功而非回退至子码流。
    assert!(!snapshot.is_sub_stream());
    assert_eq!(snapshot.width, 1920);
    assert_eq!(snapshot.height, 1080);

    let _ = fs::remove_dir_all(&temp_dir);
}

#[tokio::test]
async fn test_gop_target_decode_does_not_use_keyframe_as_evidence() {
    let ring = MainStreamRingBuffer::new(RingBufferConfig::default());
    ring.push(make_packet(1000, true));
    ring.push(make_packet(1120, false));
    ring.push(make_packet(1200, false));

    let mut decoder = MockDecoder::new("cam_gop_target", CodecType::H264, 1920, 1080);
    let (frame, is_fallback) = pipeline::SnapshotEngine::decode_target_frame_with_config(
        "cam_gop_target",
        EvidenceTarget {
            detection_pts_ms: 1200,
            main_axis_pts_ms: Some(1200),
        },
        Some(&ring),
        None,
        Some(&mut decoder),
        &SnapshotConfig::default(),
    )
    .await
    .expect("GOP 应追解到目标帧");

    assert!(!is_fallback);
    assert_eq!(frame.timestamp, 1200, "证据帧不能停在 GOP 起始 I 帧");
}
#[tokio::test]
async fn test_large_gop_sub_stream_reuse_on_large_gap() {
    let temp_dir = std::env::temp_dir().join(format!(
        "test_large_gop_sub_reuse_{}",
        uuid::Uuid::now_v7().simple()
    ));
    // 配置大偏差直接复用子码流
    let config = SnapshotConfig {
        phase_diff_threshold_ms: 500,
        max_burst_packets: 30,
        max_burst_timeout_ms: 80,
        capture_mode: SnapshotCaptureMode::SubStreamOnLargeGap,
        ..Default::default()
    };
    let manager = PipelineManager::with_evidence_dir_and_snapshot_config(&temp_dir, config);
    let cam_id = "cam_large_gop_sub_reuse";
    install_aligned_stream_clocks(&manager, cam_id).await;

    let ctx = manager.get_or_create_context(cam_id).await;
    {
        let mut decoder_guard = ctx.snapshot_decoder.lock().await;
        *decoder_guard = Some(Box::new(MockDecoder::new(
            cam_id,
            CodecType::H264,
            1920,
            1080,
        )));
    }

    // 模拟大 GOP (I 帧位于 1000ms，越界告警发生在 3000ms，偏差 2000ms >= 500ms)
    manager
        .push_main_packet(cam_id, make_packet(1000, true))
        .await;
    for pts in (1040..=3000).step_by(40) {
        manager
            .push_main_packet(cam_id, make_packet(pts, false))
            .await;
    }

    // 注入子码流在告警时刻 3000ms 真实检测命中的帧 (640x360)
    let fallback_nv12 = vec![128u8; (640 * 360 * 3 / 2) as usize].into();
    let fallback_frame = FrameRef::new(
        cam_id.to_string(),
        3000,
        640,
        360,
        StrideInfo::new(640, 360),
        PixelFormat::Nv12,
        FrameHandle::Host(fallback_nv12),
    );
    manager
        .update_sub_stream_frame(cam_id, fallback_frame)
        .await;

    // 触发抓拍 (告警时标 3000ms)
    let target = BoundingBox::new(0.1, 0.1, 0.3, 0.3);
    let snapshot = manager
        .trigger_snapshot(cam_id, 3000, Some(target))
        .await
        .expect("抓拍应成功");

    // 相位差 2000ms >= 500ms 且配置 SubStreamOnLargeGap，直接复用子码流真实检测帧 (时标绝对精准，零 VPU 压力)
    assert!(snapshot.is_sub_stream());
    assert_eq!(snapshot.width, 640);
    assert_eq!(snapshot.height, 360);

    let _ = fs::remove_dir_all(&temp_dir);
}

#[tokio::test]
async fn test_large_gop_adaptive_burst_decode() {
    let temp_dir = std::env::temp_dir().join(format!(
        "test_large_gop_burst_{}",
        uuid::Uuid::now_v7().simple()
    ));
    // 默认自适应双模配置 (max_burst_packets = 30，测试环境中放宽超时预算防止 debug 模式 CPU 抖动)
    let config = SnapshotConfig {
        phase_diff_threshold_ms: 500,
        max_burst_packets: 30,
        max_burst_timeout_ms: 1000,
        capture_mode: SnapshotCaptureMode::AdaptiveDualMode,
        ..Default::default()
    };
    let manager = PipelineManager::with_evidence_dir_and_snapshot_config(&temp_dir, config);
    let cam_id = "cam_large_gop_burst";
    install_aligned_stream_clocks(&manager, cam_id).await;

    let ctx = manager.get_or_create_context(cam_id).await;
    {
        let mut decoder_guard = ctx.snapshot_decoder.lock().await;
        *decoder_guard = Some(Box::new(MockDecoder::new(
            cam_id,
            CodecType::H264,
            1920,
            1080,
        )));
    }

    // 模拟前向 15 包 (I 帧位于 1000ms，告警位于 1600ms，偏差 600ms >= 500ms，但包数 16 <= 30)
    manager
        .push_main_packet(cam_id, make_packet(1000, true))
        .await;
    for pts in (1040..=1600).step_by(40) {
        manager
            .push_main_packet(cam_id, make_packet(pts, false))
            .await;
    }

    let fallback_nv12 = vec![128u8; (640 * 360 * 3 / 2) as usize].into();
    let fallback_frame = FrameRef::new(
        cam_id.to_string(),
        1600,
        640,
        360,
        StrideInfo::new(640, 360),
        PixelFormat::Nv12,
        FrameHandle::Host(fallback_nv12),
    );
    manager
        .update_sub_stream_frame(cam_id, fallback_frame)
        .await;

    let target = BoundingBox::new(0.1, 0.1, 0.3, 0.3);
    let snapshot = manager
        .trigger_snapshot(cam_id, 1600, Some(target))
        .await
        .expect("自适应追帧解码应成功");

    // 包数未超限，成功执行 Burst Decode 追帧至 1600ms，输出 1080P 高清大图
    assert!(!snapshot.is_sub_stream());
    assert_eq!(snapshot.width, 1920);
    assert_eq!(snapshot.height, 1080);

    let _ = fs::remove_dir_all(&temp_dir);
}

#[tokio::test]
async fn test_large_gop_adaptive_burst_fallback_on_excessive_packets() {
    let temp_dir = std::env::temp_dir().join(format!(
        "test_large_gop_overflow_{}",
        uuid::Uuid::now_v7().simple()
    ));
    // 限制最大追帧包数为 15 包
    let config = SnapshotConfig {
        phase_diff_threshold_ms: 500,
        max_burst_packets: 15,
        max_burst_timeout_ms: 80,
        capture_mode: SnapshotCaptureMode::AdaptiveDualMode,
        ..Default::default()
    };
    let manager = PipelineManager::with_evidence_dir_and_snapshot_config(&temp_dir, config);
    let cam_id = "cam_large_gop_overflow";
    install_aligned_stream_clocks(&manager, cam_id).await;

    let ctx = manager.get_or_create_context(cam_id).await;
    {
        let mut decoder_guard = ctx.snapshot_decoder.lock().await;
        *decoder_guard = Some(Box::new(MockDecoder::new(
            cam_id,
            CodecType::H264,
            1920,
            1080,
        )));
    }

    // 模拟大 GOP：I 帧位于 1000ms，告警位于 2200ms (31 个包，超出 15 包限额)
    manager
        .push_main_packet(cam_id, make_packet(1000, true))
        .await;
    for pts in (1040..=2200).step_by(40) {
        manager
            .push_main_packet(cam_id, make_packet(pts, false))
            .await;
    }

    let fallback_nv12 = vec![128u8; (640 * 360 * 3 / 2) as usize].into();
    let fallback_frame = FrameRef::new(
        cam_id.to_string(),
        2200,
        640,
        360,
        StrideInfo::new(640, 360),
        PixelFormat::Nv12,
        FrameHandle::Host(fallback_nv12),
    );
    manager
        .update_sub_stream_frame(cam_id, fallback_frame)
        .await;

    let target = BoundingBox::new(0.1, 0.1, 0.3, 0.3);
    let snapshot = manager
        .trigger_snapshot(cam_id, 2200, Some(target))
        .await
        .expect("抓拍应平滑回退成功");

    // 包数超限，自动平滑复用子码流真实检测帧，防止 VPU 争抢阻塞
    assert!(snapshot.is_sub_stream());
    assert_eq!(snapshot.width, 640);
    assert_eq!(snapshot.height, 360);

    let _ = fs::remove_dir_all(&temp_dir);
}

#[tokio::test]
async fn test_large_gop_burst_timeout_budget_fuse() {
    let temp_dir = std::env::temp_dir().join(format!(
        "test_large_gop_fuse_{}",
        uuid::Uuid::now_v7().simple()
    ));
    // 配置极其严苛的追帧耗时预算 (0ms 立即超时熔断)
    let config = SnapshotConfig {
        phase_diff_threshold_ms: 500,
        max_burst_packets: 30,
        max_burst_timeout_ms: 0,
        capture_mode: SnapshotCaptureMode::AdaptiveDualMode,
        ..Default::default()
    };
    let manager = PipelineManager::with_evidence_dir_and_snapshot_config(&temp_dir, config);
    let cam_id = "cam_large_gop_fuse";
    install_aligned_stream_clocks(&manager, cam_id).await;

    let ctx = manager.get_or_create_context(cam_id).await;
    {
        let mut decoder_guard = ctx.snapshot_decoder.lock().await;
        *decoder_guard = Some(Box::new(MockDecoder::new(
            cam_id,
            CodecType::H264,
            1920,
            1080,
        )));
    }

    // 模拟大 GOP (偏差 600ms >= 500ms)
    manager
        .push_main_packet(cam_id, make_packet(1000, true))
        .await;
    for pts in (1040..=1600).step_by(40) {
        manager
            .push_main_packet(cam_id, make_packet(pts, false))
            .await;
    }

    let fallback_nv12 = vec![128u8; (640 * 360 * 3 / 2) as usize].into();
    let fallback_frame = FrameRef::new(
        cam_id.to_string(),
        1600,
        640,
        360,
        StrideInfo::new(640, 360),
        PixelFormat::Nv12,
        FrameHandle::Host(fallback_nv12),
    );
    manager
        .update_sub_stream_frame(cam_id, fallback_frame)
        .await;

    let target = BoundingBox::new(0.1, 0.1, 0.3, 0.3);
    let snapshot = manager
        .trigger_snapshot(cam_id, 1600, Some(target))
        .await
        .expect("熔断抓拍应成功");

    // 触发延时熔断，直接优雅回退至已解码备用帧
    assert!(snapshot.is_sub_stream());
    assert_eq!(snapshot.width, 640);
    assert_eq!(snapshot.height, 360);

    let _ = fs::remove_dir_all(&temp_dir);
}

#[tokio::test]
async fn test_scheme3_main_stream_zero_decode_direct_passthrough() {
    let temp_dir = std::env::temp_dir().join(format!(
        "test_scheme3_zero_decode_{}",
        uuid::Uuid::now_v7().simple()
    ));
    let manager = PipelineManager::with_evidence_dir(&temp_dir);
    let cam_id = "cam_scheme3_main_direct";

    // 1. 标记当前摄像头为主流分析模式 (StreamMode::Main)
    manager.set_main_stream_analysis(cam_id, true).await;

    // 2. 模拟分析泵连续硬解主码流 1080P 帧并推入 decoded_ring
    //    故意不配置 ctx.snapshot_decoder，且 ring_buffer 为空，
    //    证明零解码直通完全无需启动独立的快照硬解器！
    // 目标时标取 2026 年附近的真实 UTC 毫秒，验证大数值 PTS 不会溢出
    let target_pts: i64 = 1_789_121_628_156;
    for offset in [-80, -40, 0, 40] {
        let pts = target_pts + offset;
        let frame_nv12 = vec![128u8; (1920 * 1080 * 3 / 2) as usize].into();
        let frame = FrameRef::new(
            cam_id.to_string(),
            pts,
            1920,
            1080,
            StrideInfo::new(1920, 1080),
            PixelFormat::Nv12,
            FrameHandle::Host(frame_nv12),
        );
        manager.update_decoded_frame(cam_id, frame).await;
    }

    // 3. 触发告警抓拍 (目标时标 target_pts)
    let bbox = BoundingBox::new(0.2, 0.2, 0.5, 0.6);
    let snapshot = manager
        .trigger_snapshot(cam_id, target_pts, Some(bbox))
        .await
        .expect("方案三零解码直通抓拍应成功");

    // 4. 验证零解码瞬时直通效果
    assert!(
        !snapshot.is_sub_stream(),
        "主流常驻解码帧直通，不应标记为降级"
    );
    assert_eq!(snapshot.width, 1920, "必须为 1080P 原生高保真大图");
    assert_eq!(snapshot.height, 1080);
    assert!(snapshot.file_size_bytes > 0);

    let full_img_path = temp_dir.join(&snapshot.image_rel_path);
    assert!(
        full_img_path.is_file(),
        "全景 JPEG 必须生成: {:?}",
        full_img_path
    );

    let _ = fs::remove_dir_all(&temp_dir);
}

#[tokio::test]
async fn test_scheme3_tolerance_matching_and_fallback() {
    let temp_dir = std::env::temp_dir().join(format!(
        "test_scheme3_tolerance_{}",
        uuid::Uuid::now_v7().simple()
    ));
    let manager = PipelineManager::with_evidence_dir(&temp_dir);
    let cam_id = "cam_scheme3_tol";

    manager.set_main_stream_analysis(cam_id, true).await;

    // 模拟推入 1080P 帧 (时标 1000ms)
    let frame_nv12 = vec![128u8; (1920 * 1080 * 3 / 2) as usize].into();
    let frame = FrameRef::new(
        cam_id.to_string(),
        1000,
        1920,
        1080,
        StrideInfo::new(1920, 1080),
        PixelFormat::Nv12,
        FrameHandle::Host(frame_nv12),
    );
    manager.update_sub_stream_frame(cam_id, frame).await;

    // 目标时标 1080ms (模拟 NPU 推理排队延时 80ms，仍在证据 PTS 容差内)，应成功容差命中零解码
    let target = BoundingBox::new(0.1, 0.1, 0.3, 0.3);
    let snapshot = manager
        .trigger_snapshot(cam_id, 1080, Some(target))
        .await
        .expect("容差范围内零解码直通应成功");

    assert!(!snapshot.is_sub_stream());
    assert_eq!(snapshot.width, 1920);

    let _ = fs::remove_dir_all(&temp_dir);
}

#[tokio::test]
async fn test_sub_stream_multiple_alarms_reuse_on_demand_frame() {
    let temp_dir = std::env::temp_dir().join(format!(
        "test_sub_stream_multialarm_{}",
        uuid::Uuid::now_v7().simple()
    ));
    let manager = PipelineManager::with_evidence_dir(&temp_dir);
    let cam_id = "cam_multialarm_reuse";
    install_aligned_stream_clocks(&manager, cam_id).await;

    let ctx = manager.get_or_create_context(cam_id).await;
    {
        let mut decoder_guard = ctx.snapshot_decoder.lock().await;
        *decoder_guard = Some(Box::new(MockDecoder::new(
            cam_id,
            CodecType::H264,
            1920,
            1080,
        )));
    }

    // 主流 GOP: 1000(I) .. 1080(P)
    manager
        .push_main_packet(cam_id, make_packet(1000, true))
        .await;
    manager
        .push_main_packet(cam_id, make_packet(1040, false))
        .await;
    manager
        .push_main_packet(cam_id, make_packet(1080, false))
        .await;

    // 子流推理帧 (640x360)
    let analyzed_frame = FrameRef::new(
        cam_id.to_string(),
        1080,
        640,
        360,
        StrideInfo::new(640, 360),
        PixelFormat::Nv12,
        FrameHandle::Host(vec![128u8; 640 * 360 * 3 / 2].into()),
    );

    // 告警 1 触发：从主码流 GOP 前向解码出 1080P 高清帧
    let bbox1 = BoundingBox::new(0.1, 0.1, 0.2, 0.2);
    let snap1 = manager
        .trigger_snapshot_for_frame(cam_id, 1080, Some(bbox1), analyzed_frame.clone())
        .await
        .expect("告警 1 抓拍应成功");
    assert!(!snap1.is_sub_stream());
    assert_eq!(snap1.width, 1920);

    // 验证按需解码产物已缓存
    assert!(ctx.last_on_demand_frame.read().await.is_some());

    // 故意清空主流 RingBuffer，确保告警 2 成功必须来自对同刻已解码高保真帧的直接复用
    ctx.ring_buffer.clear();

    // 告警 2 触发：相同 PTS，不同 BBox
    let bbox2 = BoundingBox::new(0.5, 0.5, 0.7, 0.7);
    let snap2 = manager
        .trigger_snapshot_for_frame(cam_id, 1080, Some(bbox2), analyzed_frame)
        .await
        .expect("告警 2 应复用同刻按需帧抓拍成功");
    assert!(!snap2.is_sub_stream());
    assert_eq!(snap2.width, 1920);
    assert_ne!(snap1.crop_image_rel_path, snap2.crop_image_rel_path);

    let _ = fs::remove_dir_all(&temp_dir);
}

#[tokio::test]
async fn test_quota_exhausted_rejects_expired_fallback_frame() {
    let temp_dir = std::env::temp_dir().join(format!(
        "test_quota_expired_{}",
        uuid::Uuid::now_v7().simple()
    ));
    let manager = PipelineManager::with_evidence_dir(&temp_dir);
    let cam_id = "cam_quota_expired";

    // 占用全部 VPU 配额信号量
    let held_permit = manager
        .snapshot_semaphore()
        .try_acquire()
        .expect("acquire permit");

    // 注入过期子流候选帧 (时标 1000ms)
    let frame = FrameRef::new(
        cam_id.to_string(),
        1000,
        640,
        360,
        StrideInfo::new(640, 360),
        PixelFormat::Nv12,
        FrameHandle::Host(vec![128u8; 640 * 360 * 3 / 2].into()),
    );
    manager.update_sub_stream_frame(cam_id, frame).await;

    // 请求时标 1500ms 的抓拍 (偏差 500ms > 100ms 阈值)
    let target = BoundingBox::new(0.1, 0.1, 0.3, 0.3);
    let result = manager.trigger_snapshot(cam_id, 1500, Some(target)).await;

    assert!(result.is_err(), "配额满载时过期候选帧应被拒绝");

    drop(held_permit);
    let _ = fs::remove_dir_all(&temp_dir);
}

/// 子码流推理取主码流证据时，必须按**换算后的主码流轴**选题。
///
/// 检测时标 1180 位于子码流轴；在接入时延差 -100ms 下同一真实时刻对应主码流轴 1080。
/// 若仍按检测时标 1180 去检索，则会选回 1160 这一帧——即现场看到的「证据帧与推理帧对不上」。
#[tokio::test]
async fn test_cross_stream_evidence_targets_converted_main_axis() {
    let ring = MainStreamRingBuffer::new(RingBufferConfig::default());
    for pts in [1000, 1040, 1080, 1120, 1160] {
        ring.push(make_packet(pts, pts == 1000));
    }

    let mut decoder = MockDecoder::new("cam_cross_axis_pick", CodecType::H264, 1920, 1080);
    let (frame, is_fallback) = pipeline::SnapshotEngine::decode_target_frame_with_config(
        "cam_cross_axis_pick",
        EvidenceTarget {
            detection_pts_ms: 1180,
            main_axis_pts_ms: Some(1080),
        },
        Some(&ring),
        None,
        Some(&mut decoder),
        &SnapshotConfig::default(),
    )
    .await
    .expect("应按主码流轴追解到目标帧");

    assert!(!is_fallback, "主码流就绪时不应降级");
    assert_eq!(
        frame.timestamp, 1080,
        "必须按主码流轴选题；沿用检测时标 1180 会选回 1160"
    );
}

/// 跨流时延未标定时，`main_axis_pts_ms` 为 `None`：不得假定偏移为 0，
/// 必须完全跳过主码流证据环，退回检测流自身的帧。
#[tokio::test]
async fn test_uncalibrated_cross_stream_evidence_refuses_main_stream() {
    let ring = MainStreamRingBuffer::new(RingBufferConfig::default());
    for pts in [1000, 1040, 1080] {
        ring.push(make_packet(pts, pts == 1000));
    }

    let detection_frame = FrameRef::new(
        "cam_uncalibrated".to_string(),
        1080,
        640,
        360,
        StrideInfo::new(640, 360),
        PixelFormat::Nv12,
        FrameHandle::Host(vec![128u8; 640 * 360 * 3 / 2].into()),
    );

    let mut decoder = MockDecoder::new("cam_uncalibrated", CodecType::H264, 1920, 1080);
    let (frame, is_fallback) = pipeline::SnapshotEngine::decode_target_frame_with_config(
        "cam_uncalibrated",
        EvidenceTarget {
            detection_pts_ms: 1080,
            main_axis_pts_ms: None,
        },
        Some(&ring),
        Some(&detection_frame),
        Some(&mut decoder),
        &SnapshotConfig::default(),
    )
    .await
    .expect("未标定时应退回检测流自身的帧");

    assert!(is_fallback, "未标定时不得使用主码流证据");
    assert_eq!(
        frame.width, 640,
        "必须是检测流自身的帧，而非分辨率更高的错位帧"
    );
    assert_eq!(frame.timestamp, 1080);
}
