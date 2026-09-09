use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use super::config::RgaPoolConfig;
use super::ffi::{rgb_color_space_mode, rgb_fill_color, ImRect, RgaRuntime, RgaWrapParams};
use super::policy::RgaPolicy;
use super::pool::{RgaBufferPool, RgaBufferSpec};
use crate::cv::engine::CvEngine;
use crate::cv::layout::compute_letterbox_layout;
use crate::cv::platforms::cpu::CpuCvEngine;
use crate::cv::types::{PixelFormat, PreprocessMode};
use crate::cv::CvBuffer;
use crate::error::AlgoError;
use crate::frame::{FrameHandleView, SafeFrame};

const RGA_FORMAT_NV12: u32 = 0x0a00;
const RGA_FORMAT_BGRA_8888: u32 = 0x0300;
const RGA_FORMAT_RGB_888: u32 = 0x0200;
const RGA_FORMAT_YUV420_PLANAR: u32 = 0x0b00;
const IM_YUV_BT601_LIMIT_RANGE: i32 = 3 << 8;
const IM_YUV_BT601_FULL_RANGE: i32 = 4 << 8;
const IM_YUV_BT709_LIMIT_RANGE: i32 = 5 << 8;
const IM_YUV_BT709_FULL_RANGE: i32 = 6 << 8;

/// Maximum number of distinct output geometries cached simultaneously by one engine.
const MAX_CACHED_POOLS: usize = 16;

/// Rockchip RGA-backed CV engine with a CPU fallback for host frames or missing librga.
pub struct RgaCvEngine {
    runtime: Option<Arc<RgaRuntime>>,
    config: RgaPoolConfig,
    pools: Mutex<HashMap<RgaBufferSpec, Arc<RgaBufferPool>>>,
    cpu: CpuCvEngine,
}

impl std::fmt::Debug for RgaCvEngine {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("RgaCvEngine")
            .field("hardware_available", &self.runtime.is_some())
            .field("config", &self.config)
            .finish()
    }
}

impl Default for RgaCvEngine {
    fn default() -> Self {
        Self::new()
    }
}

impl RgaCvEngine {
    pub fn new() -> Self {
        let config = RgaPoolConfig::from_env().unwrap_or_else(|error| {
            tracing::warn!(%error, "invalid RGA pool environment overrides; using defaults");
            RgaPoolConfig::default()
        });
        Self::new_with_config(config).unwrap_or_else(|_| {
            Self::new_with_config(RgaPoolConfig::default())
                .expect("default RgaPoolConfig is always valid")
        })
    }

    pub fn new_with_config(config: RgaPoolConfig) -> Result<Self, AlgoError> {
        config.validate()?;
        let runtime = match RgaRuntime::load() {
            Ok(runtime) => Some(runtime),
            Err(error) => {
                tracing::warn!(%error, "librga unavailable; RgaCvEngine will use CPU for host frames");
                None
            }
        };
        Ok(Self {
            runtime,
            config,
            pools: Mutex::new(HashMap::new()),
            cpu: CpuCvEngine::new(),
        })
    }

    pub fn hardware_available(&self) -> bool {
        self.runtime.is_some()
    }

    fn pool_for(&self, spec: RgaBufferSpec) -> Result<Arc<RgaBufferPool>, AlgoError> {
        {
            let pools = self.pools.lock().map_err(|_| AlgoError::Internal {
                reason: "RGA pool map lock poisoned".to_string(),
            })?;
            if let Some(pool) = pools.get(&spec).cloned() {
                return Ok(pool);
            }
            if pools.len() >= MAX_CACHED_POOLS {
                return Err(AlgoError::Preprocess {
                    reason: format!(
                        "exceeded maximum cached RGA buffer pools ({MAX_CACHED_POOLS})"
                    ),
                });
            }
        }

        let runtime = self.runtime.as_ref().ok_or_else(|| AlgoError::Internal {
            reason: "RGA runtime is unavailable".to_string(),
        })?;
        let config = self.config.for_output(spec.width, spec.height, spec.format);
        let candidate = Arc::new(RgaBufferPool::with_runtime(config, Arc::clone(runtime))?);
        let mut pools = self.pools.lock().map_err(|_| AlgoError::Internal {
            reason: "RGA pool map lock poisoned".to_string(),
        })?;
        if pools.len() >= MAX_CACHED_POOLS && !pools.contains_key(&spec) {
            return Err(AlgoError::Preprocess {
                reason: format!("exceeded maximum cached RGA buffer pools ({MAX_CACHED_POOLS})"),
            });
        }
        Ok(pools.entry(spec).or_insert(candidate).clone())
    }

