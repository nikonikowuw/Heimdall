pub mod admin_user;
pub mod alarm;
pub mod camera;
pub mod oplog;
pub mod task;

pub use admin_user::Entity as AdminUserEntity;
pub use alarm::Entity as AlarmEntity;
pub use camera::Entity as CameraEntity;
pub use oplog::Entity as OplogEntity;
pub use task::Entity as TaskEntity;
