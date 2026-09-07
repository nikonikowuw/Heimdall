//! 安全帧视图 (SafeFrame)
//! 从 `AvFrameDesc` 构造强类型只读借用视图，生命周期绑定到 C 回调栈帧。

use std::ffi::c_void;
use std::slice;

use crate::c_abi::*;
use crate::error::AlgoError;

/// YUV -> RGB 转换参数，由帧的矩阵与范围元数据决定。
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
#[derive(Debug, Clone, Copy)]
pub(crate) struct YuvConversion {
    pub y_offset: f32,
    pub y_scale: f32,
    pub r_cr: f32,
    pub g_cb: f32,
    pub g_cr: f32,
    pub b_cb: f32,
    /// vImage 使用的未按像素 range 预缩放的矩阵系数。
    pub matrix_r_cr: f32,
    pub matrix_g_cb: f32,
    pub matrix_g_cr: f32,
    pub matrix_b_cb: f32,
    pub full_range: bool,
}

/// 硬件句柄解包视图（匹配 opaque_kind 与 memory_type）
#[derive(Debug, Clone)]
pub enum FrameHandleView<'a> {
    /// Linux DRM DMA-BUF 文件描述符 (Rockchip MPP / RGA / RKNN)
    DmaBuf { fd: i32 },
    /// Apple CoreVideo CVPixelBufferRef 原生指针
    ApplePixelBuffer { ptr: *mut c_void },
    /// 华为昇腾 DVPP 原生设备显存指针
    AscendDeviceMemory { ptr: *mut c_void },
    /// Host 主机内存切片
    Host { data: &'a [u8] },
}

/// 从 AvFrameDesc 构造的强类型安全只读视图。
/// 生命周期 `'a` 严格绑定到 C 回调栈帧，在编译期防止帧数据逃逸。
#[derive(Debug, Copy, Clone)]
pub struct SafeFrame<'a> {
    desc: &'a AvFrameDesc,
}

impl<'a> SafeFrame<'a> {
    /// 从原始帧指针构造安全视图；校验失败时返回 `None`。
    ///
    /// # Safety
    /// `raw` 必须至少指向可读取 ABI 头部的内存，且指针满足 `AvFrameDesc` 对齐；
    /// 当头部通过校验后，完整结构体必须在本次调用期间可读。
    #[inline]
    pub unsafe fn from_raw(raw: *const AvFrameDesc) -> Option<Self> {
        // SAFETY: 保持旧 API 的 Option 语义；checked 版本负责完整 ABI 校验。
        unsafe { Self::from_raw_checked(raw).ok() }
    }

    /// 从 C ABI 裸指针构造并校验版本化帧描述符。
    ///
    /// # Safety
    /// `raw` 必须至少指向可读取 ABI 头部的内存，且指针满足 `AvFrameDesc` 对齐；
    /// 当头部通过校验后，完整结构体必须在本次调用期间可读。
    pub unsafe fn from_raw_checked(raw: *const AvFrameDesc) -> Result<Self, AlgoError> {
        if raw.is_null() {
            return Err(AlgoError::Preprocess {
                reason: "帧描述符为空".to_string(),
            });
        }
        if !(raw as usize).is_multiple_of(std::mem::align_of::<AvFrameDesc>()) {
            return Err(AlgoError::Preprocess {
                reason: "帧描述符未按 ABI 对齐".to_string(),
            });
        }

        // SAFETY: 调用方保证 ABI 头部两个 u32 在 raw 指针处可读。
        let header = raw.cast::<u32>();
        // SAFETY: 调用方保证 ABI 头部两个 u32 在 raw 指针处可读；raw 已完成非空与对齐校验。
        let size = unsafe { header.read() } as usize;
        // SAFETY: api_version 与 size 同属已声明的 ABI 头部。
        let api_version = unsafe { header.add(1).read() };
        let expected_size = std::mem::size_of::<AvFrameDesc>();
        if size < expected_size {
            return Err(AlgoError::Preprocess {
                reason: format!("帧描述符大小不足: {size} < {expected_size}"),
            });
        }
        if api_version != AV_ALGO_API_VERSION {
            return Err(AlgoError::UnsupportedApi);
        }

        // SAFETY: size 已覆盖当前结构体，且 raw 已通过对齐检查。
        let desc = unsafe { &*raw };
        validate_desc(desc)?;
        Ok(Self { desc })
    }

    /// 从 `&'a AvFrameDesc` 引用构造，并立即校验 ABI 与帧布局。
    #[inline]
    pub fn from_ref(desc: &'a AvFrameDesc) -> Result<Self, AlgoError> {
        validate_desc(desc)?;
        Ok(Self { desc })
    }

