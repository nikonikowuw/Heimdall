use algo_sdk::c_abi::*;
use algo_sdk::cv::{self, CvBuffer, PixelFormat, PreprocessMode};
use algo_sdk::frame::SafeFrame;
#[cfg(target_os = "macos")]
unsafe fn read_cvpixelbuffer_bgra(
    ptr: *mut std::ffi::c_void,
    width: usize,
    height: usize,
) -> Vec<u8> {
    #[link(name = "CoreVideo", kind = "framework")]
    unsafe extern "C" {
        fn CVPixelBufferLockBaseAddress(
            pixel_buffer: *mut std::ffi::c_void,
            lock_flags: u64,
        ) -> std::ffi::c_int;
        fn CVPixelBufferUnlockBaseAddress(
            pixel_buffer: *mut std::ffi::c_void,
            unlock_flags: u64,
        ) -> std::ffi::c_int;
        fn CVPixelBufferGetBaseAddress(
            pixel_buffer: *mut std::ffi::c_void,
        ) -> *mut std::ffi::c_void;
        fn CVPixelBufferGetBytesPerRow(pixel_buffer: *mut std::ffi::c_void) -> usize;
    }

    // SAFETY: 测试调用方传入的是当前仍由 CvBuffer 持有的有效 BGRA surface。
    unsafe {
        assert_eq!(CVPixelBufferLockBaseAddress(ptr, 1), 0);
        let base = CVPixelBufferGetBaseAddress(ptr) as *const u8;
        let stride = CVPixelBufferGetBytesPerRow(ptr);
        assert!(!base.is_null());
        assert!(stride >= width * 4);
        let mut result = vec![0u8; width * height * 4];
        for y in 0..height {
            std::ptr::copy_nonoverlapping(
                base.add(y * stride),
                result.as_mut_ptr().add(y * width * 4),
                width * 4,
            );
        }
        assert_eq!(CVPixelBufferUnlockBaseAddress(ptr, 1), 0);
        result
    }
}

#[test]
fn test_facade_letterbox_and_resize() {
    let w = 640u32;
    let h = 480u32;
    let mut data = vec![0u8; (w * h * 3) as usize];
    // 给四角赋特殊颜色以验证插值与黑边
    for (i, byte) in data.iter_mut().enumerate() {
        *byte = (i % 255) as u8;
    }

    let mut desc = AvFrameDesc::default_nv12(w, h, (w * 3) as i32, 0, 0);
    desc.pixel_format = AV_PIX_RGB24;
    desc.opaque = data.as_ptr() as *mut std::ffi::c_void;
    desc.opaque_kind = AV_OPAQUE_NONE;

    let frame = SafeFrame::from_ref(&desc).expect("帧描述符有效");

    // 1. 门面函数 letterbox: 640x480 -> 640x640
    let (buf, mode) = cv::letterbox(&frame, 640, 640, [114, 114, 114]).expect("letterbox 失败");
    assert_eq!(buf.width(), 640);
    assert_eq!(buf.height(), 640);
    assert_eq!(buf.format(), PixelFormat::Rgb24);
    match mode {
        PreprocessMode::Letterbox(layout) => {
            assert_eq!(layout.dst_w, 640);
            assert_eq!(layout.dst_h, 640);
            assert_eq!(layout.scaled_w, 640);
            assert_eq!(layout.scaled_h, 480);
            assert_eq!(layout.pad_left, 0);
            assert_eq!(layout.pad_top, 80); // (640 - 480) / 2
        }
        _ => panic!("预期 Letterbox 模式"),
    }

    let bytes = buf.as_host_bytes().expect("应有 host bytes");
    // 验证顶部黑边 (0, 0)
    assert_eq!(bytes[0], 114);
    assert_eq!(bytes[1], 114);
    assert_eq!(bytes[2], 114);

    // 2. 门面函数 resize: 640x480 -> 320x320
    let (buf_resize, mode_resize) = cv::resize(&frame, 320, 320).expect("resize 失败");
    assert_eq!(buf_resize.width(), 320);
    assert_eq!(buf_resize.height(), 320);
    assert_eq!(mode_resize, PreprocessMode::Resize);
    assert_eq!(
        buf_resize.as_host_bytes().expect("应存在 host bytes").len(),
        320 * 320 * 3
    );
}

