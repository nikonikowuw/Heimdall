//! 双码流快照管线集成测试
//! 验证主码流 GOP 快进解码、子码流平滑降级保底与 JPEG 高清全景及扩边特写切片生成。

use bytes::Bytes;
use media::decoders::MockDecoder;
use pipeline::PipelineManager;
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
    let snapshot = manager
        .trigger_snapshot(cam_id, 1741100010000, None)
        .await
        .expect("平滑降级抓拍应成功");

    assert!(snapshot.is_fallback_sub_stream, "应标志为降级使用子码流");
    assert_eq!(snapshot.width, 640);
    assert_eq!(snapshot.height, 360);

    let full_img_path = temp_dir.join(&snapshot.image_rel_path);
    assert!(full_img_path.is_file());

    let _ = fs::remove_dir_all(&temp_dir);
}