    fn source_layout(
        &self,
        frame: &SafeFrame<'_>,
        policy: RgaPolicy,
    ) -> Result<SourceLayout, AlgoError> {
        let format = PixelFormat::from_c_abi(frame.pixel_format());
        let width = frame.width();
        let height = frame.height();
        let alloc_width = frame.alloc_width().max(width);
        let alloc_height = frame.alloc_height().max(height);
        if width == 0 || height == 0 || alloc_width == 0 || alloc_height == 0 {
            return Err(AlgoError::Preprocess {
                reason: "RGA source dimensions must be non-zero".to_string(),
            });
        }

        let (rga_format, w_stride, h_stride, size) = match format {
            PixelFormat::Nv12 => {
                let stride_y = positive_stride(frame.stride(0), alloc_width)?;
                let stride_uv = positive_stride(frame.stride(1), stride_y)?;
                if stride_uv != stride_y {
                    return Err(AlgoError::IncompatibleFrame {
                        reason: format!(
                            "RGA NV12 requires a shared Y/UV stride: y={stride_y}, uv={stride_uv}"
                        ),
                    });
                }
                validate_rga_offsets(frame, format, stride_y, stride_uv, alloc_height)?;
                policy.validate_nv12_strides(width, height, stride_y, alloc_height)?;
                let y_start =
                    usize::try_from(frame.plane_offset(0)).map_err(|_| AlgoError::OutOfMemory)?;
                let y_len = checked_plane_len(stride_y, alloc_height)?;
                let y_end = y_start.checked_add(y_len).ok_or(AlgoError::OutOfMemory)?;
                let uv_start = plane_start(frame.plane_offset(1), y_end)?;
                let uv_len = checked_plane_len(stride_uv, alloc_height.div_ceil(2))?;
                let total = y_end.max(uv_start.checked_add(uv_len).ok_or(AlgoError::OutOfMemory)?);
                (RGA_FORMAT_NV12, stride_y, alloc_height, total)
            }
            PixelFormat::I420 => {
                if !width.is_multiple_of(2) || !height.is_multiple_of(2) {
                    return Err(AlgoError::Preprocess {
                        reason: format!("I420 dimensions must be even: {width}x{height}"),
                    });
                }
                let stride_y = positive_stride(frame.stride(0), alloc_width)?;
                let chroma_width = alloc_width.div_ceil(2);
                let stride_u = positive_stride(frame.stride(1), chroma_width)?;
                let stride_v = positive_stride(frame.stride(2), stride_u)?;
                if stride_y % 2 != 0 || stride_u != stride_y / 2 || stride_v != stride_u {
                    return Err(AlgoError::IncompatibleFrame {
                        reason: format!(
                            "RGA I420 requires Y stride twice the shared U/V stride: y={stride_y}, u={stride_u}, v={stride_v}"
                        ),
                    });
                }
                validate_rga_offsets(frame, format, stride_y, stride_u, alloc_height)?;
                let y_start =
                    usize::try_from(frame.plane_offset(0)).map_err(|_| AlgoError::OutOfMemory)?;
                let y_len = checked_plane_len(stride_y, alloc_height)?;
                let y_end = y_start.checked_add(y_len).ok_or(AlgoError::OutOfMemory)?;
                let chroma_height = alloc_height.div_ceil(2);
                let u_start = plane_start(frame.plane_offset(1), y_end)?;
                let u_len = checked_plane_len(stride_u, chroma_height)?;
                let u_end = u_start.checked_add(u_len).ok_or(AlgoError::OutOfMemory)?;
                let v_start = plane_start(frame.plane_offset(2), u_end)?;
                let v_len = checked_plane_len(stride_v, chroma_height)?;
                let total = y_end.max(v_start.checked_add(v_len).ok_or(AlgoError::OutOfMemory)?);
                policy.validate_nv12_strides(width, height, stride_y, alloc_height)?;
                policy.validate_rgb_stride(
                    PixelFormat::I420,
                    chroma_width,
                    chroma_height,
                    stride_u,
                )?;
                policy.validate_rgb_stride(
                    PixelFormat::I420,
                    chroma_width,
                    chroma_height,
                    stride_v,
                )?;
                (RGA_FORMAT_YUV420_PLANAR, stride_y, alloc_height, total)
            }
            PixelFormat::Rgb24 | PixelFormat::Bgra => {
                let bytes_per_pixel = if format == PixelFormat::Rgb24 { 3 } else { 4 };
                let default_stride = alloc_width
                    .checked_mul(bytes_per_pixel)
                    .ok_or(AlgoError::OutOfMemory)?;
                let stride_bytes = positive_stride(frame.stride(0), default_stride)?;
                if !stride_bytes.is_multiple_of(bytes_per_pixel) {
                    return Err(AlgoError::Preprocess {
                        reason: format!(
                            "{format:?} byte stride is not pixel aligned: {stride_bytes}"
                        ),
                    });
                }
                let stride_pixels = stride_bytes / bytes_per_pixel;
                validate_rga_offsets(frame, format, stride_pixels, 0, alloc_height)?;
                policy.validate_rgb_stride(format, width, height, stride_pixels)?;
                let start =
                    usize::try_from(frame.plane_offset(0)).map_err(|_| AlgoError::OutOfMemory)?;
                let length = checked_plane_len(stride_bytes, alloc_height)?;
                let total = start.checked_add(length).ok_or(AlgoError::OutOfMemory)?;
                let rga_format = if format == PixelFormat::Rgb24 {
                    RGA_FORMAT_RGB_888
                } else {
                    RGA_FORMAT_BGRA_8888
                };
                (rga_format, stride_pixels, alloc_height, total)
            }
            PixelFormat::Unknown(code) => {
                return Err(AlgoError::IncompatibleFrame {
                    reason: format!("unsupported RGA source pixel format 0x{code:x}"),
                });
            }
        };

        Ok(SourceLayout {
            rga_format,
            width,
            height,
            w_stride,
            h_stride,
            size,
            color_space_mode: source_color_space_mode(frame, format),
        })
    }

