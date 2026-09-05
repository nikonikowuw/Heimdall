//! 宿主硬件加速图像操作表代理引擎 (HostCvEngine)

use crate::c_abi::*;
use crate::cv::buffer::CvBuffer;
use crate::cv::engine::CvEngine;
use crate::cv::layout::compute_letterbox_layout;
use crate::cv::types::{PixelFormat, PreprocessMode};
use crate::error::AlgoError;
use crate::frame::SafeFrame;

/// 宿主 `AvImageOps` 代理驱动。
///
/// 操作表本身按值复制，避免保存调用方栈上的表指针；其中的 `ctx` 仍必须由宿主
/// 保证在该引擎实例存活期间有效。
#[derive(Debug, Clone, Copy)]
pub struct HostCvEngine {
    ops: AvImageOps,
}

// SAFETY: 操作表已经复制到引擎中；宿主 ABI 契约要求 ctx 与四个回调可跨线程安全调用。
unsafe impl Send for HostCvEngine {}
// SAFETY: 同上，CvEngine 要求共享引用可并发使用；同一个插件实例的 plugin 调用仍会串行化。
unsafe impl Sync for HostCvEngine {}

impl HostCvEngine {
    /// 从宿主注入的 `AvImageOps` 裸指针构造实例级代理。
    ///
    /// # Safety
    /// `ops` 为空时返回 `Ok(None)`；非空时必须至少指向两个有效的 `u32` 头字段，
    /// 并在读取完整结构体期间保持正确对齐和有效。
    pub unsafe fn from_raw(ops: *const AvImageOps) -> Result<Option<Self>, AlgoError> {
        if ops.is_null() {
            return Ok(None);
        }
        if !(ops as usize).is_multiple_of(std::mem::align_of::<AvImageOps>()) {
            return Err(AlgoError::Internal {
                reason: "AvImageOps 指针未按 ABI 对齐".to_string(),
            });
        }

        // SAFETY: 调用方保证前两个 ABI 头字段可读；完整结构体只在 size 校验后读取。
        let header = ops.cast::<u32>();
        // SAFETY: 上方已确认 ops 非空、对齐，且调用方保证 ABI 头部可读。
        let size = unsafe { header.read() } as usize;
        // SAFETY: 同一 ABI 头部的 api_version 字段可读。
        let api_version = unsafe { header.add(1).read() };
        let expected_size = std::mem::size_of::<AvImageOps>();
        if size < expected_size {
            return Err(AlgoError::Internal {
                reason: format!("AvImageOps 大小不足: {size} < {expected_size}"),
            });
        }
        if api_version != AV_ALGO_API_VERSION {
            return Err(AlgoError::UnsupportedApi);
        }

        // SAFETY: size 已证明覆盖当前结构体，且 ops 已通过对齐检查。
        let copied = unsafe { ops.read() };
        if copied.alloc.is_none() || copied.convert.is_none() || copied.free.is_none() {
            return Err(AlgoError::NotImplemented);
        }

        Ok(Some(Self { ops: copied }))
    }

    fn allocate_view(&self, width: u32, height: u32) -> Result<AvImageView, AlgoError> {
        if width == 0 || height == 0 {
            return Err(AlgoError::Preprocess {
                reason: "目标尺寸不能为 0".to_string(),
            });
        }
        let ops = &self.ops;
        let alloc_fn = ops.alloc.ok_or(AlgoError::NotImplemented)?;
        let free_fn = ops.free.ok_or(AlgoError::NotImplemented)?;
        let mut view = AvImageView {
            size: std::mem::size_of::<AvImageView>() as u32,
            api_version: AV_ALGO_API_VERSION,
            width,
            height,
            pixel_format: AV_PIX_RGB24,
            memory_type: AV_MEM_HOST,
            plane_count: 1,
            opaque_kind: AV_OPAQUE_NONE,
            stride: [0; 4],
            offset: [0; 4],
            data: std::ptr::null_mut(),
            opaque: std::ptr::null_mut(),
        };

        // SAFETY: view 是完整可写 ABI 结构体，回调由构造时校验过的宿主操作表提供。
        let status = unsafe { alloc_fn(ops.ctx, width, height, AV_PIX_RGB24, &mut view) };
        if status != AV_OK {
            AlgoError::from_c_status(status)?;
        }

        if let Err(error) = validate_view(&view, width, height) {
            // SAFETY: alloc 已成功，free_fn 与同一操作表匹配。
            unsafe {
                free_fn(ops.ctx, &mut view);
            }
            return Err(error);
        }

        Ok(view)
    }