    /// 从已由调用方证明有效的引用构造视图。
    ///
    /// # Safety
    /// `desc` 必须满足 `AvFrameDesc` 的完整 ABI、像素格式、stride、句柄和内存契约。
    #[inline]
    pub unsafe fn from_ref_unchecked(desc: &'a AvFrameDesc) -> Self {
        Self { desc }
    }

    /// 获取原始 AvFrameDesc 引用。
    #[inline]
    pub fn raw_desc(&self) -> &'a AvFrameDesc {
        self.desc
    }

    /// 校验当前帧描述符的字段与句柄契约。
    pub(crate) fn validate(&self) -> Result<(), AlgoError> {
        validate_desc(self.desc)
    }

    #[inline]
    pub fn width(&self) -> u32 {
        self.desc.width
    }

    #[inline]
    pub fn height(&self) -> u32 {
        self.desc.height
    }

    #[inline]
    pub fn alloc_width(&self) -> u32 {
        self.desc.alloc_width
    }

    #[inline]
    pub fn alloc_height(&self) -> u32 {
        self.desc.alloc_height
    }

    #[inline]
    pub fn pixel_format(&self) -> u32 {
        self.desc.pixel_format
    }

    #[inline]
    pub fn memory_type(&self) -> u32 {
        self.desc.memory_type
    }

    #[inline]
    pub fn layout(&self) -> u32 {
        self.desc.layout
    }

    #[inline]
    pub fn stride(&self, plane: usize) -> i32 {
        if plane < 4 {
            self.desc.stride[plane]
        } else {
            0
        }
    }

    /// 获取指定平面的字节偏移；零表示使用该格式的默认连续布局。
    #[inline]
    pub fn plane_offset(&self, plane: usize) -> u64 {
        self.desc.offset.get(plane).copied().unwrap_or(0)
    }

    /// 获取 NV12/I420 的色彩转换参数。
    pub(crate) fn yuv_conversion(&self) -> YuvConversion {
        let bt709 = self.desc.color_matrix == 1;
        let full_range = self.desc.color_range == 2;
        let (y_offset, y_scale, r_cr, g_cb, g_cr, b_cb) = match (bt709, full_range) {
            (true, true) => (0.0, 1.0, 1.5748, -0.1873, -0.4681, 1.8556),
            (true, false) => (16.0, 1.164, 1.793, -0.213, -0.533, 2.112),
            (false, true) => (0.0, 1.0, 1.402, -0.3441, -0.7141, 1.772),
            (false, false) => (16.0, 1.164, 1.596, -0.392, -0.813, 2.017),
        };
        let (matrix_r_cr, matrix_g_cb, matrix_g_cr, matrix_b_cb) = if bt709 {
            (1.5748, -0.1873, -0.4681, 1.8556)
        } else {
            (1.402, -0.3441, -0.7141, 1.772)
        };

        YuvConversion {
            y_offset,
            y_scale,
            r_cr,
            g_cb,
            g_cr,
            b_cb,
            matrix_r_cr,
            matrix_g_cb,
            matrix_g_cr,
            matrix_b_cb,
            full_range,
        }
    }

    #[inline]
    pub fn wall_time_ns(&self) -> i64 {
        self.desc.wall_time_ns
    }

    #[inline]
    pub fn pts_ns(&self) -> i64 {
        self.desc.pts_ns
    }

    #[inline]
    pub fn frame_id(&self) -> u64 {
        self.desc.frame_id
    }

    /// 解包底层硬件句柄视图
    pub fn handle_view(&self) -> FrameHandleView<'a> {
        match self.desc.opaque_kind {
            AV_OPAQUE_DMABUF => {
                let fd = self.desc.opaque as usize as i32;
                FrameHandleView::DmaBuf { fd }
            }
            AV_OPAQUE_CVPIXELBUFFER => FrameHandleView::ApplePixelBuffer {
                ptr: self.desc.opaque,
            },
            AV_OPAQUE_ASCEND_DEVICE_MEMORY => FrameHandleView::AscendDeviceMemory {
                ptr: self.desc.opaque,
            },
            AV_OPAQUE_NONE => {
                if self.desc.opaque.is_null() {
                    FrameHandleView::Host { data: &[] }
                } else {
                    let len = self.estimate_host_bytes().unwrap_or(0);
                    if len == 0 || len > isize::MAX as usize {
                        return FrameHandleView::Host { data: &[] };
                    }
                    // SAFETY: C ABI 帧契约要求 opaque 指向覆盖 estimate_host_bytes 的 Host
                    // allocation；长度计算使用 checked arithmetic，生命周期绑定到 desc。
                    let data = unsafe { slice::from_raw_parts(self.desc.opaque as *const u8, len) };
                    FrameHandleView::Host { data }
                }
            }
            _ => FrameHandleView::Host { data: &[] },
        }
    }

    /// 根据格式、平面偏移与步长估算 Host 缓冲区的有效字节数。
    fn estimate_host_bytes(&self) -> Option<usize> {
        plane_layout(self.desc).map(|(ranges, count)| {
            ranges[..count]
                .iter()
                .map(|(_, end)| *end)
                .max()
                .unwrap_or(0)
        })
    }
}

