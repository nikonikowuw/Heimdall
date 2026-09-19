use serde::{Deserialize, Serialize};

/// 后端内存中的 512 维归一化人脸特征。
///
/// 该类型只允许在推理、管线和 API 后台之间转移；所有面向前端的 DTO
/// 都通过 `serde(skip)` 排除它，避免隐私数据进入 WebSocket/HTTP JSON。
pub type FaceEmbedding = Box<[f32; 512]>;

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

    /// 归一化面积（画幅占比，[0, 1]）。
    ///
    /// 坐标已是全画幅归一化值，因此面积可直接当作"离镜头多远"的稳定代理量：
    /// 面积越大，目标外观细节越可读。
    pub fn area(&self) -> f32 {
        ((self.x2 - self.x1).max(0.0) * (self.y2 - self.y1).max(0.0)).clamp(0.0, 1.0)
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

    /// 判断指定点是否在边界框内部
    pub fn contains_point(&self, x: f64, y: f64) -> bool {
        let x = x as f32;
        let y = y as f32;
        x >= self.x1 && x <= self.x2 && y >= self.y1 && y <= self.y2
    }
}

/// 挂载在主体目标上的精细人脸详情
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FaceDetail {
    pub bbox: BoundingBox,
    pub confidence: f32,
    /// 人脸姿态综合质量评分 (0.0..=1.0)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub quality_score: Option<f32>,
    /// 低频人脸识别融合模板参与融合的非冗余帧数；仅算法包 sidecar 帧携带。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fused_count: Option<u32>,
    /// 低频人脸识别融合模板的质量加权均值；仅算法包 sidecar 帧携带。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub template_quality: Option<f32>,
    /// 模板首次成熟握手；普通帧与成熟后的帧均为 `None`。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub template_mature: Option<bool>,
    /// 低频人脸识别 sidecar；绝不序列化到 TrackDto 或其它前端 DTO。
    #[serde(skip)]
    pub embedding: Option<FaceEmbedding>,
}

impl FaceDetail {
    pub fn new(bbox: BoundingBox, confidence: f32) -> Self {
        Self {
            bbox,
            confidence,
            quality_score: None,
            fused_count: None,
            template_quality: None,
            template_mature: None,
            embedding: None,
        }
    }

    pub fn without_embedding(&self) -> Self {
        Self {
            bbox: self.bbox,
            confidence: self.confidence,
            quality_score: self.quality_score,
            fused_count: self.fused_count,
            template_quality: self.template_quality,
            template_mature: self.template_mature,
            embedding: None,
        }
    }
}

/// 单个目标检测结果
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Detection {
    pub class_id: usize,
    pub label: String,
    pub confidence: f32,
    /// 人脸姿态综合质量评分 (0.0..=1.0)，非人脸算法为 None
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub quality_score: Option<f32>,
    pub bbox: BoundingBox,
    /// 挂载的人脸详情 (仅在人员目标挂载人脸时有效)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub face: Option<FaceDetail>,
}

/// ByteTrack 多目标跟踪后的航迹目标
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TrackedObject {
    pub track_id: u64,
    pub class_id: usize,
    pub label: String,
    pub confidence: f32,
    /// 人脸姿态综合质量评分 (0.0..=1.0)，非人脸算法为 None
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub quality_score: Option<f32>,
    pub bbox: BoundingBox,
    /// 挂载的人脸详情 (仅在人员目标挂载人脸时有效)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub face: Option<FaceDetail>,
    /// 低频人脸识别 sidecar；绝不序列化到 TrackDto 或其它前端 DTO。
    #[serde(skip)]
    pub embedding: Option<FaceEmbedding>,
    /// 历史轨迹点集合 (通常保留最近 N 帧底边中心点，用于绊线跨越判定)
    pub trajectory: Vec<(f64, f64)>,
}

impl TrackedObject {
    /// Clone a track for high-frequency/public metadata paths without carrying the
    /// backend-only recognition sidecar.
    pub fn without_embedding(&self) -> Self {
        Self {
            track_id: self.track_id,
            class_id: self.class_id,
            label: self.label.clone(),
            confidence: self.confidence,
            quality_score: self.quality_score,
            bbox: self.bbox,
            face: self.face.as_ref().map(|f| f.without_embedding()),
            embedding: None,
            trajectory: self.trajectory.clone(),
        }
    }

    /// 证据质量分（宿主统一口径）：人脸质量 → 目标级质量 → 归一化人体框面积。
    ///
    /// 抓拍峰值选帧与 `capture_records.quality_score` 落库必须共用此函数，禁止任何一层
    /// 自行回退到 `confidence`：置信度描述"模型有多确信这是目标"，与"这张图能不能看清
    /// 外观"无关，用它当质量分会把窗口首帧当成最佳帧。人脸质量本身随
    /// `bboxJson.face.quality_score` 单独持久化，二者语义不同、互不覆盖。
    pub fn evidence_quality_score(&self) -> f32 {
        self.face
            .as_ref()
            .and_then(|face| face.quality_score)
            .or(self.quality_score)
            .unwrap_or_else(|| self.bbox.area())
            .clamp(0.0, 1.0)
    }

