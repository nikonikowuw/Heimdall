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
pub use detection::{BoundingBox, Detection, TrackedObject};
pub use error::{FrameError, TypeError};
pub use event::{TOPIC_ALARM_STATUS_CHANGED, TOPIC_ALARM_TRIGGERED, TOPIC_CAMERA_PROBE_UPDATED};
pub use frame::{FrameHandle, FrameRef, PixelFormat, StrideInfo};
pub use oplog::OperationLog;
pub use system::{
    EvictionReport, ForceSyncResponse, InterfaceCapabilities, IpConfig, IpMethod,
    NetworkChangeOperation, NetworkInterface, NetworkInterfaceState, NetworkInterfaceType,
    NetworkInterfacesResponse, NetworkManager, NetworkUpdateResult, OperationConfirmResult,
    OperationStatus, OverwriteMode, SetTimeResponse, StorageConfig, StorageHealthLevel,
    StorageStatus, SystemOverview, TimeConfig, TimeStatus,
};
pub use task::{
    AnalysisTask, DetectionLineDirection, DetectionPoint, DetectionRule, DetectionRuleRole,
    MotionGateConfig, TaskStatus,
};
