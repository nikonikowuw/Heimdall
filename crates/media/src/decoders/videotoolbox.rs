//! macOS VideoToolbox 硬件加速视频解码器
//!
//! 基于 Apple VideoToolbox / CoreMedia 框架实现：
//! 1. 解析 H.264 / H.265 (HEVC) 序列头构建 CMVideoFormatDescription；
//! 2. 创建 VTDecompressionSession 并绑定 IOSurface 与 NV12 显存属性；
//! 3. 硬件解码输出原生 CVPixelBufferRef 并由 FrameRef / FrameHandle 的 RAII 自动管理显存生命周期。

#[cfg(target_os = "macos")]
use std::ffi::c_void;
#[cfg(target_os = "macos")]
use std::ptr::NonNull;

use async_trait::async_trait;
use types::{CodecType, FrameHandle, FrameRef, PixelFormat, StrideInfo};

use crate::decoder::VideoDecoder;
use crate::error::MediaError;

#[cfg(target_os = "macos")]
#[link(name = "VideoToolbox", kind = "framework")]
extern "C" {}

#[cfg(target_os = "macos")]
#[link(name = "CoreMedia", kind = "framework")]
extern "C" {}

#[cfg(target_os = "macos")]
#[link(name = "CoreVideo", kind = "framework")]
extern "C" {}

#[cfg(target_os = "macos")]
#[link(name = "CoreFoundation", kind = "framework")]
extern "C" {}

#[cfg(target_os = "macos")]
extern "C" {
    fn CMVideoFormatDescriptionCreateFromH264ParameterSets(
        allocator: *const c_void,
        parameter_set_count: usize,
        parameter_set_pointers: *const *const u8,
        parameter_set_sizes: *const usize,
        nal_unit_header_length: i32,
        format_description_out: *mut *mut c_void,
    ) -> i32;

    fn CMVideoFormatDescriptionCreateFromHEVCParameterSets(
        allocator: *const c_void,
        parameter_set_count: usize,
        parameter_set_pointers: *const *const u8,
        parameter_set_sizes: *const usize,
        nal_unit_header_length: i32,
        extensions: *const c_void,
        format_description_out: *mut *mut c_void,
    ) -> i32;

    fn CMBlockBufferCreateWithMemoryBlock(
        allocator: *const c_void,
        memory_block: *mut c_void,
        block_length: usize,
        block_allocator: *const c_void,
        custom_block_source: *const c_void,
        offset_to_data: usize,
        data_length: usize,
        flags: u32,
        block_buffer_out: *mut *mut c_void,
    ) -> i32;

    fn CMSampleBufferCreateReady(
        allocator: *const c_void,
        data_buffer: *mut c_void,
        format_description: *mut c_void,
        num_samples: isize,
        num_sample_timing_entries: isize,
        sample_timing_array: *const c_void,
        num_sample_size_entries: isize,
        sample_size_array: *const usize,
        sample_buffer_out: *mut *mut c_void,
    ) -> i32;

    fn VTDecompressionSessionCreate(
        allocator: *const c_void,
        video_format_description: *mut c_void,
        video_decoder_specification: *const c_void,
        destination_image_buffer_attributes: *const c_void,
        output_callback_record: *const VTDecompressionOutputCallbackRecord,
        decompression_session_out: *mut *mut c_void,
    ) -> i32;

    fn VTDecompressionSessionDecodeFrame(
        session: *mut c_void,
        sample_buffer: *mut c_void,
        decode_flags: u32,
        source_frame_ref_con: *mut c_void,
        info_flags_out: *mut u32,
    ) -> i32;

    fn VTDecompressionSessionWaitForAsynchronousFrames(session: *mut c_void) -> i32;
    fn VTDecompressionSessionInvalidate(session: *mut c_void);

    fn CFRelease(cf: *const c_void);

    fn CFDictionaryCreate(
        allocator: *const c_void,
        keys: *const *const c_void,
        values: *const *const c_void,
        num_values: isize,
        key_callbacks: *const c_void,
        value_callbacks: *const c_void,
    ) -> *mut c_void;

    fn CFNumberCreate(
        allocator: *const c_void,
        the_type: isize,
        value_ptr: *const c_void,
    ) -> *mut c_void;

    static kCFTypeDictionaryKeyCallBacks: c_void;
    static kCFTypeDictionaryValueCallBacks: c_void;

    static kCVPixelBufferPixelFormatTypeKey: *const c_void;
    static kCVPixelBufferIOSurfacePropertiesKey: *const c_void;

    fn CVPixelBufferGetWidth(pixel_buffer: *mut c_void) -> usize;
    fn CVPixelBufferGetHeight(pixel_buffer: *mut c_void) -> usize;
    fn CVPixelBufferGetBytesPerRow(pixel_buffer: *mut c_void) -> usize;
    fn CVPixelBufferRetain(pixel_buffer: *mut c_void) -> *mut c_void;
}

