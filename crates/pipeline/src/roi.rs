//! 局部特写预裁剪 (Pre-crop ROI) 与线性仿射坐标还原
//!
//! 将算法包推理返回的局部归一化坐标 [0, 1] 无损映射回全景大图坐标系，
//! 实现推理算法包与全景/局部业务逻辑的彻底解耦。

use types::{BoundingBox, DetectionPoint};

/// 局部 ROI 到全景坐标系的线性仿射映射器
#[derive(Debug, Clone, Copy)]
pub struct RoiAffineMapper {
    roi: Option<BoundingBox>,
}

impl RoiAffineMapper {
    pub fn new(roi: Option<BoundingBox>) -> Self {
        Self { roi }
    }

    /// 全景直通映射器（无局部裁剪）
    pub fn identity() -> Self {
        Self { roi: None }
    }

    /// 将局部归一化边界框还原至全景归一化坐标
    pub fn map_bbox(&self, local_bbox: &BoundingBox) -> BoundingBox {
        match self.roi {
            None => *local_bbox,
            Some(roi) => {
                let roi_w = roi.x2 - roi.x1;
                let roi_h = roi.y2 - roi.y1;

                let x1 = (roi.x1 + local_bbox.x1 * roi_w).clamp(0.0, 1.0);
                let y1 = (roi.y1 + local_bbox.y1 * roi_h).clamp(0.0, 1.0);
                let x2 = (roi.x1 + local_bbox.x2 * roi_w).clamp(0.0, 1.0);
                let y2 = (roi.y1 + local_bbox.y2 * roi_h).clamp(0.0, 1.0);

                BoundingBox::new(x1, y1, x2, y2)
            }
        }
    }

    /// 将局部归一化坐标点还原至全景归一化坐标
    pub fn map_point(&self, local_pt: DetectionPoint) -> DetectionPoint {
        match self.roi {
            None => local_pt,
            Some(roi) => {
                let roi_w = (roi.x2 - roi.x1) as f64;
                let roi_h = (roi.y2 - roi.y1) as f64;

                let x = (roi.x1 as f64 + local_pt.x * roi_w).clamp(0.0, 1.0);
                let y = (roi.y1 as f64 + local_pt.y * roi_h).clamp(0.0, 1.0);

                DetectionPoint::new(x, y)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_roi_affine_identity() {
        let mapper = RoiAffineMapper::identity();
        let bbox = BoundingBox::new(0.1, 0.2, 0.3, 0.4);
        let mapped = mapper.map_bbox(&bbox);
        assert_eq!(mapped.x1, 0.1);
        assert_eq!(mapped.y1, 0.2);
        assert_eq!(mapped.x2, 0.3);
        assert_eq!(mapped.y2, 0.4);
    }

    #[test]
    fn test_roi_affine_sub_region_scaling() {
        // 假设摄像机局部 ROI 设在右下角 [0.5, 0.5, 1.0, 1.0]
        let roi = BoundingBox::new(0.5, 0.5, 1.0, 1.0);
        let mapper = RoiAffineMapper::new(Some(roi));

        // 局部算法在其中心检测出目标 [0.25, 0.25, 0.75, 0.75]
        let local_bbox = BoundingBox::new(0.25, 0.25, 0.75, 0.75);
        let global_bbox = mapper.map_bbox(&local_bbox);

        // 全景坐标应为 0.5 + 0.25 * 0.5 = 0.625, 0.5 + 0.75 * 0.5 = 0.875
        assert!((global_bbox.x1 - 0.625).abs() < 1e-5);
        assert!((global_bbox.y1 - 0.625).abs() < 1e-5);
        assert!((global_bbox.x2 - 0.875).abs() < 1e-5);
        assert!((global_bbox.y2 - 0.875).abs() < 1e-5);
    }
}
