//! Rockchip RK3568 RKNN 烟火检测算法插件
//!
//! 基于 `crates/algo-sdk` 规范构建，集成 YOLOv8 RKNN 推理、
//! 多帧时序确认与颜色方差误报过滤，提供 C ABI 导出。

pub mod config;
pub mod plugin;
pub mod rknn;
pub mod temporal_verifier;

use algo_sdk::export_algo;
use algo_sdk::plugin::AlgoPlugin;
use plugin::FireSmokeDetector;

// 导出标准 C ABI 虚拟方法表与 Panic 隔离入口
export_algo!(
    FireSmokeDetector,
    algo_id: "fire_smoke_detection",
    version: "1.0.0",
    algo_type: "object_detection",
    alarm_type_id: "fire_smoke"
);
