//! 双码流快照管线集成测试
//! 验证主码流 GOP 快进解码、子码流平滑降级保底与 JPEG 高清全景及扩边特写切片生成。

use bytes::Bytes;
use media::decoders::MockDecoder;
use pipeline::{PipelineManager, SnapshotCaptureMode, SnapshotConfig};
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
    })
}

#[tokio::test]
async fn test_dual_stream_main_stream_target_decode_flow() {
    let temp_dir = std::env::temp_dir().join(format!(
        "test_dual_stream_int_{}",
        uuid::Uuid::new_v4().simple()
    ));
    let manager = PipelineManager::with_evidence_dir(&temp_dir);
    let cam_id = "cam_test_hq_001";

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
    assert!(!snapshot.is_fallback_sub_stream, "主流就绪时不应降级");
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
        uuid::Uuid::new_v4().simple()
    ));
    let manager = PipelineManager::with_evidence_dir(&temp_dir);
    let cam_id = "cam_test_fallback_002";

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

    assert!(snapshot.is_fallback_sub_stream, "应标志为降级使用子码流");
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
        uuid::Uuid::new_v4().simple()
    ));
    let config = SnapshotConfig {
        phase_diff_threshold_ms: 500,
        max_burst_packets: 30,
        max_burst_timeout_ms: 80,
        capture_mode: SnapshotCaptureMode::AdaptiveDualMode,
    };
    let manager = PipelineManager::with_evidence_dir_and_snapshot_config(&temp_dir, config);
    let cam_id = "cam_large_gop_fast";

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

    // 告警时标位于 1200ms，与 I 帧相位差 200ms (< 500ms 阈值)
    let target = BoundingBox::new(0.1, 0.1, 0.3, 0.3);
    let snapshot = manager
        .trigger_snapshot(cam_id, 1200, Some(target))
        .await
        .expect("极速模式单帧解码抓拍应成功");

    // 命中极速模式：单帧硬解 I 帧，不降级子码流，保持 1080P 高清
    assert!(!snapshot.is_fallback_sub_stream);
    assert_eq!(snapshot.width, 1920);
    assert_eq!(snapshot.height, 1080);

    let _ = fs::remove_dir_all(&temp_dir);
}

#[tokio::test]
async fn test_large_gop_sub_stream_reuse_on_large_gap() {
    let temp_dir = std::env::temp_dir().join(format!(
        "test_large_gop_sub_reuse_{}",
        uuid::Uuid::new_v4().simple()
    ));
    // 配置大偏差直接复用子码流
    let config = SnapshotConfig {
        phase_diff_threshold_ms: 500,
        max_burst_packets: 30,
        max_burst_timeout_ms: 80,
        capture_mode: SnapshotCaptureMode::SubStreamOnLargeGap,
    };
    let manager = PipelineManager::with_evidence_dir_and_snapshot_config(&temp_dir, config);
    let cam_id = "cam_large_gop_sub_reuse";

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
    assert!(snapshot.is_fallback_sub_stream);
    assert_eq!(snapshot.width, 640);
    assert_eq!(snapshot.height, 360);

    let _ = fs::remove_dir_all(&temp_dir);
}

#[tokio::test]
async fn test_large_gop_adaptive_burst_decode() {
    let temp_dir = std::env::temp_dir().join(format!(
        "test_large_gop_burst_{}",
        uuid::Uuid::new_v4().simple()
    ));
    // 默认自适应双模配置 (max_burst_packets = 30，测试环境中放宽超时预算防止 debug 模式 CPU 抖动)
    let config = SnapshotConfig {
        phase_diff_threshold_ms: 500,
        max_burst_packets: 30,
        max_burst_timeout_ms: 1000,
        capture_mode: SnapshotCaptureMode::AdaptiveDualMode,
    };
    let manager = PipelineManager::with_evidence_dir_and_snapshot_config(&temp_dir, config);
    let cam_id = "cam_large_gop_burst";

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
    assert!(!snapshot.is_fallback_sub_stream);
    assert_eq!(snapshot.width, 1920);
    assert_eq!(snapshot.height, 1080);

    let _ = fs::remove_dir_all(&temp_dir);
}

#[tokio::test]
async fn test_large_gop_adaptive_burst_fallback_on_excessive_packets() {
    let temp_dir = std::env::temp_dir().join(format!(
        "test_large_gop_overflow_{}",
        uuid::Uuid::new_v4().simple()
    ));
    // 限制最大追帧包数为 15 包
    let config = SnapshotConfig {
        phase_diff_threshold_ms: 500,
        max_burst_packets: 15,
        max_burst_timeout_ms: 80,
        capture_mode: SnapshotCaptureMode::AdaptiveDualMode,
    };
    let manager = PipelineManager::with_evidence_dir_and_snapshot_config(&temp_dir, config);
    let cam_id = "cam_large_gop_overflow";

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
    assert!(snapshot.is_fallback_sub_stream);
    assert_eq!(snapshot.width, 640);
    assert_eq!(snapshot.height, 360);

    let _ = fs::remove_dir_all(&temp_dir);
}

#[tokio::test]
async fn test_large_gop_burst_timeout_budget_fuse() {
    let temp_dir = std::env::temp_dir().join(format!(
        "test_large_gop_fuse_{}",
        uuid::Uuid::new_v4().simple()
    ));
    // 配置极其严苛的追帧耗时预算 (0ms 立即超时熔断)
    let config = SnapshotConfig {
        phase_diff_threshold_ms: 500,
        max_burst_packets: 30,
        max_burst_timeout_ms: 0,
        capture_mode: SnapshotCaptureMode::AdaptiveDualMode,
    };
    let manager = PipelineManager::with_evidence_dir_and_snapshot_config(&temp_dir, config);
    let cam_id = "cam_large_gop_fuse";

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

    // 触发延时熔断，直接优雅回退至子码流当前帧
    assert!(snapshot.is_fallback_sub_stream);
    assert_eq!(snapshot.width, 640);
    assert_eq!(snapshot.height, 360);

    let _ = fs::remove_dir_all(&temp_dir);
}
