//! 事件录像运行时（Phase 4）
//!
//! - [`config`]：通道级录像配置
//! - [`worker`]：`RecordingWorker` 专用 OS 线程状态机

pub mod config;
pub mod worker;

pub use config::{RecordingConfig, RecordingMode};
pub use worker::{
    FinishedRecording, LinkedEvent, RecordingEventType, RecordingTrigger, RecordingWorker,
};
