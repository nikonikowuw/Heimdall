//! RK3568 RKNN 安全帽检测算法插件
//!
//! 基于 `algo-sdk` 规范构建，提供 C ABI 导出、Rockchip RGA 硬件零拷贝预处理与 RK3568 NPU 推理接入。
//! 模型检测 2 个类别：`Hardhat`（安全帽）、`NO-Hardhat`（未戴安全帽）。

pub mod config;
pub mod plugin;
pub mod postprocess;
pub mod rknn;

use algo_sdk::export_algo;
use algo_sdk::plugin::AlgoPlugin;
use plugin::SafetyHelmetDetector;

// 导出标准 C ABI 虚拟方法表与 Panic 隔离入口
export_algo!(
    SafetyHelmetDetector,
    algo_id: "safetyhelmet_detection",
    version: "1.0.0",
    algo_type: "object_detection",
    alarm_type_id: "safety_violation"
);
