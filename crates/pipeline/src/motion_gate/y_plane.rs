//! 帧载体适配：把各平台原生帧转换成差分判定所需的 Y 平面片段
//!
//! 与设计文档 §4「跨平台 Y 平面提取策略」一一对应：
//! - `FrameHandle::Host`：零拷贝切片借用 NV12 前 $W \times H$ 字节（带虚宽时先搬进 Scratchpad）；
//! - `FrameHandle::ApplePixelBuffer`：锁定后方 Unified Memory 直接读 Plane 0，零拷贝；
//! - `FrameHandle::DmaBuf`：RGA 设备侧降采样出常驻缩略图，只回读 320×180 的 Y 平面（57.6KB/帧，与源分辨率无关）；
//! - 尚未接入缩略图链路的载体（如昇腾 DeviceMemory）：保守放行，但计入绕过计数并按类首次告警。
//!
//! 平台 FFI（CoreVideo / RGA / DMA-BUF 栅障）只允许出现在本模块：平台差异收敛在载体边界内，
//! 上层差分判定只看一段连续 Y 平面切片。
//!
//! DMA-BUF 路径会调用 librga 与 DMA-BUF 可读栅障（单次可阻塞至百毫秒级），因此**必须**在
//! 每路一个的专用 OS 线程内执行（生产路径见 [`crate::motion_gate_worker`]），
//! 不得直接运行在 Tokio Worker 上。

use types::{FrameHandle, FrameRef, PixelFormat};

#[cfg(all(target_os = "linux", feature = "rga"))]
use media::motion_thumb::MotionThumbnailScaler;

use super::telemetry::BypassKind;
#[cfg(all(target_os = "linux", feature = "rga"))]
use super::thumbnail_target;
use super::{MotionGate, MotionGateDecision};

/// 缩略图链路连续失败熔断阈值
///
/// 单帧失败重试能容忍偶发总线竞争；但若连续失败，则说明该路源帧布局或链路本身不可用，
/// 继续逐帧重试只会白白消耗 RGA 与 CPU（并逐帧构造错误字符串），必须熔断为保守放行。
#[cfg(all(target_os = "linux", feature = "rga"))]
const THUMB_FAILURE_STREAK_LIMIT: u32 = 3;

#[cfg(target_os = "macos")]
#[link(name = "CoreVideo", kind = "framework")]
extern "C" {
    fn CVPixelBufferLockBaseAddress(pixel_buffer: *mut std::ffi::c_void, lock_flags: u64) -> i32;
    fn CVPixelBufferUnlockBaseAddress(pixel_buffer: *mut std::ffi::c_void, lock_flags: u64) -> i32;
    fn CVPixelBufferGetBaseAddressOfPlane(
        pixel_buffer: *mut std::ffi::c_void,
        plane_index: usize,
    ) -> *mut std::ffi::c_void;
    fn CVPixelBufferGetBytesPerRowOfPlane(
        pixel_buffer: *mut std::ffi::c_void,
        plane_index: usize,
    ) -> usize;
}

