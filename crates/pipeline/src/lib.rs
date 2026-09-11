pub mod coordinator;
pub mod error;
pub mod events;
pub mod geometry;
pub mod manager;
pub mod motion_gate;
pub mod pump;
pub mod roi;
pub mod rules;
pub mod snapshot;
pub mod storage_cleaner;
pub mod thermal;
pub mod tracker;

pub use coordinator::{
    ActiveRuntimeEntry, CameraPipelineRuntimeInfo, CoordinatorError, InstanceLaunchConfig,
    InstanceRuntimeInfo, StartCameraPipelineParams, TaskRuntimeCoordinator, TaskRuntimeService,
};
pub use error::PipelineError;
pub use events::{
    EvidenceStatus, PipelineAlarmEvent, PipelineAnalysisEvent, PipelineCaptureEvent,
    PipelineTrackEvent, DEFAULT_ANALYSIS_EVENT_CHANNEL_CAPACITY,
};
pub use geometry::{check_line_crossing, point_in_polygon};
pub use manager::{
    AnalysisOutcome, PipelineManager, DEFAULT_MAX_CONCURRENT_SNAPSHOT_DECODERS,
    DEFAULT_SNAPSHOT_PERMIT_TIMEOUT_MS,
};
pub use motion_gate::MotionGate;
pub use pump::{
    AnalysisFpsGovernor, InstanceMetrics, PumpMetrics, SubStreamAnalysisPump, SubStreamPumpConfig,
    WorkerInstanceConfig,
};
pub use roi::RoiAffineMapper;
pub use rules::{RuleEvaluator, RuleIdentifier, TriggeredAlarm, DEFAULT_FULLSCREEN_RULE_INDEX};
pub use snapshot::{SnapshotCaptureMode, SnapshotConfig, SnapshotEngine, SnapshotResult};
pub use storage_cleaner::{
    detect_emmc_health, get_disk_free_ratio, get_sqlite_wal_size, stat_fs, EmmcHealthInfo,
    EvictionMetricsSnapshot, EvictionReport, EvictionStore, FsStorageStat, ReconciliationReport,
    StorageCircuitBreaker, StorageCleaner, StorageCleanerConfig, StorageDecision,
    StorageHealthLevel, StorageWatermarkThresholds,
};
pub use thermal::{
    ThermalActionPlan, ThermalGuard, ThermalLevel, ThermalPolicyConfig, ThermalZoneInfo,
};
pub use tracker::SimpleTracker;

/// 统一证据存储默认根目录
pub const DEFAULT_EVIDENCE_DIR: &str = "var/data/evidence";
