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

/// 实时推送给前端播放器的目标检测框与航迹 DTO (采用扁平数组降低高频传输开销)
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TrackDto {
    pub track_id: u64,
    pub label: String,
    pub confidence: f32,
    /// 归一化坐标 [x1, y1, x2, y2]
    pub bbox: [f32; 4],
    /// 历史轨迹坐标序列 [[x, y], ...]
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub trajectory: Vec<(f64, f64)>,
}

impl From<&TrackedObject> for TrackDto {
    fn from(obj: &TrackedObject) -> Self {
        Self {
            track_id: obj.track_id,
            label: obj.label.clone(),
            confidence: obj.confidence,
            bbox: [obj.bbox.x1, obj.bbox.y1, obj.bbox.x2, obj.bbox.y2],
            trajectory: obj.trajectory.clone(),
        }
    }
}

/// WebSocket camera.tracks 实时广播载荷
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CameraTracksPayload {
    pub camera_id: String,
    pub timestamp: i64,
    pub tracks: Vec<TrackDto>,
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn test_track_dto_and_payload_serialization() {
        let obj = TrackedObject {
            track_id: 12,
            class_id: 0,
            label: "person".to_string(),
            confidence: 0.89,
            bbox: BoundingBox::new(0.15, 0.22, 0.35, 0.68),
            trajectory: vec![(0.25, 0.65), (0.25, 0.68)],
        };
        let dto = TrackDto::from(&obj);
        assert_eq!(dto.track_id, 12);
        assert_eq!(dto.bbox, [0.15, 0.22, 0.35, 0.68]);

        let payload = CameraTracksPayload {
            camera_id: "CAM-01".to_string(),
            timestamp: 1741100000120,
            tracks: vec![dto],
        };

        let json_val = serde_json::to_value(&payload).expect("Serialization failed");
        assert_eq!(json_val["cameraId"], "CAM-01");
        assert_eq!(json_val["timestamp"], 1741100000120i64);
        assert_eq!(json_val["tracks"][0]["trackId"], 12);
        assert_eq!(json_val["tracks"][0]["label"], "person");
        let conf = json_val["tracks"][0]["confidence"].as_f64().unwrap();
        assert!((conf - 0.89).abs() < 1e-4);
        let x1 = json_val["tracks"][0]["bbox"][0].as_f64().unwrap();
        assert!((x1 - 0.15).abs() < 1e-4);
        let y2 = json_val["tracks"][0]["bbox"][3].as_f64().unwrap();
        assert!((y2 - 0.68).abs() < 1e-4);
        assert_eq!(
            json_val["tracks"][0]["trajectory"]
                .as_array()
                .unwrap()
                .len(),
            2
        );
    }
}
