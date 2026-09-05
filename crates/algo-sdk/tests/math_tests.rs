use algo_sdk::cv::{compute_letterbox_layout, PreprocessMode};
use algo_sdk::math::{calculate_iou, clamp_bbox, fast_nms, fast_nms_agnostic, unmap_box, NormBox};

#[test]
fn test_iou_extremes_and_known_values() {
    let b1 = NormBox::new(0.0, 0.0, 1.0, 1.0, 1.0, 0);
    let b2 = NormBox::new(0.0, 0.0, 1.0, 1.0, 1.0, 0);
    assert!((calculate_iou(&b1, &b2) - 1.0).abs() < 1e-6);

    let b_disjoint = NormBox::new(2.0, 2.0, 1.0, 1.0, 1.0, 0);
    assert_eq!(calculate_iou(&b1, &b_disjoint), 0.0);

    // a = [0, 0, 2, 2] (area 4)
    // b = [1, 1, 2, 2] (area 4)
    // inter = [1, 1, 2, 2] (width 1, height 1, area 1)
    // union = 4 + 4 - 1 = 7, IoU = 1/7
    let a = NormBox::new(0.0, 0.0, 2.0, 2.0, 1.0, 0);
    let b = NormBox::new(1.0, 1.0, 2.0, 2.0, 1.0, 0);
    let iou = calculate_iou(&a, &b);
    assert!((iou - (1.0 / 7.0)).abs() < 1e-6);
}

#[test]
fn test_nms_boundaries() {
    // 1. 空输入
    let mut empty_boxes = vec![];
    fast_nms(&mut empty_boxes, 0.5);
    assert!(empty_boxes.is_empty());

    // 2. 单框输入
    let mut single_box = vec![NormBox::new(0.1, 0.1, 0.2, 0.2, 0.9, 0)];
    fast_nms(&mut single_box, 0.5);
    assert_eq!(single_box.len(), 1);

    // 3. 高度重叠 3 框应被抑制为 1 框
    let mut three_overlapping = vec![
        NormBox::new(0.1, 0.1, 0.5, 0.5, 0.90, 0),
        NormBox::new(0.1, 0.1, 0.5, 0.5, 0.85, 0),
        NormBox::new(0.1, 0.1, 0.5, 0.5, 0.70, 0),
    ];
    fast_nms(&mut three_overlapping, 0.5);
    assert_eq!(three_overlapping.len(), 1);
    assert_eq!(three_overlapping[0].confidence, 0.90);

    // 4. 类无关抑制
    let mut multi_class_overlap = vec![
        NormBox::new(0.1, 0.1, 0.5, 0.5, 0.90, 0),
        NormBox::new(0.1, 0.1, 0.5, 0.5, 0.85, 1),
    ];
    fast_nms_agnostic(&mut multi_class_overlap, 0.5);
    assert_eq!(multi_class_overlap.len(), 1);
}

#[test]
fn test_unmap_box_and_clamp() {
    // 1920x1080 -> 640x640: scaled_w=640, scaled_h=360, pad_top=140, pad_left=0
    let layout = compute_letterbox_layout(1920, 1080, 640, 640);
    let mode = PreprocessMode::Letterbox(layout);

    // 模型空间中贴满有效画面的框
    let box_in_model = NormBox::new(0.0, 140.0 / 640.0, 1.0, 360.0 / 640.0, 0.95, 0);
    let unmapped = unmap_box(&box_in_model, &mode, 1920, 1080);
    assert!((unmapped.x - 0.0).abs() < 1e-4);
    assert!((unmapped.y - 0.0).abs() < 1e-4);
    assert!((unmapped.w - 1.0).abs() < 1e-4);
    assert!((unmapped.h - 1.0).abs() < 1e-4);

    // Resize 模式
    let resize_mode = PreprocessMode::Resize;
    let b_resize = NormBox::new(0.2, 0.3, 0.4, 0.5, 0.9, 0);
    let unmapped_resize = unmap_box(&b_resize, &resize_mode, 1920, 1080);
    assert_eq!(unmapped_resize, b_resize);

    // 边界截断测试
    let mut out_of_bounds = NormBox::new(-0.5, -0.5, 2.0, 2.0, 0.9, 0);
    clamp_bbox(&mut out_of_bounds);
    assert_eq!(out_of_bounds.x, 0.0);
    assert_eq!(out_of_bounds.y, 0.0);
    assert_eq!(out_of_bounds.w, 1.0);
    assert_eq!(out_of_bounds.h, 1.0);
}
