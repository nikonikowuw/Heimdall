pub mod error;
pub mod geometry;
pub mod manager;
pub mod motion_gate;
pub mod snapshot;
pub mod tracker;

pub use error::PipelineError;
pub use geometry::{check_line_crossing, point_in_polygon};
pub use manager::PipelineManager;
pub use motion_gate::MotionGate;
pub use snapshot::{SnapshotEngine, SnapshotResult};
pub use tracker::SimpleTracker;
