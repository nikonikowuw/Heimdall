pub mod backend;
pub mod backends;
pub mod error;

pub use backend::InferenceBackend;
pub use backends::{CoreMlBackend, CpuBackend};
pub use error::InferError;
