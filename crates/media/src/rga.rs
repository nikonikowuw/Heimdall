//! Rockchip RGA 硬件加速防御性抽象模块
//!
//! 在代码层面严格防护生产级 Rockchip 流水线隐患：
//! 1. 根除 RGA DMA-BUF 5 阶段级联崩溃：统一 Handle 模式，杜绝每帧调用 `wrapbuffer_fd`；
//! 2. 防御 RGA3 分辨率与缩放限制：输入/输出宽高 $\ge 68$，缩放比 $1/8\sim 8\times$；小图自动校验拦截；
//! 3. 防御 RGA2 4GB 物理地址硬限制：32 位 MMU 校验，推荐 `/dev/dma_heap/system-dma32`；
//! 4. 步长对齐防御：校验 NV12 4/16 像素对齐，拒绝未经 Runtime 查询的静态假定。

use crate::error::MediaError;

// ============================================================================
// 常量与硬件限制定义
// ============================================================================

/// RGA2 32 位 MMU 最大物理寻址极限 (4GB - 1)
pub const RGA2_MAX_PHYS_ADDR: u64 = 0xFFFF_FFFF;

/// Linux 内核标准 DMA32 堆节点（保证物理地址落入 0~4GB 空间）
pub const RGA_DMA32_HEAP_PATH: &str = "/dev/dma_heap/system-dma32";

/// Linux 内核通用系统堆节点（可能分配到 4GB 以上空间）
pub const RGA_SYSTEM_HEAP_PATH: &str = "/dev/dma_heap/system";

/// RGA3 硬件最小输入输出分辨率限制（像素）
pub const RGA3_MIN_DIMENSION: u32 = 68;

/// RGA2 硬件最小输入输出分辨率限制（像素）
pub const RGA2_MIN_DIMENSION: u32 = 2;

// ============================================================================
// RGA 核心调度与策略
// ============================================================================

/// RGA 硬件核心掩码定义
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RgaCore {
    /// 自动由驱动调度（在 RK3588 上可能导致 RGA2/RGA3 跨帧混跑画面抖动，不推荐高帧率视频路径）
    Auto,
    /// 强制绑定 RGA2 核心（RK3568/RK3576/RK3588 通用，支持小尺寸与大缩放，限 4GB 物理地址）
    Rga2,
    /// 强制绑定 RGA3 Core 0（RK3588 专属，40-bit 寻址，支持 4GB 以上，要求尺寸 >= 68）
    Rga3Core0,
    /// 强制绑定 RGA3 Core 1（RK3588 专属）
    Rga3Core1,
}

impl RgaCore {
    /// 检查该核心是否具备 40 位物理寻址能力（免疫 4GB 越界限制）
    #[inline]
    pub fn supports_over_4gb(self) -> bool {
        matches!(self, Self::Rga3Core0 | Self::Rga3Core1)
    }

    /// 获取硬件最小支持的分辨率（宽高单边像素）
    #[inline]
    pub fn min_dimension(self) -> u32 {
        match self {
            Self::Rga3Core0 | Self::Rga3Core1 => RGA3_MIN_DIMENSION,
            Self::Rga2 | Self::Auto => RGA2_MIN_DIMENSION,
        }
    }

    /// 获取硬件支持的最大缩小比例（如 1/8 或 1/16）
    #[inline]
    pub fn min_scale_ratio(self) -> f32 {
        match self {
            Self::Rga3Core0 | Self::Rga3Core1 => 0.125, // 1/8x
            Self::Rga2 | Self::Auto => 0.0625,          // 1/16x
        }
    }

    /// 获取硬件支持的最大放大比例（如 8x 或 16x）
    #[inline]
    pub fn max_scale_ratio(self) -> f32 {
        match self {
            Self::Rga3Core0 | Self::Rga3Core1 => 8.0,
            Self::Rga2 | Self::Auto => 16.0,
        }
    }
}