#[test]
fn test_cv_buffer_ownership_transfer() {
    let data = vec![10u8; 100];
    let buf = CvBuffer::from_host(data, 10, 10, PixelFormat::Rgb24);

    // 验证 Send 特性：跨线程传递所有权
    let handle = std::thread::spawn(move || {
        assert_eq!(buf.width(), 10);
        assert_eq!(buf.as_host_bytes().expect("应存在 host bytes")[0], 10);
    });

    handle.join().expect("线程正常结束");
}

#[cfg(target_os = "macos")]
#[test]
fn test_apple_hardware_cv_engine() {
    use algo_sdk::cv::platforms::apple::AppleCvEngine;
    use algo_sdk::cv::CvEngine;
    use algo_sdk::testing::MockFrameBuilder;

    // 1. 生成 64 字节硬件步长对齐的 NV12，并挂载真实 CVPixelBuffer 硬件显存句柄
    let w = 320u32;
    let h = 240u32;
    let mut rgb = vec![0u8; (w * h * 3) as usize];
    for (i, p) in rgb.iter_mut().enumerate() {
        *p = ((i * 7) % 256) as u8;
    }

    let mock_frame = MockFrameBuilder::new()
        .dimensions(w, h)
        .host_data(rgb)
        .to_nv12(64)
        .opaque_kind(AV_OPAQUE_CVPIXELBUFFER)
        .build();
    let safe_frame = mock_frame.as_safe_frame();

    let engine = AppleCvEngine::new();

    // 2. 硬件加速 Letterbox: 320x240 -> 640x640 (Accelerate SIMD + CVPixelBuffer)
    let (buf, mode) = engine
        .letterbox(&safe_frame, 640, 640, [128, 128, 128])
        .expect("AppleCvEngine 硬件 letterbox 失败");

    assert_eq!(buf.width(), 640);
    assert_eq!(buf.height(), 640);
    assert_eq!(buf.format(), PixelFormat::Bgra);

    if let PreprocessMode::Letterbox(layout) = mode {
        assert_eq!(layout.scaled_w, 640);
        assert_eq!(layout.scaled_h, 480);
        assert_eq!(layout.pad_top, 80);
    } else {
        panic!("预期 PreprocessMode::Letterbox");
    }

    // SAFETY: buf 持有本测试创建的 CVPixelBuffer，读取函数只在当前借用期间锁定并读取它。
    let bytes = unsafe {
        read_cvpixelbuffer_bgra(buf.as_raw_ptr().expect("CVPixelBuffer 输出"), 640, 640)
    };
    assert_eq!(bytes.len(), 640 * 640 * 4);
    // 验证黑边
    assert_eq!(bytes[0], 128);
    assert_eq!(bytes[1], 128);
    assert_eq!(bytes[2], 128);

    // 3. 硬件加速 Resize: 320x240 -> 224x224 (Accelerate SIMD)
    let (buf_resize, _mode_resize) = engine
        .resize(&safe_frame, 224, 224)
        .expect("AppleCvEngine 硬件 resize 失败");

    assert_eq!(buf_resize.format(), PixelFormat::Bgra);
    // SAFETY: buf_resize 持有本测试创建的 CVPixelBuffer，读取函数只在当前借用期间锁定并读取它。
    let resize_bytes = unsafe {
        read_cvpixelbuffer_bgra(
            buf_resize.as_raw_ptr().expect("CVPixelBuffer 输出"),
            224,
            224,
        )
    };
    assert_eq!(resize_bytes.len(), 224 * 224 * 4);
}
