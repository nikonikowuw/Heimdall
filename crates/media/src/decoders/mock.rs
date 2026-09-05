//! 模拟/回退视频解码器
//! 用于单元测试和跨平台非硬件环境的管线逻辑验证。

use async_trait::async_trait;
use types::{CodecType, FrameHandle, FrameRef, PixelFormat, StrideInfo};

use crate::decoder::{DecodeDeliveryPolicy, VideoDecoder};
use crate::error::MediaError;

/// 模拟视频解码器
#[derive(Debug)]
pub struct MockDecoder {
    camera_id: String,
    _codec: CodecType,
    width: u32,
    height: u32,
    decoded_count: usize,
    policy: DecodeDeliveryPolicy,
}

impl MockDecoder {
    pub fn new(camera_id: impl Into<String>, codec: CodecType, width: u32, height: u32) -> Self {
        Self {
            camera_id: camera_id.into(),
            _codec: codec,
            width,
            height,
            decoded_count: 0,
            policy: DecodeDeliveryPolicy::default(),
        }
    }
}

#[async_trait]
impl VideoDecoder for MockDecoder {
    async fn decode_packet(
        &mut self,
        _packet: &[u8],
        pts: i64,
    ) -> Result<Option<FrameRef>, MediaError> {
        self.decoded_count += 1;

        // 生成测试用的 Host 内存帧
        let dummy_size = (self.width * self.height * 3 / 2) as usize; // NV12
        let dummy_data = vec![128u8; dummy_size].into();

        let frame = FrameRef::new(
            self.camera_id.clone(),
            pts,
            self.width,
            self.height,
            StrideInfo::new(self.width, self.height),
            PixelFormat::Nv12,
            FrameHandle::Host(dummy_data),
        );

        Ok(Some(frame))
    }

    async fn flush(&mut self) -> Result<Vec<FrameRef>, MediaError> {
        Ok(Vec::new())
    }

    fn set_delivery_policy(&mut self, policy: DecodeDeliveryPolicy) {
        self.policy = policy;
    }

    fn delivery_policy(&self) -> DecodeDeliveryPolicy {
        self.policy
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_mock_decoder_delivery_policy_switch() {
        let mut decoder = MockDecoder::new("cam1", CodecType::H264, 1920, 1080);
        assert_eq!(
            decoder.delivery_policy(),
            DecodeDeliveryPolicy::LosslessBackpressure
        );

        decoder.set_delivery_policy(DecodeDeliveryPolicy::RealtimeDropOldest);
        assert_eq!(
            decoder.delivery_policy(),
            DecodeDeliveryPolicy::RealtimeDropOldest
        );

        let frame = decoder.decode_packet(b"fake", 1000).await.unwrap();
        assert!(frame.is_some());
    }
}