    /// 获取人脸检测框（若挂载人脸）
    pub fn face_bbox(&self) -> Option<BoundingBox> {
        self.face.as_ref().map(|f| f.bbox)
    }

    /// 人脸特写裁剪目标。
    ///
    /// 嵌套 `face` 详情优先；**纯人脸包**（标签即 `face`，不挂载嵌套详情）的检测框本身
    /// 就是人脸框，不能当成"无人脸"——否则识别复核会失去人脸特写这一唯一凭据。
    pub fn face_crop_target(&self) -> Option<BoundingBox> {
        self.face_bbox()
            .or_else(|| self.label.eq_ignore_ascii_case("face").then_some(self.bbox))
    }

    /// 获取人脸特征向量（优先从 face 读取，若无则从根字段读取）
    pub fn embedding(&self) -> Option<&FaceEmbedding> {
        self.face
            .as_ref()
            .and_then(|f| f.embedding.as_ref())
            .or(self.embedding.as_ref())
    }
}

/// 算法分类与执行责任流向
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AlgorithmKind {
    /// 目标检测类 (防范告警流：触犯空间规则/全屏布防产生违规告警)
    #[default]
    Detection,
    /// 目标识别类 (客观通行抓拍流：如人脸、车牌识别，无违规属性，产出通行抓拍凭证)
    Recognition,
}

impl AlgorithmKind {
    /// 从原始算法类型字符串稳健推断算法流向分类（支持 "recognition", "face_recognition" 等变体）
    pub fn parse(raw: &str) -> Self {
        let lower = raw.trim().to_ascii_lowercase();
        if lower.contains("recognition") || lower.contains("recognize") {
            AlgorithmKind::Recognition
        } else {
            AlgorithmKind::Detection
        }
    }

    pub fn is_recognition(&self) -> bool {
        matches!(self, AlgorithmKind::Recognition)
    }

    pub fn is_detection(&self) -> bool {
        matches!(self, AlgorithmKind::Detection)
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            AlgorithmKind::Detection => "detection",
            AlgorithmKind::Recognition => "recognition",
        }
    }
}

impl From<&str> for AlgorithmKind {
    fn from(s: &str) -> Self {
        Self::parse(s)
    }
}

impl From<String> for AlgorithmKind {
    fn from(s: String) -> Self {
        Self::parse(&s)
    }
}

impl From<&String> for AlgorithmKind {
    fn from(s: &String) -> Self {
        Self::parse(s.as_str())
    }
}

impl std::fmt::Display for AlgorithmKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

/// 推送给前端播放器的精细人脸航迹详情
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FaceTrackDto {
    /// 归一化两点坐标 [x1, y1, x2, y2]
    pub bbox: [f32; 4],
    pub confidence: f32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub quality_score: Option<f32>,
}