#[cfg(target_os = "macos")]
#[repr(C)]
#[derive(Debug, Copy, Clone)]
pub struct CMTime {
    pub value: i64,
    pub timescale: i32,
    pub flags: u32,
    pub epoch: i64,
}

#[cfg(target_os = "macos")]
#[repr(C)]
struct VTDecompressionOutputCallbackRecord {
    decompression_output_callback: Option<
        unsafe extern "C" fn(
            decompression_output_ref_con: *mut c_void,
            source_frame_ref_con: *mut c_void,
            status: i32,
            info_flags: u32,
            image_buffer: *mut c_void,
            presentation_time_stamp: CMTime,
            presentation_duration: CMTime,
        ),
    >,
    decompression_output_ref_con: *mut c_void,
}

/// macOS VideoToolbox 硬件解码器
#[derive(Debug)]
pub struct VideoToolboxDecoder {
    camera_id: String,
    codec: CodecType,
    #[cfg(target_os = "macos")]
    session: *mut c_void,
    #[cfg(target_os = "macos")]
    format_desc: *mut c_void,
    sps: Option<Vec<u8>>,
    pps: Option<Vec<u8>>,
    vps: Option<Vec<u8>>,
}

// SAFETY: VideoToolbox 解码会话由 Mutex 或独占所有者调度，满足 Send 要求
unsafe impl Send for VideoToolboxDecoder {}

impl VideoToolboxDecoder {
    pub fn new(camera_id: impl Into<String>, codec: CodecType) -> Self {
        Self {
            camera_id: camera_id.into(),
            codec,
            #[cfg(target_os = "macos")]
            session: std::ptr::null_mut(),
            #[cfg(target_os = "macos")]
            format_desc: std::ptr::null_mut(),
            sps: None,
            pps: None,
            vps: None,
        }
    }

    #[cfg(target_os = "macos")]
    fn build_destination_attributes() -> *mut c_void {
        // 绑定 NV12 格式 (kCVPixelFormatType_420YpCbCr8BiPlanarVideoRange = 0x34323076 / '420v')
        let pixel_format: u32 = 0x34323076;
        // SAFETY: 创建 CoreFoundation 32 位整数格式枚举常量
        let num_format = unsafe {
            CFNumberCreate(
                std::ptr::null(),
                3, // kCFNumberSInt32Type = 3
                &pixel_format as *const u32 as *const c_void,
            )
        };

        // SAFETY: 创建空的 CoreFoundation 字典作为 IOSurface 支撑配置
        let io_surface_dict = unsafe {
            CFDictionaryCreate(
                std::ptr::null(),
                std::ptr::null(),
                std::ptr::null(),
                0,
                &kCFTypeDictionaryKeyCallBacks,
                &kCFTypeDictionaryValueCallBacks,
            )
        };

        // SAFETY: 读取 CoreVideo 导出的标准属性键常量
        let keys = unsafe {
            [
                kCVPixelBufferPixelFormatTypeKey,
                kCVPixelBufferIOSurfacePropertiesKey,
            ]
        };
        let values = [
            num_format as *const c_void,
            io_surface_dict as *const c_void,
        ];

        // SAFETY: 构建输出图像属性字典
        let attr_dict = unsafe {
            CFDictionaryCreate(
                std::ptr::null(),
                keys.as_ptr(),
                values.as_ptr(),
                2,
                &kCFTypeDictionaryKeyCallBacks,
                &kCFTypeDictionaryValueCallBacks,
            )
        };

        // SAFETY: 释放临时 CFNumber 与 CFDictionary 对象的局部所有权
        unsafe {
            CFRelease(num_format);
            CFRelease(io_surface_dict);
        }

        attr_dict
    }