    fn content_view(
        &self,
        view: &AvImageView,
        layout: &crate::cv::types::LetterboxLayout,
    ) -> Result<AvImageView, AlgoError> {
        let stride = usize::try_from(view.stride[0]).map_err(|_| AlgoError::Preprocess {
            reason: "Host 输出 stride 无效".to_string(),
        })?;
        let x_bytes =
            (layout.pad_left as usize)
                .checked_mul(3)
                .ok_or_else(|| AlgoError::Preprocess {
                    reason: "Letterbox 横向偏移溢出".to_string(),
                })?;
        let y_bytes = (layout.pad_top as usize)
            .checked_mul(stride)
            .ok_or_else(|| AlgoError::Preprocess {
                reason: "Letterbox 纵向偏移溢出".to_string(),
            })?;
        let region_offset = y_bytes
            .checked_add(x_bytes)
            .ok_or_else(|| AlgoError::Preprocess {
                reason: "Letterbox 输出偏移溢出".to_string(),
            })?;
        let content_row =
            (layout.scaled_w as usize)
                .checked_mul(3)
                .ok_or_else(|| AlgoError::Preprocess {
                    reason: "Letterbox 内容宽度溢出".to_string(),
                })?;
        let last_row = (layout.scaled_h as usize)
            .checked_sub(1)
            .and_then(|last| last.checked_mul(stride))
            .and_then(|last| last.checked_add(content_row))
            .ok_or_else(|| AlgoError::Preprocess {
                reason: "Letterbox 内容区域无效".to_string(),
            })?;
        let view_bytes =
            stride
                .checked_mul(view.height as usize)
                .ok_or_else(|| AlgoError::Preprocess {
                    reason: "Host 输出大小溢出".to_string(),
                })?;
        if region_offset > view_bytes
            || region_offset
                .checked_add(last_row)
                .is_none_or(|end| end > view_bytes)
        {
            return Err(AlgoError::Preprocess {
                reason: "Letterbox 内容区域超出 Host 输出".to_string(),
            });
        }

        let base_offset = usize::try_from(view.offset[0]).map_err(|_| AlgoError::Preprocess {
            reason: "Host 输出 plane offset 无效".to_string(),
        })?;
        let content_offset =
            base_offset
                .checked_add(region_offset)
                .ok_or_else(|| AlgoError::Preprocess {
                    reason: "Host 输出 plane offset 溢出".to_string(),
                })?;
        if content_offset > isize::MAX as usize
            || content_offset
                .checked_add(last_row)
                .is_none_or(|end| end > isize::MAX as usize)
        {
            return Err(AlgoError::Preprocess {
                reason: "Host 输出 plane offset 超出地址空间".to_string(),
            });
        }

        // AvImageView 的 data 是分配基址，offset[0] 是首平面的 byte offset。
        // 只调整 offset，避免宿主 convert 同时应用 data 与 offset 导致双重偏移。
        let mut content = *view;
        content.width = layout.scaled_w;
        content.height = layout.scaled_h;
        content.offset[0] = content_offset as u64;
        Ok(content)
    }

