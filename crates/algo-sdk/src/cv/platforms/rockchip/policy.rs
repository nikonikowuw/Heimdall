use super::config::RgaCore;
use crate::cv::types::PixelFormat;
use crate::error::AlgoError;

/// RGA2's 32-bit physical address limit.
#[allow(dead_code)]
pub const RGA2_MAX_PHYS_ADDR: u64 = 0xffff_ffff;
/// Preferred DMA32 heap for buffers that may be scheduled on RGA2.
pub const RGA_DMA32_HEAP_PATH: &str = "/dev/dma_heap/system-dma32";
/// Generic DMA heap fallback for RGA3-capable systems.
pub const RGA_SYSTEM_HEAP_PATH: &str = "/dev/dma_heap/system";
/// Maximum raster width stride accepted by the common RGA policy.
pub const RGA_MAX_STRIDE: u32 = 32_768;
pub const RGA3_MIN_DIMENSION: u32 = 68;
pub const RGA2_MIN_DIMENSION: u32 = 2;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct RgaPolicy {
    core: RgaCore,
}

impl RgaPolicy {
    pub(crate) fn new(core: RgaCore) -> Self {
        Self { core }
    }

    pub(crate) fn min_dimension(self) -> u32 {
        match self.core {
            RgaCore::Rga3Core0 | RgaCore::Rga3Core1 => RGA3_MIN_DIMENSION,
            RgaCore::Auto | RgaCore::Rga2 => RGA2_MIN_DIMENSION,
        }
    }

    pub(crate) fn scale_limits(self) -> (u32, u32) {
        match self.core {
            RgaCore::Rga3Core0 | RgaCore::Rga3Core1 => (1, 8),
            RgaCore::Auto | RgaCore::Rga2 => (1, 16),
        }
    }

    pub(crate) fn stride_alignment(self, _format: PixelFormat) -> u32 {
        match self.core {
            RgaCore::Rga3Core0 | RgaCore::Rga3Core1 => 16,
            RgaCore::Auto | RgaCore::Rga2 => 4,
        }
    }

    pub(crate) fn validate_scale_job(
        self,
        src_w: u32,
        src_h: u32,
        dst_w: u32,
        dst_h: u32,
    ) -> Result<(), AlgoError> {
        let min_dim = self.min_dimension();
        if [src_w, src_h, dst_w, dst_h]
            .into_iter()
            .any(|dimension| dimension < min_dim)
        {
            return Err(AlgoError::Preprocess {
                reason: format!(
                    "RGA {:?} requires dimensions >= {min_dim}: src={src_w}x{src_h}, dst={dst_w}x{dst_h}",
                    self.core
                ),
            });
        }

        let (min_scale_num, max_scale_den) = self.scale_limits();
        if !ratio_in_range(src_w, dst_w, min_scale_num, max_scale_den)
            || !ratio_in_range(src_h, dst_h, min_scale_num, max_scale_den)
        {
            return Err(AlgoError::Preprocess {
                reason: format!(
                    "RGA {:?} scale is outside 1/{max_scale_den}..{max_scale_den}: src={src_w}x{src_h}, dst={dst_w}x{dst_h}",
                    self.core
                ),
            });
        }
        Ok(())
    }

    pub(crate) fn validate_nv12_strides(
        self,
        width: u32,
        height: u32,
        hor_stride: u32,
        ver_stride: u32,
    ) -> Result<(), AlgoError> {
        if width == 0 || height == 0 {
            return Err(AlgoError::Preprocess {
                reason: "NV12 dimensions must be non-zero".to_string(),
            });
        }
        if !width.is_multiple_of(2) || !height.is_multiple_of(2) {
            return Err(AlgoError::Preprocess {
                reason: format!("NV12 dimensions must be even: {width}x{height}"),
            });
        }
        if hor_stride < width || ver_stride < height {
            return Err(AlgoError::Preprocess {
                reason: format!(
                    "NV12 stride is smaller than the visible frame: {hor_stride}x{ver_stride} < {width}x{height}"
                ),
            });
        }
        let alignment = self.stride_alignment(PixelFormat::Nv12);
        if hor_stride > RGA_MAX_STRIDE {
            return Err(AlgoError::Preprocess {
                reason: format!(
                    "NV12 horizontal stride {hor_stride} exceeds RGA maximum {RGA_MAX_STRIDE}"
                ),
            });
        }
        if !hor_stride.is_multiple_of(alignment) || !ver_stride.is_multiple_of(2) {
            return Err(AlgoError::Preprocess {
                reason: format!(
                    "NV12 stride {hor_stride}x{ver_stride} violates alignment {alignment}x2"
                ),
            });
        }
        Ok(())
    }

