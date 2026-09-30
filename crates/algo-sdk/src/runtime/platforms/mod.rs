//! 异构芯片 NPU 平台驱动实现模块
//!
//! 按硬件生态特性条件编译隔离：
//! - `rockchip`: 瑞芯微 RK3568 / RK3576 / RK3588 (librknnrt)
//! - 未来扩展：`ascend` (华为昇腾 CANN/ACL)、`coreml` (Apple Neural Engine) 等

#[cfg(feature = "rknn")]
pub mod rockchip;