fn plane_layout(desc: &AvFrameDesc) -> Option<([(usize, usize); 3], usize)> {
    let width = usize::try_from(desc.width).ok()?;
    let height = if desc.alloc_height > 0 {
        usize::try_from(desc.alloc_height).ok()?
    } else {
        usize::try_from(desc.height).ok()?
    };
    let allocation_width = if desc.alloc_width > 0 {
        usize::try_from(desc.alloc_width).ok()?
    } else {
        width
    };
    let offset0 = usize::try_from(desc.offset[0]).ok()?;
    let default_stride0 = match desc.pixel_format {
        AV_PIX_NV12 | AV_PIX_I420 => allocation_width,
        AV_PIX_RGB24 => allocation_width.checked_mul(3)?,
        AV_PIX_BGRA => allocation_width.checked_mul(4)?,
        _ => return None,
    };
    let stride0 = if desc.stride[0] > 0 {
        usize::try_from(desc.stride[0]).ok()?
    } else {
        default_stride0
    };
    let y_len = stride0.checked_mul(height)?;
    let y_end = offset0.checked_add(y_len)?;
    let mut ranges = [(offset0, y_end), (0, 0), (0, 0)];

    let count = match desc.pixel_format {
        AV_PIX_NV12 => {
            let chroma_width = allocation_width.div_ceil(2).checked_mul(2)?;
            let stride1 = if desc.stride[1] > 0 {
                usize::try_from(desc.stride[1]).ok()?
            } else {
                stride0.max(chroma_width)
            };
            let uv_len = stride1.checked_mul(height.div_ceil(2))?;
            let uv_start = if desc.offset[1] > 0 {
                usize::try_from(desc.offset[1]).ok()?
            } else {
                y_end
            };
            ranges[1] = (uv_start, uv_start.checked_add(uv_len)?);
            2
        }
        AV_PIX_I420 => {
            let chroma_width = allocation_width.div_ceil(2);
            let stride1 = if desc.stride[1] > 0 {
                usize::try_from(desc.stride[1]).ok()?
            } else {
                chroma_width
            };
            let stride2 = if desc.stride[2] > 0 {
                usize::try_from(desc.stride[2]).ok()?
            } else {
                stride1
            };
            let chroma_rows = height.div_ceil(2);
            let u_len = stride1.checked_mul(chroma_rows)?;
            let v_len = stride2.checked_mul(chroma_rows)?;
            let u_start = if desc.offset[1] > 0 {
                usize::try_from(desc.offset[1]).ok()?
            } else {
                y_end
            };
            let u_end = u_start.checked_add(u_len)?;
            let v_start = if desc.offset[2] > 0 {
                usize::try_from(desc.offset[2]).ok()?
            } else {
                u_end
            };
            ranges[1] = (u_start, u_end);
            ranges[2] = (v_start, v_start.checked_add(v_len)?);
            3
        }
        AV_PIX_RGB24 | AV_PIX_BGRA => 1,
        _ => return None,
    };

    if ranges[..count]
        .iter()
        .any(|(start, end)| *end < *start || *end > isize::MAX as usize)
    {
        return None;
    }
    for left in 0..count {
        for right in (left + 1)..count {
            let (left_start, left_end) = ranges[left];
            let (right_start, right_end) = ranges[right];
            if left_start < right_end && right_start < left_end {
                return None;
            }
        }
    }
    Some((ranges, count))
}