impl MotionGate {
    /// 针对 `FrameRef` 评估跳帧决策。
    ///
    /// 对于 Host 内存切片帧，零拷贝提取 Y 分量并执行差分；
    /// 对于 macOS ApplePixelBuffer，在 Unified Memory 下零拷贝直接读取 Y 平面；
    /// 对于物理设备帧：Rockchip DMA-BUF 经 RGA 硬件降采样出常驻缩略图后差分（只回读 Y 平面）；
    /// 尚未接入缩略图链路的载体（如昇腾 DeviceMemory）保守放行，
    /// **但计入 `bypassed_frames` 并首次告警，绝不静默失效**。
    pub fn evaluate_frame(&mut self, frame: &FrameRef, timestamp_ms: i64) -> MotionGateDecision {
        if !self.config.enabled {
            return MotionGateDecision {
                should_skip: false,
                motion_score: 0.0,
                is_keepalive: false,
            };
        }

        let width = frame.width as usize;
        let height = frame.height as usize;
        let Some(total_pixels) = width.checked_mul(height) else {
            return MotionGateDecision {
                should_skip: false,
                motion_score: 0.0,
                is_keepalive: false,
            };
        };
        if width == 0 || height == 0 {
            return MotionGateDecision {
                should_skip: false,
                motion_score: 0.0,
                is_keepalive: false,
            };
        }

        match frame.handle() {
            FrameHandle::Host(bytes) => {
                let hor_stride = frame.stride.hor_stride.max(frame.width) as usize;

                match frame.format {
                    PixelFormat::Nv12 | PixelFormat::Yuv420p => {
                        if hor_stride == width {
                            if bytes.len() >= total_pixels {
                                return self.evaluate(
                                    &bytes[..total_pixels],
                                    width,
                                    height,
                                    timestamp_ms,
                                );
                            }
                            return MotionGateDecision {
                                should_skip: false,
                                motion_score: 0.0,
                                is_keepalive: false,
                            };
                        }

                        // 带水平虚宽 (hor_stride > width) 提取连续可见区域至预分配 Scratchpad
                        if self.scratch_y.len() != total_pixels {
                            self.scratch_y.resize(total_pixels, 0);
                        }

                        for y in 0..height {
                            let src_start = y * hor_stride;
                            let src_end = src_start + width;
                            let dst_start = y * width;
                            let dst_end = dst_start + width;
                            if src_end > bytes.len() {
                                return MotionGateDecision {
                                    should_skip: false,
                                    motion_score: 0.0,
                                    is_keepalive: false,
                                };
                            }
                            self.scratch_y[dst_start..dst_end]
                                .copy_from_slice(&bytes[src_start..src_end]);
                        }

                        self.evaluate_scratch(width, height, timestamp_ms)
                    }
                    PixelFormat::Rgb24 | PixelFormat::Bgr24 => {
                        // 3 通道像素帧 (测试/Mock 环境)，快速加权提取灰度 Y ≈ (R + 2G + B) >> 2
                        let required_bytes = total_pixels.saturating_mul(3);
                        if bytes.len() < required_bytes {
                            return MotionGateDecision {
                                should_skip: false,
                                motion_score: 0.0,
                                is_keepalive: false,
                            };
                        }
                        if self.scratch_y.len() != total_pixels {
                            self.scratch_y.resize(total_pixels, 0);
                        }

                        for i in 0..total_pixels {
                            let b0 = bytes[i * 3] as u16;
                            let b1 = bytes[i * 3 + 1] as u16;
                            let b2 = bytes[i * 3 + 2] as u16;
                            self.scratch_y[i] = ((b0 + (b1 << 1) + b2) >> 2) as u8;
                        }

                        self.evaluate_scratch(width, height, timestamp_ms)
                    }
                    _ => MotionGateDecision {
                        should_skip: false,
                        motion_score: 0.0,
                        is_keepalive: false,
                    },
                }
            }
            #[cfg(target_os = "macos")]
            FrameHandle::ApplePixelBuffer { ptr } => {
                let raw_ptr = ptr.as_ptr();
                // SAFETY: 锁定 CVPixelBuffer 基础地址以只读方式访问 (1 = kCVPixelBufferLock_ReadOnly)
                let lock_status = unsafe { CVPixelBufferLockBaseAddress(raw_ptr, 1) };
                if lock_status != 0 {
                    return MotionGateDecision::passthrough();
                }

                struct BufferUnlockGuard(*mut std::ffi::c_void);
                impl Drop for BufferUnlockGuard {
                    fn drop(&mut self) {
                        // SAFETY: 解锁 CVPixelBuffer 基础地址
                        unsafe {
                            CVPixelBufferUnlockBaseAddress(self.0, 1);
                        }
                    }
                }
                let _unlock_guard = BufferUnlockGuard(raw_ptr);

                // SAFETY: 在锁定生命周期内读取 Y 平面（Plane 0）基址与行步长
                let (y_ptr, hor_stride) = unsafe {
                    let y = CVPixelBufferGetBaseAddressOfPlane(raw_ptr, 0) as *const u8;
                    let ys = CVPixelBufferGetBytesPerRowOfPlane(raw_ptr, 0);
                    (y, ys)
                };

                if y_ptr.is_null() || hor_stride < width {
                    return MotionGateDecision::passthrough();
                }

                let total_plane_bytes = hor_stride * height;
                // SAFETY: CVPixelBuffer 已锁定且 y_ptr 非空，内存区在 _unlock_guard 作用域内有效
                let plane_slice = unsafe { std::slice::from_raw_parts(y_ptr, total_plane_bytes) };

                if hor_stride == width {
                    return self.evaluate(
                        &plane_slice[..total_pixels],
                        width,
                        height,
                        timestamp_ms,
                    );
                }

                if self.scratch_y.len() != total_pixels {
                    self.scratch_y.resize(total_pixels, 0);
                }

                for y in 0..height {
                    let src_start = y * hor_stride;
                    let src_end = src_start + width;
                    let dst_start = y * width;
                    let dst_end = dst_start + width;
                    if src_end > plane_slice.len() {
                        return MotionGateDecision::passthrough();
                    }
                    self.scratch_y[dst_start..dst_end]
                        .copy_from_slice(&plane_slice[src_start..src_end]);
                }

                self.evaluate_scratch(width, height, timestamp_ms)
            }
            #[cfg(target_os = "linux")]
            FrameHandle::DmaBuf { .. } => self.evaluate_dma_buf(frame, timestamp_ms),
            _ => {
                // 昇腾 DeviceMemory 等载体尚未接入 VPC 缩略图链路：保守放行，但绝不静默
                self.telemetry
                    .note_bypass(BypassKind::CarrierUnsupported, timestamp_ms);
                MotionGateDecision::passthrough()
            }
        }
    }

