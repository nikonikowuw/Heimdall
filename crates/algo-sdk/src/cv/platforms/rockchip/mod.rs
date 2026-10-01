mod config;
mod dma_alloc;
mod engine;
mod ffi;
mod policy;
mod pool;

pub use config::{RgaCore, RgaPoolConfig, RgaPoolConfigBuilder};
pub use engine::RgaCvEngine;
pub use pool::{PooledRgaBuffer, RgaBufferPool, RgaBufferSpec, RgaPoolStats};

// 连续失败追踪与 RGA 无绑定关系，已上移至 `crate::cv::diagnostic` 供跨平台复用；
// 此处保留再导出，避免既有算法包的 `cv::platforms::rockchip::FailureTracker` 路径失效。
pub use crate::cv::diagnostic::{DiagnosticConfig, FailureTracker, FailureTrackerStatus};