    fn free_view(&self, view: &mut AvImageView) {
        if let Some(free_fn) = self.ops.free {
            // SAFETY: view 由同一操作表的 alloc 产生且尚未转移到 CvBuffer。
            unsafe {
                free_fn(self.ops.ctx, view);
            }
        }
    }
}

fn validate_view(view: &AvImageView, width: u32, height: u32) -> Result<(), AlgoError> {
    let expected_size = std::mem::size_of::<AvImageView>() as u32;
    if view.size < expected_size || view.api_version != AV_ALGO_API_VERSION {
        return Err(AlgoError::Internal {
            reason: "宿主返回的 AvImageView ABI 头无效".to_string(),
        });
    }
    if width == 0 || height == 0 {
        return Err(AlgoError::Preprocess {
            reason: "宿主返回的图像视图尺寸不能为 0".to_string(),
        });
    }
    if view.width != width || view.height != height || view.pixel_format != AV_PIX_RGB24 {
        return Err(AlgoError::Preprocess {
            reason: "宿主返回的图像视图尺寸或格式不匹配".to_string(),
        });
    }
    if view.plane_count != 1 {
        return Err(AlgoError::Preprocess {
            reason: "宿主返回的 RGB24 图像视图平面数无效".to_string(),
        });
    }
    if view.stride.iter().any(|stride| *stride < 0) {
        return Err(AlgoError::Preprocess {
            reason: "宿主返回的 stride 无效".to_string(),
        });
    }
    let min_stride = (width as usize)
        .checked_mul(3)
        .ok_or_else(|| AlgoError::Preprocess {
            reason: "Host 输出 stride 溢出".to_string(),
        })?;
    let stride = usize::try_from(view.stride[0]).map_err(|_| AlgoError::Preprocess {
        reason: "宿主返回的 stride 无效".to_string(),
    })?;
    if stride < min_stride {
        return Err(AlgoError::Preprocess {
            reason: "宿主返回的 stride 小于 RGB24 最小行宽".to_string(),
        });
    }
    let offset = usize::try_from(view.offset[0]).map_err(|_| AlgoError::Preprocess {
        reason: "宿主返回的 plane offset 无效".to_string(),
    })?;
    let image_bytes = stride
        .checked_mul(height as usize)
        .ok_or_else(|| AlgoError::Preprocess {
            reason: "宿主返回的图像大小溢出".to_string(),
        })?;
    if offset
        .checked_add(image_bytes)
        .is_none_or(|end| end > isize::MAX as usize)
        || view.offset[1..]
            .iter()
            .any(|value| *value > isize::MAX as u64)
    {
        return Err(AlgoError::Preprocess {
            reason: "宿主返回的 plane offset 超出地址空间".to_string(),
        });
    }
    if view.data.is_null() && view.opaque.is_null() {
        return Err(AlgoError::Preprocess {
            reason: "宿主返回的图像视图没有有效句柄".to_string(),
        });
    }

    match view.opaque_kind {
        AV_OPAQUE_NONE => {
            if view.memory_type != AV_MEM_HOST || view.data.is_null() {
                return Err(AlgoError::Preprocess {
                    reason: "Host 图像视图必须包含 data 指针".to_string(),
                });
            }
        }
        AV_OPAQUE_DMABUF => {
            if view.memory_type != AV_MEM_PLATFORM_SURFACE
                || view.opaque.is_null()
                || (view.opaque as usize) > i32::MAX as usize
            {
                return Err(AlgoError::Preprocess {
                    reason: "宿主返回的 DMA-BUF 句柄无效".to_string(),
                });
            }
        }
        AV_OPAQUE_CVPIXELBUFFER | AV_OPAQUE_ASCEND_DEVICE_MEMORY => {
            if view.memory_type != AV_MEM_PLATFORM_SURFACE || view.opaque.is_null() {
                return Err(AlgoError::Preprocess {
                    reason: "宿主返回的平台图像句柄无效".to_string(),
                });
            }
        }
        _ => {
            return Err(AlgoError::Preprocess {
                reason: "宿主返回的 opaque_kind 未知".to_string(),
            });
        }
    }
    Ok(())
}