/// RGA 任务参数防御性校验器
#[derive(Debug, Clone, Copy)]
pub struct RgaPolicyChecker;

impl RgaPolicyChecker {
    /// 校验缩放与裁剪作业是否符合目标核心的硬性物理规范
    pub fn validate_scale_job(
        core: RgaCore,
        src_w: u32,
        src_h: u32,
        dst_w: u32,
        dst_h: u32,
    ) -> Result<(), MediaError> {
        let min_dim = core.min_dimension();
        if src_w < min_dim || src_h < min_dim || dst_w < min_dim || dst_h < min_dim {
            return Err(MediaError::Decode {
                reason: format!(
                    "RGA 核心 ({core:?}) 违背最小分辨率限制 ({min_dim}px): src=({src_w}x{src_h}), dst=({dst_w}x{dst_h})。若需小图抠图请强制调度至 RGA2 核心",
                ),
            });
        }

        let scale_x = dst_w as f32 / src_w as f32;
        let scale_y = dst_h as f32 / src_h as f32;
        let min_scale = core.min_scale_ratio();
        let max_scale = core.max_scale_ratio();

        if scale_x < min_scale || scale_x > max_scale || scale_y < min_scale || scale_y > max_scale
        {
            return Err(MediaError::Decode {
                reason: format!(
                    "RGA 核心 ({core:?}) 违背缩放倍率范围 [{min_scale}x, {max_scale}x]: scale_x={scale_x:.2}, scale_y={scale_y:.2}",
                ),
            });
        }

        Ok(())
    }

    /// 校验物理地址是否违背 RGA2 4GB 硬件寻址上限
    pub fn validate_phys_address(core: RgaCore, phys_addr: u64) -> Result<(), MediaError> {
        if !core.supports_over_4gb() && phys_addr > RGA2_MAX_PHYS_ADDR {
            return Err(MediaError::Decode {
                reason: format!(
                    "物理地址 0x{phys_addr:x} 超过 4GB 寻址空间，无法被 {core:?} (32位 MMU) 访问。请将分配器切换至 {RGA_DMA32_HEAP_PATH} 或调度至 RGA3 核心",
                ),
            });
        }
        Ok(())
    }

    /// 校验输入 YUV420SP (NV12) 步长与几何尺寸
    pub fn validate_nv12_strides(
        core: RgaCore,
        width: u32,
        height: u32,
        hor_stride: u32,
        ver_stride: u32,
    ) -> Result<(), MediaError> {
        if width == 0 || height == 0 {
            return Err(MediaError::Decode {
                reason: "几何尺寸宽高不能为 0".to_string(),
            });
        }

        // YUV420SP 要求逻辑宽高与偏移为偶数
        if !width.is_multiple_of(2) || !height.is_multiple_of(2) {
            return Err(MediaError::Decode {
                reason: format!("NV12 逻辑尺寸必须为偶数: {width}x{height}"),
            });
        }

        if hor_stride < width || ver_stride < height {
            return Err(MediaError::Decode {
                reason: format!(
                    "步长小于图像尺寸: hor_stride={hor_stride} < width={width} 或 ver_stride={ver_stride} < height={height}"
                ),
            });
        }

        // 对齐要求：RGA3 NV12 要求水平步长 16 像素对齐，RGA2 要求至少 4 像素对齐
        let align_req = match core {
            RgaCore::Rga3Core0 | RgaCore::Rga3Core1 => 16,
            RgaCore::Rga2 | RgaCore::Auto => 4,
        };

        if !hor_stride.is_multiple_of(align_req) {
            return Err(MediaError::Decode {
                reason: format!(
                    "NV12 水平步长 ({hor_stride}) 未满足核心 ({core:?}) 要求的 {align_req} 像素对齐约束",
                ),
            });
        }

        // 垂直高度要求至少 2 像素对齐
        if !ver_stride.is_multiple_of(2) {
            return Err(MediaError::Decode {
                reason: format!("NV12 垂直步长 ({ver_stride}) 必须满足偶数对齐约束"),
            });
        }

        Ok(())
    }
}

