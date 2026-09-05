pub mod error;
pub mod geometry;
pub mod manager;
pub mod motion_gate;
pub mod roi;
pub mod rules;
pub mod snapshot;
pub mod storage_cleaner;
pub mod thermal;
pub mod tracker;

pub use error::PipelineError;
pub use geometry::{check_line_crossing, point_in_polygon};
pub use manager::{
    PipelineManager, DEFAULT_MAX_CONCURRENT_SNAPSHOT_DECODERS, DEFAULT_SNAPSHOT_PERMIT_TIMEOUT_MS,
};
pub use motion_gate::MotionGate;
pub use roi::RoiAffineMapper;
pub use rules::{RuleEvaluator, TriggeredAlarm};
pub use snapshot::{SnapshotCaptureMode, SnapshotConfig, SnapshotEngine, SnapshotResult};
pub use storage_cleaner::{
    get_disk_free_ratio, EvictionReport, EvictionStore, StorageCleaner, StorageCleanerConfig,
};
pub use thermal::{ThermalGuard, ThermalLevel};
pub use tracker::SimpleTracker;