/// 实时推送给前端播放器的目标检测框与航迹 DTO (采用扁平数组降低高频传输开销)
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TrackDto {
    pub track_id: u64,
    pub label: String,
    pub confidence: f32,
    /// 人脸姿态综合质量评分 (0.0..=1.0)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub quality_score: Option<f32>,
    /// 归一化坐标 [x1, y1, x2, y2]
    pub bbox: [f32; 4],
    /// 挂载的人脸详情 (若人员检出并关联人脸)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub face: Option<FaceTrackDto>,
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
            quality_score: obj
                .quality_score
                .or_else(|| obj.face.as_ref().and_then(|f| f.quality_score)),
            bbox: [obj.bbox.x1, obj.bbox.y1, obj.bbox.x2, obj.bbox.y2],
            face: obj.face.as_ref().map(|f| FaceTrackDto {
                bbox: [f.bbox.x1, f.bbox.y1, f.bbox.x2, f.bbox.y2],
                confidence: f.confidence,
                quality_score: f.quality_score,
            }),
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
    fn public_track_clone_drops_embedding_sidecar() {
        let object = TrackedObject {
            track_id: 7,
            class_id: 0,
            label: "person".to_string(),
            confidence: 0.9,
            quality_score: Some(0.8),
            bbox: BoundingBox::new(0.1, 0.2, 0.3, 0.4),
            face: Some(FaceDetail {
                bbox: BoundingBox::new(0.12, 0.22, 0.28, 0.38),
                confidence: 0.95,
                quality_score: Some(0.8),
                fused_count: Some(3),
                template_quality: Some(0.78),
                template_mature: Some(true),
                embedding: Some(Box::new([0.25; 512])),
            }),
            embedding: Some(Box::new([0.25; 512])),
            trajectory: vec![(0.2, 0.6)],
        };

        let public = object.without_embedding();
        assert!(object.embedding.is_some());
        assert!(object.face.as_ref().unwrap().embedding.is_some());
        assert!(public.embedding.is_none());
        assert!(public.face.as_ref().unwrap().embedding.is_none());
        assert_eq!(public.face.as_ref().unwrap().fused_count, Some(3));
        assert_eq!(public.face.as_ref().unwrap().template_quality, Some(0.78));
        assert_eq!(public.face.as_ref().unwrap().template_mature, Some(true));
        assert_eq!(public.track_id, object.track_id);
        assert_eq!(public.face_bbox(), object.face_bbox());
        assert_eq!(public.trajectory, object.trajectory);
    }

    #[test]
    fn test_track_dto_and_payload_serialization() {
        let obj = TrackedObject {
            track_id: 12,
            class_id: 0,
            label: "person".to_string(),
            confidence: 0.89,
            quality_score: Some(0.92),
            bbox: BoundingBox::new(0.15, 0.22, 0.35, 0.68),
            face: Some(FaceDetail {
                bbox: BoundingBox::new(0.20, 0.22, 0.30, 0.35),
                confidence: 0.96,
                quality_score: Some(0.92),
                fused_count: None,
                template_quality: None,
                template_mature: None,
                embedding: None,
            }),
            embedding: None,
            trajectory: vec![(0.25, 0.65), (0.25, 0.68)],
        };
        let dto = TrackDto::from(&obj);
        assert_eq!(dto.track_id, 12);
        assert_eq!(dto.bbox, [0.15, 0.22, 0.35, 0.68]);
        assert_eq!(
            dto.face,
            Some(FaceTrackDto {
                bbox: [0.20, 0.22, 0.30, 0.35],
                confidence: 0.96,
                quality_score: Some(0.92),
            })
        );
        assert_eq!(dto.quality_score, Some(0.92));
        let serialized_track = serde_json::to_value(&obj).expect("航迹序列化应成功");
        assert!(serialized_track.get("embedding").is_none());

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
        let q = json_val["tracks"][0]["qualityScore"].as_f64().unwrap();
        assert!((q - 0.92).abs() < 1e-4);
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

    #[test]
    fn test_algorithm_kind_parsing_and_conversions() {
        assert_eq!(AlgorithmKind::parse("detection"), AlgorithmKind::Detection);
        assert_eq!(
            AlgorithmKind::parse("general_detection"),
            AlgorithmKind::Detection
        );
        assert_eq!(
            AlgorithmKind::parse("face_recognition"),
            AlgorithmKind::Recognition
        );
        assert_eq!(
            AlgorithmKind::parse("plate_recognize"),
            AlgorithmKind::Recognition
        );
        assert_eq!(
            AlgorithmKind::parse("  RECOGNITION  "),
            AlgorithmKind::Recognition
        );
        assert_eq!(
            AlgorithmKind::parse("unknown_algo"),
            AlgorithmKind::Detection
        );

        assert!(AlgorithmKind::Detection.is_detection());
        assert!(!AlgorithmKind::Detection.is_recognition());
        assert!(AlgorithmKind::Recognition.is_recognition());
        assert!(!AlgorithmKind::Recognition.is_detection());

        assert_eq!(
            AlgorithmKind::from("face_recognition"),
            AlgorithmKind::Recognition
        );
        let s = "recognition".to_string();
        assert_eq!(AlgorithmKind::from(&s), AlgorithmKind::Recognition);
        assert_eq!(AlgorithmKind::from(s), AlgorithmKind::Recognition);
        assert_eq!(AlgorithmKind::default(), AlgorithmKind::Detection);
        assert_eq!(AlgorithmKind::Recognition.to_string(), "recognition");
        assert_eq!(AlgorithmKind::Detection.to_string(), "detection");
    }

    #[test]
    fn evidence_quality_prefers_face_then_target_then_area() {
        let mut obj = TrackedObject {
            track_id: 1,
            class_id: 0,
            label: "person".to_string(),
            confidence: 0.99,
            quality_score: None,
            bbox: BoundingBox::new(0.40, 0.20, 0.60, 0.70),
            face: None,
            embedding: None,
            trajectory: Vec::new(),
        };

        // 无脸目标：回退归一化面积（0.2 × 0.5 = 0.1），而不是 confidence。
        assert!((obj.bbox.area() - 0.1).abs() < 1e-5);
        assert!((obj.evidence_quality_score() - 0.1).abs() < 1e-5);

        // 目标级质量优先于面积。
        obj.quality_score = Some(0.6);
        assert!((obj.evidence_quality_score() - 0.6).abs() < 1e-5);

        // 人脸质量优先于目标级质量（与抓拍峰值选帧口径一致）。
        obj.face = Some(FaceDetail {
            bbox: BoundingBox::new(0.45, 0.20, 0.55, 0.30),
            confidence: 0.9,
            quality_score: Some(0.85),
            fused_count: None,
            template_quality: None,
            template_mature: None,
            embedding: None,
        });
        assert!((obj.evidence_quality_score() - 0.85).abs() < 1e-5);

        // 退化框（倒置/零面积）不得产生 NaN 或负值。
        obj.quality_score = None;
        obj.face = None;
        obj.bbox = BoundingBox::new(0.6, 0.7, 0.4, 0.2);
        assert_eq!(obj.bbox.area(), 0.0);
        assert_eq!(obj.evidence_quality_score(), 0.0);
    }
}
