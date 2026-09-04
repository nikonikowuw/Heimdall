use types::{Detection, TrackedObject};

/// 航迹跟踪器抽象
#[derive(Debug, Default)]
pub struct SimpleTracker {
    next_track_id: u64,
}

impl SimpleTracker {
    pub fn new() -> Self {
        Self { next_track_id: 1 }
    }

    /// 更新并关联航迹
    pub fn update(&mut self, detections: Vec<Detection>) -> Vec<TrackedObject> {
        let mut tracked = Vec::with_capacity(detections.len());
        for det in detections {
            let id = self.next_track_id;
            self.next_track_id += 1;
            let center = det.bbox.bottom_center();
            tracked.push(TrackedObject {
                track_id: id,
                class_id: det.class_id,
                label: det.label,
                confidence: det.confidence,
                bbox: det.bbox,
                trajectory: vec![center],
            });
        }
        tracked
    }
}