    /// DMA-BUF 帧（Rockchip 硬解产物）门控评估：RGA 硬件降采样出常驻缩略图，再复用同一套差分判定。
    #[cfg(all(target_os = "linux", feature = "rga"))]
    fn evaluate_dma_buf(&mut self, frame: &FrameRef, timestamp_ms: i64) -> MotionGateDecision {
        // 已熔断（初始化失败或连续失败，如无 /dev/rga、dma-heap 分配失败）：
        // 直接放行，不再逐帧重试硬件、不再逐帧构造错误字符串
        if self.thumb_link_down {
            self.telemetry
                .note_bypass(BypassKind::ThumbnailDown, timestamp_ms);
            return MotionGateDecision::passthrough();
        }

        let Some((dst_w, dst_h)) = thumbnail_target(frame.width, frame.height) else {
            self.telemetry
                .note_bypass(BypassKind::SourceTooSmall, timestamp_ms);
            return MotionGateDecision::passthrough();
        };

        // 源分辨率切换（相机换码流档位）时重建缩略图，保证阈值口径始终一致
        if self.thumb_scaler.as_ref().map(MotionThumbnailScaler::size) != Some((dst_w, dst_h)) {
            match MotionThumbnailScaler::new(dst_w, dst_h) {
                Ok(scaler) => {
                    tracing::info!(
                        camera = %self.telemetry.camera_id(),
                        width = dst_w,
                        height = dst_h,
                        y_plane_bytes = (dst_w as usize) * (dst_h as usize),
                        "运动门控缩略图链路就绪 (RGA 硬件降采样)"
                    );
                    self.thumb_scaler = Some(scaler);
                    self.thumb_fail_streak = 0;
                }
                Err(error) => {
                    self.thumb_link_down = true;
                    self.thumb_scaler = None;
                    if self.telemetry.count_bypass(BypassKind::ThumbnailDown) {
                        tracing::error!(
                            camera = %self.telemetry.camera_id(),
                            error = %error,
                            width = dst_w,
                            height = dst_h,
                            "运动门控缩略图链路初始化失败，本路熔断为保守放行（不受门控约束）"
                        );
                    }
                    self.telemetry
                        .log_bypass(BypassKind::ThumbnailDown, timestamp_ms);
                    return MotionGateDecision::passthrough();
                }
            }
        }

        // 取出缩放器：`scale_y_plane` 返回的 Y 平面切片借用缩放器，故先 `take` 成局部变量，
        // 使 `self.evaluate` 的 `&mut self` 借用与切片借用互不冲突（与 `evaluate_scratch` 同构）。
        let Some(mut scaler) = self.thumb_scaler.take() else {
            // 仅在链路重建失败的帧上可达；重建路径已计入绕过计数
            return MotionGateDecision::passthrough();
        };

        // 错误仅在需要写日志时才物化（见 `note_bypass_with_error`），此处不转 String
        let outcome = match scaler.scale_y_plane(frame) {
            Ok(y_plane) => Ok(self.evaluate(y_plane, dst_w as usize, dst_h as usize, timestamp_ms)),
            Err(error) => Err(error),
        };
        self.thumb_scaler = Some(scaler);

        match outcome {
            Ok(decision) => {
                self.thumb_fail_streak = 0;
                decision
            }
            Err(error) => {
                self.thumb_fail_streak = self.thumb_fail_streak.saturating_add(1);
                if self.thumb_fail_streak >= THUMB_FAILURE_STREAK_LIMIT {
                    // 熔断：释放常驻缩略图资源（DMA-BUF 与 RGA 句柄），后续帧不再触碰硬件
                    self.thumb_link_down = true;
                    self.thumb_scaler = None;
                    if self.telemetry.count_bypass(BypassKind::ThumbnailDown) {
                        tracing::error!(
                            camera = %self.telemetry.camera_id(),
                            error = %error,
                            streak = self.thumb_fail_streak,
                            "运动门控缩略图连续降采样失败，本路熔断为保守放行（不再逐帧重试 RGA）"
                        );
                    }
                    self.telemetry
                        .log_bypass(BypassKind::ThumbnailDown, timestamp_ms);
                } else {
                    self.telemetry.note_bypass_with_error(
                        BypassKind::ThumbnailError,
                        timestamp_ms,
                        self.thumb_fail_streak,
                        &error,
                    );
                }
                MotionGateDecision::passthrough()
            }
        }
    }

