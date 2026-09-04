use serde::{Deserialize, Serialize};

/// 归一化矩形边界框 [0.0, 1.0]
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct BoundingBox {
    pub x1: f32,
    pub y1: f32,
    pub x2: f32,
    pub y2: f32,
}

impl BoundingBox {
    pub fn new(x1: f32, y1: f32, x2: f32, y2: f32) -> Self {
        Self { x1, y1, x2, y2 }
    }

    /// 获取底部中心点（通常用于地面空间规则侵入判定）
    pub fn bottom_center(&self) -> (f64, f64) {
        let cx = ((self.x1 + self.x2) / 2.0) as f64;
        let cy = self.y2 as f64;
        (cx, cy)
    }

    /// 获取中心点
    pub fn center(&self) -> (f64, f64) {
        let cx = ((self.x1 + self.x2) / 2.0) as f64;
        let cy = ((self.y1 + self.y2) / 2.0) as f64;
        (cx, cy)
    }
}

/// 单个目标检测结果
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Detection {
    pub class_id: usize,
    pub label: String,
    pub confidence: f32,
    pub bbox: BoundingBox,
}

/// ByteTrack 多目标跟踪后的航迹目标
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TrackedObject {
    pub track_id: u64,
    pub class_id: usize,
    pub label: String,
    pub confidence: f32,
    pub bbox: BoundingBox,
    /// 历史轨迹点集合 (通常保留最近 N 帧底边中心点，用于绊线跨越判定)
    pub trajectory: Vec<(f64, f64)>,
}
