pub mod connection;
pub mod entity;
pub mod error;
pub mod migration;
pub mod repository;
pub mod schema;

pub use connection::init_db;
pub use error::DbError;
pub use migration::{init_test_db, reset_database, run_migrations, run_migrations_on_seaorm};
pub use repository::{
    algorithm::{AlgorithmRepo, AlgorithmStats, UpsertAlgorithmParams, UpsertVersionParams},
    algorithm_instance::{AlgorithmInstanceRepo, CreateInstanceParams, UpdateInstanceParams},
    camera::ProbeUpdateParams,
    AdminUserRepo, AlarmRepo, CameraRepo, CaptureRepo, GalleryFaceRepo, GalleryRepo,
    OperationalLogRepo, OplogRepo, PersonnelRepo, RecognitionRepo, SaveTaskAlgorithmInstanceParams,
    SaveTaskParams, SaveTaskWithInstancesParams, SystemConfigRepo, TaskInstanceStateUpdate,
    TaskRepo, UpdateTaskInstanceParams,
};
pub use schema::create_tables_if_not_exist;
pub use sea_orm::{self, DatabaseConnection};
