//! 多目标跟踪与状态估计模块 (track)
//!
//! 提供工业级纯 Rust 多目标航迹跟踪器与状态估计算法。
//!
//! # 核心算法
//!
//! - [`bytetrack::ByteTracker`] — 尺度自适应线性卡尔曼滤波与 KM 全局最优匹配的高性能 ByteTrack。

pub mod bytetrack;

pub use bytetrack::{
    box_iou, ByteTrackConfig, ByteTracker, KalmanBoxTracker, Rect, STrack, TrackDetection,
    TrackStatus,
};
pub use TrackStatus as TrackState;
