pub mod backend;
pub mod backends;
pub mod c_abi;
pub mod error;
pub mod package;
pub mod sandbox;

pub use backend::InferenceBackend;
pub use backends::{CoreMlBackend, CpuBackend};
pub use error::InferError;
pub use package::{AlgoInstance, AlgoPackage, AlgoRegistry};
pub use sandbox::{current_platform_id, normalize_platform_id, AlgoManifest, AlgoSandbox};