impl CvEngine for HostCvEngine {
    fn letterbox(
        &self,
        frame: &SafeFrame<'_>,
        dst_w: u32,
        dst_h: u32,
        fill_color: [u8; 3],
    ) -> Result<(CvBuffer, PreprocessMode), AlgoError> {
        frame.validate()?;
        let pad_fn = self.ops.pad.ok_or(AlgoError::NotImplemented)?;
        let convert_fn = self.ops.convert.ok_or(AlgoError::NotImplemented)?;
        let free_fn = self.ops.free.ok_or(AlgoError::NotImplemented)?;
        let mut view = self.allocate_view(dst_w, dst_h)?;
        let layout = compute_letterbox_layout(frame.width(), frame.height(), dst_w, dst_h);
        if layout.scaled_w == 0 || layout.scaled_h == 0 {
            self.free_view(&mut view);
            return Err(AlgoError::Preprocess {
                reason: "Letterbox 内容区域为空".to_string(),
            });
        }

        let bg_color = [fill_color[0], fill_color[1], fill_color[2], 255];
        // SAFETY: view 是 alloc 返回的完整目标视图，bg_color 在同步调用期间有效。
        let pad_status = unsafe { pad_fn(self.ops.ctx, &view, std::ptr::null(), &bg_color) };
        if pad_status != AV_OK {
            self.free_view(&mut view);
            return Err(match AlgoError::from_c_status(pad_status) {
                Ok(()) => AlgoError::Preprocess {
                    reason: format!("宿主 pad 失败: {pad_status}"),
                },
                Err(error) => error,
            });
        }

        let content = match self.content_view(&view, &layout) {
            Ok(content) => content,
            Err(error) => {
                self.free_view(&mut view);
                return Err(error);
            }
        };
        // SAFETY: src_desc 在 process 回调内有效，content 仅描述已分配目标区域。
        let convert_status = unsafe {
            convert_fn(
                self.ops.ctx,
                frame.raw_desc(),
                std::ptr::null(),
                &content,
                0,
            )
        };
        if convert_status != AV_OK {
            self.free_view(&mut view);
            return Err(match AlgoError::from_c_status(convert_status) {
                Ok(()) => AlgoError::Preprocess {
                    reason: format!("宿主 convert 失败: {convert_status}"),
                },
                Err(error) => error,
            });
        }

        let buffer = CvBuffer::from_host_ops(
            view,
            self.ops.ctx,
            free_fn,
            dst_w,
            dst_h,
            PixelFormat::Rgb24,
        );
        Ok((buffer, PreprocessMode::Letterbox(layout)))
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
                reason: "目标尺寸不能为 0".to_string(),
            });
        }
        let convert_fn = self.ops.convert.ok_or(AlgoError::NotImplemented)?;
        let free_fn = self.ops.free.ok_or(AlgoError::NotImplemented)?;
        let mut view = self.allocate_view(dst_w, dst_h)?;

        // SAFETY: src_desc 在 process 回调内有效，view 由宿主 alloc 返回且保持有效。
        let convert_status =
            unsafe { convert_fn(self.ops.ctx, frame.raw_desc(), std::ptr::null(), &view, 0) };
        if convert_status != AV_OK {
            self.free_view(&mut view);
            return Err(match AlgoError::from_c_status(convert_status) {
                Ok(()) => AlgoError::Preprocess {
                    reason: format!("宿主 convert 失败: {convert_status}"),
                },
                Err(error) => error,
            });
        }

        let buffer = CvBuffer::from_host_ops(
            view,
            self.ops.ctx,
            free_fn,
            dst_w,
            dst_h,
            PixelFormat::Rgb24,
        );
        Ok((buffer, PreprocessMode::Resize))
    }
}
