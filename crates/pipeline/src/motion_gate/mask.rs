//! 空间规则光栅化：Mask 屏蔽与 ROI 正向防区位图
//!
//! 规则以归一化坐标（$[0,1]$）声明，在**评估栅格**上光栅化：硬件门控链路的评估栅格是设备侧
//! 降采样出的定长缩略图（16:9 源为 320×180），Host / CVPixelBuffer 载体则是源帧可见尺寸。
//! 位图索引必须与差分栅格一一对应，否则几何会错位，因此 [`MaskBitmap::slices_for`] 在尺寸不匹配时
//! 直接退化为「无规则」（按无遮蔽 + 全图防区放行语义），而不是拿错位位图去判定。
//!
//! 两条不变式：
//! - **亚像素规则不得消失**：面积不足一个栅格像素的 Mask / 防区，逐像素中心测试会全部落空，
//!   必须保守标记 AABB 中心像素，否则遮罩形同不存在、防区全图失效；
//! - **Precrop 不参与门控**：取景框只决定送模画幅，混入防区会让「只画取景框」静默变成「只算框内」。

use types::{DetectionRule, DetectionRuleRole};

/// 空间掩模与 ROI 栅格化位图缓存
#[derive(Debug, Clone, Default)]
pub struct MaskBitmap {
    pub width: usize,
    pub height: usize,
    /// 1 表示被遮罩屏蔽，0 表示正常计算
    mask: Vec<u8>,
    /// 是否声明了 ROI 正向防区规则
    pub declared_roi: bool,
    /// 是否在 ROI 防区内部，0 表示在 ROI 外部
    roi: Vec<u8>,
}

impl MaskBitmap {
    /// 根据空间规则创建或更新掩模位图
    pub fn new(rules: &[DetectionRule], width: usize, height: usize) -> Self {
        if width == 0 || height == 0 {
            return Self::default();
        }

        let Some(total_pixels) = width.checked_mul(height) else {
            return Self::default();
        };
        let mut mask = vec![0u8; total_pixels];
        let mut roi = vec![0u8; total_pixels];
        let mut declared_roi = false;

        for rule in rules {
            if rule.points.len() < 3 {
                continue;
            }

            // 转换并钳制归一化多边形到绝对像素坐标；非法顶点不参与光栅化。
            let poly_pixels: Vec<(f32, f32)> = rule
                .points
                .iter()
                .filter_map(|pt| {
                    if !pt.x.is_finite() || !pt.y.is_finite() {
                        return None;
                    }
                    Some((
                        pt.x.clamp(0.0, 1.0) as f32 * width as f32,
                        pt.y.clamp(0.0, 1.0) as f32 * height as f32,
                    ))
                })
                .collect();
            if poly_pixels.len() < 3 {
                continue;
            }

            // 计算多边形的外接包围盒 (AABB)，仅在包围盒范围内执行光栅化
            let mut min_x = width as f32;
            let mut max_x = 0.0f32;
            let mut min_y = height as f32;
            let mut max_y = 0.0f32;

            for &(px, py) in &poly_pixels {
                min_x = min_x.min(px);
                max_x = max_x.max(px);
                min_y = min_y.min(py);
                max_y = max_y.max(py);
            }

            let start_x = (min_x.floor() as usize).min(width);
            let end_x = (max_x.ceil() as usize).min(width);
            let start_y = (min_y.floor() as usize).min(height);
            let end_y = (max_y.ceil() as usize).min(height);

            // 取景预裁剪（Precrop）仅决定送模画幅，不参与运动门控：
            // 若把它并入侵入防区的 ROI 掩码，只画取景框的任务会静默变成「只算框内运动」。
            let target = match rule.role {
                DetectionRuleRole::Mask => &mut mask,
                DetectionRuleRole::Roi => {
                    declared_roi = true;
                    &mut roi
                }
                _ => continue,
            };

            let mut marked = false;
            for y in start_y..end_y {
                let row_offset = y * width;
                let py = y as f32 + 0.5;
                for x in start_x..end_x {
                    let px = x as f32 + 0.5;
                    if Self::point_in_polygon(px, py, &poly_pixels) {
                        target[row_offset + x] = 1;
                        marked = true;
                    }
                }
            }

            // 亚像素规则不得整体消失：面积小于一个格子的 Mask/防区，逐像素中心测试会全部落空，
            // 于是「遮罩形同不存在」或「防区全图失效」。此时保守标记 AABB 中心所在像素，
            // 保证已声明的规则始终至少覆盖一个像素。
            if !marked {
                let cx = (((min_x + max_x) * 0.5).floor() as usize).min(width - 1);
                let cy = (((min_y + max_y) * 0.5).floor() as usize).min(height - 1);
                target[cy * width + cx] = 1;
            }
        }

        Self {
            width,
            height,
            mask,
            declared_roi,
            roi,
        }
    }

    /// ROI 防区实际覆盖的像素数（覆盖数低于 `contour_area` 时该防区永远不可能触发运动）
    pub fn roi_pixels(&self) -> usize {
        self.roi.iter().filter(|&&v| v != 0).count()
    }

    /// 射线交叉法判定点是否在多边形内部
    #[inline]
    fn point_in_polygon(x: f32, y: f32, poly: &[(f32, f32)]) -> bool {
        let mut inside = false;
        let n = poly.len();
        if n < 3 {
            return false;
        }
        let mut j = n - 1;
        for i in 0..n {
            let (xi, yi) = poly[i];
            let (xj, yj) = poly[j];
            let intersect = ((yi > y) != (yj > y)) && (x < (xj - xi) * (y - yi) / (yj - yi) + xi);
            if intersect {
                inside = !inside;
            }
            j = i;
        }
        inside
    }