fn validate_desc(desc: &AvFrameDesc) -> Result<(), AlgoError> {
    let expected_size = std::mem::size_of::<AvFrameDesc>() as u32;
    if desc.size < expected_size {
        return Err(AlgoError::Preprocess {
            reason: format!("帧描述符大小不足: {} < {expected_size}", desc.size),
        });
    }
    if desc.api_version != AV_ALGO_API_VERSION {
        return Err(AlgoError::UnsupportedApi);
    }
    if desc.width == 0 || desc.height == 0 {
        return Err(AlgoError::Preprocess {
            reason: "帧宽高不能为 0".to_string(),
        });
    }
    if (desc.alloc_width != 0 && desc.alloc_width < desc.width)
        || (desc.alloc_height != 0 && desc.alloc_height < desc.height)
    {
        return Err(AlgoError::Preprocess {
            reason: "帧分配尺寸小于有效尺寸".to_string(),
        });
    }
    if desc.memory_type != AV_MEM_HOST && desc.memory_type != AV_MEM_PLATFORM_SURFACE {
        return Err(AlgoError::Preprocess {
            reason: "帧 memory_type 未知".to_string(),
        });
    }
    let required_planes = match desc.pixel_format {
        AV_PIX_NV12 => 2,
        AV_PIX_I420 => 3,
        AV_PIX_RGB24 | AV_PIX_BGRA => 1,
        _ => {
            return Err(AlgoError::IncompatibleFrame {
                reason: format!("帧 pixel_format 未知: {}", desc.pixel_format),
            });
        }
    };
    if desc.plane_count < required_planes {
        return Err(AlgoError::Preprocess {
            reason: "帧 plane_count 小于像素格式要求".to_string(),
        });
    }
    if desc.stride.iter().any(|stride| *stride < 0) {
        return Err(AlgoError::Preprocess {
            reason: "帧 stride 不能为负数".to_string(),
        });
    }
    let width = usize::try_from(desc.width).map_err(|_| AlgoError::Preprocess {
        reason: "帧宽度无法转换为平台地址空间尺寸".to_string(),
    })?;
    let allocation_width = if desc.alloc_width > 0 {
        usize::try_from(desc.alloc_width).map_err(|_| AlgoError::Preprocess {
            reason: "帧分配宽度无法转换为平台地址空间尺寸".to_string(),
        })?
    } else {
        width
    };
    let chroma_width = allocation_width.div_ceil(2);
    let minimum_strides = match desc.pixel_format {
        AV_PIX_NV12 => [
            allocation_width,
            chroma_width.checked_mul(2).ok_or(AlgoError::OutOfMemory)?,
            0,
            0,
        ],
        AV_PIX_I420 => [allocation_width, chroma_width, chroma_width, 0],
        AV_PIX_RGB24 => [
            allocation_width
                .checked_mul(3)
                .ok_or(AlgoError::OutOfMemory)?,
            0,
            0,
            0,
        ],
        AV_PIX_BGRA => [
            allocation_width
                .checked_mul(4)
                .ok_or(AlgoError::OutOfMemory)?,
            0,
            0,
            0,
        ],
        _ => unreachable!("pixel_format 已在上方校验"),
    };
    for (stride, minimum) in desc.stride.iter().zip(minimum_strides) {
        if *stride > 0 && (*stride as usize) < minimum {
            return Err(AlgoError::Preprocess {
                reason: "帧 stride 小于分配尺寸要求".to_string(),
            });
        }
    }
    if desc.offset.iter().any(|offset| *offset > isize::MAX as u64) || plane_layout(desc).is_none()
    {
        return Err(AlgoError::Preprocess {
            reason: "帧 plane offset、stride 或平面布局无效".to_string(),
        });
    }
    match desc.opaque_kind {
        AV_OPAQUE_NONE => {
            if desc.memory_type != AV_MEM_HOST {
                return Err(AlgoError::Preprocess {
                    reason: "Host 帧必须使用 AV_MEM_HOST + AV_OPAQUE_NONE".to_string(),
                });
            }
            if desc.opaque.is_null() {
                return Err(AlgoError::Preprocess {
                    reason: "Host 帧 data 指针为空".to_string(),
                });
            }
        }
        AV_OPAQUE_DMABUF => {
            if desc.memory_type != AV_MEM_PLATFORM_SURFACE {
                return Err(AlgoError::Preprocess {
                    reason: "DMA-BUF 帧必须使用平台显存类型".to_string(),
                });
            }
            if desc.opaque.is_null() || (desc.opaque as usize) > i32::MAX as usize {
                return Err(AlgoError::Preprocess {
                    reason: "DMA-BUF fd 无效".to_string(),
                });
            }
        }
        AV_OPAQUE_CVPIXELBUFFER | AV_OPAQUE_ASCEND_DEVICE_MEMORY => {
            if desc.memory_type != AV_MEM_PLATFORM_SURFACE || desc.opaque.is_null() {
                return Err(AlgoError::Preprocess {
                    reason: "平台原生帧句柄或 memory_type 无效".to_string(),
                });
            }
        }
        _ => {
            return Err(AlgoError::Preprocess {
                reason: "帧 opaque_kind 未知".to_string(),
            });
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_safe_frame_dmabuf() {
        let mut desc = AvFrameDesc::default_nv12(1920, 1080, 1920, 1920, 12345);
        desc.opaque = 42_usize as *mut c_void;
        desc.opaque_kind = AV_OPAQUE_DMABUF;
        desc.memory_type = AV_MEM_PLATFORM_SURFACE;

        let frame = SafeFrame::from_ref(&desc).expect("DMA-BUF 帧描述符有效");
        assert_eq!(frame.height(), 1080);
        assert_eq!(frame.stride(0), 1920);
        assert_eq!(frame.stride(1), 1920);
        assert_eq!(frame.stride(4), 0);

        match frame.handle_view() {
            FrameHandleView::DmaBuf { fd } => assert_eq!(fd, 42),
            _ => panic!("预期 DmaBuf 句柄"),
        }
    }

    #[test]
    fn test_safe_frame_host() {
        let dummy = vec![128u8; 1920 * 1080 * 3 / 2];
        let mut desc = AvFrameDesc::default_nv12(1920, 1080, 1920, 1920, 54321);
        desc.opaque = dummy.as_ptr() as *mut c_void;
        desc.opaque_kind = AV_OPAQUE_NONE;

        let frame = SafeFrame::from_ref(&desc).expect("Host 帧描述符有效");
        match frame.handle_view() {
            FrameHandleView::Host { data } => {
                assert_eq!(data.len(), 1920 * 1080 * 3 / 2);
                assert_eq!(data[0], 128);
            }
            _ => panic!("预期 Host 句柄"),
        }
    }

    #[test]
    fn test_safe_frame_from_raw_null() {
        // SAFETY: 传递空指针，验证安全性返回 None。
        let frame = unsafe { SafeFrame::from_raw(std::ptr::null()) };
        assert!(frame.is_none());
    }

    #[test]
    fn test_default_nv12_layout_uses_alloc_height() {
        let width = 4u32;
        let height = 3u32;
        let alloc_height = 6u32;
        let y_stride = 4i32;
        let uv_stride = 4i32;
        let data = vec![128u8; y_stride as usize * alloc_height as usize + uv_stride as usize * 3];
        let mut desc = AvFrameDesc::default_nv12(width, height, y_stride, uv_stride, 0);
        desc.alloc_height = alloc_height;
        desc.offset = [0; 4];
        desc.opaque = data.as_ptr() as *mut c_void;

        let frame = SafeFrame::from_ref(&desc).expect("带 allocation height 的 NV12 应有效");
        match frame.handle_view() {
            FrameHandleView::Host { data: view } => {
                assert_eq!(view.len(), data.len());
            }
            _ => panic!("预期 Host 句柄"),
        }
    }

    #[test]
    fn test_explicit_overlapping_plane_offsets_are_rejected() {
        let data = [0u8; 64];
        let mut desc = AvFrameDesc::default_nv12(4, 4, 4, 4, 0);
        desc.offset = [0, 8, 0, 0];
        desc.opaque = data.as_ptr() as *mut c_void;

        let error = SafeFrame::from_ref(&desc).expect_err("重叠的 NV12 planes 必须拒绝");
        assert!(error.to_string().contains("plane"));
    }

    #[test]
    fn test_safe_frame_rejects_invalid_descriptor_fields() {
        let data = [0u8; 16];
        let mut desc = AvFrameDesc::default_nv12(4, 4, 4, 4, 0);
        desc.opaque = data.as_ptr() as *mut c_void;
        desc.stride[0] = 3;
        let error = SafeFrame::from_ref(&desc).expect_err("小于分配宽度的 stride 必须拒绝");
        assert!(error.to_string().contains("stride"));
    }

    #[test]
    fn test_safe_frame_from_raw_rejects_short_abi_size() {
        let mut desc = AvFrameDesc::default_nv12(4, 4, 4, 4, 0);
        desc.size = 8;
        // SAFETY: desc 是对齐且完整可读的结构体，测试只修改 ABI size 字段。
        let error = unsafe { SafeFrame::from_raw_checked(&desc) }.expect_err("短 ABI 必须拒绝");
        assert!(error.to_string().contains("大小不足"));
    }
}
