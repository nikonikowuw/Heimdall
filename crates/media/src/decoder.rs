use async_trait::async_trait;
use types::FrameRef;

use crate::error::MediaError;

/// 视频硬解器抽象接口
#[async_trait]
pub trait VideoDecoder: Send + 'static {
    /// 输入压缩的视频 NALU 或 Packet，解码输出零拷贝 FrameRef 帧
    async fn decode_packet(
        &mut self,
        packet: &[u8],
        pts: i64,
    ) -> Result<Option<FrameRef>, MediaError>;

    /// 刷新解码器内部残留缓冲帧
    async fn flush(&mut self) -> Result<Vec<FrameRef>, MediaError>;
}
