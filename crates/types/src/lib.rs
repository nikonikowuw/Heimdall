pub mod alarm;
pub mod auth;
pub mod camera;
pub mod detection;
pub mod error;
pub mod event;
pub mod frame;
pub mod oplog;
pub mod system;
pub mod task;

pub use alarm::{AlarmRecord, AlarmSeverity, AlarmStatus, AlarmType};
pub use auth::{
    AdminUser, AdminUserDto, AuthClaims, ChangePasswordRequest, InitStatusResponse,
    InitializeRequest, LoginRequest, LoginResponse,
};
pub use camera::{
    Camera, CameraProbeEvent, CameraProtocol, CameraTelemetryEvent, CodecType, CreateCameraRequest,
    EncodedPacket, ProbeResult, ProbeStatus, StreamKey, StreamType, TransportPolicy,
    UpdateCameraRequest,
};
pub use detection::{BoundingBox, CameraTracksPayload, Detection, TrackDto, TrackedObject};
pub use error::{FrameError, TypeError};
pub use event::{
    TOPIC_ALARM_STATUS_CHANGED, TOPIC_ALARM_TRIGGERED, TOPIC_CAMERA_PROBE_UPDATED,
    TOPIC_CAMERA_TRACKS,
};
pub use frame::{FrameHandle, FrameRef, PixelFormat, StrideInfo};
pub use oplog::OperationLog;
pub use system::{
    CoreMetrics, CpuMetrics, DiskMetrics, EvictionReport, ForceSyncResponse, InterfaceCapabilities,
    IpConfig, IpMethod, MemoryMetrics, NetworkChangeOperation, NetworkDiagnosticRequest,
    NetworkDiagnosticResult, NetworkDiagnosticType, NetworkInterface, NetworkInterfaceMetrics,
    NetworkInterfaceState, NetworkInterfaceType, NetworkInterfacesResponse, NetworkManager,
    NetworkUpdateResult, NpuCoreMetrics, NpuMetrics, OperationConfirmResult, OperationStatus,
    OverwriteMode, ProcessMetrics, SetTimeResponse, StorageConfig, StorageHealthLevel,
    StorageStatus, SystemOverview, ThermalMetrics, ThermalZone, TimeConfig, TimeStatus,
};
pub use task::{
    aggregate_task_instance_status, validate_task_algorithm_instances, AnalysisTask,
    DetectionLineDirection, DetectionPoint, DetectionRule, DetectionRuleRole, MotionGateConfig,
    TaskAlgorithmInstanceConfig, TaskStatus,
};