    pub(crate) fn validate_rgb_stride(
        self,
        format: PixelFormat,
        width: u32,
        height: u32,
        stride: u32,
    ) -> Result<(), AlgoError> {
        if width == 0 || height == 0 || stride < width {
            return Err(AlgoError::Preprocess {
                reason: format!("invalid {format:?} frame stride {stride} for {width}x{height}"),
            });
        }
        let alignment = self.stride_alignment(format);
        if stride > RGA_MAX_STRIDE {
            return Err(AlgoError::Preprocess {
                reason: format!("{format:?} stride {stride} exceeds RGA maximum {RGA_MAX_STRIDE}"),
            });
        }
        if !stride.is_multiple_of(alignment) {
            return Err(AlgoError::Preprocess {
                reason: format!("{format:?} stride {stride} is not aligned to {alignment}"),
            });
        }
        Ok(())
    }

    pub(crate) fn validate_output(
        self,
        width: u32,
        height: u32,
        w_stride: u32,
        h_stride: u32,
        format: PixelFormat,
    ) -> Result<(), AlgoError> {
        if width < self.min_dimension() || height < self.min_dimension() {
            return Err(AlgoError::Preprocess {
                reason: format!(
                    "RGA {:?} output dimensions are below minimum {}: {width}x{height}",
                    self.core,
                    self.min_dimension()
                ),
            });
        }
        self.validate_rgb_stride(format, width, height, w_stride)?;
        if h_stride < height || !h_stride.is_multiple_of(2) {
            return Err(AlgoError::Preprocess {
                reason: format!("output height stride {h_stride} is invalid for {height}"),
            });
        }
        Ok(())
    }

    pub(crate) fn validate_dma_allocation(self, dma32: bool) -> Result<(), AlgoError> {
        if matches!(self.core, RgaCore::Rga2) && !dma32 {
            return Err(AlgoError::Preprocess {
                reason: format!(
                    "RGA2 requires a DMA32 heap below 4GB (maximum physical address 0x{RGA2_MAX_PHYS_ADDR:x})"
                ),
            });
        }
        Ok(())
    }
}

fn ratio_in_range(src: u32, dst: u32, min_num: u32, max_den: u32) -> bool {
    let src = u64::from(src);
    let dst = u64::from(dst);
    dst.saturating_mul(u64::from(max_den)) >= src.saturating_mul(u64::from(min_num))
        && dst <= src.saturating_mul(u64::from(max_den))
}

pub(crate) fn align_up(value: u32, alignment: u32) -> Option<u32> {
    if alignment == 0 {
        return None;
    }
    let remainder = value % alignment;
    value.checked_add((alignment - remainder) % alignment)
}

pub(crate) fn checked_rgb_bytes(w_stride: u32, h_stride: u32) -> Result<usize, AlgoError> {
    let width = usize::try_from(w_stride).map_err(|_| AlgoError::OutOfMemory)?;
    let height = usize::try_from(h_stride).map_err(|_| AlgoError::OutOfMemory)?;
    width
        .checked_mul(height)
        .and_then(|pixels| pixels.checked_mul(3))
        .ok_or(AlgoError::OutOfMemory)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rga3_rejects_small_dimensions_and_extreme_scale() {
        let policy = RgaPolicy::new(RgaCore::Rga3Core0);
        assert!(policy.validate_scale_job(64, 128, 128, 128).is_err());
        assert!(policy.validate_scale_job(640, 640, 32, 640).is_err());
        assert!(policy.validate_scale_job(640, 640, 320, 320).is_ok());
    }

    #[test]
    fn rga2_accepts_small_valid_job() {
        let policy = RgaPolicy::new(RgaCore::Rga2);
        assert!(policy.validate_scale_job(32, 32, 2, 2).is_ok());
    }

    #[test]
    fn stride_and_address_guards_are_explicit() {
        let policy = RgaPolicy::new(RgaCore::Rga2);
        assert!(policy.validate_nv12_strides(640, 480, 640, 480).is_ok());
        assert!(policy.validate_nv12_strides(641, 480, 644, 480).is_err());
        assert!(policy
            .validate_rgb_stride(PixelFormat::Rgb24, 32_769, 640, 32_772)
            .is_err());
        assert!(RgaPolicy::new(RgaCore::Rga2)
            .validate_dma_allocation(false)
            .is_err());
        assert!(RgaPolicy::new(RgaCore::Rga3Core0)
            .validate_dma_allocation(false)
            .is_ok());
    }

    #[test]
    fn alignment_and_size_are_checked() {
        assert_eq!(align_up(640, 16), Some(640));
        assert_eq!(align_up(641, 16), Some(656));
        assert_eq!(checked_rgb_bytes(16, 2), Ok(96));
    }
}