    #[cfg(target_os = "macos")]
    fn init_session_if_ready(&mut self) -> Result<bool, MediaError> {
        if !self.session.is_null() {
            return Ok(true);
        }

        let sps = match &self.sps {
            Some(s) => s,
            None => return Ok(false),
        };
        let pps = match &self.pps {
            Some(p) => p,
            None => return Ok(false),
        };

        let mut format_desc: *mut c_void = std::ptr::null_mut();

        match self.codec {
            CodecType::H264 => {
                let pointers = [sps.as_ptr(), pps.as_ptr()];
                let sizes = [sps.len(), pps.len()];

                // SAFETY: pointers 和 sizes 指向有效的 SPS/PPS 内存
                let status = unsafe {
                    CMVideoFormatDescriptionCreateFromH264ParameterSets(
                        std::ptr::null(),
                        2,
                        pointers.as_ptr(),
                        sizes.as_ptr(),
                        4,
                        &mut format_desc,
                    )
                };

                if status != 0 || format_desc.is_null() {
                    return Err(MediaError::DecoderInit {
                        codec: "H264".to_string(),
                        reason: format!("CMVideoFormatDescription H264 失败, code: {status}"),
                    });
                }
            }
            CodecType::H265 => {
                let vps = match &self.vps {
                    Some(v) => v,
                    None => return Ok(false),
                };

                let pointers = [vps.as_ptr(), sps.as_ptr(), pps.as_ptr()];
                let sizes = [vps.len(), sps.len(), pps.len()];

                // SAFETY: pointers 和 sizes 指向有效的 VPS/SPS/PPS 内存
                let status = unsafe {
                    CMVideoFormatDescriptionCreateFromHEVCParameterSets(
                        std::ptr::null(),
                        3,
                        pointers.as_ptr(),
                        sizes.as_ptr(),
                        4,
                        std::ptr::null(),
                        &mut format_desc,
                    )
                };

                if status != 0 || format_desc.is_null() {
                    return Err(MediaError::DecoderInit {
                        codec: "H265".to_string(),
                        reason: format!("CMVideoFormatDescription HEVC 失败, code: {status}"),
                    });
                }
            }
        }

        let cb_record = VTDecompressionOutputCallbackRecord {
            decompression_output_callback: Some(decompression_callback),
            decompression_output_ref_con: std::ptr::null_mut(),
        };

        let destination_attributes = Self::build_destination_attributes();
        let mut session: *mut c_void = std::ptr::null_mut();

        // SAFETY: 创建 VideoToolbox 解码会话并绑定 IOSurface 输出
        let session_status = unsafe {
            let res = VTDecompressionSessionCreate(
                std::ptr::null(),
                format_desc,
                std::ptr::null(),
                destination_attributes,
                &cb_record,
                &mut session,
            );
            if !destination_attributes.is_null() {
                CFRelease(destination_attributes);
            }
            res
        };

        if session_status != 0 || session.is_null() {
            // SAFETY: 会话创建失败时释放 format_desc
            unsafe { CFRelease(format_desc) };
            return Err(MediaError::DecoderInit {
                codec: format!("{:?}", self.codec),
                reason: format!("VTDecompressionSessionCreate 失败, code: {session_status}"),
            });
        }

        self.format_desc = format_desc;
        self.session = session;
        tracing::info!(
            camera_id = %self.camera_id,
            codec = ?self.codec,
            "macOS VideoToolbox 硬件解码会话已成功建立 (已绑定 NV12 与 IOSurface)"
        );

        Ok(true)
    }

