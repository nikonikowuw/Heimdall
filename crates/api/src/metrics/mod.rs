//! 系统指标采集模块
//!
//! 提供跨平台的系统指标统一采集接口，直接产出 `types::` API 响应类型。
//!
//! - Linux: 读取 `/proc/stat`、`/proc/meminfo`、`/sys/class/net/`、`/sys/class/thermal/`
//! - macOS: Mach `host_processor_info` + `sysctl` + `netstat`
//!
//! 所有采集器均为异步（内部通过 `tokio::task::spawn_blocking` 处理阻塞 I/O）。

pub mod cpu;
pub mod disk;
#[cfg(target_os = "macos")]
pub mod macos_ticks;
pub mod memory;
pub mod network;
pub mod thermal;

pub use cpu::CpuCollector;
pub use disk::DiskCollector;
pub use memory::MemoryCollector;
pub use network::NetworkCollector;
pub use thermal::ThermalCollector;
