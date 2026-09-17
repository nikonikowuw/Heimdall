//! 人脸视觉几何与图像处理模块 (face)
//!
//! 提供通用人脸仿射对齐、图像预处理与五官几何拓扑/姿态评估。
//!
//! 注：时域特征融合与最佳抓拍管理属于特定算法包和模型的策略状态机，
//! 不在此通用无状态算子层中维护。

pub mod align;
pub mod quality;

pub use align::{
    align_face, align_face_pixels, aligned_source_bounds, apply_affine,
    enhance_face_details_inplace, estimate_affine, estimate_similarity_checked, inverse_coeffs,
    normalize_illumination_inplace, AffineMatrix2D, ALIGNED_SIZE, ARC_FACE_TEMPLATE,
};
pub use quality::{
    compute_quality, estimate_pitch, estimate_yaw, is_landmark_geometry_plausible, FaceQuality,
    QualityConfig,
};
