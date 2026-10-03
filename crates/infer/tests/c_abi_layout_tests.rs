//! 1:1 映射断言测试：确保 Rust 结构体与 C ABI (64位) 规范尺寸与偏移零漂移。

use infer::c_abi::*;
use std::mem::{align_of, offset_of, size_of};

#[test]
fn test_c_abi_structure_sizes() {
    assert_eq!(size_of::<AvFrameDesc>(), 120, "AvFrameDesc size mismatch");
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
        24,
        "AvFaceExtractInput size mismatch"
    );
    assert_eq!(
        size_of::<AvFaceExtractOutput>(),
        56,
        "AvFaceExtractOutput size mismatch"
    );
    assert_eq!(
        size_of::<AvFaceCandidate>(),
        32,
        "AvFaceCandidate size mismatch"
    );
    assert_eq!(
        size_of::<AvGalleryBulkEntry>(),
        24,
        "AvGalleryBulkEntry size mismatch"
    );
    assert_eq!(
        align_of::<AvGalleryBulkEntry>(),
        8,
        "AvGalleryBulkEntry alignment mismatch"
    );
    assert_eq!(
        size_of::<AvAlgoGalleryAbi>(),
        64,
        "AvAlgoGalleryAbi size mismatch"
    );

    // 放置扩展与回执结构体验证
    assert_eq!(
        size_of::<AvAlgoPlacementCapsPod>(),
        32,
        "AvAlgoPlacementCapsPod size mismatch"
    );
    assert_eq!(
        align_of::<AvAlgoPlacementCapsPod>(),
        4,
        "AvAlgoPlacementCapsPod alignment mismatch"
    );
    assert_eq!(
        size_of::<AvAlgoInstanceReceiptPod>(),
        72,
        "AvAlgoInstanceReceiptPod size mismatch"
    );
    assert_eq!(
        align_of::<AvAlgoInstanceReceiptPod>(),
        8,
        "AvAlgoInstanceReceiptPod alignment mismatch"
    );
    assert_eq!(
        size_of::<AvAlgoCleanupReceiptPod>(),
        72,
        "AvAlgoCleanupReceiptPod size mismatch"
    );
    assert_eq!(
        align_of::<AvAlgoCleanupReceiptPod>(),
        8,
        "AvAlgoCleanupReceiptPod alignment mismatch"
    );
    assert_eq!(
        size_of::<AvAlgoPlacementExtensionV1>(),
        64,
        "AvAlgoPlacementExtensionV1 size mismatch"
    );
    assert_eq!(
        align_of::<AvAlgoPlacementExtensionV1>(),
        std::mem::align_of::<usize>(),
        "AvAlgoPlacementExtensionV1 alignment mismatch"
    );
}

