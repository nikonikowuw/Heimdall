//! 模型推理输出后处理通用工具库
//!
//! 提供与具体模型无关的量化、解码原语，以及特定模型架构的组合解析器。
//!
//! # 模块组织
//!
//! - [`quantize`] — INT8 量化 / 反量化（任何量化模型通用）
//! - [`dfl`] — Distribution Focal Loss 解码（YOLOv8 系列通用）
//! - [`yolov8_rknn`] — YOLOv8 RKNN 多分支 INT8 解析器（组合上述原语）
//!
//! 未来新增模型（YOLOv11、RT-DETR 等）的后处理解析器也放在本模块下。

pub mod dfl;
pub mod quantize;
pub mod yolov8_rknn;

// 统一导出，方便算法包一键使用
pub use dfl::decode_dfl;
pub use quantize::{dequant_i8, quant_f32};
pub use yolov8_rknn::{parse_yolov8_int8, RknnTensorOutput, Yolov8ParseContext, Yolov8RknnConfig};