    fn is_parameter_set(&self, nalu: &[u8]) -> bool {
        if nalu.is_empty() {
            return false;
        }
        match self.codec {
            CodecType::H264 => {
                let t = nalu[0] & 0x1F;
                t == 7 || t == 8
            }
            CodecType::H265 => {
                let t = (nalu[0] >> 1) & 0x3F;
                t == 32 || t == 33 || t == 34
            }
        }
    }

    #[cfg(target_os = "macos")]
    fn reset_session(&mut self) {
        if !self.session.is_null() {
            // SAFETY: 销毁并释放 VideoToolbox 会话
            unsafe {
                VTDecompressionSessionInvalidate(self.session);
                CFRelease(self.session);
            }
            self.session = std::ptr::null_mut();
        }
        if !self.format_desc.is_null() {
            // SAFETY: 释放格式描述符
            unsafe {
                CFRelease(self.format_desc);
            }
            self.format_desc = std::ptr::null_mut();
        }
    }

    /// 遍历提取输入切片中的参数集 NALU（SPS/PPS/VPS）
    fn update_parameter_sets(&mut self, nalus: &[&[u8]]) {
        for nalu in nalus {
            if nalu.is_empty() {
                continue;
            }

            match self.codec {
                CodecType::H264 => match nalu[0] & 0x1F {
                    7 if self.sps.as_deref() != Some(nalu) => {
                        self.sps = Some(nalu.to_vec());
                        #[cfg(target_os = "macos")]
                        self.reset_session();
                    }
                    8 if self.pps.as_deref() != Some(nalu) => {
                        self.pps = Some(nalu.to_vec());
                        #[cfg(target_os = "macos")]
                        self.reset_session();
                    }
                    _ => {}
                },
                CodecType::H265 => match (nalu[0] >> 1) & 0x3F {
                    32 if self.vps.as_deref() != Some(nalu) => {
                        self.vps = Some(nalu.to_vec());
                        #[cfg(target_os = "macos")]
                        self.reset_session();
                    }
                    33 if self.sps.as_deref() != Some(nalu) => {
                        self.sps = Some(nalu.to_vec());
                        #[cfg(target_os = "macos")]
                        self.reset_session();
                    }
                    34 if self.pps.as_deref() != Some(nalu) => {
                        self.pps = Some(nalu.to_vec());
                        #[cfg(target_os = "macos")]
                        self.reset_session();
                    }
                    _ => {}
                },
            }
        }
    }
}