    fn can_use_hardware(&self, frame: &SafeFrame<'_>) -> bool {
        self.runtime.is_some() && matches!(frame.handle_view(), FrameHandleView::DmaBuf { .. })
    }

    fn hardware_preprocess(
        &self,
        frame: &SafeFrame<'_>,
        source: &SourceLayout,
        dst_w: u32,
        dst_h: u32,
        dst_rect: ImRect,
        fill_color: Option<[u8; 3]>,
    ) -> Result<CvBuffer, AlgoError> {
        let runtime = self.runtime.as_ref().ok_or_else(|| AlgoError::Internal {
            reason: "RGA runtime is unavailable".to_string(),
        })?;
        let spec =
            RgaBufferSpec::from_config(&self.config.for_output(dst_w, dst_h, PixelFormat::Rgb24))?;
        let pool = self.pool_for(spec)?;
        let lease = pool.acquire()?;
        let dma_fd = dma_fd(frame)?;
        let src_handle = runtime.import_buffer_fd(
            dma_fd,
            source.size,
            source.w_stride,
            source.h_stride,
            source.rga_format,
        )?;
        let _source_guard = runtime.source_guard(src_handle);
        let src = runtime.wrap_buffer(RgaWrapParams {
            handle: src_handle,
            width: source.width,
            height: source.height,
            wstride: source.w_stride,
            hstride: source.h_stride,
            format: source.rga_format,
            color_space_mode: source.color_space_mode,
        })?;
        let dst = runtime.wrap_buffer(RgaWrapParams {
            handle: lease.handle(),
            width: spec.width,
            height: spec.height,
            wstride: spec.w_stride,
            hstride: spec.h_stride,
            format: RGA_FORMAT_RGB_888,
            color_space_mode: rgb_color_space_mode(),
        })?;
        let src_rect = im_rect(0, 0, source.width, source.height)?;
        let fill = match fill_color {
            Some(color) => Some((
                im_rect(0, 0, spec.width, spec.height)?,
                rgb_fill_color(color),
            )),
            None => None,
        };
        runtime.execute(src, dst, src_rect, dst_rect, fill)?;
        let fd = lease.fd();
        Ok(CvBuffer::from_dma_buf_with_layout(
            fd,
            spec.width,
            spec.height,
            PixelFormat::Rgb24,
            spec.size,
            [
                spec.w_stride.checked_mul(3).ok_or(AlgoError::OutOfMemory)?,
                0,
                0,
                0,
            ],
            spec.h_stride,
            Some(Box::new(lease)),
        ))
    }