    /// 未编译 RGA 缩略图链路时的退化路径：放行，但计入绕过计数并按类首次告警。
    #[cfg(all(target_os = "linux", not(feature = "rga")))]
    fn evaluate_dma_buf(&mut self, _frame: &FrameRef, timestamp_ms: i64) -> MotionGateDecision {
        self.telemetry
            .note_bypass(BypassKind::RgaDisabled, timestamp_ms);
        MotionGateDecision::passthrough()
    }

    /// 内部辅助：利用预分配的 `scratch_y` 执行差分评估，并在评估结束后将缓冲安全归还。
    fn evaluate_scratch(
        &mut self,
        width: usize,
        height: usize,
        timestamp_ms: i64,
    ) -> MotionGateDecision {
        let scratch = std::mem::take(&mut self.scratch_y);
        let decision = self.evaluate(&scratch, width, height, timestamp_ms);
        self.scratch_y = scratch;
        decision
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use types::{MotionGateConfig, StrideInfo};

    #[test]
    fn test_evaluate_frame_with_host_nv12() {
        let mut gate = MotionGate::new(MotionGateConfig::default());
        let width = 64;
        let height = 64;
        let yuv_bytes = vec![128u8; width * height * 3 / 2];

        let frame = FrameRef::new(
            "cam_01".into(),
            1000,
            width as u32,
            height as u32,
            StrideInfo::new(width as u32, height as u32),
            PixelFormat::Nv12,
            FrameHandle::Host(yuv_bytes.into()),
        );

        let d1 = gate.evaluate_frame(&frame, 1000);
        assert!(!d1.should_skip);
        assert!(d1.is_keepalive);

        let d2 = gate.evaluate_frame(&frame, 1040);
        assert!(d2.should_skip);
    }
    #[test]
    fn test_evaluate_frame_with_strided_nv12_ignores_padding() {
        let mut gate = MotionGate::new(MotionGateConfig {
            threshold: 25,
            contour_area: 16,
            motion_hold_frames: 0,
            ..Default::default()
        });
        let width = 8usize;
        let height = 8usize;
        let stride = 16usize;

        let mut first = vec![0u8; stride * height + stride * height / 2];
        for y in 0..height {
            first[y * stride..y * stride + width].fill(100);
        }
        let frame1 = FrameRef::new(
            "cam_stride".into(),
            1000,
            width as u32,
            height as u32,
            StrideInfo::new(stride as u32, height as u32),
            PixelFormat::Nv12,
            FrameHandle::Host(first.into()),
        );
        assert!(!gate.evaluate_frame(&frame1, 1000).should_skip);

        let mut padding_only = vec![0u8; stride * height + stride * height / 2];
        for y in 0..height {
            padding_only[y * stride..y * stride + width].fill(100);
            padding_only[y * stride + width..(y + 1) * stride].fill(255);
        }
        let frame2 = FrameRef::new(
            "cam_stride".into(),
            1040,
            width as u32,
            height as u32,
            StrideInfo::new(stride as u32, height as u32),
            PixelFormat::Nv12,
            FrameHandle::Host(padding_only.into()),
        );
        assert!(
            gate.evaluate_frame(&frame2, 1040).should_skip,
            "stride padding changes must not be treated as motion"
        );

        let mut visible_motion = vec![0u8; stride * height + stride * height / 2];
        for y in 0..height {
            visible_motion[y * stride..y * stride + 4].fill(200);
            visible_motion[y * stride + 4..(y + 1) * stride].fill(100);
        }
        let frame3 = FrameRef::new(
            "cam_stride".into(),
            1080,
            width as u32,
            height as u32,
            StrideInfo::new(stride as u32, height as u32),
            PixelFormat::Nv12,
            FrameHandle::Host(visible_motion.into()),
        );
        assert!(!gate.evaluate_frame(&frame3, 1080).should_skip);
    }
    #[test]
    fn test_unsupported_hardware_frame_is_counted_not_silent() {
        // 昇腾 DeviceMemory 等载体尚未接入缩略图链路：必须放行，但必须留下可观测计数
        let mut backing = vec![0u8; 64 * 64 * 3 / 2];
        let ptr = std::ptr::NonNull::new(backing.as_mut_ptr() as *mut std::ffi::c_void)
            .expect("测试缓冲非空");
        let frame = FrameRef::new(
            "cam_dev".into(),
            1000,
            64,
            64,
            StrideInfo::new(64, 64),
            PixelFormat::Nv12,
            FrameHandle::DeviceMemory {
                ptr,
                size: backing.len(),
                _lease: std::sync::Arc::new(()),
            },
        );

        let mut gate = MotionGate::new(MotionGateConfig::default());
        assert!(!gate.evaluate_frame(&frame, 1000).should_skip);
        assert!(!gate.evaluate_frame(&frame, 1040).should_skip);
        assert_eq!(
            gate.bypassed_frames(),
            2,
            "未接入缩略图链路的载体必须计入绕过计数，不允许静默放行"
        );
    }
    #[cfg(target_os = "macos")]
    #[test]
    fn test_evaluate_frame_with_apple_pixel_buffer() {
        extern "C" {
            fn CVPixelBufferCreate(
                allocator: *mut std::ffi::c_void,
                width: usize,
                height: usize,
                pixel_format_type: u32,
                pixel_buffer_attributes: *mut std::ffi::c_void,
                pixel_buffer_out: *mut *mut std::ffi::c_void,
            ) -> i32;
        }

        // kCVPixelFormatType_420YpCbCr8BiPlanarVideoRange = '420v' = 0x34323076
        const PIXEL_FORMAT_NV12: u32 = 0x34323076;

        let width = 64usize;
        let height = 64usize;
        let mut pixel_buffer: *mut std::ffi::c_void = std::ptr::null_mut();
        // SAFETY: 仅用于测试环境中分配一张标准的 64x64 NV12 CoreVideo 内存帧
        let status = unsafe {
            CVPixelBufferCreate(
                std::ptr::null_mut(),
                width,
                height,
                PIXEL_FORMAT_NV12,
                std::ptr::null_mut(),
                &mut pixel_buffer,
            )
        };
        assert_eq!(status, 0, "CVPixelBufferCreate 必须成功");

        let frame = FrameRef::new(
            "cam_mac".into(),
            1000,
            width as u32,
            height as u32,
            StrideInfo::new(width as u32, height as u32),
            PixelFormat::Nv12,
            FrameHandle::ApplePixelBuffer {
                ptr: std::ptr::NonNull::new(pixel_buffer)
                    .expect("CVPixelBuffer 创建成功后指针非空"),
            },
        );

        let mut gate = MotionGate::new(MotionGateConfig::default());
        let d1 = gate.evaluate_frame(&frame, 1000);
        assert!(!d1.should_skip);
        assert!(d1.is_keepalive);

        let d2 = gate.evaluate_frame(&frame, 1040);
        assert!(d2.should_skip, "静止 ApplePixelBuffer 帧应当被门控跳过");
    }
}
