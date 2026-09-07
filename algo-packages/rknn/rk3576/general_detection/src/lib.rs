//! Rockchip RK3576 RKNN 通用目标检测算法插件
//!
//! 基于 `crates/algo-sdk` 规范构建，提供 C ABI 导出、Rockchip RGA 硬件零拷贝预处理与 NPU 推理接入。

pub mod config;
pub mod plugin;
pub mod postprocess;
pub mod rknn;

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