    fn hardware_letterbox(
        &self,
        frame: &SafeFrame<'_>,
        dst_w: u32,
        dst_h: u32,
        fill_color: [u8; 3],
    ) -> Result<(CvBuffer, PreprocessMode), AlgoError> {
        let policy = RgaPolicy::new(self.config.core);
        let source = self.source_layout(frame, policy)?;
        let layout = compute_letterbox_layout(source.width, source.height, dst_w, dst_h);
        policy.validate_scale_job(
            source.width,
            source.height,
            layout.scaled_w,
            layout.scaled_h,
        )?;
        let dst_rect = im_rect(
            layout.pad_left,
            layout.pad_top,
            layout.scaled_w,
            layout.scaled_h,
        )?;
        let buffer =
            self.hardware_preprocess(frame, &source, dst_w, dst_h, dst_rect, Some(fill_color))?;
        Ok((buffer, PreprocessMode::Letterbox(layout)))
    }

    fn hardware_resize(
        &self,
        frame: &SafeFrame<'_>,
        dst_w: u32,
        dst_h: u32,
    ) -> Result<(CvBuffer, PreprocessMode), AlgoError> {
        let policy = RgaPolicy::new(self.config.core);
        let source = self.source_layout(frame, policy)?;
        policy.validate_scale_job(source.width, source.height, dst_w, dst_h)?;
        let dst_rect = im_rect(0, 0, dst_w, dst_h)?;
        let buffer = self.hardware_preprocess(frame, &source, dst_w, dst_h, dst_rect, None)?;
        Ok((buffer, PreprocessMode::Resize))
    }
}

impl CvEngine for RgaCvEngine {
    fn letterbox(
        &self,
        frame: &SafeFrame<'_>,
        dst_w: u32,
        dst_h: u32,
        fill_color: [u8; 3],
    ) -> Result<(CvBuffer, PreprocessMode), AlgoError> {
        frame.validate()?;
        if dst_w == 0 || dst_h == 0 {
            return Err(AlgoError::Preprocess {
                reason: "RGA destination dimensions must be non-zero".to_string(),
            });
        }
        if !self.can_use_hardware(frame) {
            return self.cpu.letterbox(frame, dst_w, dst_h, fill_color);
        }
        self.hardware_letterbox(frame, dst_w, dst_h, fill_color)
    }

    fn resize(
        &self,
        frame: &SafeFrame<'_>,
        dst_w: u32,
        dst_h: u32,
    ) -> Result<(CvBuffer, PreprocessMode), AlgoError> {
        frame.validate()?;
        if dst_w == 0 || dst_h == 0 {
            return Err(AlgoError::Preprocess {
                reason: "RGA destination dimensions must be non-zero".to_string(),
            });
        }
        if !self.can_use_hardware(frame) {
            return self.cpu.resize(frame, dst_w, dst_h);
        }
        self.hardware_resize(frame, dst_w, dst_h)
    }
}

#[derive(Debug, Clone, Copy)]
struct SourceLayout {
    rga_format: u32,
    width: u32,
    height: u32,
    w_stride: u32,
    h_stride: u32,
    size: usize,
    color_space_mode: i32,
}

fn dma_fd(frame: &SafeFrame<'_>) -> Result<i32, AlgoError> {
    match frame.handle_view() {
        FrameHandleView::DmaBuf { fd } if fd >= 0 => Ok(fd),
        FrameHandleView::DmaBuf { .. } => Err(AlgoError::IncompatibleFrame {
            reason: "RGA source DMA-BUF fd is negative".to_string(),
        }),
        _ => Err(AlgoError::IncompatibleFrame {
            reason: "RGA source is not a DMA-BUF".to_string(),
        }),
    }
}

