pub mod admin_user;
pub mod alarm;
pub mod algorithm;
pub mod algorithm_instance;
pub mod camera;
pub mod capture;
pub mod gallery_face;
pub mod gb28181_device;
pub mod operational_log;
pub mod oplog;
pub mod personnel;
pub mod recognition;
pub mod sys_gb28181_config;
pub mod system_config;
pub mod task;

pub use admin_user::AdminUserRepo;
pub use alarm::AlarmRepo;
pub use algorithm::{AlgorithmRepo, AlgorithmStats, UpsertAlgorithmParams, UpsertVersionParams};
pub use algorithm_instance::{AlgorithmInstanceRepo, CreateInstanceParams, UpdateInstanceParams};
pub use camera::CameraRepo;
pub use capture::{CaptureFilter, CaptureRepo};
pub use gallery_face::GalleryFaceRepo;
pub use gb28181_device::Gb28181DeviceRepo;
pub use operational_log::OperationalLogRepo;
pub use oplog::OplogRepo;
pub use personnel::PersonnelRepo;
pub use recognition::{RecognitionRepo, UpdateRecognitionReviewParams};
pub use sys_gb28181_config::SysGb28181ConfigRepo;
pub use system_config::SystemConfigRepo;
pub use task::{
    SaveTaskAlgorithmInstanceParams, SaveTaskParams, SaveTaskWithInstancesParams,
    TaskInstanceStateUpdate, TaskRepo, UpdateTaskInstanceParams,
};
