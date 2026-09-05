use async_trait::async_trait;
use bytes::Bytes;
use tokio::sync::oneshot;
use types::FrameRef;

use crate::error::MediaError;

use std::time::Duration;

/// 工作线程优雅关停超时上限（500ms，超时后强制解离防止拖死守护进程退出）
pub const DEFAULT_THREAD_SHUTDOWN_TIMEOUT: Duration = Duration::from_millis(500);

/// 解码线程收到的异步控制命令
#[allow(dead_code)]
#[derive(Debug)]
pub(crate) enum DecodeCommand {
    /// 送入一个压缩包并提取解码帧
    Decode {
        packet: Bytes,
        pts: i64,
        reply: oneshot::Sender<Result<Option<FrameRef>, MediaError>>,
    },
    /// 刷新解码器内部残留缓冲帧
    Flush {
        reply: oneshot::Sender<Result<Vec<FrameRef>, MediaError>>,
    },
    /// 显式通知工作线程优雅停止并退出事件循环
    Stop,
}

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
