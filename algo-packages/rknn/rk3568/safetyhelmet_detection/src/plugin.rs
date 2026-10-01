//! 安全帽检测算法插件实现 (RK3568 RKNN SafetyHelmetDetector)
//!
//! 基于 `algo-sdk` Composable YOLO 检测器架构构建。

use algo_sdk::prelude::*;

/// 安全帽检测模型规格
#[derive(Debug)]
pub struct SafetyHelmetSpec;

impl YoloSpec for SafetyHelmetSpec {
    const INPUT_DIM: (u32, u32) = (640, 384);
    const NUM_CLASSES: usize = 2;
    const LABELS: &'static [&'static str] = &["Hardhat", "NO-Hardhat"];
    const MODEL_PATH: &'static str = "model/yolov8_hard_hat.rknn";
    /// 官方 6-tensor 分层输出（box_8/16/32 + score_8/16/32），无 score_sum 分支
    const USE_SCORE_SUM: bool = false;
    /// 导出 ONNX 时 sigmoid 已移出计算图，score 分支为 logits
    const CLS_IS_LOGITS: bool = true;
    const DFL_BINS: usize = 16;
}

/// 基于通用 YOLO 骨架的安全帽检测器插件
pub type SafetyHelmetDetector = GenericYoloDetector<SafetyHelmetSpec>;