// ============================================================================
// 句柄生命周期管理模式（彻底消灭 5 阶段级联崩溃与 Mixed Handle Trap）
// ============================================================================

pub type RgaBufferHandleRaw = i32;

/// 模拟/真实 librga 句柄释放钩子
type ReleaseFn = unsafe fn(handle: RgaBufferHandleRaw);

/// 静态 Buffer（例如 RKNN 输入张量、长驻输出缓冲）
///
/// 契约：
/// 1. 初始化时单次调用 `importbuffer_fd` 获取 `handle`；
/// 2. 整个生命周期内复用 `handle`；
/// 3. 析构时调用 `releasebuffer_handle`；
/// 4. 杜绝每帧重新 import，杜绝句柄泄漏。
#[derive(Debug)]
pub struct RgaStaticBuffer {
    handle: RgaBufferHandleRaw,
    size: usize,
    release_fn: Option<ReleaseFn>,
}

impl RgaStaticBuffer {
    pub fn new(
        handle: RgaBufferHandleRaw,
        size: usize,
        release_fn: Option<ReleaseFn>,
    ) -> Result<Self, MediaError> {
        if handle <= 0 {
            return Err(MediaError::Decode {
                reason: format!("非法 RGA 静态句柄: {handle}"),
            });
        }
        Ok(Self {
            handle,
            size,
            release_fn,
        })
    }

    #[inline]
    pub fn handle(&self) -> RgaBufferHandleRaw {
        self.handle
    }

    #[inline]
    pub fn size(&self) -> usize {
        self.size
    }
}

impl Drop for RgaStaticBuffer {
    fn drop(&mut self) {
        if self.handle > 0 {
            if let Some(release) = self.release_fn {
                // SAFETY: self.handle 有效，在静态缓冲生命周期终结时安全释放
                unsafe {
                    release(self.handle);
                }
            }
        }
    }
}

/// 动态 Buffer 作用域守卫（针对 MPP 每一帧解出的动态 DMA-BUF）
///
/// 契约：
/// 1. 每帧处理前显式 `importbuffer_fd`；
/// 2. 在离开本作用域时**强制显式释放句柄**；
/// 3. **严禁调用 `wrapbuffer_fd`**，从代码结构上彻底切断 librga 基于 fd 命中过期脏缓存的级联崩溃！
#[derive(Debug)]
pub struct RgaScopedBuffer {
    handle: RgaBufferHandleRaw,
    release_fn: Option<ReleaseFn>,
}

impl RgaScopedBuffer {
    pub fn new(
        handle: RgaBufferHandleRaw,
        release_fn: Option<ReleaseFn>,
    ) -> Result<Self, MediaError> {
        if handle <= 0 {
            return Err(MediaError::Decode {
                reason: format!("非法 RGA 作用域句柄: {handle}"),
            });
        }
        Ok(Self { handle, release_fn })
    }

    #[inline]
    pub fn handle(&self) -> RgaBufferHandleRaw {
        self.handle
    }
}

impl Drop for RgaScopedBuffer {
    fn drop(&mut self) {
        if self.handle > 0 {
            if let Some(release) = self.release_fn {
                // SAFETY: 动态句柄在帧处理退出时立即归还，保证句柄缓存无残留
                unsafe {
                    release(self.handle);
                }
            }
        }
    }
}