#[test]
fn test_c_abi_field_offsets() {
    // av_frame_desc 关键偏移
    assert_eq!(offset_of!(AvFrameDesc, size), 0);
    assert_eq!(offset_of!(AvFrameDesc, api_version), 4);
    assert_eq!(offset_of!(AvFrameDesc, frame_id), 8);
    assert_eq!(offset_of!(AvFrameDesc, pts_ns), 16);
    assert_eq!(offset_of!(AvFrameDesc, width), 24);
    assert_eq!(offset_of!(AvFrameDesc, height), 28);
    assert_eq!(offset_of!(AvFrameDesc, alloc_width), 32);
    assert_eq!(offset_of!(AvFrameDesc, alloc_height), 36);
    assert_eq!(offset_of!(AvFrameDesc, pixel_format), 40);
    assert_eq!(offset_of!(AvFrameDesc, opaque_kind), 44);
    assert_eq!(offset_of!(AvFrameDesc, color_space), 48);
    assert_eq!(offset_of!(AvFrameDesc, reserved), 52);
    assert_eq!(offset_of!(AvFrameDesc, opaque), 56);
    assert_eq!(offset_of!(AvFrameDesc, frame_token), 64);
    assert_eq!(offset_of!(AvFrameDesc, stride), 72);
    assert_eq!(offset_of!(AvFrameDesc, offset), 88);

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

    // av_face_candidate 偏移
    assert_eq!(offset_of!(AvFaceCandidate, size), 0);
    assert_eq!(offset_of!(AvFaceCandidate, rank), 4);
    assert_eq!(offset_of!(AvFaceCandidate, id), 8);
    assert_eq!(offset_of!(AvFaceCandidate, similarity), 16);
    assert_eq!(offset_of!(AvFaceCandidate, raw_score), 20);
    assert_eq!(offset_of!(AvFaceCandidate, reserved0), 24);

    // av_gallery_bulk_entry 偏移与对齐
    //
    // 算法包侧与宿主侧对同一片内存做指针解引用：`feature_bytes` 之前必须有 8 字节
    // 的对齐间隙，否则在 32 位对齐的算法包中读到的 `id` 会是错位的半个指针。
    assert_eq!(offset_of!(AvGalleryBulkEntry, id), 0);
    assert_eq!(offset_of!(AvGalleryBulkEntry, feature_bytes), 8);
    assert_eq!(offset_of!(AvGalleryBulkEntry, feature_len), 16);
    assert_eq!(offset_of!(AvGalleryBulkEntry, reserved0), 20);

    // av_algo_gallery_abi 偏移
    assert_eq!(offset_of!(AvAlgoGalleryAbi, gallery_create), 8);
    assert_eq!(offset_of!(AvAlgoGalleryAbi, gallery_destroy), 16);
    assert_eq!(offset_of!(AvAlgoGalleryAbi, gallery_clear), 24);
    assert_eq!(offset_of!(AvAlgoGalleryAbi, gallery_insert), 32);
    assert_eq!(offset_of!(AvAlgoGalleryAbi, gallery_remove), 40);
    assert_eq!(offset_of!(AvAlgoGalleryAbi, gallery_search), 48);
    assert_eq!(offset_of!(AvAlgoGalleryAbi, gallery_count), 56);

    // av_algo_placement_caps_pod 偏移
    assert_eq!(offset_of!(AvAlgoPlacementCapsPod, size), 0);
    assert_eq!(offset_of!(AvAlgoPlacementCapsPod, api_version), 4);
    assert_eq!(offset_of!(AvAlgoPlacementCapsPod, caps), 8);
    assert_eq!(offset_of!(AvAlgoPlacementCapsPod, supported_core_mask), 12);
    assert_eq!(
        offset_of!(AvAlgoPlacementCapsPod, max_child_contexts_per_root),
        16
    );

    // av_algo_instance_receipt_pod 偏移
    assert_eq!(offset_of!(AvAlgoInstanceReceiptPod, size), 0);
    assert_eq!(offset_of!(AvAlgoInstanceReceiptPod, api_version), 4);
    assert_eq!(offset_of!(AvAlgoInstanceReceiptPod, status), 8);
    assert_eq!(offset_of!(AvAlgoInstanceReceiptPod, assigned_core_mask), 12);
    assert_eq!(offset_of!(AvAlgoInstanceReceiptPod, actual_core_mask), 16);
    assert_eq!(
        offset_of!(AvAlgoInstanceReceiptPod, weight_sharing_confirmed),
        20
    );
    assert_eq!(offset_of!(AvAlgoInstanceReceiptPod, generation), 24);
    assert_eq!(offset_of!(AvAlgoInstanceReceiptPod, sdk_error_code), 32);
    assert_eq!(offset_of!(AvAlgoInstanceReceiptPod, reserved0), 36);
    assert_eq!(offset_of!(AvAlgoInstanceReceiptPod, reservation_id), 40);

    // av_algo_cleanup_receipt_pod 偏移
    assert_eq!(offset_of!(AvAlgoCleanupReceiptPod, size), 0);
    assert_eq!(offset_of!(AvAlgoCleanupReceiptPod, api_version), 4);
    assert_eq!(offset_of!(AvAlgoCleanupReceiptPod, cleanup_status), 8);
    assert_eq!(offset_of!(AvAlgoCleanupReceiptPod, sdk_error_code), 12);
    assert_eq!(offset_of!(AvAlgoCleanupReceiptPod, generation), 16);
    assert_eq!(offset_of!(AvAlgoCleanupReceiptPod, reservation_id), 24);
    assert_eq!(offset_of!(AvAlgoCleanupReceiptPod, reserved0), 56);
    assert_eq!(offset_of!(AvAlgoCleanupReceiptPod, reserved1), 64);

    // av_algo_placement_extension_v1 偏移
    assert_eq!(offset_of!(AvAlgoPlacementExtensionV1, size), 0);
    assert_eq!(offset_of!(AvAlgoPlacementExtensionV1, api_version), 4);
    assert_eq!(
        offset_of!(AvAlgoPlacementExtensionV1, query_capabilities),
        8
    );
    assert_eq!(
        offset_of!(AvAlgoPlacementExtensionV1, query_instance_receipt),
        16
    );
    assert_eq!(
        offset_of!(AvAlgoPlacementExtensionV1, query_cleanup_receipt),
        24
    );
}
