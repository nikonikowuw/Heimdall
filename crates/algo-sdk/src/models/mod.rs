//! 常见视觉模型抽象与预制架构 (`models`)

pub mod yolo;

pub use yolo::{
    GenericDetector, GenericYoloDetector, StandardYoloConfig, YoloDecodeContext, YoloDecoder,
    YoloSpec, Yolov8SpecDecoder,
};
