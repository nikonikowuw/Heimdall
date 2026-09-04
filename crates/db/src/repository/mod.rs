pub mod admin_user;
pub mod alarm;
pub mod camera;
pub mod oplog;
pub mod system_config;
pub mod task;

pub use admin_user::AdminUserRepo;
pub use alarm::AlarmRepo;
pub use camera::CameraRepo;
pub use oplog::OplogRepo;
pub use system_config::SystemConfigRepo;
pub use task::TaskRepo;
