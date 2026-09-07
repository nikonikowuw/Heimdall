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

/// 预处理变换模式（供后处理坐标反算使用）
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum PreprocessMode {
    Letterbox(LetterboxLayout),
    Resize,
}
