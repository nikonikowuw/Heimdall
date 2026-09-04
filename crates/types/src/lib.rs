pub mod alarm;
pub mod camera;
pub mod detection;
pub mod error;
pub mod frame;
pub mod oplog;
pub mod task;

pub use alarm::{AlarmRecord, AlarmType};
pub use camera::{Camera, ProbeStatus, TransportPolicy};
pub use detection::{BoundingBox, Detection, TrackedObject};
pub use error::{FrameError, TypeError};
pub use frame::{FrameHandle, FrameRef, PixelFormat, StrideInfo};
pub use oplog::OperationLog;
pub use task::{
    AnalysisTask, DetectionLineDirection, DetectionPoint, DetectionRule, DetectionRuleRole,
    MotionGateConfig, TaskStatus,
};
