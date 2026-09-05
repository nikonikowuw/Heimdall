pub mod mock;
#[cfg(target_os = "macos")]
pub mod videotoolbox;

pub use mock::MockDecoder;
#[cfg(target_os = "macos")]
pub use videotoolbox::VideoToolboxDecoder;

use crate::decoder::VideoDecoder;
use types::CodecType;

/// 构造当前平台的默认硬件/模拟视频解码器
pub fn create_decoder(camera_id: &str, codec: CodecType) -> Box<dyn VideoDecoder + Send> {
    #[cfg(target_os = "macos")]
    {
        Box::new(VideoToolboxDecoder::new(camera_id, codec))
    }
    #[cfg(not(target_os = "macos"))]
    {
        Box::new(MockDecoder::new(camera_id, codec, 1920, 1080))
    }
}
