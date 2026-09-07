#[cfg(target_os = "macos")]
pub mod apple;
pub mod cpu;
pub mod host_ops;
#[cfg(all(target_os = "linux", feature = "rga"))]
pub mod rockchip;

#[cfg(target_os = "macos")]
pub use apple::AppleCvEngine;
pub use cpu::CpuCvEngine;
pub use host_ops::HostCvEngine;
#[cfg(all(target_os = "linux", feature = "rga"))]
pub use rockchip::{RgaBufferPool, RgaCvEngine, RgaPoolConfig};