#[async_trait]
impl VideoDecoder for VideoToolboxDecoder {
    async fn decode_packet(
        &mut self,
        packet: &[u8],
        pts: i64,
    ) -> Result<Option<FrameRef>, MediaError> {
        #[cfg(not(target_os = "macos"))]
        {
            let _ = (packet, pts);
            return Err(MediaError::UnsupportedCodec(
                "VideoToolbox 仅在 macOS 平台受支持".to_string(),
            ));
        }

        #[cfg(target_os = "macos")]
        {
            // 1. 拆分 NALU 并更新 SPS/PPS/VPS 参数集
            let nalus = split_annex_b_nalus(packet);
            self.update_parameter_sets(&nalus);

            // 2. 检查会话就绪状态
            if !self.init_session_if_ready()? {
                // SPS/PPS/VPS 尚未凑齐，等待关键帧
                return Ok(None);
            }

            // 3. 将 Annex B 格式转换为 AVCC 格式 (提取所有非参数集切片，前置 4 字节大端长度)
            let mut avcc_buffer = Vec::with_capacity(packet.len() + 16);

            for nalu in nalus {
                if !self.is_parameter_set(nalu) {
                    let nalu_len = (nalu.len() as u32).to_be_bytes();
                    avcc_buffer.extend_from_slice(&nalu_len);
                    avcc_buffer.extend_from_slice(nalu);
                }
            }

            if avcc_buffer.is_empty() {
                // 该包纯粹是参数集头，无图像切片
                return Ok(None);
            }

            let mut block_buffer: *mut c_void = std::ptr::null_mut();
            const K_CM_BLOCK_BUFFER_ALWAYS_COPY_DATA_FLAG: u32 = 0x00000020;

            // SAFETY: 创建 CMBlockBuffer，传入 kCMBlockBufferAlwaysCopyDataFlag = 0x00000020
            // 确保 CoreMedia 立即分配内部内存并复制切片，彻底消除栈变量生命周期悬垂与 UAF 风险
            let block_status = unsafe {
                CMBlockBufferCreateWithMemoryBlock(
                    std::ptr::null(),
                    avcc_buffer.as_ptr() as *mut c_void,
                    avcc_buffer.len(),
                    std::ptr::null(),
                    std::ptr::null(),
                    0,
                    avcc_buffer.len(),
                    K_CM_BLOCK_BUFFER_ALWAYS_COPY_DATA_FLAG,
                    &mut block_buffer,
                )
            };

            if block_status != 0 || block_buffer.is_null() {
                return Err(MediaError::Decode {
                    reason: format!("CMBlockBufferCreate 失败, code: {block_status}"),
                });
            }

            let mut sample_buffer: *mut c_void = std::ptr::null_mut();
            let sample_size = avcc_buffer.len();

            // SAFETY: 构建 CMSampleBuffer
            let sample_status = unsafe {
                CMSampleBufferCreateReady(
                    std::ptr::null(),
                    block_buffer,
                    self.format_desc,
                    1,
                    0,
                    std::ptr::null(),
                    1,
                    &sample_size,
                    &mut sample_buffer,
                )
            };

            // SAFETY: sample_buffer 内部已保留 block_buffer 引用，释放外部局部引用
            unsafe { CFRelease(block_buffer) };

            if sample_status != 0 || sample_buffer.is_null() {
                return Err(MediaError::Decode {
                    reason: format!("CMSampleBufferCreate 失败, code: {sample_status}"),
                });
            }

            // 4. 调用 VideoToolbox 同步解码
            let mut decoded_buffer: Option<*mut c_void> = None;
            let context_ptr = &mut decoded_buffer as *mut Option<*mut c_void> as *mut c_void;

            let mut info_flags: u32 = 0;
            // SAFETY: 调用 VideoToolbox 同步解码当前 SampleBuffer
            let decode_status = unsafe {
                VTDecompressionSessionDecodeFrame(
                    self.session,
                    sample_buffer,
                    0,
                    context_ptr,
                    &mut info_flags,
                )
            };

            // SAFETY: 释放 sample_buffer
            unsafe { CFRelease(sample_buffer) };

            if decode_status != 0 {
                return Err(MediaError::Decode {
                    reason: format!(
                        "VTDecompressionSessionDecodeFrame 失败, code: {decode_status}"
                    ),
                });
            }

            // SAFETY: 确保异步等待队列处理完毕，杜绝回调悬垂
            unsafe {
                VTDecompressionSessionWaitForAsynchronousFrames(self.session);
            }

            // 5. 检查回调输出的 CVPixelBufferRef
            if let Some(pixel_buffer) = decoded_buffer {
                // SAFETY: 从 pixel_buffer 查询图像真实尺寸与行步长
                let (width, height, bytes_per_row) = unsafe {
                    (
                        CVPixelBufferGetWidth(pixel_buffer) as u32,
                        CVPixelBufferGetHeight(pixel_buffer) as u32,
                        CVPixelBufferGetBytesPerRow(pixel_buffer) as u32,
                    )
                };

                // 包装为零拷贝 FrameRef (其 Drop 会自动调用 CVPixelBufferRelease 归还显存)
                let non_null = match NonNull::new(pixel_buffer) {
                    Some(p) => p,
                    None => return Ok(None),
                };

                let frame_handle = FrameHandle::ApplePixelBuffer { ptr: non_null };
                let frame_ref = FrameRef::new(
                    self.camera_id.clone(),
                    pts,
                    width,
                    height,
                    StrideInfo::new(bytes_per_row, height),
                    PixelFormat::Nv12,
                    frame_handle,
                );

                return Ok(Some(frame_ref));
            }

            Ok(None)
        }
    }

