//! 图像预处理类型与像素格式定义

use serde::{Deserialize, Serialize};

use crate::c_abi::*;

/// 像素格式枚举
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum PixelFormat {
    Nv12,
    Bgra,
    Rgb24,
    I420,
    Unknown(u32),
}

impl PixelFormat {
    pub fn from_c_abi(code: u32) -> Self {
        match code {
            AV_PIX_NV12 => Self::Nv12,
            AV_PIX_BGRA => Self::Bgra,
            AV_PIX_RGB24 => Self::Rgb24,
            AV_PIX_I420 => Self::I420,
            other => Self::Unknown(other),
        }
    }

    pub fn to_c_abi(&self) -> u32 {
        match self {
            Self::Nv12 => AV_PIX_NV12,
            Self::Bgra => AV_PIX_BGRA,
            Self::Rgb24 => AV_PIX_RGB24,
            Self::I420 => AV_PIX_I420,
            Self::Unknown(c) => *c,
        }
    }
}

/// Letterbox 变换几何布局
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LetterboxLayout {
    /// 原图缩放比例
    pub scale: f32,
    /// 目标图像左侧黑边填充像素
    pub pad_left: u32,
    /// 目标图像上侧黑边填充像素
    pub pad_top: u32,
    /// 目标宽度
    pub dst_w: u32,
    /// 目标高度
    pub dst_h: u32,
    /// 缩放后未加黑边的实际内容宽度
    pub scaled_w: u32,
    /// 缩放后未加黑边的实际内容高度
    pub scaled_h: u32,
}

/// 低频快照使用的像素 ROI。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CropRect {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

impl CropRect {
    pub fn validate(
        self,
        frame_width: u32,
        frame_height: u32,
    ) -> Result<Self, crate::error::AlgoError> {
        if self.width == 0 || self.height == 0 {
            return Err(crate::error::AlgoError::Preprocess {
                reason: "ROI 尺寸不能为 0".to_string(),
            });
        }
        let right = self
            .x
            .checked_add(self.width)
            .ok_or(crate::error::AlgoError::Preprocess {
                reason: "ROI 横向范围溢出".to_string(),
            })?;
        let bottom =
            self.y
                .checked_add(self.height)
                .ok_or(crate::error::AlgoError::Preprocess {
                    reason: "ROI 纵向范围溢出".to_string(),
                })?;
        if right > frame_width || bottom > frame_height {
            return Err(crate::error::AlgoError::Preprocess {
                reason: format!(
                    "ROI 超出帧边界: roi={}x{}+{},{} frame={}x{}",
                    self.width, self.height, self.x, self.y, frame_width, frame_height
                ),
            });
        }
        Ok(self)
    }
}

/// 预处理变换模式（供后处理坐标反算使用）
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum PreprocessMode {
    Letterbox(LetterboxLayout),
    Resize,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crop_rect_accepts_in_bounds_region() {
        let rect = CropRect {
            x: 4,
            y: 6,
            width: 8,
            height: 10,
        };
        assert_eq!(rect.validate(16, 20), Ok(rect));
    }

    #[test]
    fn crop_rect_rejects_overflow_and_out_of_bounds() {
        assert!(CropRect {
            x: u32::MAX,
            y: 0,
            width: 2,
            height: 2,
        }
        .validate(u32::MAX, 2)
        .is_err());
        assert!(CropRect {
            x: 8,
            y: 0,
            width: 9,
            height: 2,
        }
        .validate(16, 2)
        .is_err());
    }
}
