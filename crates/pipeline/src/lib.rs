pub mod error;
pub mod geometry;
pub mod manager;
pub mod motion_gate;
pub mod tracker;

pub use error::PipelineError;
pub use geometry::{check_line_crossing, point_in_polygon};
pub use manager::PipelineManager;
pub use motion_gate::MotionGate;
pub use tracker::SimpleTracker;
