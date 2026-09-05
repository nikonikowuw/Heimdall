//! 1:1 映射断言测试：确保 algo-sdk 结构体与 C ABI (64位) 规范尺寸与偏移零漂移。
//! 必须与 crates/infer/tests/c_abi_layout_tests.rs 保持严格一致！

use algo_sdk::c_abi::*;
use std::mem::{offset_of, size_of};

#[test]
fn test_c_abi_structure_sizes() {
    assert_eq!(size_of::<AvFrameDesc>(), 152, "AvFrameDesc size mismatch");
    assert_eq!(size_of::<AvFrameOps>(), 32, "AvFrameOps size mismatch");
    assert_eq!(size_of::<AvRect>(), 24, "AvRect size mismatch");
    assert_eq!(size_of::<AvImageView>(), 96, "AvImageView size mismatch");
    assert_eq!(size_of::<AvImageOps>(), 48, "AvImageOps size mismatch");
    assert_eq!(size_of::<AvPoint>(), 8, "AvPoint size mismatch");
    assert_eq!(size_of::<AvRule>(), 40, "AvRule size mismatch");
    assert_eq!(
        size_of::<AvAlgoLibraryArgs>(),
        48,
        "AvAlgoLibraryArgs size mismatch"
    );
    assert_eq!(
        size_of::<AvAlgoLibraryInfo>(),
        200,
        "AvAlgoLibraryInfo size mismatch"
    );
    assert_eq!(
        size_of::<AvAlgoInstanceArgs>(),
        96,
        "AvAlgoInstanceArgs size mismatch"
    );
    assert_eq!(size_of::<AvFrameCaps>(), 84, "AvFrameCaps size mismatch");
    assert_eq!(size_of::<AvAlgoAbi>(), 96, "AvAlgoAbi size mismatch");
    assert_eq!(
        size_of::<AvAlgoImageReq>(),
        32,
        "AvAlgoImageReq size mismatch"
    );
    assert_eq!(size_of::<AvAlgoResult>(), 48, "AvAlgoResult size mismatch");
    assert_eq!(
        size_of::<AvFaceExtractInput>(),
        40,
        "AvFaceExtractInput size mismatch"
    );
    assert_eq!(
        size_of::<AvFaceExtractOutput>(),
        67892,
        "AvFaceExtractOutput size mismatch"
    );
}

#[test]
fn test_c_abi_field_offsets() {
    // av_frame_desc 关键偏移
    assert_eq!(offset_of!(AvFrameDesc, size), 0);
    assert_eq!(offset_of!(AvFrameDesc, api_version), 4);
    assert_eq!(offset_of!(AvFrameDesc, frame_id), 8);
    assert_eq!(offset_of!(AvFrameDesc, wall_time_ns), 16);
    assert_eq!(offset_of!(AvFrameDesc, pts_ns), 24);
    assert_eq!(offset_of!(AvFrameDesc, modifier), 32);
    assert_eq!(offset_of!(AvFrameDesc, offset), 40);
    assert_eq!(offset_of!(AvFrameDesc, opaque), 72);
    assert_eq!(offset_of!(AvFrameDesc, frame_token), 80);
    assert_eq!(offset_of!(AvFrameDesc, platform_tag), 88);
    assert_eq!(offset_of!(AvFrameDesc, opaque_kind), 92);
    assert_eq!(offset_of!(AvFrameDesc, memory_type), 96);
    assert_eq!(offset_of!(AvFrameDesc, pixel_format), 100);
    assert_eq!(offset_of!(AvFrameDesc, layout), 104);
    assert_eq!(offset_of!(AvFrameDesc, width), 108);
    assert_eq!(offset_of!(AvFrameDesc, height), 112);
    assert_eq!(offset_of!(AvFrameDesc, alloc_width), 116);
    assert_eq!(offset_of!(AvFrameDesc, alloc_height), 120);
    assert_eq!(offset_of!(AvFrameDesc, stride), 124);
    assert_eq!(offset_of!(AvFrameDesc, color_primaries), 140);
    assert_eq!(offset_of!(AvFrameDesc, color_transfer), 142);
    assert_eq!(offset_of!(AvFrameDesc, color_matrix), 144);
    assert_eq!(offset_of!(AvFrameDesc, color_range), 146);
    assert_eq!(offset_of!(AvFrameDesc, plane_count), 147);
    assert_eq!(offset_of!(AvFrameDesc, time_synced), 148);
    assert_eq!(offset_of!(AvFrameDesc, reserved), 149);

    // av_image_view 偏移
    assert_eq!(offset_of!(AvImageView, data), 80);
    assert_eq!(offset_of!(AvImageView, opaque), 88);

    // av_algo_instance_args 偏移
    assert_eq!(offset_of!(AvAlgoInstanceArgs, config_json), 32);
    assert_eq!(offset_of!(AvAlgoInstanceArgs, frame_ops), 48);
    assert_eq!(offset_of!(AvAlgoInstanceArgs, on_result), 64);
    assert_eq!(offset_of!(AvAlgoInstanceArgs, rules), 80);
    assert_eq!(offset_of!(AvAlgoInstanceArgs, rule_count), 88);

    // av_algo_result 偏移
    assert_eq!(offset_of!(AvAlgoResult, json), 24);
    assert_eq!(offset_of!(AvAlgoResult, json_len), 32);
    assert_eq!(offset_of!(AvAlgoResult, image_count), 36);
    assert_eq!(offset_of!(AvAlgoResult, images), 40);
}