// ============================================================================
// 单元测试
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[test]
    fn test_rga3_min_dimension_enforcement() {
        // RGA3 要求 >= 68px
        let res = RgaPolicyChecker::validate_scale_job(RgaCore::Rga3Core0, 64, 64, 128, 128);
        assert!(res.is_err());
        let err = res.expect_err("应当超出最小分辨率限制").to_string();
        assert!(err.contains("68px"));

        // RGA2 允许 64x64
        let res2 = RgaPolicyChecker::validate_scale_job(RgaCore::Rga2, 64, 64, 128, 128);
        assert!(res2.is_ok());
    }

    #[test]
    fn test_rga3_scale_ratio_enforcement() {
        // 1920 -> 120 缩小了 16 倍，超过 RGA3 1/8x 限制
        let res = RgaPolicyChecker::validate_scale_job(
            RgaCore::Rga3Core0,
            1920,
            1080,
            120,
            67, // 既超缩小比又小于 68
        );
        assert!(res.is_err());

        // 1920 -> 480 缩小 4 倍，在 1/8x ~ 8x 内
        let res_ok = RgaPolicyChecker::validate_scale_job(RgaCore::Rga3Core0, 1920, 1080, 480, 270);
        assert!(res_ok.is_ok());
    }

    #[test]
    fn test_rga2_4gb_address_boundary_defense() {
        // 地址在 4GB 以内：RGA2 与 RGA3 均通过
        let low_addr = 0x8000_0000;
        assert!(RgaPolicyChecker::validate_phys_address(RgaCore::Rga2, low_addr).is_ok());
        assert!(RgaPolicyChecker::validate_phys_address(RgaCore::Rga3Core0, low_addr).is_ok());

        // 地址超出 4GB (例如 8GB 系统的 0x1_2000_0000)
        let high_addr = 0x1_2000_0000;
        // RGA2 必须直接报错拦截！
        let rga2_res = RgaPolicyChecker::validate_phys_address(RgaCore::Rga2, high_addr);
        assert!(rga2_res.is_err());
        assert!(rga2_res
            .expect_err("应当超出 4GB 限制")
            .to_string()
            .contains("4GB"));

        // RGA3 支持 40-bit 寻址，正常放行
        assert!(RgaPolicyChecker::validate_phys_address(RgaCore::Rga3Core0, high_addr).is_ok());
    }

    #[test]
    fn test_rga_nv12_stride_alignment() {
        // RGA3 要求 16 字节对齐
        assert!(RgaPolicyChecker::validate_nv12_strides(
            RgaCore::Rga3Core0,
            1920,
            1080,
            1920,
            1080
        )
        .is_ok());
        // 水平步长 1924 (非 16 倍数) 应当报错
        assert!(RgaPolicyChecker::validate_nv12_strides(
            RgaCore::Rga3Core0,
            1920,
            1080,
            1924,
            1080
        )
        .is_err());

        // RGA2 要求 4 字节对齐，1924 允许
        assert!(
            RgaPolicyChecker::validate_nv12_strides(RgaCore::Rga2, 1920, 1080, 1924, 1080).is_ok()
        );

        // 奇数宽高应当被拦截
        assert!(
            RgaPolicyChecker::validate_nv12_strides(RgaCore::Rga2, 1921, 1080, 1924, 1080).is_err()
        );
    }

    static RELEASED_COUNT: AtomicUsize = AtomicUsize::new(0);
    unsafe fn mock_release(handle: RgaBufferHandleRaw) {
        if handle == 42 {
            RELEASED_COUNT.fetch_add(1, Ordering::SeqCst);
        }
    }

    #[test]
    fn test_rga_buffer_lifecycle_raii() {
        RELEASED_COUNT.store(0, Ordering::SeqCst);
        {
            let scoped = RgaScopedBuffer::new(42, Some(mock_release));
            assert!(scoped.is_ok());
            assert_eq!(scoped.as_ref().expect("应当创建成功").handle(), 42);
        }
        assert_eq!(RELEASED_COUNT.load(Ordering::SeqCst), 1);

        {
            let stat = RgaStaticBuffer::new(42, 1024, Some(mock_release));
            assert!(stat.is_ok());
            assert_eq!(stat.as_ref().expect("应当创建成功").handle(), 42);
        }
        assert_eq!(RELEASED_COUNT.load(Ordering::SeqCst), 2);
    }
}
