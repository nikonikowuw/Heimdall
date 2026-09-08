pub mod backend;
pub mod backends;
pub mod c_abi;
pub mod error;
pub mod npu;
pub mod package;
pub mod sandbox;
pub mod worker;

pub use backend::InferenceBackend;
pub use backends::{CoreMlBackend, CpuBackend};
pub use error::InferError;
pub use package::{
    compute_dir_size, discover_package_dirs, AlgoInstance, AlgoPackage, AlgoRegistry,
    ALGO_MANIFEST_FILENAME, DEFAULT_ALGO_PACKAGES_DIR,
};
pub use sandbox::{
    current_platform_id, normalize_platform_id, AlgoManifest, AlgoSandbox, VERIFY_ALGO_ARG,
};
pub use worker::{InferenceWorker, InferenceWorkerConfig, InferenceWorkerHandle};

// NPU 监控模块
pub use npu::monitor::{global_monitor, NpuMonitor, NpuMonitorEvent};
pub use npu::{NpuDevice, NpuDeviceMetrics, NpuDeviceType, NpuError};