fn validate_rga_offsets(
    frame: &SafeFrame<'_>,
    format: PixelFormat,
    stride0: u32,
    stride1: u32,
    alloc_height: u32,
) -> Result<(), AlgoError> {
    let offsets = [
        frame.plane_offset(0),
        frame.plane_offset(1),
        frame.plane_offset(2),
        frame.plane_offset(3),
    ];
    if offsets[0] != 0 {
        return Err(AlgoError::IncompatibleFrame {
            reason: "RGA handle path cannot represent a non-zero base plane offset".to_string(),
        });
    }

    let expected_plane_offset =
        |stride: u32, height: u32| u64::from(stride).checked_mul(u64::from(height));
    let expected1 = expected_plane_offset(stride0, alloc_height).ok_or(AlgoError::OutOfMemory)?;
    match format {
        PixelFormat::Nv12 => {
            if offsets[1] != 0 && offsets[1] != expected1 {
                return Err(AlgoError::IncompatibleFrame {
                    reason: "RGA NV12 wrapper cannot represent the declared UV plane offset"
                        .to_string(),
                });
            }
            if offsets[2] != 0 || offsets[3] != 0 {
                return Err(AlgoError::IncompatibleFrame {
                    reason: "RGA NV12 wrapper received an unexpected extra plane offset"
                        .to_string(),
                });
            }
        }
        PixelFormat::I420 => {
            let chroma_height = alloc_height.div_ceil(2);
            let expected2 = expected1
                .checked_add(
                    expected_plane_offset(stride1, chroma_height).ok_or(AlgoError::OutOfMemory)?,
                )
                .ok_or(AlgoError::OutOfMemory)?;
            if (offsets[1] != 0 && offsets[1] != expected1)
                || (offsets[2] != 0 && offsets[2] != expected2)
                || offsets[3] != 0
            {
                return Err(AlgoError::IncompatibleFrame {
                    reason: "RGA I420 wrapper cannot represent the declared plane offsets"
                        .to_string(),
                });
            }
        }
        PixelFormat::Rgb24 | PixelFormat::Bgra => {
            if offsets[1] != 0 || offsets[2] != 0 || offsets[3] != 0 {
                return Err(AlgoError::IncompatibleFrame {
                    reason: "RGA packed wrapper received an unexpected extra plane offset"
                        .to_string(),
                });
            }
        }
        PixelFormat::Unknown(_) => {
            return Err(AlgoError::IncompatibleFrame {
                reason: "RGA wrapper received an unknown pixel format".to_string(),
            });
        }
    }
    Ok(())
}
fn positive_stride(stride: i32, fallback: u32) -> Result<u32, AlgoError> {
    if stride > 0 {
        u32::try_from(stride).map_err(|_| AlgoError::Preprocess {
            reason: "frame stride exceeds RGA range".to_string(),
        })
    } else {
        Ok(fallback)
    }
}

fn im_rect(x: u32, y: u32, width: u32, height: u32) -> Result<ImRect, AlgoError> {
    Ok(ImRect {
        x: i32::try_from(x).map_err(|_| AlgoError::Preprocess {
            reason: "RGA coordinate exceeds C ABI range".to_string(),
        })?,
        y: i32::try_from(y).map_err(|_| AlgoError::Preprocess {
            reason: "RGA coordinate exceeds C ABI range".to_string(),
        })?,
        width: i32::try_from(width).map_err(|_| AlgoError::Preprocess {
            reason: "RGA dimension exceeds C ABI range".to_string(),
        })?,
        height: i32::try_from(height).map_err(|_| AlgoError::Preprocess {
            reason: "RGA dimension exceeds C ABI range".to_string(),
        })?,
    })
}

fn checked_plane_len(stride: u32, height: u32) -> Result<usize, AlgoError> {
    let stride = usize::try_from(stride).map_err(|_| AlgoError::OutOfMemory)?;
    let height = usize::try_from(height).map_err(|_| AlgoError::OutOfMemory)?;
    stride.checked_mul(height).ok_or(AlgoError::OutOfMemory)
}

fn plane_start(offset: u64, default_start: usize) -> Result<usize, AlgoError> {
    if offset == 0 {
        Ok(default_start)
    } else {
        usize::try_from(offset).map_err(|_| AlgoError::OutOfMemory)
    }
}

