//! macOS Apple Silicon CoreML yolo26n 通用目标检测算法插件
//!
//! 基于 `crates/algo-sdk` 规范构建，提供 C ABI 导出、Apple 统一显存零拷贝硬件加速与 ANE 推理。

pub mod config;
pub mod coreml;
pub mod plugin;
pub mod postprocess;

use algo_sdk::export_algo;
use algo_sdk::plugin::AlgoPlugin;
use plugin::GeneralDetector;

// 导出标准 C ABI 虚拟方法表与 Panic 隔离入口
export_algo!(
    GeneralDetector,
    algo_id: "general_detection",
    version: "1.0.0",
    algo_type: "object_detection",
    alarm_type_id: "object_detect"
);