    /// 检查指定像素点是否被 Mask 屏蔽
    #[inline(always)]
    pub fn is_masked(&self, x: usize, y: usize) -> bool {
        if self.mask.is_empty() || x >= self.width || y >= self.height {
            return false;
        }
        self.mask[y * self.width + x] != 0
    }

    /// 检查指定像素点是否在有效 ROI 内部（若未配置 ROI 则恒视为在有效防区内）
    #[inline(always)]
    pub fn is_in_roi(&self, x: usize, y: usize) -> bool {
        if !self.declared_roi {
            return true;
        }
        if self.roi.is_empty() || x >= self.width || y >= self.height {
            return false;
        }
        self.roi[y * self.width + x] != 0
    }

    /// 取与评估栅格匹配的规则切片（供差分热路径线性索引，零拷贝）
    ///
    /// 尺寸不匹配时返回「无规则」：宁可按“无遮蔽 + 全图防区”的放行语义处理，
    /// 也不能用错位位图去判定运动。
    pub(super) fn slices_for(
        &self,
        width: usize,
        height: usize,
    ) -> (Option<&[u8]>, Option<&[u8]>, bool) {
        if self.width != width || self.height != height {
            return (None, None, false);
        }
        (
            (!self.mask.is_empty()).then_some(&self.mask[..]),
            (!self.roi.is_empty()).then_some(&self.roi[..]),
            self.declared_roi,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use types::DetectionPoint;

    /// 亚像素规则（1080p 上宽度不足 1 像素的 Mask / 防区）在门控栅格上不得整体消失：
    /// 遮罩消失会造成误报，防区消失会让该路除保活外永不推理。
    #[test]
    fn test_subpixel_rules_cannot_vanish() {
        let sliver_points = || {
            vec![
                DetectionPoint {
                    x: 0.5000,
                    y: 0.5000,
                },
                DetectionPoint {
                    x: 0.5005,
                    y: 0.5000,
                },
                DetectionPoint {
                    x: 0.5005,
                    y: 0.5005,
                },
            ]
        };

        let roi_bitmap = MaskBitmap::new(
            &[DetectionRule {
                role: DetectionRuleRole::Roi,
                line_direction: types::DetectionLineDirection::Both,
                points: sliver_points(),
            }],
            320,
            180,
        );
        assert!(roi_bitmap.declared_roi);
        assert!(
            roi_bitmap.roi_pixels() >= 1,
            "亚像素防区必须至少覆盖一个栅格像素"
        );
        assert!((0..180).any(|y| (0..320).any(|x| roi_bitmap.is_in_roi(x, y))));

        let mask_bitmap = MaskBitmap::new(
            &[DetectionRule {
                role: DetectionRuleRole::Mask,
                line_direction: types::DetectionLineDirection::Both,
                points: sliver_points(),
            }],
            320,
            180,
        );
        assert!(
            (0..180).any(|y| (0..320).any(|x| mask_bitmap.is_masked(x, y))),
            "亚像素遮罩必须至少覆盖一个栅格像素"
        );
    }

    /// 栅格尺寸不匹配时必须退化为「无规则」，不得用错位位图判定
    #[test]
    fn test_slices_for_requires_matching_grid() {
        let make = |role| DetectionRule {
            role,
            line_direction: types::DetectionLineDirection::Both,
            points: vec![
                DetectionPoint { x: 0.0, y: 0.0 },
                DetectionPoint { x: 0.5, y: 0.0 },
                DetectionPoint { x: 0.5, y: 0.5 },
                DetectionPoint { x: 0.0, y: 0.5 },
            ],
        };

        let mask_bitmap = MaskBitmap::new(&[make(DetectionRuleRole::Mask)], 64, 64);
        let (mask, roi, declared_roi) = mask_bitmap.slices_for(64, 64);
        assert!(mask.is_some(), "同尺寸必须给出可线性索引的 Mask 切片");
        assert!(!declared_roi, "未声明防区规则时不得视为有防区");
        assert!(
            mask_bitmap.is_in_roi(0, 0),
            "未声明防区规则时必须恒视为在防区内（否则整路会被静默屏蔽）"
        );
        assert!(
            roi.is_none() || roi.is_some_and(|r| r.iter().all(|&v| v == 0)),
            "未声明防区规则时防区位图不得标记任何像素"
        );

        let (mask, roi, declared_roi) = mask_bitmap.slices_for(32, 32);
        assert!(
            mask.is_none() && roi.is_none() && !declared_roi,
            "栅格不匹配必须退化为无规则，不得拿错位位图判定"
        );

        let roi_bitmap = MaskBitmap::new(&[make(DetectionRuleRole::Roi)], 64, 64);
        let (mask, roi, declared_roi) = roi_bitmap.slices_for(64, 64);
        assert!(declared_roi, "声明了防区规则必须标记，否则防区会被当成全图");
        assert!(roi.is_some(), "声明防区后必须给出可线性索引的防区切片");
        assert!(
            roi_bitmap.is_in_roi(0, 0) && !roi_bitmap.is_in_roi(63, 63),
            "左上 32x32 防区之外的像素必须被排除"
        );
        assert!(
            mask.is_none_or(|m| m.iter().all(|&v| v == 0)) && !roi_bitmap.is_masked(0, 0),
            "只声明防区时不得屏蔽任何像素（除非另有 Mask 规则）"
        );
    }
}