    async fn flush(&mut self) -> Result<Vec<FrameRef>, MediaError> {
        #[cfg(target_os = "macos")]
        {
            if !self.session.is_null() {
                // SAFETY: 等待异步帧清空
                unsafe {
                    VTDecompressionSessionWaitForAsynchronousFrames(self.session);
                }
            }
        }
        Ok(Vec::new())
    }
}

impl Drop for VideoToolboxDecoder {
    fn drop(&mut self) {
        #[cfg(target_os = "macos")]
        {
            self.reset_session();
            tracing::info!(
                camera_id = %self.camera_id,
                "macOS VideoToolbox 硬件解码会话已安全释放"
            );
        }
    }
}

#[cfg(target_os = "macos")]
unsafe extern "C" fn decompression_callback(
    _ref_con: *mut c_void,
    source_frame_ref_con: *mut c_void,
    status: i32,
    _info_flags: u32,
    image_buffer: *mut c_void,
    _pts: CMTime,
    _duration: CMTime,
) {
    let _ = std::panic::catch_unwind(|| {
        if status == 0 && !image_buffer.is_null() && !source_frame_ref_con.is_null() {
            // SAFETY: 保留 CVPixelBufferRef 引用计数
            let retained = unsafe { CVPixelBufferRetain(image_buffer) };
            // SAFETY: source_frame_ref_con 是 &mut Option<*mut c_void> 指针
            let target = unsafe { &mut *(source_frame_ref_con as *mut Option<*mut c_void>) };
            *target = Some(retained);
        }
    });
}

/// 将字节切片中的 Annex B NALU 单元拆分
pub fn split_annex_b_nalus(data: &[u8]) -> Vec<&[u8]> {
    let len = data.len();
    if len < 3 {
        return if data.is_empty() {
            Vec::new()
        } else {
            vec![data]
        };
    }

    let mut start_codes = Vec::new();
    let mut i = 0;
    while i < len - 2 {
        if data[i] == 0 && data[i + 1] == 0 {
            if i + 3 < len && data[i + 2] == 0 && data[i + 3] == 1 {
                start_codes.push((i, i + 4));
                i += 4;
                continue;
            } else if data[i + 2] == 1 {
                start_codes.push((i, i + 3));
                i += 3;
                continue;
            }
        }
        i += 1;
    }

    if start_codes.is_empty() {
        return vec![data];
    }

    let mut nalus = Vec::with_capacity(start_codes.len());
    for (idx, &(_, payload_start)) in start_codes.iter().enumerate() {
        let payload_end = if idx + 1 < start_codes.len() {
            start_codes[idx + 1].0
        } else {
            len
        };
        if payload_start < payload_end {
            nalus.push(&data[payload_start..payload_end]);
        }
    }

    nalus
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_split_annex_b_nalus() {
        let multi_nalu_data =
            b"\x00\x00\x00\x01\x67sps_data\x00\x00\x01\x68pps_data\x00\x00\x00\x01\x65idr_data";
        let nalus = split_annex_b_nalus(multi_nalu_data);

        assert_eq!(nalus.len(), 3);
        assert_eq!(nalus[0], b"\x67sps_data");
        assert_eq!(nalus[1], b"\x68pps_data");
        assert_eq!(nalus[2], b"\x65idr_data");
    }
}