fn source_color_space_mode(frame: &SafeFrame<'_>, format: PixelFormat) -> i32 {
    if !matches!(format, PixelFormat::Nv12 | PixelFormat::I420) {
        return rgb_color_space_mode();
    }
    match frame.raw_desc().color_space {
        crate::c_abi::AV_COLOR_SPACE_BT709_FULL => IM_YUV_BT709_FULL_RANGE,
        crate::c_abi::AV_COLOR_SPACE_BT709_LIMITED => IM_YUV_BT709_LIMIT_RANGE,
        crate::c_abi::AV_COLOR_SPACE_BT601_FULL => IM_YUV_BT601_FULL_RANGE,
        _ => IM_YUV_BT601_LIMIT_RANGE,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::c_abi::{AvFrameDesc, AV_OPAQUE_NONE, AV_PIX_RGB24};
    use crate::cv::platforms::rockchip::config::RgaCore;

    #[test]
    fn source_color_space_follows_frame_metadata() {
        let data = vec![0u8; 640 * 480 * 3];
        let mut desc = AvFrameDesc::default_nv12(640, 480, 1920, 0, 0);
        desc.pixel_format = AV_PIX_RGB24;
        desc.opaque = data.as_ptr() as *mut std::ffi::c_void;
        desc.opaque_kind = AV_OPAQUE_NONE;
        let frame = SafeFrame::from_ref(&desc).expect("valid frame");
        assert_eq!(
            source_color_space_mode(&frame, PixelFormat::Rgb24),
            rgb_color_space_mode()
        );
    }

    #[test]
    fn source_layout_rejects_split_nv12_strides() {
        let data = vec![0u8; 640 * 480 + 672 * 240];
        let mut desc = AvFrameDesc::default_nv12(640, 480, 640, 672, 0);
        desc.opaque = data.as_ptr() as *mut std::ffi::c_void;
        desc.opaque_kind = AV_OPAQUE_NONE;
        let frame = SafeFrame::from_ref(&desc).expect("valid frame");
        let engine = RgaCvEngine::new_with_config(RgaPoolConfig::default()).expect("config");
        assert!(matches!(
            engine.source_layout(&frame, RgaPolicy::new(RgaCore::Rga2)),
            Err(AlgoError::IncompatibleFrame { .. })
        ));
    }

    #[test]
    fn source_layout_rejects_independent_i420_strides() {
        let data = vec![0u8; 640 * 480 + 320 * 240 + 336 * 240];
        let mut desc = AvFrameDesc::default_nv12(640, 480, 640, 320, 0);
        desc.pixel_format = crate::c_abi::AV_PIX_I420;
        desc.stride = [640, 320, 336, 0];
        desc.opaque = data.as_ptr() as *mut std::ffi::c_void;
        desc.opaque_kind = AV_OPAQUE_NONE;
        let frame = SafeFrame::from_ref(&desc).expect("valid frame");
        let engine = RgaCvEngine::new_with_config(RgaPoolConfig::default()).expect("config");
        assert!(matches!(
            engine.source_layout(&frame, RgaPolicy::new(RgaCore::Rga2)),
            Err(AlgoError::IncompatibleFrame { .. })
        ));
    }

    #[test]
    fn plane_start_uses_contiguous_default_for_zero_offset() {
        assert_eq!(plane_start(0, 640), Ok(640));
        assert_eq!(plane_start(1024, 640), Ok(1024));
    }

    #[test]
    fn pool_for_enforces_maximum_cached_pools() {
        struct DummyFactory;
        impl super::super::pool::SlotFactory for DummyFactory {
            fn create(&self) -> Result<super::super::pool::SlotData, AlgoError> {
                let file = std::fs::File::open("/dev/null").map_err(|_| AlgoError::OutOfMemory)?;
                use std::os::fd::{FromRawFd, IntoRawFd};
                // SAFETY: into_raw_fd transfers ownership of this freshly opened /dev/null descriptor to OwnedFd.
                let fd = unsafe { std::os::fd::OwnedFd::from_raw_fd(file.into_raw_fd()) };
                Ok(super::super::pool::SlotData { fd, handle: 1 })
            }
            fn release(&self, _handle: u32) {}
        }

        let engine = RgaCvEngine::new_with_config(RgaPoolConfig::default()).expect("config");
        let dummy_pool = Arc::new(
            RgaBufferPool::with_factory(
                RgaPoolConfig {
                    min_idle: 0,
                    max_size: 1,
                    ..RgaPoolConfig::default()
                },
                Arc::new(DummyFactory),
            )
            .expect("dummy pool"),
        );

        {
            let mut pools = engine.pools.lock().expect("pools lock");
            for i in 1..=MAX_CACHED_POOLS {
                let spec = RgaBufferSpec {
                    width: i as u32 * 16,
                    height: i as u32 * 16,
                    w_stride: i as u32 * 16,
                    h_stride: i as u32 * 16,
                    format: PixelFormat::Rgb24,
                    size: i * 16 * i * 16 * 3,
                };
                pools.insert(spec, Arc::clone(&dummy_pool));
            }
        }

        let new_spec = RgaBufferSpec {
            width: 9999,
            height: 9999,
            w_stride: 9999,
            h_stride: 9999,
            format: PixelFormat::Rgb24,
            size: 9999 * 9999 * 3,
        };
        let result = engine.pool_for(new_spec);
        assert!(matches!(
            result,
            Err(AlgoError::Preprocess { ref reason }) if reason.contains("exceeded maximum cached RGA buffer pools")
        ));
    }
}
